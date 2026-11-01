// Core primitives: paths, key, journal, console encoding, background jobs.
using System.Diagnostics;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace PcBridge.Core;

public static class Paths
{
    public static string DataDir
    {
        get
        {
            var baseDir = Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData);
            var dir = Path.Combine(baseDir, "PC-Bridge");
            Directory.CreateDirectory(dir);
            return dir;
        }
    }

    public static string KeyFile => Path.Combine(DataDir, "pairing.key");
    public static string LogFile => Path.Combine(DataDir, "pc_agent_log.txt");
    public static string BinDir
    {
        get
        {
            var dir = Path.Combine(DataDir, "bin");
            Directory.CreateDirectory(dir);
            return dir;
        }
    }
}

public static class Keys
{
    /// 32 hex characters = 128 bits from the system CSPRNG.
    public static string NewKey()
    {
        var bytes = RandomNumberGenerator.GetBytes(16);
        return Convert.ToHexString(bytes).ToLowerInvariant();
    }
}

public sealed record JournalEntry(long Seq, string Time, string Kind, string Text);

/// Ring buffer plus an append-only file: the journal is the agent's work log.
public sealed class Journal
{
    private readonly object _lock = new();
    private readonly LinkedList<JournalEntry> _entries = new();
    private readonly string _path;
    private long _seq;
    private const int Max = 2000;

    public event Action<JournalEntry>? Added;

    public Journal(string path) => _path = path;

    public JournalEntry Push(string kind, string text)
    {
        var entry = new JournalEntry(0, DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss"), kind, Clean(text));
        lock (_lock)
        {
            entry = entry with { Seq = ++_seq };
            _entries.AddLast(entry);
            while (_entries.Count > Max) _entries.RemoveFirst();
            try
            {
                File.AppendAllText(_path, $"[{entry.Time}] [{entry.Kind}] {entry.Text}\n", Encoding.UTF8);
            }
            catch { /* the log file is best effort */ }
        }
        try { Added?.Invoke(entry); } catch { }
        return entry;
    }

    private static string Clean(string text)
    {
        var flat = text.Replace("\r", " ").Replace("\n", " ");
        return flat.Length > 4000 ? flat[..4000] + "…" : flat;
    }

    public List<JournalEntry> Recent(int limit)
    {
        lock (_lock) return _entries.Skip(Math.Max(0, _entries.Count - limit)).ToList();
    }
}

/// Console output arrives in the OEM (CP866) or ANSI (CP1251) code page, not UTF-8.
public static class ConsoleText
{
    private const string Cp866 = "\u0410\u0411\u0412\u0413\u0414\u0415\u0416\u0417\u0418\u0419\u041A\u041B\u041C\u041D\u041E\u041F\u0420\u0421\u0422\u0423\u0424\u0425\u0426\u0427\u0428\u0429\u042A\u042B\u042C\u042D\u042E\u042F\u0430\u0431\u0432\u0433\u0434\u0435\u0436\u0437\u0438\u0439\u043A\u043B\u043C\u043D\u043E\u043F\u2591\u2592\u2593\u2502\u2524\u2561\u2562\u2556\u2555\u2563\u2551\u2557\u255D\u255C\u255B\u2510\u2514\u2534\u252C\u251C\u2500\u253C\u255E\u255F\u255A\u2554\u2569\u2566\u2560\u2550\u256C\u2567\u2568\u2564\u2565\u2559\u2558\u2552\u2553\u256B\u256A\u2518\u250C\u2588\u2584\u258C\u2590\u2580\u0440\u0441\u0442\u0443\u0444\u0445\u0446\u0447\u0448\u0449\u044A\u044B\u044C\u044D\u044E\u044F\u0401\u0451\u0404\u0454\u0407\u0457\u040E\u045E\u00B0\u2219\u00B7\u221A\u2116\u00A4\u25A0\u00A0";
    private const string Cp1251 = "\u0402\u0403\u201A\u0453\u201E\u2026\u2020\u2021\u20AC\u2030\u0409\u2039\u040A\u040C\u040B\u040F\u0452\u2018\u2019\u201C\u201D\u2022\u2013\u2014\uFFFD\u2122\u0459\u203A\u045A\u045C\u045B\u045F\u00A0\u040E\u045E\u0408\u00A4\u0490\u00A6\u00A7\u0401\u00A9\u0404\u00AB\u00AC\u00AD\u00AE\u0407\u00B0\u00B1\u0406\u0456\u0491\u00B5\u00B6\u00B7\u0451\u2116\u0454\u00BB\u0458\u0405\u0455\u0457\u0410\u0411\u0412\u0413\u0414\u0415\u0416\u0417\u0418\u0419\u041A\u041B\u041C\u041D\u041E\u041F\u0420\u0421\u0422\u0423\u0424\u0425\u0426\u0427\u0428\u0429\u042A\u042B\u042C\u042D\u042E\u042F\u0430\u0431\u0432\u0433\u0434\u0435\u0436\u0437\u0438\u0439\u043A\u043B\u043C\u043D\u043E\u043F\u0440\u0441\u0442\u0443\u0444\u0445\u0446\u0447\u0448\u0449\u044A\u044B\u044C\u044D\u044E\u044F";

    /// UTF-8 if the bytes are valid UTF-8, otherwise the code page that reads best.
    public static string Decode(byte[] bytes)
    {
        try
        {
            return new UTF8Encoding(false, true).GetString(bytes);
        }
        catch (DecoderFallbackException)
        {
            var oem = Map(bytes, Cp866);
            var ansi = Map(bytes, Cp1251);
            return Score(oem) >= Score(ansi) ? oem : ansi;
        }
    }

    private static string Map(byte[] bytes, string table)
    {
        var sb = new StringBuilder(bytes.Length);
        foreach (var b in bytes) sb.Append(b < 0x80 ? (char)b : table[b - 0x80]);
        return sb.ToString();
    }

    private static int Score(string text)
    {
        var score = 0;
        foreach (var c in text)
        {
            if (c == '\uFFFD') score -= 6;
            else if (c < 0x20 && c != '\n' && c != '\r' && c != '\t') score -= 4;
            else if (c >= 0x400 && c <= 0x4FF) score += 3;
            else if (c >= 0x20 && c <= 0x7E) score += 1;
        }
        return score;
    }
}

public sealed record JobState(string Id, bool Running, long StartedMs, long FinishedMs, string Output, int ExitCode);

/// Long or short jobs that must survive an HTTP request being cut at ~100 s by Cloudflare.
public sealed class Jobs
{
    private readonly object _lock = new();
    private readonly Dictionary<string, JobState> _jobs = new();
    private static long Now => DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();

    public string Start()
    {
        var id = Keys.NewKey()[..16];
        lock (_lock)
        {
            if (_jobs.Count > 60)
            {
                foreach (var stale in _jobs.Where(p => !p.Value.Running).Take(10).Select(p => p.Key).ToList())
                    _jobs.Remove(stale);
            }
            _jobs[id] = new JobState(id, true, Now, 0, "", -1);
        }
        return id;
    }

    public void Finish(string id, string output, int exitCode)
    {
        lock (_lock)
        {
            if (_jobs.TryGetValue(id, out var state))
                _jobs[id] = state with { Running = false, FinishedMs = Now, Output = output, ExitCode = exitCode };
        }
    }

    private static JsonObject Json(JobState state) => new()
    {
        ["job_id"] = state.Id,
        ["status"] = state.Running ? "running" : "done",
        ["output"] = state.Output,
        ["exit_code"] = state.ExitCode,
        ["took_ms"] = (state.Running ? Now : state.FinishedMs) - state.StartedMs,
    };

    public JsonNode? Get(string id)
    {
        lock (_lock) return _jobs.TryGetValue(id, out var state) ? Json(state) : null;
    }

    public JsonArray List(int limit = 20)
    {
        var array = new JsonArray();
        lock (_lock)
        {
            foreach (var state in _jobs.Values.OrderByDescending(s => s.StartedMs).Take(limit)) array.Add(Json(state));
        }
        return array;
    }
}
