// Loopback HTTP API — the only thing the Cloudflare tunnel exposes.
using System.Net;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace PcBridge.Core;

public static class Relay
{
    public const int ShotWidth = 1100;
    public const int ShotQuality = 65;

    public static int Port { get; private set; }
    private static HttpListener? _listener;
    private static AppState? _state;

    public static void Start(AppState state)
    {
        _state = state;
        Port = FreePort();
        var listener = new HttpListener();
        listener.Prefixes.Add($"http://127.0.0.1:{Port}/");
        listener.Start();
        _listener = listener;
        state.Journal.Push("relay", $"релей слушает 127.0.0.1:{Port}");

        Task.Run(async () =>
        {
            while (listener.IsListening)
            {
                HttpListenerContext context;
                try { context = await listener.GetContextAsync(); }
                catch { break; }
                _ = Task.Run(() => Handle(context));
            }
        });
    }

    public static void Stop()
    {
        try { _listener?.Stop(); } catch { }
        try { _listener?.Close(); } catch { }
    }

    private static int FreePort()
    {
        var probe = new System.Net.Sockets.TcpListener(IPAddress.Loopback, 0);
        probe.Start();
        var port = ((IPEndPoint)probe.LocalEndpoint).Port;
        probe.Stop();
        return port;
    }

    private static bool Authorized(HttpListenerRequest request)
    {
        var header = request.Headers["Authorization"];
        return header is not null && header == $"Bearer {_state!.Key}";
    }

    private static async Task Handle(HttpListenerContext context)
    {
        var request = context.Request;
        var response = context.Response;
        try
        {
            var path = request.Url?.AbsolutePath ?? "/";

            if (path == "/health")
            {
                WriteJson(response, 200, new JsonObject { ["ok"] = true, ["service"] = "pc-bridge", ["version"] = "3.0.0" });
                return;
            }
            if (!Authorized(request))
            {
                WriteJson(response, 403, new JsonObject { ["error"] = "bad token" });
                return;
            }
            if (path == "/browser/shot")
            {
                var body = await Body(request);
                var (bytes, meta) = await Browser.Shot(
                    Int(body, "quality", 80), Bool(body, "full"), body["page"]);
                response.StatusCode = 200;
                response.ContentType = "image/jpeg";
                response.ContentLength64 = bytes.Length;
                response.OutputStream.Write(bytes, 0, bytes.Length);
                response.OutputStream.Close();
                _state!.Journal.Push("ui", $"скриншот страницы → {bytes.Length / 1024} КБ");
                _ = meta;
                return;
            }

            if (path == "/shot")
            {
                // Raw JPEG: no JSON envelope, so a browser or an <img> can use it directly.
                var shot = Win32.Capture(
                    ToInt(Query(request.Url?.Query ?? "", "width"), ShotWidth),
                    ToInt(Query(request.Url?.Query ?? "", "quality"), ShotQuality));
                response.StatusCode = 200;
                response.ContentType = "image/jpeg";
                response.ContentLength64 = shot.Jpeg.Length;
                response.OutputStream.Write(shot.Jpeg, 0, shot.Jpeg.Length);
                response.OutputStream.Close();
                _state!.Journal.Push("ui", $"скриншот {shot.ScreenWidth}x{shot.ScreenHeight} → {shot.Jpeg.Length / 1024} КБ");
                return;
            }

            var (status, answer) = await Route(request, path);
            WriteJson(response, status, answer);
        }
        catch (Exception error)
        {
            try { WriteJson(response, 500, new JsonObject { ["error"] = error.Message }); } catch { }
        }
    }

    private static string? Str(JsonNode? body, string name) =>
        body?[name] is JsonValue value && value.TryGetValue<string>(out var text) ? text : null;

    private static bool Bool(JsonNode? body, string name) =>
        body?[name] is JsonValue value && value.TryGetValue<bool>(out var flag) && flag;

    private static long? Long(JsonNode? body, string name) =>
        body?[name] is JsonValue value && value.TryGetValue<long>(out var number) ? number : null;

    private static int Int(JsonNode? body, string name, int fallback) =>
        body?[name] is JsonValue value && value.TryGetValue<int>(out var number) ? number : fallback;

    private static int QueryInt(string query, string name, int fallback) =>
        int.TryParse(Query(query, name), out var value) ? value : fallback;

    private static long? Hwnd(string query) =>
        long.TryParse(Query(query, "hwnd"), out var value) ? value : null;

    private static JsonNode? UrlSelector(string query) =>
        Query(query, "page") is { } page ? JsonValue.Create(page) : null;

    /// Any exception becomes a readable answer instead of a bare 500.
    private static (int, JsonNode?) Try(Func<JsonNode> work)
    {
        try { return (200, work()); }
        catch (Exception error) { return (500, new JsonObject { ["error"] = error.Message, ["exit_code"] = 1 }); }
    }

    private static async Task<(int, JsonNode?)> TryAsync(Func<Task<JsonNode>> work)
    {
        try { return (200, await work()); }
        catch (Exception error) { return (500, new JsonObject { ["error"] = error.Message, ["exit_code"] = 1 }); }
    }

    private static int ToInt(string? text, int fallback) => int.TryParse(text, out var value) ? value : fallback;

    /// Tiny query parser: `?path=a%20b&mode=text`. No dependency on System.Web.
    private static string? Query(string query, string name)
    {
        if (query.StartsWith("?")) query = query[1..];
        foreach (var pair in query.Split('&', StringSplitOptions.RemoveEmptyEntries))
        {
            var split = pair.Split('=', 2);
            if (split.Length == 0) continue;
            if (!Uri.UnescapeDataString(split[0].Replace('+', ' ')).Equals(name, StringComparison.OrdinalIgnoreCase)) continue;
            return split.Length > 1 ? Uri.UnescapeDataString(split[1].Replace('+', ' ')) : "";
        }
        return null;
    }

    private static async Task<(int, JsonNode?)> Route(HttpListenerRequest request, string path)
    {
        var state = _state!;
        var query = request.Url?.Query ?? "";

        switch (path)
        {
            case "/ping":
                return (200, new JsonObject { ["ok"] = true, ["version"] = "3.0.0", ["ms_since_start"] = state.UptimeSeconds * 1000 });

            case "/status":
                return (200, state.Snapshot());

            case "/board":
                return (200, new JsonObject
                {
                    ["status"] = state.Snapshot(),
                    ["journal"] = JournalJson(state.Journal.Recent(30)),
                    ["windows"] = WindowsJson(state.WindowsRecent(TimeSpan.FromMilliseconds(700))),
                    ["active_window"] = WindowJson(Win32.ActiveWindow()),
                });

            case "/windows":
                return (200, new JsonObject { ["windows"] = WindowsJson(state.WindowsRecent(TimeSpan.FromMilliseconds(700))) });

            case "/journal":
                return (200, new JsonObject { ["entries"] = JournalJson(state.Journal.Recent(100)) });

            case "/jobs":
                return (200, new JsonObject { ["jobs"] = state.Jobs.List() });

            case "/files":
                return (200, await FilesUpload(request, state));

            case "/file":
                return FileDownload(Query(query, "path") ?? "", Query(query, "mode") ?? "text");

            case "/wait":
                return (200, await WaitFor(request));

            case "/note":
            {
                var answer = await Note(request, state);
                return (answer["error"] is not null ? 400 : 200, answer);
            }

            case "/result":
                return (200, new JsonObject { ["ok"] = true });

            case "/cmd":
                // "Пауза" in the window parks the bridge without dropping the tunnel.
                if (!state.Active)
                    return (503, new JsonObject { ["error"] = "мост на паузе", ["hint"] = "нажмите «Продолжить» в окне PC-Bridge" });
            {
                // A 400 from the job layer must reach the caller as a real HTTP 400,
                // otherwise a broken command looks like success.
                var answer = await Command(request, state);
                var failed = answer["error"] is not null && (answer["exit_code"]?.GetValue<int>() ?? 0) == 400;
                return (failed ? 400 : 200, answer);
            }

            // ---------- UI Automation: нажать по имени, без курсора ----------
            case "/ui/tree":
                return Try(() => Ui.Tree(
                    Query(query, "window"), Query(query, "process"), Hwnd(query),
                    QueryInt(query, "depth", 3), QueryInt(query, "limit", 400)));

            case "/ui/invoke":
            {
                var body = await Body(request);
                return Try(() => Ui.Invoke(
                    Str(body, "window"), Str(body, "process"), Long(body, "hwnd"),
                    Str(body, "name"), Str(body, "automation_id"), Str(body, "kind")));
            }

            case "/ui/set":
            {
                var body = await Body(request);
                return Try(() => Ui.SetValue(
                    Str(body, "window"), Str(body, "process"), Long(body, "hwnd"),
                    Str(body, "name"), Str(body, "automation_id"), Str(body, "value") ?? ""));
            }

            case "/ui/read":
            {
                var body = await Body(request);
                return Try(() => Ui.Read(
                    Str(body, "window"), Str(body, "process"), Long(body, "hwnd"),
                    Str(body, "name"), Str(body, "automation_id")));
            }

            // ---------- браузер через DevTools Protocol ----------
            case "/browser/start":
            {
                var body = await Body(request);
                return await TryAsync(() => Browser.Start(
                    QueryInt(query, "port", Int(body, "port", 9333)),
                    Bool(body, "headless"), Bool(body, "user_profile"), Str(body, "browser") ?? "chrome"));
            }

            case "/browser/status":
                return await TryAsync(Browser.Status);

            case "/browser/pages":
                return await TryAsync(Browser.ListPages);

            case "/browser/open":
            {
                var body = await Body(request);
                var url = Str(body, "url") ?? "about:blank";
                return await TryAsync(() => Browser.Open(url, Bool(body, "background")));
            }

            case "/browser/navigate":
            {
                var body = await Body(request);
                return await TryAsync(() => Browser.Navigate(Str(body, "url") ?? "about:blank", body["page"]));
            }

            case "/browser/eval":
            {
                var body = await Body(request);
                var expression = Str(body, "expression") ?? Str(body, "js");
                if (string.IsNullOrWhiteSpace(expression))
                    return (400, new JsonObject { ["error"] = "нужен expression", ["expected"] = "{\"expression\":\"document.title\"}", ["exit_code"] = 400 });
                return await TryAsync(() => Browser.Evaluate(expression, body["page"]));
            }

            case "/browser/click":
            {
                var body = await Body(request);
                return await TryAsync(() => Browser.Click(
                    Str(body, "selector"),
                    body["x"]?.GetValue<double>(), body["y"]?.GetValue<double>(), body["page"],
                    Bool(body, "real")));
            }

            case "/browser/type":
            {
                var body = await Body(request);
                return await TryAsync(() => Browser.Type(Str(body, "selector"), Str(body, "text") ?? "", Bool(body, "replace"), body["page"]));
            }

            case "/browser/key":
            {
                var body = await Body(request);
                var keys = (body["keys"] as JsonArray)?.Select(k => k?.GetValue<string>() ?? "").ToArray()
                           ?? new[] { Str(body, "key") ?? "" };
                return await TryAsync(() => Browser.Key(keys.Where(k => k.Length > 0).ToArray(), body["page"]));
            }

            case "/browser/text":
            {
                var body = await Body(request);
                return await TryAsync(() => Browser.Text(Str(body, "selector") ?? Query(query, "selector"), body["page"] ?? UrlSelector(query)));
            }

            case "/browser/wait":
            {
                var body = await Body(request);
                return await TryAsync(() => Browser.WaitFor(
                    Str(body, "selector"), Str(body, "text"), Int(body, "timeout", 30), body["page"]));
            }

            default:
                if (path.StartsWith("/job/"))
                {
                    var id = path["/job/".Length..];
                    var job = state.Jobs.Get(id);
                    return job is null ? (404, new JsonObject { ["error"] = "unknown job id", ["job_id"] = id }) : (200, job);
                }
                return (404, new JsonObject { ["error"] = "unknown path", ["path"] = path });
        }
    }

    // ---------- helpers ----------
    private static JsonArray JournalJson(IEnumerable<JournalEntry> entries)
    {
        var array = new JsonArray();
        foreach (var entry in entries)
        {
            array.Add(new JsonObject
            {
                ["seq"] = entry.Seq,
                ["time"] = entry.Time,
                ["kind"] = entry.Kind,
                ["text"] = entry.Text,
            });
        }
        return array;
    }

    private static JsonObject WindowJson(WindowInfo window) => new()
    {
        ["hwnd"] = window.Hwnd,
        ["title"] = window.Title,
        ["pid"] = window.Pid,
        ["process"] = window.Process,
    };

    private static JsonArray WindowsJson(IEnumerable<WindowInfo> windows)
    {
        var array = new JsonArray();
        foreach (var window in windows) array.Add(WindowJson(window));
        return array;
    }

    private static async Task<JsonNode> Body(HttpListenerRequest request)
    {
        using var stream = new MemoryStream();
        await request.InputStream.CopyToAsync(stream);
        if (stream.Length == 0) return new JsonObject();
        try { return JsonNode.Parse(stream.ToArray()) ?? new JsonObject(); }
        catch { return new JsonObject(); }
    }

    private static void WriteJson(HttpListenerResponse response, int status, JsonNode? body)
    {
        var bytes = Encoding.UTF8.GetBytes(body?.ToJsonString() ?? "{}");
        response.StatusCode = status;
        response.ContentType = "application/json; charset=utf-8";
        response.ContentLength64 = bytes.Length;
        response.OutputStream.Write(bytes, 0, bytes.Length);
        response.OutputStream.Close();
    }

    private static async Task<JsonNode> Note(HttpListenerRequest request, AppState state)
    {
        var payload = await Body(request);
        var text = payload["text"]?.GetValue<string>()?.Trim();
        if (string.IsNullOrEmpty(text))
            return new JsonObject { ["error"] = "text is required", ["exit_code"] = 400, ["expected"] = "{\"text\":\"что делаю\"}" };
        var kind = payload["kind"]?.GetValue<string>() ?? "note";
        if (kind is not ("stage" or "step" or "result" or "error")) kind = "note";
        if (kind == "stage") state.SetStage(text);
        var sequence = state.BumpNotes();
        state.Journal.Push(kind, kind == "stage" ? text : $"{sequence:00} · {text}");
        return new JsonObject { ["ok"] = true, ["sequence"] = sequence, ["kind"] = kind };
    }

    private static async Task<JsonNode> FilesUpload(HttpListenerRequest request, AppState state)
    {
        var payload = await Body(request);
        var items = payload["files"] as JsonArray ?? new JsonArray { payload };
        var written = new JsonArray();
        foreach (var item in items)
        {
            var path = item?["path"]?.GetValue<string>() ?? "";
            if (path.Length == 0) { written.Add(new JsonObject { ["ok"] = false, ["error"] = "path is required" }); continue; }
            try
            {
                byte[] bytes;
                if (item!["text"] is { } textNode) bytes = Encoding.UTF8.GetBytes(textNode.GetValue<string>());
                else if (item["data"] is { } dataNode) bytes = Convert.FromBase64String(dataNode.GetValue<string>());
                else { written.Add(new JsonObject { ["path"] = path, ["ok"] = false, ["error"] = "either 'text' or 'data' is required" }); continue; }

                var parent = Path.GetDirectoryName(path);
                if (!string.IsNullOrEmpty(parent)) Directory.CreateDirectory(parent);
                File.WriteAllBytes(path, bytes);
                state.Journal.Push("ui", $"записан файл {path} ({bytes.Length} байт)");
                written.Add(new JsonObject { ["path"] = path, ["ok"] = true, ["bytes"] = bytes.Length });
            }
            catch (Exception error)
            {
                written.Add(new JsonObject { ["path"] = path, ["ok"] = false, ["error"] = error.Message });
            }
        }
        return new JsonObject { ["files"] = written, ["count"] = written.Count };
    }

    private static (int, JsonNode?) FileDownload(string path, string mode)
    {
        if (path.Length == 0) return (400, new JsonObject { ["error"] = "path is required" });
        if (!File.Exists(path)) return (404, new JsonObject { ["error"] = "no such file", ["path"] = path });
        var bytes = File.ReadAllBytes(path);
        if (mode == "base64")
            return (200, new JsonObject { ["path"] = path, ["bytes"] = bytes.Length, ["data"] = Convert.ToBase64String(bytes) });
        return (200, new JsonObject { ["path"] = path, ["bytes"] = bytes.Length, ["text"] = ConsoleText.Decode(bytes) });
    }

    private static async Task<JsonNode> WaitFor(HttpListenerRequest request)
    {
        var payload = await Body(request);
        var timeout = Math.Clamp(payload["timeout"]?.GetValue<int>() ?? 30, 1, 120);
        var started = DateTime.UtcNow;
        Func<bool> probe;

        if (payload["port"] is { } portNode)
        {
            var port = portNode.GetValue<int>();
            probe = () =>
            {
                using var client = new System.Net.Sockets.TcpClient();
                try { return client.ConnectAsync(IPAddress.Loopback, port).Wait(300); } catch { return false; }
            };
        }
        else if (payload["process"] is { } processNode)
        {
            var needle = processNode.GetValue<string>().ToLowerInvariant();
            probe = () => Win32.Windows().Any(w => w.Process.ToLowerInvariant().Contains(needle));
        }
        else if (payload["window"] is { } windowNode)
        {
            var needle = windowNode.GetValue<string>().ToLowerInvariant();
            probe = () => Win32.Windows().Any(w => w.Title.ToLowerInvariant().Contains(needle));
        }
        else if (payload["file"] is { } fileNode)
        {
            var file = fileNode.GetValue<string>();
            probe = () => File.Exists(file) || Directory.Exists(file);
        }
        else
        {
            return new JsonObject
            {
                ["error"] = "wait needs one of: port, process, window, file",
                ["expected"] = "{\"port\":9333,\"timeout\":30}",
                ["exit_code"] = 400,
            };
        }

        while ((DateTime.UtcNow - started).TotalSeconds < timeout)
        {
            if (probe()) return new JsonObject { ["ok"] = true, ["waited_ms"] = (long)(DateTime.UtcNow - started).TotalMilliseconds };
            await Task.Delay(250);
        }
        return new JsonObject
        {
            ["ok"] = false,
            ["waited_ms"] = (long)(DateTime.UtcNow - started).TotalMilliseconds,
            ["exit_code"] = 124,
            ["error"] = "condition did not become true in time",
        };
    }

    private static async Task<JsonNode> Command(HttpListenerRequest request, AppState state)
    {
        var payload = await Body(request);
        var started = DateTime.UtcNow;
        state.BumpCommands();

        if (payload["note"]?.GetValue<string>() is { Length: > 0 } note)
        {
            var noteKind = payload["note_kind"]?.GetValue<string>() ?? "note";
            state.Journal.Push(noteKind, note);
        }

        // `background` means two different things: a job for plain commands, background
        // input for gui. Keeping one word is friendlier than adding a second flag.
        var guiBackground = payload["background"]?.GetValue<bool>() == true && payload["type"]?.GetValue<string>() == "gui";
        var background = payload["background"]?.GetValue<bool>() == true && !guiBackground;
        if (background)
        {
            var id = state.Jobs.Start();
            _ = Task.Run(async () =>
            {
                var answer = await RunJob(payload, state);
                state.Jobs.Finish(id, answer["output"]?.GetValue<string>() ?? "", answer["exit_code"]?.GetValue<int>() ?? -1);
            });
            return new JsonObject
            {
                ["job_id"] = id,
                ["status"] = "running",
                ["poll"] = $"/job/{id}",
                ["hint"] = "опрос /job/<id> каждые 2–5 секунд",
            };
        }

        var result = await RunJob(payload, state);
        result["took_ms"] = (long)(DateTime.UtcNow - started).TotalMilliseconds;
        return result;
    }

    private static async Task<JsonNode> RunJob(JsonNode payload, AppState state)
    {
        var kind = payload["type"]?.GetValue<string>() ?? "shell";

        switch (kind)
        {
            case "shell":
            {
                var command = payload["command"]?.GetValue<string>() ?? "";
                if (command.Trim().Length == 0)
                    return new JsonObject
                    {
                        ["error"] = "no command",
                        ["exit_code"] = 400,
                        ["expected"] = "{\"command\":\"dir C:\\\\\",\"shell\":\"cmd|powershell\",\"timeout\":60}",
                    };
                var shell = payload["shell"]?.GetValue<string>();
                var timeout = payload["timeout"] is { } timeoutNode ? TimeSpan.FromSeconds(timeoutNode.GetValue<int>()) : (TimeSpan?)null;
                state.Journal.Push("shell", "$ " + command);
                var result = await Task.Run(() => Shell.Run(command, shell, timeout));
                state.Journal.Push("shell", $"exit={result.ExitCode} · {Trim(result.Output, 240)}");
                if (result.Warning is not null) state.Journal.Push("error", result.Warning);
                return new JsonObject
                {
                    ["output"] = result.Output,
                    ["exit_code"] = result.ExitCode,
                    ["host"] = state.Hostname,
                    ["warning"] = result.Warning,
                };
            }
            case "batch":
            {
                var commands = payload["commands"] as JsonArray ?? new JsonArray();
                if (commands.Count == 0) return new JsonObject { ["error"] = "batch needs 'commands'", ["exit_code"] = 400 };
                var shell = payload["shell"]?.GetValue<string>();
                var timeout = payload["timeout"] is { } timeoutNode ? TimeSpan.FromSeconds(timeoutNode.GetValue<int>()) : (TimeSpan?)null;
                var results = new JsonArray();
                var last = 0;
                foreach (var item in commands)
                {
                    var command = item?.GetValue<string>() ?? "";
                    var single = await Task.Run(() => Shell.Run(command, shell, timeout));
                    last = single.ExitCode;
                    results.Add(new JsonObject { ["command"] = command, ["output"] = single.Output, ["exit_code"] = last });
                }
                return new JsonObject { ["results"] = results, ["count"] = results.Count, ["exit_code"] = last };
            }
            case "screenshot":
            {
                var width = payload["width"]?.GetValue<int>() ?? ShotWidth;
                var quality = payload["quality"]?.GetValue<int>() ?? ShotQuality;
                var shot = await Task.Run(() => Win32.Capture(width, quality));
                var active = Win32.ActiveWindow();
                return new JsonObject
                {
                    ["image_base64"] = Convert.ToBase64String(shot.Jpeg),
                    ["screen_width"] = shot.ScreenWidth,
                    ["screen_height"] = shot.ScreenHeight,
                    ["active_window"] = WindowJson(active),
                    ["exit_code"] = 0,
                };
            }
            case "gui":
            {
                var actions = payload["actions"] as JsonArray ?? new JsonArray();
                if (actions.Count == 0) return new JsonObject { ["error"] = "gui needs 'actions'", ["exit_code"] = 400 };

                // background: messages go straight to the window — no cursor, no focus, the user
                // keeps typing in their own window. Works for classic apps; browsers need /browser.
                // Inside a gui job `background` can only mean background input.
                if (payload["mode"]?.GetValue<string>() == "background" || payload["background"]?.GetValue<bool>() == true)
                {
                    var handle = Background.Resolve(
                        payload["window"]?.GetValue<string>(),
                        payload["process"]?.GetValue<string>(),
                        payload["hwnd"] is { } hwndNode ? hwndNode.GetValue<long>() : null);
                    var restore = payload["restore"]?.GetValue<bool>() == true;
                    var (backgroundCode, backgroundOutput) = await Task.Run(
                        () => Background.Run(handle, payload["window"]?.GetValue<string>(), payload["process"]?.GetValue<string>(), actions, restore));
                    return new JsonObject
                    {
                        ["output"] = backgroundOutput,
                        ["exit_code"] = backgroundCode,
                        ["mode"] = "background",
                        ["host"] = state.Hostname,
                    };
                }

                var (code, output, done) = await Task.Run(() => Input.Run(actions));
                return new JsonObject { ["output"] = output, ["exit_code"] = code, ["actions_done"] = done, ["host"] = state.Hostname };
            }
            default:
                return new JsonObject
                {
                    ["error"] = $"unknown type: {kind}",
                    ["exit_code"] = 400,
                    ["expected"] = "{\"command\":\"…\"} or {\"type\":\"screenshot|gui|batch\"}",
                };
        }
    }

    private static string Trim(string text, int limit)
        => text.Length <= limit ? text : text[..limit] + "…";
}
