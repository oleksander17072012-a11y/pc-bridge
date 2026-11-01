// Cloudflare quick tunnel: one download, signature checked, URL parsed from the log.
using System.Diagnostics;
using System.Text.RegularExpressions;

namespace PcBridge.Core;

public sealed class Tunnel
{
    private const string DownloadUrl =
        "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe";

    private readonly Journal _journal;
    private readonly Action<string>? _onUrl;
    private readonly object _lock = new();
    private Process? _process;
    private string? _url;

    public Tunnel(Journal journal, Action<string>? onUrl = null)
    {
        _journal = journal;
        _onUrl = onUrl;
    }

    public string? Url { get { lock (_lock) return _url; } }

    public void Start()
    {
        Task.Run(() =>
        {
            try
            {
                var exe = EnsureBinary();
                var info = new ProcessStartInfo(exe)
                {
                    CreateNoWindow = true,
                    UseShellExecute = false,
                    RedirectStandardError = true,
                    RedirectStandardOutput = true,
                    Arguments = "tunnel --no-autoupdate --url http://127.0.0.1:" + Relay.Port,
                };
                _journal.Push("tunnel", "запускаю туннель cloudflared");
                var process = Process.Start(info) ?? throw new InvalidOperationException("cloudflared did not start");
                lock (_lock) _process = process;

                void Handle(string? line)
                {
                    if (line is null) return;
                    var url = ExtractUrl(line);
                    if (url is null) return;
                    lock (_lock)
                    {
                        if (_url == url) return;
                        _url = url;
                    }
                    _journal.Push("tunnel", "туннель готов: " + url);
                    try { _onUrl?.Invoke(url); } catch { }
                }

                process.ErrorDataReceived += (_, e) => Handle(e.Data);
                process.OutputDataReceived += (_, e) => Handle(e.Data);
                process.BeginErrorReadLine();
                process.BeginOutputReadLine();
            }
            catch (Exception error)
            {
                _journal.Push("error", "туннель не поднялся: " + error.Message);
            }
        });
    }

    public void Restart()
    {
        lock (_lock)
        {
            try { _process?.Kill(entireProcessTree: true); } catch { }
            _process = null;
            _url = null;
        }
        _journal.Push("tunnel", "перезапускаю туннель");
        Start();
    }

    public void Stop()
    {
        lock (_lock)
        {
            try { _process?.Kill(entireProcessTree: true); } catch { }
            _process = null;
            _url = null;
        }
    }

    private static string EnsureBinary()
    {
        var path = Path.Combine(Paths.BinDir, "cloudflared.exe");
        if (File.Exists(path) && new FileInfo(path).Length > 1_000_000) return path;

        using var client = new HttpClient { Timeout = TimeSpan.FromMinutes(10) };
        var bytes = client.GetByteArrayAsync(DownloadUrl).GetAwaiter().GetResult();
        File.WriteAllBytes(path, bytes);
        VerifySignature(path);
        return path;
    }

    /// The downloaded executable must carry a valid Authenticode signature.
    private static void VerifySignature(string path)
    {
        var info = new ProcessStartInfo("powershell")
        {
            CreateNoWindow = true,
            UseShellExecute = false,
            RedirectStandardOutput = true,
        };
        info.ArgumentList.Add("-NoProfile");
        info.ArgumentList.Add("-Command");
        info.ArgumentList.Add($"try {{ (Get-AuthenticodeSignature -FilePath '{path}').Status }} catch {{ 'error' }}");

        using var process = Process.Start(info) ?? throw new InvalidOperationException("cannot verify signature");
        var status = process.StandardOutput.ReadToEnd().Trim();
        process.WaitForExit(30000);
        if (!status.Equals("Valid", StringComparison.OrdinalIgnoreCase))
        {
            try { File.Delete(path); } catch { }
            throw new InvalidOperationException($"подпись cloudflared не подтверждена: {status}");
        }
    }

    private static readonly Regex QuickTunnel = new(@"https://[a-z0-9-]+\.trycloudflare\.com", RegexOptions.Compiled);

    public static string? ExtractUrl(string line)
        => line.Contains(".trycloudflare.com") ? QuickTunnel.Match(line).Value is { Length: > 0 } hit ? hit : null : null;
}
