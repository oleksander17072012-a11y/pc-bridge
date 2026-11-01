// Mouse and keyboard for the `gui` job: the real cursor is used, with a corner failsafe.
using System.Text.Json.Nodes;

namespace PcBridge.Core;

public static class Input
{
    private static readonly Dictionary<string, byte> SpecialKeys = new(StringComparer.OrdinalIgnoreCase)
    {
        ["ctrl"] = 0x11, ["control"] = 0x11,
        ["shift"] = 0x10, ["alt"] = 0x12, ["win"] = 0x5B,
        ["enter"] = 0x0D, ["return"] = 0x0D, ["tab"] = 0x09, ["esc"] = 0x1B, ["escape"] = 0x1B,
        ["space"] = 0x20, ["backspace"] = 0x08, ["delete"] = 0x2E, ["up"] = 0x26, ["down"] = 0x28,
        ["left"] = 0x25, ["right"] = 0x27, ["home"] = 0x24, ["end"] = 0x23,
        ["f1"] = 0x70, ["f2"] = 0x71, ["f3"] = 0x72, ["f4"] = 0x73, ["f5"] = 0x74,
        ["f6"] = 0x75, ["f7"] = 0x76, ["f8"] = 0x77, ["f9"] = 0x78, ["f10"] = 0x79,
        ["f11"] = 0x7A, ["f12"] = 0x7B,
    };

    public static byte? VirtualKeyOf(string name) => VirtualKey(name);

    private static byte? VirtualKey(string name)
    {
        if (SpecialKeys.TryGetValue(name, out var code)) return code;
        if (name.Length == 1)
        {
            var upper = char.ToUpperInvariant(name[0]);
            if (upper is >= 'A' and <= 'Z' or >= '0' and <= '9') return (byte)upper;
        }
        return null;
    }

    /// Returns (exitCode, output). 130 means the failsafe stopped the run.
    public static (int Code, string Output, int Done) Run(JsonArray actions)
    {
        var done = 0;
        foreach (var item in actions)
        {
            if (item is not JsonObject action) continue;
            var kind = action["action"]?.GetValue<string>() ?? "";
            var x = action["x"]?.GetValue<int>() ?? 0;
            var y = action["y"]?.GetValue<int>() ?? 0;

            // Failsafe: the top-left corner aborts, like pyautogui's corner.
            if (kind is "moveTo" or "click" && x <= 12 && y <= 12)
            {
                return (130, "[aborted] failsafe: виртуальный курсор в левом верхнем углу", done);
            }

            switch (kind)
            {
                case "moveTo":
                    Win32.MoveTo(x, y);
                    break;
                case "click":
                    Win32.Click(x, y, action["button"]?.GetValue<string>() ?? "left");
                    break;
                case "typewrite":
                case "type":
                    var text = action["text"]?.GetValue<string>() ?? "";
                    foreach (var symbol in text) TypeChar(symbol);
                    break;
                case "hotkey":
                    var keys = action["keys"]?.AsArray();
                    if (keys is null) break;
                    var codes = keys.Select(k => VirtualKey(k?.GetValue<string>() ?? "")).Where(c => c.HasValue).Select(c => c!.Value).ToList();
                    foreach (var code in codes) Win32.KeyDown(code);
                    foreach (var code in codes.AsEnumerable().Reverse()) Win32.KeyUp(code);
                    break;
                case "key":
                    if (VirtualKey(action["key"]?.GetValue<string>() ?? "") is { } single) Win32.TapKey(single);
                    break;
                case "sleep":
                    Thread.Sleep(Math.Clamp(action["ms"]?.GetValue<int>() ?? 200, 0, 10000));
                    break;
            }
            done++;
            Thread.Sleep(20);
        }
        return (0, $"выполнено действий: {done}", done);
    }

    private static void TypeChar(char symbol)
    {
        var scan = VkKeyScan(symbol);
        if (scan == -1) return;
        var vk = (byte)(scan & 0xFF);
        var shift = (scan & 0x100) != 0;
        if (shift) Win32.KeyDown(0x10);
        Win32.TapKey(vk);
        if (shift) Win32.KeyUp(0x10);
        Thread.Sleep(8);
    }

    [System.Runtime.InteropServices.DllImport("user32.dll")]
    private static extern short VkKeyScan(char ch);
}
