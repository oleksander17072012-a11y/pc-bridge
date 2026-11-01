// The pairing instructions, embedded at build time.
namespace PcBridge.Core;

public static class Prompt
{
    private static readonly Lazy<string> Text = new(() =>
    {
        var assembly = typeof(Prompt).Assembly;
        using var stream = assembly.GetManifestResourceStream("PcBridge.PairingPrompt.txt");
        if (stream is null) return "";
        using var reader = new StreamReader(stream, Encoding.UTF8);
        return reader.ReadToEnd().Replace("\r\n", "\n").TrimEnd() + "\n";
    });

    /// The whole document that goes to the assistant after the pairing block.
    public static string Body => Text.Value;

    public static int Bytes => Encoding.UTF8.GetByteCount(Body);
}
