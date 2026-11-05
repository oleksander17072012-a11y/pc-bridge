// Shared state of the bridge.
using System.Text.Json.Nodes;

namespace PcBridge.Core;

public sealed class AppState
{
    public required Journal Journal { get; init; }
    public required Tunnel Tunnel { get; init; }
    public required Jobs Jobs { get; init; }
    public PlanBoard Plan { get; } = new();
    private string _key = string.Empty;
    public string Key { get { lock (_lock) return _key; } init { _key = value; } }
    public string Hostname { get; } = Environment.MachineName;
    public string StartedAt { get; } = DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss");
    public DateTime StartedUtc { get; } = DateTime.UtcNow;

    private readonly object _lock = new();
    private readonly List<WindowInfo> _windows = new();
    private DateTime _windowsAt = DateTime.MinValue;
    private string? _stage;
    private bool _active = true;
    private long _commands;
    private long _notes;

    public bool Active { get { lock (_lock) return _active; } }
    public long UptimeSeconds => (long)(DateTime.UtcNow - StartedUtc).TotalSeconds;
    public string? Stage { get { lock (_lock) return _stage; } }

    public void SetActive(bool active) { lock (_lock) _active = active; }

    /// A new key: the old one stops working the moment this returns, every paired chat has to be
    /// given the fresh block. The pairing file is rewritten at once, so screen and file agree.
    public string RotateKey()
    {
        var fresh = Keys.NewKey();
        lock (_lock) _key = fresh;
        WriteKeyFile();
        Journal.Push("ui", "создан новый ключ доступа");
        return fresh;
    }
    public long BumpCommands() { lock (_lock) return ++_commands; }
    public long BumpNotes() { lock (_lock) return ++_notes; }
    public long CommandCount { get { lock (_lock) return _commands; } }
    public long NoteCount { get { lock (_lock) return _notes; } }
    public void SetStage(string? stage) { lock (_lock) _stage = stage; }

    /// The window list is not rebuilt on a timer: it is cached for a moment.
    public List<WindowInfo> WindowsRecent(TimeSpan maxAge)
    {
        lock (_lock)
        {
            if (DateTime.UtcNow - _windowsAt < maxAge) return _windows.ToList();
        }
        return RefreshWindows();
    }

    public List<WindowInfo> RefreshWindows()
    {
        var list = Win32.Windows();
        lock (_lock)
        {
            _windows.Clear();
            _windows.AddRange(list);
            _windowsAt = DateTime.UtcNow;
        }
        return list;
    }

    public string PairingBlock()
    {
        var keyVisible = Key;
        var url = Tunnel.Url ?? "нет связи";
        return $"PC-BRIDGE-PAIRING v1\nurl={url}\nkey={keyVisible}\ncreated={StartedAt}";
    }

    /// Key block plus the operating instructions: one paste, nothing to attach afterwards.
    public string PairingBundle() => PairingBlock() + "\n\n---\n\n" + Prompt.Body;

    public void WriteKeyFile()
    {
        try { File.WriteAllText(Paths.KeyFile, PairingBlock()); } catch { }
    }

    public JsonObject Snapshot() => new()
    {
        ["version"] = "3.6.1",
        ["hostname"] = Hostname,
        ["port"] = Relay.Port,
        ["tunnel_url"] = Tunnel.Url,
        ["pairing"] = PairingBlock(),
        ["active"] = Active,
        ["stage"] = Stage,
        ["elevated_hint"] = Win32.Elevated,
        ["uptime_seconds"] = UptimeSeconds,
        ["commands"] = CommandCount,
        ["notes"] = NoteCount,
        ["started_at"] = StartedAt,
        ["data_dir"] = Paths.DataDir,
        ["prompt_bytes"] = Prompt.Bytes,
    };
}
