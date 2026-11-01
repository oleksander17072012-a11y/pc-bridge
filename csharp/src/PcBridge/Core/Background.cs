// Input that never touches the shared input queue: posted messages go straight to a window,
// so the real cursor stays where the user left it and two people can type at once.
using System.Runtime.InteropServices;

namespace PcBridge.Core;

public static class Background
{
    private const int WM_MOUSEMOVE = 0x0200;
    private const int WM_LBUTTONDOWN = 0x0201, WM_LBUTTONUP = 0x0202;
    private const int WM_RBUTTONDOWN = 0x0204, WM_RBUTTONUP = 0x0205;
    private const int WM_KEYDOWN = 0x0100, WM_KEYUP = 0x0101, WM_CHAR = 0x0102;
    private const uint MK_LBUTTON = 0x0001, MK_RBUTTON = 0x0002;
    private const uint CWP_SKIPINVISIBLE = 0x0001, CWP_SKIPDISABLED = 0x0002, CWP_SKIPTRANSPARENT = 0x0004;

    [DllImport("user32.dll")] private static extern bool PostMessage(IntPtr hwnd, int message, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] private static extern IntPtr ChildWindowFromPointEx(IntPtr parent, POINT point, uint flags);
    [DllImport("user32.dll")] private static extern int MapWindowPoints(IntPtr from, IntPtr to, ref POINT point, int count);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr hwnd, out int pid);
    [DllImport("user32.dll")] private static extern bool GetGUIThreadInfo(uint thread, ref GuiThreadInfo info);
    [DllImport("user32.dll")] private static extern bool IsIconic(IntPtr hwnd);
    [DllImport("user32.dll")] private static extern bool ShowWindow(IntPtr hwnd, int command);
    [DllImport("user32.dll")] private static extern uint MapVirtualKey(uint code, uint type);

    [StructLayout(LayoutKind.Sequential)]
    private struct POINT { public int X, Y; }

    [StructLayout(LayoutKind.Sequential)]
    private struct RECT { public int Left, Top, Right, Bottom; }

    [StructLayout(LayoutKind.Sequential)]
    private struct GuiThreadInfo
    {
        public int Size;
        public int Flags;
        public IntPtr Active, Focus, Capture, MenuOwner, MoveSize, Caret;
        public RECT CaretRect;
    }

    private static IntPtr MakeLParam(int low, int high) => (IntPtr)((high << 16) | (low & 0xFFFF));

    /// Finds the window to type into: an explicit hwnd, or the first visible window whose
    /// title / process matches the given text.
    public static long? Resolve(string? title, string? process, long? hwnd)
    {
        if (hwnd is > 0) return hwnd;
        if (string.IsNullOrWhiteSpace(title) && string.IsNullOrWhiteSpace(process)) return null;
        foreach (var window in Win32.Windows())
        {
            if (!string.IsNullOrWhiteSpace(title) &&
                window.Title.Contains(title, StringComparison.OrdinalIgnoreCase)) return window.Hwnd;
            if (!string.IsNullOrWhiteSpace(process) &&
                window.Process.Contains(process, StringComparison.OrdinalIgnoreCase)) return window.Hwnd;
        }
        return null;
    }

    /// The control that owns keyboard focus inside that window — works even when the window
    /// belongs to the background, which is why nothing has to be activated.
    private static IntPtr FocusOf(IntPtr hwnd)
    {
        var thread = GetWindowThreadProcessId(hwnd, out _);
        var info = new GuiThreadInfo { Size = Marshal.SizeOf<GuiThreadInfo>() };
        if (GetGUIThreadInfo(thread, ref info) && info.Focus != IntPtr.Zero) return info.Focus;
        return hwnd;
    }

    /// Walks into the deepest child under a client point, because the frame itself ignores clicks.
    private static IntPtr DeepestChild(IntPtr parent, ref POINT point)
    {
        var current = parent;
        while (true)
        {
            var child = ChildWindowFromPointEx(current, point, CWP_SKIPINVISIBLE | CWP_SKIPDISABLED | CWP_SKIPTRANSPARENT);
            if (child == IntPtr.Zero || child == current) return current;
            MapWindowPoints(current, child, ref point, 1);
            current = child;
        }
    }

    public static (int Code, string Output) Run(long? hwnd, string? title, string? process, JsonArray actions, bool restore)
    {
        var target = Resolve(title, process, hwnd);
        if (target is null) return (400, "[error] окно не найдено: укажите hwnd, window или process");

        var handle = (IntPtr)target.Value;
        if (restore && IsIconic(handle)) ShowWindow(handle, 4); // SW_SHOWNOACTIVATE: visible, still not focused

        var done = 0;
        foreach (var item in actions)
        {
            if (item is not JsonObject action) continue;
            var kind = action["action"]?.GetValue<string>() ?? "";
            switch (kind)
            {
                case "click":
                {
                    var point = new POINT
                    {
                        X = action["x"]?.GetValue<int>() ?? 0,
                        Y = action["y"]?.GetValue<int>() ?? 0,
                    };
                    var receiver = DeepestChild(handle, ref point);
                    bool right = action["button"]?.GetValue<string>() == "right";
                    var lParam = MakeLParam(point.X, point.Y);
                    PostMessage(receiver, WM_MOUSEMOVE, IntPtr.Zero, lParam);
                    PostMessage(receiver, right ? WM_RBUTTONDOWN : WM_LBUTTONDOWN,
                        (IntPtr)(right ? MK_RBUTTON : MK_LBUTTON), lParam);
                    PostMessage(receiver, right ? WM_RBUTTONUP : WM_LBUTTONUP, IntPtr.Zero, lParam);
                    break;
                }
                case "typewrite":
                case "type":
                {
                    var focus = FocusOf(handle);
                    var textMode = action["text_mode"]?.GetValue<string>() ?? "auto";
                    SendText(focus, action["text"]?.GetValue<string>() ?? "", textMode);
                    break;
                }
                case "key":
                case "hotkey":
                {
                    var names = kind == "key"
                        ? new JsonArray(action["key"]?.GetValue<string>() ?? "")
                        : action["keys"]?.AsArray() ?? new JsonArray();
                    var codes = names
                        .Select(n => Input.VirtualKeyOf(n?.GetValue<string>() ?? ""))
                        .Where(c => c.HasValue).Select(c => c!.Value).ToList();
                    if (codes.Count == 0) break;
                    var focus = FocusOf(handle);
                    foreach (var code in codes) PostKey(focus, code, true);
                    foreach (var code in Enumerable.Reverse(codes)) PostKey(focus, code, false);
                    break;
                }
                case "sleep":
                    Thread.Sleep(Math.Clamp(action["ms"]?.GetValue<int>() ?? 200, 0, 10000));
                    break;
                default:
                    break; // moveTo is meaningless here: nothing moves
            }
            done++;
            Thread.Sleep(25);
        }
        return (0, $"выполнено в фоне: {done} · окно {handle}");
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetClassName(IntPtr hwnd, System.Text.StringBuilder name, int size);

    private static string ClassOf(IntPtr hwnd)
    {
        var buffer = new System.Text.StringBuilder(256);
        GetClassName(hwnd, buffer, buffer.Capacity);
        return buffer.ToString();
    }

    private static bool Ascii(string text) => text.All(c => c < 128);

    /// WM_CHAR carries one UTF-16 unit. Unicode-aware controls (Chromium, WPF, RichEdit)
    /// take it as is; a classic ANSI "Edit" truncates it to the low byte, so for those we
    /// send the CP1251 bytes instead. `text_mode` can force either behaviour.
    private static void SendText(IntPtr focus, string text, string mode)
    {
        var ansi = mode.Equals("ansi", StringComparison.OrdinalIgnoreCase) ||
                   (mode.Equals("auto", StringComparison.OrdinalIgnoreCase) &&
                    !Ascii(text) && ClassOf(focus).Equals("Edit", StringComparison.OrdinalIgnoreCase));

        if (ansi)
        {
            foreach (var symbol in text)
                PostMessage(focus, WM_CHAR, (IntPtr)Cp1251(symbol), (IntPtr)1);
            return;
        }
        foreach (var symbol in text) PostMessage(focus, WM_CHAR, (IntPtr)symbol, (IntPtr)1);
    }

    /// CP1251 has Cyrillic in one contiguous block, so the mapping is arithmetic plus a few
    /// punctuation characters — no code-page provider needed.
    private static byte Cp1251(char symbol)
    {
        if (symbol < 0x80) return (byte)symbol;
        if (symbol is >= '\u0410' and <= '\u044F') return (byte)(0xC0 + (symbol - 0x0410));
        return symbol switch
        {
            '\u0401' => 0xA8, '\u0451' => 0xB8, '\u2116' => 0xB9, '\u00AB' => 0xAB, '\u00BB' => 0xBB,
            '\u2013' => 0x96, '\u2014' => 0x97, '\u201C' => 0x93, '\u201D' => 0x94,
            '\u2018' => 0x91, '\u2019' => 0x92, '\u2026' => 0x85, '\u00A0' => 0xA0,
            _ => (byte)'?',
        };
    }

    private static void PostKey(IntPtr hwnd, byte virtualKey, bool down)
    {
        var scan = MapVirtualKey(virtualKey, 0);
        var lParam = (IntPtr)(1 | (scan << 16) | (down ? 0 : (0xC0000000u)));
        PostMessage(hwnd, down ? WM_KEYDOWN : WM_KEYUP, (IntPtr)virtualKey, lParam);
    }
}
