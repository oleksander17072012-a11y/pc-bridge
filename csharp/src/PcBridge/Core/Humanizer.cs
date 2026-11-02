// Turns a shell command into a plain Russian sentence for the "Логги" panel.
// The journal is for the person in front of the screen: they should read what is happening,
// not a wall of PowerShell and base64.
using System.Text;
using System.Text.RegularExpressions;

namespace PcBridge.Core;

public static class Humanizer
{
    private sealed record Rule(Regex Pattern, Func<Match, string, string> Describe);

    private static Rule Simple(string pattern, string phrase) =>
        new(new Regex(pattern, RegexOptions.IgnoreCase | RegexOptions.Compiled), (_, _) => phrase);

    private static Rule WithTarget(string pattern, string phrase) =>
        new(new Regex(pattern, RegexOptions.IgnoreCase | RegexOptions.Compiled),
            (_, command) => Target(command) is { Length: > 0 } name ? $"{phrase}: {name}" : phrase);

    // Order matters: the first match wins, so the most specific patterns go first.
    private static readonly Rule[] Rules =
    {
        Simple(@"FromBase64String|\[IO\.File\]|WriteAllBytes", "передаю файл от песочницы на ПК"),
        Simple(@"dotnet\s+publish|dotnet\s+build", "компилирую сборку"),
        Simple(@"dotnet\s", "запускаю .NET"),
        Simple(@"Get-Command|where\.exe", "ищу установленную программу"),
        Simple(@"python|py\.exe|\bpy\s", "запускаю Python-скрипт"),
        Simple(@"git\s", "работаю с git"),
        new(new Regex(@"Get-FileHash", RegexOptions.IgnoreCase | RegexOptions.Compiled),
            (_, command) => $"считаю контрольную сумму{Tail(command, "файла")}"),
        WithTarget(@"Copy-Item", "копирую файл"),
        // "\b" is essential here: "Move-Item" also matches inside "Remove-Item".
        WithTarget(@"\bMove-Item", "переношу файл"),
        WithTarget(@"New-Item", "создаю папку или файл"),
        WithTarget(@"\bRemove-Item|Remove-ItemProperty", "удаляю файл"),
        new(new Regex(@"^\s*Get-Content|Get-Content", RegexOptions.IgnoreCase | RegexOptions.Compiled),
            (_, command) => $"читаю файл{Tail(command, "")}"),
        new(new Regex(@"Set-Content|Add-Content|Out-File", RegexOptions.IgnoreCase | RegexOptions.Compiled),
            (_, command) => $"записываю файл{Tail(command, "")}"),
        Simple(@"Expand-Archive", "распаковываю архив"),
        Simple(@"Compress-Archive", "упаковываю архив"),
        Simple(@"Get-ChildItem|\bdir\b|\bls\b", "смотрю содержимое папки"),
        Simple(@"Test-Path|Get-Item\b", "проверяю, есть ли файл"),
        Simple(@"Stop-Process[\s\S]*Start-Process|Start-Process[\s\S]*Stop-Process", "перезапускаю программу"),
        Simple(@"Stop-Process", "закрываю процессы"),
        Simple(@"Start-Process", "запускаю программу"),
        Simple(@"Get-CimInstance|Get-WmiObject|Get-Process", "смотрю запущенные процессы"),
        Simple(@"Get-NetTCPConnection|netstat", "смотрю сетевые порты"),
        Simple(@"Test-NetConnection|\bping\b|Resolve-DnsName", "проверяю сеть"),
        Simple(@"Get-Service|Start-Service|Stop-Service", "работаю со службой Windows"),
        Simple(@"Get-ItemProperty|reg\s+add|reg\s+query|HKLM:|HKCU:", "читаю реестр"),
        Simple(@"mklink|New-Item\s+-ItemType\s+Junction", "создаю ссылку на папку"),
        Simple(@"Get-Volume|Get-PSDrive|Get-Disk", "смотрю диски"),
        Simple(@"explorer", "открываю проводник"),
        Simple(@"notepad", "открываю блокнот"),
        Simple(@"Get-WindowsOptionalFeature|dism|sfc\s|Get-WindowsCapability", "проверяю компоненты Windows"),
        Simple(@"winget|choco|msiexec", "устанавливаю программу"),
        Simple(@"Start-Sleep", "жду немного"),
        Simple(@"Add-Type", "готовлю системный вызов"),
        Simple(@"Get-ComputerInfo|systeminfo|Get-CimInstance\s+Win32_ComputerSystem", "смотрю сведения о компьютере"),
        Simple(@"Invoke-WebRequest|Invoke-RestMethod|\bcurl\b|\bwget\b", "качаю файл из сети"),
        Simple(@"Get-Clipboard|Set-Clipboard|clip\.exe", "работаю с буфером обмена"),
        Simple(@"chrome|--remote-debugging-port", "работаю с браузером"),
    };

    /// A sentence instead of the command itself.
    public static string Describe(string command, string? shell)
    {
        var text = (command ?? "").Trim();
        if (text.Length == 0) return "пустая команда";

        foreach (var rule in Rules)
        {
            if (rule.Pattern.IsMatch(text)) return rule.Describe(rule.Pattern.Match(text), text);
        }

        // Unknown command: name its verb, which is short and still honest.
        var verb = Regex.Match(text, @"^[A-Za-z][\w-]+").Value;
        return string.IsNullOrEmpty(verb)
            ? "выполняю системную команду"
            : $"выполняю команду «{verb}»";
    }

    /// A short readable summary of what the command printed.
    public static string DescribeResult(int exitCode, string? output)
    {
        var text = (output ?? "").Trim();
        if (exitCode != 0)
        {
            // The first line that is not a stack trace usually says what went wrong.
            var reason = text.Split('\n').Select(line => line.Trim())
                .FirstOrDefault(line => line.Length > 0 && line.Length < 200) ?? "";
            reason = Collapse(reason);
            return reason.Length > 0 ? $"не получилось (код {exitCode}) · {reason}" : $"не получилось (код {exitCode})";
        }

        if (text.Length == 0) return "готово";
        if (LooksEncoded(text)) return "готово";
        var single = Collapse(text);
        if (single.Length == 0) return "готово";
        return single.Length <= 90 ? $"готово · {single}" : $"готово · {single[..90]}…";
    }


    /// Commands that carry no information for a human reader: polling loops and idling.
    private static readonly Regex[] Noise =
    {
        new(@"^\s*(Start-Sleep|Wait-Process|Wait-Job|Read-Host)\b", RegexOptions.IgnoreCase | RegexOptions.Compiled),
        new(@"pairing\.key", RegexOptions.IgnoreCase | RegexOptions.Compiled),
        // Inside a verbatim string a quote is doubled, not escaped.
        new(@"^\s*['""]?(ok|true|готово)['""]?\s*$", RegexOptions.IgnoreCase | RegexOptions.Compiled),
        new(@"^\s*(echo|Write-Output)(\s+(ok|true|готово))?\s*$", RegexOptions.IgnoreCase | RegexOptions.Compiled),
    };

    public static bool IsNoise(string command)
    {
        var text = (command ?? "").Trim();
        if (text.Length == 0) return true;
        foreach (var pattern in Noise)
            if (pattern.IsMatch(text)) return true;
        return Describe(text, null) == "жду немного";
    }

    /// One line per command: what it did, and only the output that says something.
    public static string Line(string description, int exitCode, string? output)
    {
        if (exitCode != 0) return description + " · " + DescribeResult(exitCode, output);
        var text = (output ?? "").Trim();
        if (text.Length == 0 || LooksEncoded(text)) return description;
        var single = Collapse(text);
        if (single.Length == 0) return description;
        if (single.Equals("ok", StringComparison.OrdinalIgnoreCase) ||
            single.Equals("true", StringComparison.OrdinalIgnoreCase) ||
            single.Equals("готово", StringComparison.OrdinalIgnoreCase)) return description;
        var brief = single.Length <= 60 ? single : single[..60] + "…";
        return description + " · " + brief;
    }

    /// Files and folders look better as bare names in a sentence.
    private static string Tail(string command, string noun)
    {
        var path = FirstPath(command);
        if (path is null) return noun.Length > 0 ? " " + noun : "";
        var name = path.TrimEnd('\\', '/');
        var index = name.LastIndexOfAny(new[] { '\\', '/' });
        var shortName = index >= 0 ? name[(index + 1)..] : name;
        return " " + (shortName.Length > 0 ? shortName : name);
    }

    private static string Target(string command)
    {
        var path = FirstPath(command);
        if (path is null) return "";
        var name = path.TrimEnd('\\', '/');
        var index = name.LastIndexOfAny(new[] { '\\', '/' });
        return index >= 0 ? name[(index + 1)..] : name;
    }

    private static string? FirstPath(string command)
    {
        var quoted = Regex.Match(command, @"['""]([^'""]{3,240})['""]");
        if (quoted.Success && quoted.Groups[1].Value is { Length: > 2 } value &&
            (value.Contains('\\') || value.Contains('/') || value.Contains('.'))) return value;
        var bare = Regex.Match(command, @"[A-Za-z]:\\[^\s'""]{1,200}");
        return bare.Success ? bare.Value : null;
    }

    private static bool LooksEncoded(string text)
    {
        // Long runs of base64 or a single giant word mean machine data, not a message.
        if (Regex.IsMatch(text, @"[A-Za-z0-9+/=]{48,}")) return true;
        if (text.Length > 200 && !text.Contains(' ')) return true;
        return false;
    }

    private static string Collapse(string text)
    {
        var builder = new StringBuilder(text.Length);
        var space = false;
        foreach (var symbol in text)
        {
            if (char.IsWhiteSpace(symbol))
            {
                if (!space && builder.Length > 0) builder.Append(' ');
                space = true;
                continue;
            }
            space = false;
            builder.Append(symbol);
        }
        return builder.ToString().Trim();
    }
}
