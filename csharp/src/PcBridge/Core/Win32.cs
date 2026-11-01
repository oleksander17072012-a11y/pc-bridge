// Win32 layer: screen capture, window list, rounded outline, input, elevation.
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Windows.Media;
using System.Windows.Media.Imaging;

namespace PcBridge.Core;

public sealed record WindowInfo(long Hwnd, string Title, int Pid, string Process);

public static class Win32
{
    // ---------- screen capture ----------
    [DllImport("user32.dll")] private static extern IntPtr GetDC(IntPtr hwnd);
    [DllImport("user32.dll")] private static extern int ReleaseDC(IntPtr hwnd, IntPtr hdc);
    [DllImport("user32.dll")] private static extern int GetSystemMetrics(int index);
    [DllImport("gdi32.dll")] private static extern IntPtr CreateCompatibleDC(IntPtr hdc);
    [DllImport("gdi32.dll")] private static extern IntPtr CreateCompatibleBitmap(IntPtr hdc, int width, int height);
    [DllImport("gdi32.dll")] private static extern IntPtr SelectObject(IntPtr hdc, IntPtr obj);
    [DllImport("gdi32.dll")] private static extern bool DeleteObject(IntPtr obj);
    [DllImport("gdi32.dll")] private static extern bool DeleteDC(IntPtr hdc);
    [DllImport("gdi32.dll")] private static extern bool StretchBlt(IntPtr dest, int x, int y, int w, int h,
        IntPtr src, int sx, int sy, int sw, int sh, uint rop);
    [DllImport("gdi32.dll")] private static extern int SetStretchBltMode(IntPtr hdc, int mode);
    [DllImport("gdi32.dll")] private static extern int GetDIBits(IntPtr hdc, IntPtr bitmap, uint start, uint lines,
        byte[] bits, ref BitmapInfo info, uint usage);
    [DllImport("gdi32.dll")] private static extern IntPtr CreateRoundRectRgn(int l, int t, int r, int b, int w, int h);

    private const int SM_CXSCREEN = 0;
    private const int SM_CYSCREEN = 1;
    private const uint SRCCOPY = 0x00CC0020;
    private const int HALFTONE = 4;
    private const uint DIB_RGB_COLORS = 0;

    [StructLayout(LayoutKind.Sequential)]
    private struct BitmapInfoHeader
    {
        public uint Size;
        public int Width;
        public int Height;
        public ushort Planes;
        public ushort BitCount;
        public uint Compression;
        public uint SizeImage;
        public int XPelsPerMeter;
        public int YPelsPerMeter;
        public uint ClrUsed;
        public uint ClrImportant;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct BitmapInfo
    {
        public BitmapInfoHeader Header;
        public uint Colors;
    }

    public sealed record Shot(byte[] Jpeg, int ScreenWidth, int ScreenHeight);

    /// Downscales inside GDI (StretchBlt) so a 4K desktop never costs a full-frame buffer.
    public static Shot Capture(int maxWidth, int quality)
    {
        var screenWidth = GetSystemMetrics(SM_CXSCREEN);
        var screenHeight = GetSystemMetrics(SM_CYSCREEN);
        if (screenWidth <= 0 || screenHeight <= 0) throw new InvalidOperationException("screen size unknown");

        var targetWidth = Math.Clamp(maxWidth, 320, screenWidth);
        var targetHeight = Math.Max(1, (int)Math.Round(screenHeight * (double)targetWidth / screenWidth));

        var screenDc = GetDC(IntPtr.Zero);
        if (screenDc == IntPtr.Zero) throw new InvalidOperationException("GetDC failed");
        try
        {
            var memoryDc = CreateCompatibleDC(screenDc);
            var bitmap = CreateCompatibleBitmap(screenDc, targetWidth, targetHeight);
            if (memoryDc == IntPtr.Zero || bitmap == IntPtr.Zero) throw new InvalidOperationException("GDI context failed");
            var previous = SelectObject(memoryDc, bitmap);
            try
            {
                SetStretchBltMode(memoryDc, HALFTONE);
                if (!StretchBlt(memoryDc, 0, 0, targetWidth, targetHeight, screenDc, 0, 0, screenWidth, screenHeight, SRCCOPY))
                    throw new InvalidOperationException("StretchBlt failed");

                var info = new BitmapInfo
                {
                    Header = new BitmapInfoHeader
                    {
                        Size = (uint)Marshal.SizeOf<BitmapInfoHeader>(),
                        Width = targetWidth,
                        Height = -targetHeight, // top-down rows
                        Planes = 1,
                        BitCount = 32,
                        Compression = 0,
                    },
                };
                var pixels = new byte[targetWidth * targetHeight * 4];
                if (GetDIBits(memoryDc, bitmap, 0, (uint)targetHeight, pixels, ref info, DIB_RGB_COLORS) == 0)
                    throw new InvalidOperationException("GetDIBits failed");

                var source = BitmapSource.Create(targetWidth, targetHeight, 96, 96,
                    PixelFormats.Bgra32, null, pixels, targetWidth * 4);
                var encoder = new JpegBitmapEncoder { QualityLevel = Math.Clamp(quality, 30, 95) };
                encoder.Frames.Add(BitmapFrame.Create(source));
                using var stream = new MemoryStream();
                encoder.Save(stream);
                return new Shot(stream.ToArray(), screenWidth, screenHeight);
            }
            finally
            {
                SelectObject(memoryDc, previous);
                DeleteObject(bitmap);
                DeleteDC(memoryDc);
            }
        }
        finally
        {
            ReleaseDC(IntPtr.Zero, screenDc);
        }
    }

    // ---------- windows ----------
    private delegate bool EnumProc(IntPtr hwnd, IntPtr param);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc callback, IntPtr param);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetWindowTextW(IntPtr hwnd, System.Text.StringBuilder text, int max);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr hwnd);
    [DllImport("user32.dll")] private static extern int GetWindowThreadProcessId(IntPtr hwnd, out int pid);
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern bool SetWindowRgn(IntPtr hwnd, IntPtr region, bool redraw);
    [DllImport("user32.dll")] private static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] private static extern void mouse_event(uint flags, int dx, int dy, uint data, IntPtr extra);
    [DllImport("user32.dll")] private static extern void keybd_event(byte code, byte scan, uint flags, IntPtr extra);
    [DllImport("shell32.dll")] private static extern bool IsUserAnAdmin();

    public static bool Elevated => IsUserAnAdmin();

    public static List<WindowInfo> Windows()
    {
        var result = new List<WindowInfo>();
        EnumWindows((hwnd, _) =>
        {
            if (!IsWindowVisible(hwnd)) return true;
            var buffer = new System.Text.StringBuilder(512);
            GetWindowTextW(hwnd, buffer, buffer.Capacity);
            var title = buffer.ToString();
            if (string.IsNullOrWhiteSpace(title)) return true;
            GetWindowThreadProcessId(hwnd, out var pid);
            string process = "";
            try { process = Process.GetProcessById(pid).ProcessName + ".exe"; } catch { }
            result.Add(new WindowInfo((long)hwnd, title, pid, process));
            return result.Count < 120;
        }, IntPtr.Zero);
        return result;
    }

    public static WindowInfo ActiveWindow()
    {
        var hwnd = GetForegroundWindow();
        var buffer = new System.Text.StringBuilder(512);
        GetWindowTextW(hwnd, buffer, buffer.Capacity);
        GetWindowThreadProcessId(hwnd, out var pid);
        string process = "";
        try { process = Process.GetProcessById(pid).ProcessName + ".exe"; } catch { }
        return new WindowInfo((long)hwnd, buffer.ToString(), pid, process);
    }

    // ---------- rounded outline (Windows 10 has no DWM rounding) ----------
    public static void Round(IntPtr hwnd, double widthDip, double heightDip, double scale, double radiusDip)
    {
        if (hwnd == IntPtr.Zero) return;
        var radius = (int)Math.Round(radiusDip * scale);
        var region = CreateRoundRectRgn(0, 0, (int)Math.Round(widthDip * scale) + 1, (int)Math.Round(heightDip * scale) + 1, radius * 2, radius * 2);
        if (region != IntPtr.Zero) SetWindowRgn(hwnd, region, true);
    }

    // ---------- input ----------
    private const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
    private const uint MOUSEEVENTF_LEFTUP = 0x0004;
    private const uint MOUSEEVENTF_RIGHTDOWN = 0x0008;
    private const uint MOUSEEVENTF_RIGHTUP = 0x0010;
    private const uint KEYEVENTF_KEYUP = 0x0002;

    public static void MoveTo(int x, int y) => SetCursorPos(x, y);

    public static void Click(int x, int y, string button)
    {
        SetCursorPos(x, y);
        Thread.Sleep(30);
        if (button == "right") { mouse_event(MOUSEEVENTF_RIGHTDOWN, 0, 0, 0, IntPtr.Zero); mouse_event(MOUSEEVENTF_RIGHTUP, 0, 0, 0, IntPtr.Zero); }
        else { mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, IntPtr.Zero); mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, IntPtr.Zero); }
    }

    public static void TapKey(byte virtualKey)
    {
        keybd_event(virtualKey, 0, 0, IntPtr.Zero);
        keybd_event(virtualKey, 0, KEYEVENTF_KEYUP, IntPtr.Zero);
    }

    public static void KeyDown(byte virtualKey) => keybd_event(virtualKey, 0, 0, IntPtr.Zero);
    public static void KeyUp(byte virtualKey) => keybd_event(virtualKey, 0, KEYEVENTF_KEYUP, IntPtr.Zero);
}
