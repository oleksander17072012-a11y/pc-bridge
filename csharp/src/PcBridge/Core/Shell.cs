// Running shell commands: UTF-8 forced, Cyrillic decoded, long lines moved to a script.
using System.Diagnostics;
using System.Text;

namespace PcBridge.Core;

public sealed record ShellResult(string Output, int ExitCode, string? Warning);

public static class Shell
{
    /// Above this a one-liner approaches the Windows command-line limit (32 767 bytes),
    /// which surfaces as `os error 206`.
    private const int ScriptThreshold = 6000;

    /// Not a blocklist: these run, but they are shown in red in the journal.
    private static readonly string[] Watchlist =
    {
        "diskpart", "bcdedit", "vssadmin delete", "wbadmin delete", "cipher /w:", "clear-disk",
        "c:\\windows", "system32", "program files", "set-mppreference", "netsh advfirewall",
        "sc delete", "sc stop", "reg add", "reg delete", "icacls",
        "net user", "net localgroup", "schtasks /create", "schtasks /delete", "new-itemproperty",
    };

    /// `format C:` erases a disk; `Format-Table` and `-Format` do not. Plain words are too blunt here.
    private static readonly System.Text.RegularExpressions.Regex[] Patterns =
    {
        new(@"\bformat\s+[a-z]:", System.Text.RegularExpressions.RegexOptions.IgnoreCase),
        new(@"\bremove-item\b[^\r\n]*-recurse\b", System.Text.RegularExpressions.RegexOptions.IgnoreCase),
    };

    public static string? WatchlistHit(string command)
    {
        var lowered = command.ToLowerInvariant();
        foreach (var pattern in Patterns)
        {
            var match = pattern.Match(command);
            if (match.Success) return match.Value;
        }
        return Watchlist.FirstOrDefault(item => lowered.Contains(item));
    }

    public static ShellResult Run(string command, string? shell, TimeSpan? timeout)
    {
        var warning = WatchlistHit(command) is { } hit
            ? $"внимание: команда содержит «{hit.Trim()}» — необратимое действие, выполнено по настройке"
            : null;

        var powershell = shell is "powershell" or "pwsh";
        string? scriptPath = null;
        ProcessStartInfo info;

        if (command.Length > ScriptThreshold)
        {
            scriptPath = WriteScript(command, powershell);
            info = new ProcessStartInfo(powershell ? "powershell" : "cmd.exe")
            {
                CreateNoWindow = true,
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
            };
            if (powershell)
            {
                info.ArgumentList.Add("-NoProfile");
                info.ArgumentList.Add("-NonInteractive");
                info.ArgumentList.Add("-ExecutionPolicy");
                info.ArgumentList.Add("Bypass");
                info.ArgumentList.Add("-File");
                info.ArgumentList.Add(scriptPath);
            }
            else
            {
                info.ArgumentList.Add("/c");
                info.ArgumentList.Add(scriptPath);
            }
        }
        else if (powershell)
        {
            info = new ProcessStartInfo("powershell")
            {
                CreateNoWindow = true,
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
            };
            // Both the console and the pipeline must speak UTF-8.
            info.ArgumentList.Add("-NoProfile");
            info.ArgumentList.Add("-NonInteractive");
            info.ArgumentList.Add("-Command");
            info.ArgumentList.Add(
                "$ProgressPreference='SilentlyContinue'; " +
                "[Console]::OutputEncoding=[System.Text.Encoding]::UTF8; " +
                "$OutputEncoding=[System.Text.Encoding]::UTF8; " + command);
        }
        else
        {
            info = new ProcessStartInfo("cmd.exe")
            {
                CreateNoWindow = true,
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
            };
            info.ArgumentList.Add("/c");
            info.ArgumentList.Add("chcp 65001>nul & " + command);
        }

        try
        {
            using var process = Process.Start(info) ?? throw new InvalidOperationException("process did not start");
            var stdout = new MemoryStream();
            var stderr = new MemoryStream();
            var copyOut = process.StandardOutput.BaseStream.CopyToAsync(stdout);
            var copyErr = process.StandardError.BaseStream.CopyToAsync(stderr);

            var finished = timeout is null
                ? process.WaitForExit(int.MaxValue) || true
                : process.WaitForExit((int)timeout.Value.TotalMilliseconds);

            if (!finished)
            {
                try { process.Kill(entireProcessTree: true); } catch { }
                return new ShellResult($"[timeout after {timeout!.Value.TotalSeconds:0}s — command killed]", 124, warning);
            }

            Task.WaitAll(new[] { copyOut, copyErr }, TimeSpan.FromSeconds(10));
            var text = ConsoleText.Decode(stdout.ToArray()) + ConsoleText.Decode(stderr.ToArray());
            return new ShellResult(text, process.ExitCode, warning);
        }
        catch (Exception error)
        {
            return new ShellResult($"[error] {error.Message}", 1, warning);
        }
        finally
        {
            if (scriptPath is not null)
            {
                try { File.Delete(scriptPath); } catch { }
            }
        }
    }

    private static string WriteScript(string command, bool powershell)
    {
        var path = Path.Combine(Path.GetTempPath(), $"pcbridge-{Keys.NewKey()[..12]}.{(powershell ? "ps1" : "bat")}");
        var body = powershell
            ? $"$ErrorActionPreference='Continue'\r\n{command}\r\n"
            : $"@echo off\r\nchcp 65001>nul\r\n{command}\r\n";
        // .ps1 needs a UTF-8 BOM or PowerShell 5.1 reads Cyrillic as ANSI;
        // .bat must have no BOM at all or cmd.exe echoes "@echo off" as a command.
        File.WriteAllText(path, body, new UTF8Encoding(powershell));
        return path;
    }
}
