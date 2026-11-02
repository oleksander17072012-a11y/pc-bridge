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

    /// Not a blocklist: these run, but the journal flags them. The label says what kind of
    /// irreversible action it is — the command itself is already described a line above.
    private static readonly (string Needle, string Label)[] Watchlist =
    {
        ("format ", "форматирование диска"),
        ("diskpart", "работа с разделами диска"),
        ("clear-disk", "стирание диска"),
        ("bcdedit", "правка загрузчика"),
        ("vssadmin delete", "удаление теневых копий"),
        ("wbadmin delete", "удаление резервных копий"),
        ("cipher /w:", "затирание свободного места"),
        ("reg add", "правка реестра"),
        ("reg delete", "удаление из реестра"),
        ("new-itemproperty", "правка реестра"),
        ("icacls", "смена прав доступа"),
        ("net user", "учётные записи"),
        ("net localgroup", "группы пользователей"),
        ("schtasks /create", "новое задание планировщика"),
        ("schtasks /delete", "удаление задания планировщика"),
        ("sc delete", "удаление службы"),
        ("sc stop", "остановка службы"),
        ("set-mppreference", "настройки Defender"),
        ("netsh advfirewall", "правила брандмауэра"),
        ("c:\\windows", "системная папка Windows"),
        ("system32", "системная папка"),
        ("program files", "папка Program Files"),
    };

    /// `format C:` erases a disk; `Format-Table` and `-Format` do not. Plain words are too blunt here.
    private static readonly System.Text.RegularExpressions.Regex[] Patterns =
    {
        new(@"(?<![\w-])format\s+[a-z]:", System.Text.RegularExpressions.RegexOptions.IgnoreCase),
        new(@"\bremove-item\b[^\r\n]*-recurse\b", System.Text.RegularExpressions.RegexOptions.IgnoreCase),
        new(@"\brm\b[^\r\n]*-recurse\b", System.Text.RegularExpressions.RegexOptions.IgnoreCase),
    };

    public static string? WatchlistHit(string command)
    {
        foreach (var pattern in Patterns)
        {
            if (!pattern.IsMatch(command)) continue;
            // A delete with -Recurse inside a build folder is routine; say so plainly.
            return pattern.ToString().Contains("remove-item") || pattern.ToString().Contains("rm")
                ? "удаление с -Recurse"
                : "необратимая команда";
        }
        var lowered = command.ToLowerInvariant();
        foreach (var (needle, label) in Watchlist)
            if (lowered.Contains(needle)) return label;
        return null;
    }

    public static ShellResult Run(string command, string? shell, TimeSpan? timeout)
    {
        var warning = WatchlistHit(command) is { } hit
            ? $"осторожно: {hit} · выполнено"
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
            // `cmd /c "chcp 65001>nul & mkdir "C:\path""` breaks: the nested quotes split the
            // argument, and cmd then reports "Синтаксическая ошибка в имени файла". Writing the
            // command to a .bat keeps every quote, Cyrillic character and & exactly as written.
            scriptPath = WriteScript(command, powershell: false);
            info = new ProcessStartInfo("cmd.exe")
            {
                CreateNoWindow = true,
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
            };
            info.ArgumentList.Add("/c");
            info.ArgumentList.Add(scriptPath);
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
