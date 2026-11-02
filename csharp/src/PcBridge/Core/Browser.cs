// Browser control over the Chrome DevTools Protocol: a separate browser process with its own
// profile, driven by TCP — the cursor, the keyboard and the user's own tabs are untouched.
using System.Net.WebSockets;

namespace PcBridge.Core;

public static class Browser
{
    // System proxy must not intercept 127.0.0.1, otherwise the DevTools port looks dead.
    private static readonly HttpClient Http = new(new HttpClientHandler { UseProxy = false })
    {
        Timeout = TimeSpan.FromSeconds(15),
    };
    private static readonly Dictionary<string, Page> Sessions = new();
    private static int _port = 9333;

    public static int Port => _port;

    // ---------- a page session ----------
    private sealed class Page : IDisposable
    {
        public string TargetId { get; init; } = "";
        public string Url { get; set; } = "";
        private readonly ClientWebSocket _socket = new();
        private readonly Dictionary<int, TaskCompletionSource<JsonObject>> _pending = new();
        private readonly MemoryStream _buffer = new();
        private int _next;
        private bool _closed;

        public static async Task<Page> ConnectAsync(string targetId, string webSocketUrl)
        {
            var page = new Page { TargetId = targetId };
            await page._socket.ConnectAsync(new Uri(webSocketUrl), CancellationToken.None);
            _ = Task.Run(page.Receive);
            return page;
        }

        private async Task Receive()
        {
            var chunk = new byte[1 << 16];
            try
            {
                while (_socket.State == WebSocketState.Open)
                {
                    var result = await _socket.ReceiveAsync(chunk, CancellationToken.None);
                    if (result.MessageType == WebSocketMessageType.Close) break;
                    _buffer.Write(chunk, 0, result.Count);
                    if (!result.EndOfMessage) continue;
                    var text = Encoding.UTF8.GetString(_buffer.ToArray());
                    _buffer.SetLength(0);
                    Dispatch(text);
                }
            }
            catch { /* the page went away */ }
            finally { _closed = true; }
        }

        private void Dispatch(string text)
        {
            JsonObject? message;
            try { message = JsonNode.Parse(text) as JsonObject; } catch { return; }
            if (message is null) return;
            if (message["id"]?.GetValue<int>() is not { } id) return;
            if (!_pending.Remove(id, out var waiter)) return;
            waiter.TrySetResult(message);
        }

        public async Task<JsonObject> Send(string method, JsonObject? parameters = null, int timeoutSeconds = 60)
        {
            if (_closed) throw new InvalidOperationException("сессия страницы закрыта");
            var id = Interlocked.Increment(ref _next);
            var waiter = new TaskCompletionSource<JsonObject>(TaskCreationOptions.RunContinuationsAsynchronously);
            _pending[id] = waiter;

            var payload = new JsonObject { ["id"] = id, ["method"] = method };
            if (parameters is not null) payload["params"] = parameters;
            var bytes = Encoding.UTF8.GetBytes(payload.ToJsonString());
            try
            {
                await _socket.SendAsync(bytes, WebSocketMessageType.Text, true, CancellationToken.None);
            }
            catch (Exception error)
            {
                _pending.Remove(id);
                throw new InvalidOperationException("не удалось отправить команду: " + error.Message);
            }

            var finished = await Task.WhenAny(waiter.Task, Task.Delay(TimeSpan.FromSeconds(timeoutSeconds)));
            if (finished != waiter.Task)
            {
                _pending.Remove(id);
                throw new TimeoutException($"{method} не ответил за {timeoutSeconds} с");
            }
            return await waiter.Task;
        }

        public async Task<JsonObject> Evaluate(string expression, int timeoutSeconds = 60)
        {
            var answer = await Send("Runtime.evaluate", new JsonObject
            {
                ["expression"] = expression,
                ["returnByValue"] = true,
                ["awaitPromise"] = true,
                ["userGesture"] = true,
            }, timeoutSeconds);

            if (answer["result"]?["exceptionDetails"] is JsonObject problem)
                throw new InvalidOperationException("ошибка в странице: " + (problem["text"]?.GetValue<string>() ?? "unknown"));
            return answer["result"]?["result"] as JsonObject ?? new JsonObject();
        }

        public void Dispose()
        {
            try { _socket.Dispose(); } catch { }
        }
    }

    // ---------- discovery ----------
    private static string? _base;

    /// Chrome may bind its DevTools socket to ::1 and something else may already own
    /// 127.0.0.1:9333, so both hosts and a range of ports are probed before anything
    /// is called "dead".
    private static async Task<bool> LooksLikeDevTools(string baseUrl)
    {
        // A probe must be cheap: a dead port has to answer "no" in a fraction of a second,
        // otherwise /browser/status can hang for minutes and the tunnel drops the request.
        using var timeout = new CancellationTokenSource(TimeSpan.FromMilliseconds(700));
        try
        {
            using var response = await Http.GetAsync(baseUrl + "/json/version", timeout.Token);
            if (!response.IsSuccessStatusCode) return false;
            var text = await response.Content.ReadAsStringAsync(timeout.Token);
            return text.Contains("webSocketDebuggerUrl", StringComparison.OrdinalIgnoreCase);
        }
        catch { return false; }
    }

    private static DateTime _noEndpointUntil = DateTime.MinValue;
    private static readonly List<int> KnownPorts = LoadPorts();

    private static string PortFile => Path.Combine(Paths.DataDir, "browser-port.txt");

    private static List<int> LoadPorts()
    {
        var ports = new List<int> { 9333 };
        try
        {
            if (File.Exists(PortFile) && int.TryParse(File.ReadAllText(PortFile).Trim(), out var saved) &&
                saved > 1024 && !ports.Contains(saved)) ports.Insert(0, saved);
        }
        catch { }
        return ports;
    }

    private static void SavePort(int port)
    {
        try { File.WriteAllText(PortFile, port.ToString()); } catch { }
    }

    private static async Task<string?> ResolveBase(bool rescan = false)
    {
        if (_base is not null && !rescan) return _base;
        // Nothing answered a moment ago: do not make every request pay for the scan again.
        if (!rescan && DateTime.UtcNow < _noEndpointUntil) return null;

        foreach (var port in KnownPorts.Distinct().ToList())
        {
            foreach (var host in new[] { "127.0.0.1", "[::1]" })
            {
                // A TCP check first: an HTTP connect to a dead port can hang for a second
                // on Windows, and scanning 13 ports twice would cost the caller 17 s.
                if (!await Accepts(host.Trim('[', ']'), port)) continue;
                var candidate = $"http://{host}:{port}";
                if (await LooksLikeDevTools(candidate))
                {
                    _base = candidate;
                    _port = port;
                    if (!KnownPorts.Contains(port)) KnownPorts.Insert(0, port);
                    SavePort(port);
                    _noEndpointUntil = DateTime.MinValue;
                    return _base;
                }
            }
        }

        // Last resort: ask the running browser processes which port they opened. This is how
        // the bridge re-attaches after its own restart, when nothing remembered the port.
        if (DiscoverPort() is { } discovered)
        {
            foreach (var host in new[] { "127.0.0.1", "[::1]" })
            {
                var candidate = $"http://{host}:{discovered}";
                if (!await LooksLikeDevTools(candidate)) continue;
                _base = candidate;
                _port = discovered;
                SavePort(discovered);
                if (!KnownPorts.Contains(discovered)) KnownPorts.Insert(0, discovered);
                return _base;
            }
        }

        _noEndpointUntil = DateTime.UtcNow.AddSeconds(20);
        return null;
    }

    private static async Task<bool> Accepts(string host, int port)
    {
        using var timeout = new CancellationTokenSource(TimeSpan.FromMilliseconds(200));
        try
        {
            using var client = new System.Net.Sockets.TcpClient();
            await client.ConnectAsync(host, port, timeout.Token);
            return true;
        }
        catch { return false; }
    }

    private static int? DiscoverPort()
    {
        var pids = RunPidsRaw("Get-CimInstance Win32_Process -Filter \"Name='chrome.exe'\" | " +
                              "Where-Object { $_.CommandLine -like '*--remote-debugging-port=*' -and $_.CommandLine -like '*PC-Bridge*' } | " +
                              "ForEach-Object { $_.CommandLine }");
        foreach (var line in pids)
        {
            var match = System.Text.RegularExpressions.Regex.Match(line, @"--remote-debugging-port=(\d{2,5})");
            if (match.Success && int.TryParse(match.Groups[1].Value, out var port)) return port;
        }
        return null;
    }

    private static bool PortFree(int port)
    {
        try
        {
            var probe = new System.Net.Sockets.TcpListener(System.Net.IPAddress.Loopback, port);
            probe.Start();
            probe.Stop();
            return true;
        }
        catch { return false; }
    }

    private static async Task<string?> HttpText(string path)
    {
        if (await ResolveBase() is not { } root) return null;
        try { return await Http.GetStringAsync(root + path); }
        catch { return null; }
    }

    private static async Task<JsonArray> TargetsAsync()
    {
        var text = await HttpText("/json/list");
        if (text is null) return new JsonArray();
        var array = new JsonArray();
        if (JsonNode.Parse(text) is not JsonArray list) return array;
        foreach (var item in list.OfType<JsonObject>())
        {
            if (item["type"]?.GetValue<string>() != "page") continue;
            array.Add(new JsonObject
            {
                ["id"] = item["id"]?.GetValue<string>(),
                ["title"] = item["title"]?.GetValue<string>(),
                ["url"] = item["url"]?.GetValue<string>(),
                ["ws"] = item["webSocketDebuggerUrl"]?.GetValue<string>(),
            });
        }
        return array;
    }

    private static async Task<Page> PickAsync(JsonNode? selector)
    {
        var targets = await TargetsAsync();
        if (targets.Count == 0) throw new InvalidOperationException("нет открытых страниц — POST /browser/start");

        JsonObject? chosen = null;
        var wanted = selector?.GetValueKind() switch
        {
            System.Text.Json.JsonValueKind.Number => selector.GetValue<int>(),
            System.Text.Json.JsonValueKind.String => -1,
            _ => 0,
        };
        string? wantedText = selector?.GetValueKind() == System.Text.Json.JsonValueKind.String ? selector.GetValue<string>() : null;

        var index = 0;
        foreach (var item in targets.OfType<JsonObject>())
        {
            var id = item["id"]?.GetValue<string>() ?? "";
            var url = item["url"]?.GetValue<string>() ?? "";
            if (wantedText is not null && (id == wantedText || url.Contains(wantedText, StringComparison.OrdinalIgnoreCase))) { chosen = item; break; }
            if (wantedText is null && index == wanted) { chosen = item; break; }
            index++;
        }
        chosen ??= targets[0] as JsonObject;
        if (chosen is null) throw new InvalidOperationException("страница не выбрана");

        var targetId = chosen["id"]!.GetValue<string>();
        if (Sessions.TryGetValue(targetId, out var cached)) return cached;

        var page = await Page.ConnectAsync(targetId, chosen["ws"]!.GetValue<string>());
        page.Url = chosen["url"]?.GetValue<string>() ?? "";
        Sessions[targetId] = page;
        return page;
    }

    // ---------- lifecycle ----------
    private static string? FindExecutable(string kind)
    {
        var roots = new[]
        {
            Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles),
            Environment.GetFolderPath(Environment.SpecialFolder.ProgramFilesX86),
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
        };
        var chrome = new[] { @"Google\Chrome\Application\chrome.exe", @"Chromium\Application\chrome.exe" };
        var edge = new[] { @"Microsoft\Edge\Application\msedge.exe" };

        var wanted = kind == "edge" ? edge : chrome;
        foreach (var root in roots)
        {
            if (string.IsNullOrEmpty(root)) continue;
            foreach (var tail in wanted)
            {
                var path = Path.Combine(root, tail);
                if (File.Exists(path)) return path;
            }
        }
        if (kind != "edge") return FindExecutable("edge");
        return null;
    }

    [System.Runtime.InteropServices.DllImport("user32.dll")]
    private static extern bool PostMessage(IntPtr hwnd, int message, IntPtr w, IntPtr l);

    private static string ProcessName(string kind) => kind == "edge" ? "msedge" : "chrome";

    /// Where the browser keeps the real profile the user is signed in to.
    private static string UserDataDir(string kind) => kind == "edge"
        ? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Microsoft", "Edge", "User Data")
        : Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Google", "Chrome", "User Data");

    /// Chrome 136+ refuses to open the debug port when --user-data-dir points at the default
    /// profile, so the same folder is reached through a junction: a different path, the same
    /// logins, bookmarks and extensions.
    private static string JunctionPath(string kind) =>
        Path.Combine(Paths.DataDir, kind == "edge" ? "edge-user" : "chrome-user");

    private static string? EnsureJunction(string link, string target)
    {
        if (Directory.Exists(link)) return null;
        if (!Directory.Exists(target)) return "профиль не найден: " + target;

        // cmd does not understand backslash-escaped quotes, so the whole tail is passed as
        // one argument string instead of ArgumentList.
        var info = new System.Diagnostics.ProcessStartInfo("cmd.exe")
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardError = true,
        };
        info.Arguments = "/c mklink /J \"" + link + "\" \"" + target + "\"";

        using var process = System.Diagnostics.Process.Start(info) ?? throw new InvalidOperationException("mklink не запустился");
        var error = ConsoleText.Decode(process.StandardError.BaseStream is { } stream
            ? ReadAll(stream)
            : Array.Empty<byte>());
        process.WaitForExit(15000);
        return Directory.Exists(link) ? null : "не удалось создать junction: " + error.Trim();
    }

    private static byte[] ReadAll(Stream stream)
    {
        using var buffer = new MemoryStream();
        stream.CopyTo(buffer);
        return buffer.ToArray();
    }

    private static bool IsRunning(string kind) =>
        System.Diagnostics.Process.GetProcessesByName(ProcessName(kind)).Length > 0;

    /// Asks the running browser to close the way a click on ✕ does, giving it time to save
    /// the session, instead of killing it.
    private static bool CloseGracefully(string kind, int waitSeconds)
    {
        var deadline = DateTime.UtcNow.AddSeconds(waitSeconds);
        foreach (var process in System.Diagnostics.Process.GetProcessesByName(ProcessName(kind)))
        {
            try { if (process.MainWindowHandle != IntPtr.Zero) PostMessage(process.MainWindowHandle, 0x0010 /*WM_CLOSE*/, IntPtr.Zero, IntPtr.Zero); }
            catch { }
        }
        while (DateTime.UtcNow < deadline)
        {
            if (!IsRunning(kind)) return true;
            Thread.Sleep(300);
        }
        return !IsRunning(kind);
    }

    private static string ModeFile => Path.Combine(Paths.DataDir, "browser-mode.txt");

    /// Which profile the browser on the debug port belongs to: "bridge" or "user".
    private static string? CurrentMode()
    {
        try { return File.Exists(ModeFile) ? File.ReadAllText(ModeFile).Trim() : null; }
        catch { return null; }
    }

    private static void RememberMode(string mode)
    {
        try { File.WriteAllText(ModeFile, mode); } catch { }
    }

    /// Only processes started with a given --user-data-dir: the user's own browser and ours
    /// are both "chrome.exe", so the command line is the only honest way to tell them apart.
    /// Every browser process except the ones started by the bridge. A normally started
    /// Chrome has no --user-data-dir at all, so matching on "User Data" misses it.
    private static List<int> FindUserBrowser(string kind) =>
        RunPids("Get-CimInstance Win32_Process -Filter \"Name='" + ProcessName(kind) + ".exe'\" | " +
                "Where-Object { $_.CommandLine -notlike '*PC-Bridge*' } | " +
                "ForEach-Object { $_.ProcessId }");

    private static List<int> FindByProfile(string kind, string needle) =>
        RunPids("Get-CimInstance Win32_Process -Filter \"Name='" + ProcessName(kind) + ".exe'\" | " +
                "Where-Object { $_.CommandLine -like '*" + needle + "*' } | " +
                "ForEach-Object { $_.ProcessId }");

    private static List<int> RunPids(string script) =>
        RunPidsRaw(script).Select(line => int.TryParse(line.Trim(), out var pid) ? pid : 0)
            .Where(pid => pid > 0).Distinct().ToList();

    private static List<string> RunPidsRaw(string script)
    {
        var info = new System.Diagnostics.ProcessStartInfo("powershell")
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        };
        info.ArgumentList.Add("-NoProfile");
        info.ArgumentList.Add("-Command");
        info.ArgumentList.Add(script);
        using var process = System.Diagnostics.Process.Start(info);
        if (process is null) return new List<string>();
        var output = process.StandardOutput.ReadToEnd();
        process.WaitForExit(20000);
        return output.Split('\n', StringSplitOptions.RemoveEmptyEntries).ToList();
    }

    private static bool ClosePids(IEnumerable<int> pids, int waitSeconds)
    {
        var list = pids.ToList();
        foreach (var pid in list)
        {
            try { System.Diagnostics.Process.GetProcessById(pid).CloseMainWindow(); } catch { }
        }
        var deadline = DateTime.UtcNow.AddSeconds(waitSeconds);
        while (DateTime.UtcNow < deadline)
        {
            var alive = list.Where(pid => { try { System.Diagnostics.Process.GetProcessById(pid); return true; } catch { return false; } }).ToList();
            if (alive.Count == 0) return true;
            Thread.Sleep(300);
        }
        foreach (var pid in list)
        {
            try { System.Diagnostics.Process.GetProcessById(pid).Kill(); } catch { }
        }
        Thread.Sleep(500);
        return true;
    }

    public static async Task<JsonNode> Start(int port, bool headless, bool userProfile, string kind, bool restart, string[]? extensions)
    {
        _port = port is > 1024 and < 65535 ? port : 9333;

        var wantedMode = userProfile ? "user" : "bridge";

        if (await ResolveBase(rescan: true) is { } existing)
        {
            if (CurrentMode() == wantedMode)
            {
                var version = JsonNode.Parse(await HttpText("/json/version") ?? "{}") as JsonObject;
                return new JsonObject
                {
                    ["ok"] = true,
                    ["already_running"] = true,
                    ["port"] = _port,
                    ["endpoint"] = existing,
                    ["profile"] = wantedMode == "user" ? "ваш" : "отдельный",
                    ["browser"] = version?["Browser"]?.GetValue<string>(),
                };
            }

            // An automation browser from the other mode is holding the port; it is ours to close.
            var others = FindByProfile(kind, "PC-Bridge");
            if (others.Count > 0) ClosePids(others, 10);
            _base = null;
            await Task.Delay(700);
        }

        var exe = FindExecutable(kind);
        if (exe is null)
            return new JsonObject { ["ok"] = false, ["exit_code"] = 404, ["error"] = "не нашёл Chrome или Edge" };

        var chosen = 0;
        for (var candidate = _port; candidate < _port + 20 && chosen == 0; candidate++)
            if (PortFree(candidate)) chosen = candidate;
        if (chosen == 0)
            return new JsonObject { ["ok"] = false, ["exit_code"] = 1, ["error"] = "нет свободного порта рядом с " + _port };
        _port = chosen;
        if (!KnownPorts.Contains(chosen)) KnownPorts.Add(chosen);

        var arguments = new List<string>
        {
            $"--remote-debugging-port={_port}",
            "--remote-allow-origins=*",
            "--no-first-run",
            "--no-default-browser-check",
        };

        if (userProfile)
        {
            // The signed-in profile, reached through a junction so Chrome still opens the
            // debug port. Chrome holds a lock on a profile in use, so it must not be running.
            var link = JunctionPath(kind);
            var error = EnsureJunction(link, UserDataDir(kind));
            if (error is not null) return new JsonObject { ["ok"] = false, ["exit_code"] = 1, ["error"] = error };

            var userBrowser = FindUserBrowser(kind); // their windows: everything that is not ours
            if (userBrowser.Count > 0)
            {
                if (!restart)
                    return new JsonObject
                    {
                        ["ok"] = false,
                        ["exit_code"] = 409,
                        ["error"] = "браузер уже запущен, а с занятым профилем порт не откроется",
                        ["processes"] = userBrowser.Count,
                        ["how_to"] = "передайте \"restart\": true — окна закроются штатно, а после запуска вернутся (--restore-last-session)",
                    };
                ClosePids(userBrowser, 20);
                for (var attempt = 0; attempt < 20; attempt++)
                {
                    if (FindUserBrowser(kind).Count == 0) break;
                    await Task.Delay(500);
                }
                if (FindUserBrowser(kind).Count > 0)
                    return new JsonObject { ["ok"] = false, ["exit_code"] = 409, ["error"] = "браузер не закрылся — закройте окна вручную" };
                await Task.Delay(1200); // профиль отпускает lockfile не мгновенно
            }
            arguments.Add("--user-data-dir=" + link);
            arguments.Add("--profile-directory=Default");
            arguments.Add("--restore-last-session");
        }
        else
        {
            // A dedicated profile: the user's own logins, cookies and tabs are not touched.
            var profileDir = Path.Combine(Paths.DataDir, "browser");
            Directory.CreateDirectory(profileDir);
            arguments.Add("--user-data-dir=" + profileDir);
            arguments.Add("--new-window");
        }

        if (extensions is { Length: > 0 })
        {
            // Unpacked extensions for the automation profile: both flags are required in
            // current Chrome, otherwise --load-extension is ignored.
            var list = string.Join(",", extensions);
            arguments.Add("--disable-extensions-except=" + list);
            arguments.Add("--load-extension=" + list);
        }
        if (headless) arguments.Add("--headless=new");
        // Their session comes back from --restore-last-session; a URL argument would replace it.
        if (!userProfile) arguments.Add("about:blank");

        var info = new System.Diagnostics.ProcessStartInfo(exe)
        {
            UseShellExecute = false,
            CreateNoWindow = true,
        };
        foreach (var argument in arguments) info.ArgumentList.Add(argument);
        try { System.Diagnostics.Process.Start(info); }
        catch (Exception error)
        {
            return new JsonObject { ["ok"] = false, ["exit_code"] = 1, ["error"] = "не удалось запустить: " + error.Message };
        }

        for (var i = 0; i < 40; i++)
        {
            await Task.Delay(500);
            if (await ResolveBase(rescan: true) is { } ready)
            {
                RememberMode(wantedMode);
                SavePort(_port);
                var version = JsonNode.Parse(await HttpText("/json/version") ?? "{}") as JsonObject;
                return new JsonObject
                {
                    ["ok"] = true,
                    ["port"] = _port,
                    ["endpoint"] = ready,
                    ["browser"] = version?["Browser"]?.GetValue<string>(),
                    ["profile"] = userProfile ? "ваш (через junction)" : "отдельный (PC-Bridge\\browser)",
                    ["headless"] = headless,
                    ["exe"] = exe,
                };
            }
        }
        return new JsonObject
        {
            ["ok"] = false,
            ["exit_code"] = 124,
            ["error"] = "за 20 с DevTools не ответил",
            ["port"] = _port,
            ["hint"] = userProfile ? "закройте все окна Chrome и попробуйте снова, либо profile=bridge" : null,
        };
    }

    public static async Task<JsonNode> Status()
    {
        if (await HttpText("/json/version") is not { } text)
            return new JsonObject { ["ok"] = false, ["exit_code"] = 503, ["error"] = "браузер не запущен — POST /browser/start", ["port"] = _port, ["endpoint"] = _base };
        var version = JsonNode.Parse(text) as JsonObject;
        var targets = await TargetsAsync();
        return new JsonObject
        {
            ["ok"] = true,
            ["port"] = _port,
            ["browser"] = version?["Browser"]?.GetValue<string>(),
            ["protocol"] = version?["Protocol-Version"]?.GetValue<string>(),
            ["pages"] = targets.Count,
        };
    }

    public static async Task<JsonNode> ListPages()
    {
        var targets = await TargetsAsync();
        if (targets.Count == 0 && await HttpText("/json/version") is null)
            return new JsonObject { ["ok"] = false, ["exit_code"] = 503, ["error"] = "браузер не запущен — POST /browser/start" };
        return new JsonObject { ["ok"] = true, ["pages"] = targets };
    }

    /// Everything the browser runs, including extension workers — the quickest proof
    /// that the signed-in profile with its extensions is the one attached.
    public static async Task<JsonNode> AllTargets()
    {
        if (await HttpText("/json/version") is not { } versionText)
            return new JsonObject { ["ok"] = false, ["exit_code"] = 503, ["error"] = "браузер не запущен — POST /browser/start" };
        var version = JsonNode.Parse(versionText) as JsonObject;
        var browserWs = version?["webSocketDebuggerUrl"]?.GetValue<string>();
        if (browserWs is null) return new JsonObject { ["ok"] = false, ["exit_code"] = 1, ["error"] = "нет webSocketDebuggerUrl" };

        var browser = await Page.ConnectAsync("browser", browserWs);
        try
        {
            var answer = await browser.Send("Target.getTargets");
            var list = new JsonArray();
            var extensions = 0;
            if (answer["result"]?["targetInfos"] is JsonArray infos)
            {
                foreach (var info in infos.OfType<JsonObject>())
                {
                    var url = info["url"]?.GetValue<string>() ?? "";
                    var kind = info["type"]?.GetValue<string>() ?? "";
                    if (url.StartsWith("chrome-extension://", StringComparison.OrdinalIgnoreCase)) extensions++;
                    if (kind is not ("page" or "background_page" or "service_worker" or "iframe")) continue;
                    if (kind == "service_worker" && !url.StartsWith("chrome-extension://", StringComparison.OrdinalIgnoreCase)) continue;
                    list.Add(new JsonObject
                    {
                        ["id"] = info["targetId"]?.GetValue<string>(),
                        ["type"] = kind,
                        ["title"] = info["title"]?.GetValue<string>(),
                        ["url"] = url.Length > 120 ? url[..120] : url,
                    });
                }
            }
            return new JsonObject
            {
                ["ok"] = true,
                ["profile"] = CurrentMode() == "user" ? "ваш" : "отдельный",
                ["extensions"] = extensions,
                ["targets"] = list,
            };
        }
        finally { browser.Dispose(); }
    }

    public static async Task<JsonNode> Open(string url, bool background)
    {
        if (await HttpText("/json/version") is not { } versionText)
            return new JsonObject { ["ok"] = false, ["exit_code"] = 503, ["error"] = "браузер не запущен — POST /browser/start" };
        var version = JsonNode.Parse(versionText) as JsonObject;
        var browserWs = version?["webSocketDebuggerUrl"]?.GetValue<string>();
        if (browserWs is null) return new JsonObject { ["ok"] = false, ["exit_code"] = 1, ["error"] = "нет webSocketDebuggerUrl" };

        var browser = await Page.ConnectAsync("browser", browserWs);
        var answer = await browser.Send("Target.createTarget", new JsonObject { ["url"] = url, ["background"] = background });
        var targetId = answer["result"]?["targetId"]?.GetValue<string>();
        browser.Dispose();
        if (targetId is null) return new JsonObject { ["ok"] = false, ["exit_code"] = 1, ["error"] = "страница не создана", ["answer"] = answer };

        for (var i = 0; i < 20; i++)
        {
            await Task.Delay(200);
            var targets = await TargetsAsync();
            if (targets.OfType<JsonObject>().FirstOrDefault(t => t["id"]?.GetValue<string>() == targetId) is { } found)
            {
                var page = await Page.ConnectAsync(targetId, found["ws"]!.GetValue<string>());
                page.Url = url;
                Sessions[targetId] = page;
                return new JsonObject { ["ok"] = true, ["id"] = targetId, ["url"] = url };
            }
        }
        return new JsonObject { ["ok"] = false, ["exit_code"] = 124, ["error"] = "страница не появилась" };
    }

    // ---------- actions ----------
    public static async Task<JsonNode> Navigate(string url, JsonNode? selector)
    {
        var page = await PickAsync(selector);
        var answer = await page.Send("Page.navigate", new JsonObject { ["url"] = url });
        page.Url = url;
        return new JsonObject { ["ok"] = true, ["url"] = url, ["frame"] = answer["result"]?["frameId"]?.GetValue<string>() };
    }

    public static async Task<JsonNode> Evaluate(string expression, JsonNode? selector)
    {
        var page = await PickAsync(selector);
        var value = await page.Evaluate(expression);
        return new JsonObject
        {
            ["ok"] = true,
            ["value"] = value["value"]?.DeepClone(),
            ["type"] = value["type"]?.GetValue<string>(),
            ["description"] = value["description"]?.GetValue<string>(),
        };
    }

    public static async Task<JsonNode> Click(string? selector, double? x, double? y, JsonNode? pageSelector, bool real)
    {
        var page = await PickAsync(pageSelector);
        try { await page.Send("Page.bringToFront"); } catch { }

        // By selector the DOM click is used: it works in background tabs, where Chrome
        // ignores synthesized mouse input. Coordinates always mean a real mouse event.
        if (selector is not null && !real)
        {
            var clicked = await page.Evaluate(
                "(() => { const e = document.querySelector(" + Json.ToJs(selector) + ");" +
                " if (!e) return 'no-element'; e.scrollIntoView({block:'center', inline:'center'});" +
                " e.click(); return 'ok'; })()");
            var state = clicked["value"]?.GetValue<string>() ?? "";
            if (state == "no-element")
                return new JsonObject { ["ok"] = false, ["exit_code"] = 404, ["error"] = "элемент не найден", ["selector"] = selector };
            return new JsonObject { ["ok"] = true, ["via"] = "dom", ["selector"] = selector };
        }

        double clickX, clickY;
        if (selector is not null)
        {
            var rect = await page.Evaluate(
                "(() => { const e = document.querySelector(" + Json.ToJs(selector) + ");" +
                " if (!e) return null; e.scrollIntoView({block:'center', inline:'center'});" +
                " const r = e.getBoundingClientRect();" +
                " return {x: r.left + r.width/2, y: r.top + r.height/2}; })()");
            if (rect["value"] is not JsonObject point)
                return new JsonObject { ["ok"] = false, ["exit_code"] = 404, ["error"] = "элемент не найден", ["selector"] = selector };
            clickX = point["x"]!.GetValue<double>();
            clickY = point["y"]!.GetValue<double>();
        }
        else
        {
            clickX = x ?? 0;
            clickY = y ?? 0;
        }

        // Chrome ignores a press without the buttons bitmask, and a click without a move first.
        await page.Send("Input.dispatchMouseEvent", new JsonObject
        {
            ["type"] = "mouseMoved",
            ["x"] = clickX,
            ["y"] = clickY,
            ["button"] = "none",
            ["buttons"] = 0,
        });

        foreach (var (type, buttons) in new[] { ("mousePressed", 1), ("mouseReleased", 0) })
        {
            await page.Send("Input.dispatchMouseEvent", new JsonObject
            {
                ["type"] = type,
                ["x"] = clickX,
                ["y"] = clickY,
                ["button"] = "left",
                ["buttons"] = buttons,
                ["clickCount"] = 1,
            });
        }
        return new JsonObject { ["ok"] = true, ["via"] = "mouse", ["x"] = clickX, ["y"] = clickY };
    }

    public static async Task<JsonNode> Type(string? selector, string text, bool replace, JsonNode? pageSelector)
    {
        var page = await PickAsync(pageSelector);
        if (selector is not null)
        {
            var focused = await page.Evaluate(
                "(() => { const e = document.querySelector(" + Json.ToJs(selector) + "); if (!e) return false;" +
                " e.focus();" + (replace ? " if ('value' in e) { e.value = ''; }" : "") + " return true; })()");
            if (focused["value"]?.GetValue<bool>() != true)
                return new JsonObject { ["ok"] = false, ["exit_code"] = 404, ["error"] = "элемент не найден", ["selector"] = selector };
        }
        await page.Send("Input.insertText", new JsonObject { ["text"] = text });
        return new JsonObject { ["ok"] = true, ["typed"] = text.Length };
    }

    public static async Task<JsonNode> Key(string[] keys, JsonNode? pageSelector)
    {
        var page = await PickAsync(pageSelector);
        var modifiers = 0;
        var main = new List<string>();
        foreach (var key in keys)
        {
            switch (key.ToLowerInvariant())
            {
                case "ctrl": case "control": modifiers |= 2; break;
                case "alt": modifiers |= 1; break;
                case "shift": modifiers |= 8; break;
                case "meta": case "win": modifiers |= 4; break;
                default: main.Add(key); break;
            }
        }
        if (main.Count == 0) return new JsonObject { ["ok"] = false, ["exit_code"] = 400, ["error"] = "нет основной клавиши" };
        var last = main[^1];
        var description = Json.KeyDescriptor(last, modifiers);

        foreach (var type in new[] { "rawKeyDown", "keyUp" })
            await page.Send("Input.dispatchKeyEvent", new JsonObject
            {
                ["type"] = type,
                ["modifiers"] = modifiers,
                ["key"] = description.Key,
                ["code"] = description.Code,
                ["windowsVirtualKeyCode"] = description.VirtualKey,
                ["text"] = type == "keyUp" ? "" : description.Text,
            });
        return new JsonObject { ["ok"] = true, ["key"] = description.Key, ["modifiers"] = modifiers };
    }

    public static async Task<JsonNode> Text(string? selector, JsonNode? pageSelector)
    {
        var page = await PickAsync(pageSelector);
        var expression = selector is null
            ? "document.body ? document.body.innerText : ''"
            : "(() => { const e = document.querySelector(" + Json.ToJs(selector) + ");" +
              " return e ? (e.innerText || e.value || e.textContent || '') : null; })()";
        var result = await page.Evaluate(expression);
        var value = result["value"];
        if (value is null && selector is not null)
            return new JsonObject { ["ok"] = false, ["exit_code"] = 404, ["error"] = "элемент не найден", ["selector"] = selector };
        var body = value?.GetValue<string>() ?? "";
        return new JsonObject { ["ok"] = true, ["url"] = page.Url, ["text"] = body, ["length"] = body.Length };
    }

    public static async Task<(byte[] Bytes, JsonNode Meta)> Shot(int quality, bool fullPage, JsonNode? pageSelector)
    {
        var page = await PickAsync(pageSelector);
        var answer = await page.Send("Page.captureScreenshot", new JsonObject
        {
            ["format"] = "jpeg",
            ["quality"] = Math.Clamp(quality, 10, 100),
            ["captureBeyondViewport"] = fullPage,
        }, 120);
        var data = answer["result"]?["data"]?.GetValue<string>();
        if (data is null) throw new InvalidOperationException("скриншот не получен");
        var bytes = Convert.FromBase64String(data);
        return (bytes, new JsonObject { ["bytes"] = bytes.Length, ["url"] = page.Url });
    }

    public static async Task<JsonNode> WaitFor(string? selector, string? text, int timeout, JsonNode? pageSelector)
    {
        var page = await PickAsync(pageSelector);
        var started = DateTime.UtcNow;
        while ((DateTime.UtcNow - started).TotalSeconds < timeout)
        {
            var expression = selector is not null
                ? $"!!document.querySelector({Json.ToJs(selector)})"
                : $"(document.body ? document.body.innerText : '').includes({Json.ToJs(text ?? "")})";
            try
            {
                var answer = await page.Evaluate(expression);
                if (answer["value"]?.GetValue<bool>() == true)
                    return new JsonObject { ["ok"] = true, ["waited_ms"] = (long)(DateTime.UtcNow - started).TotalMilliseconds };
            }
            catch { }
            await Task.Delay(300);
        }
        return new JsonObject
        {
            ["ok"] = false,
            ["exit_code"] = 124,
            ["waited_ms"] = (long)(DateTime.UtcNow - started).TotalMilliseconds,
            ["error"] = "условие не наступило",
        };
    }
}

internal static class Json
{
    /// JavaScript string literal from a C# string (quotes, backslashes, newlines).
    public static string ToJs(string value) =>
        "\"" + value.Replace("\\", "\\\\").Replace("\"", "\\\"").Replace("\n", "\\n").Replace("\r", "") + "\"";

    public sealed record KeyCode(string Key, string Code, int VirtualKey, string Text = "");

    public static KeyCode KeyDescriptor(string key, int modifiers)
    {
        var lowered = key.ToLowerInvariant();
        var shift = (modifiers & 8) != 0;
        switch (lowered)
        {
            case "enter": case "return": return new KeyCode("Enter", "Enter", 13, "\r");
            case "tab": return new KeyCode("Tab", "Tab", 9);
            case "escape": case "esc": return new KeyCode("Escape", "Escape", 27);
            case "backspace": return new KeyCode("Backspace", "Backspace", 8);
            case "delete": return new KeyCode("Delete", "Delete", 46);
            case "arrowup": case "up": return new KeyCode("ArrowUp", "ArrowUp", 38);
            case "arrowdown": case "down": return new KeyCode("ArrowDown", "ArrowDown", 40);
            case "arrowleft": case "left": return new KeyCode("ArrowLeft", "ArrowLeft", 37);
            case "arrowright": case "right": return new KeyCode("ArrowRight", "ArrowRight", 39);
            case "home": return new KeyCode("Home", "Home", 36);
            case "end": return new KeyCode("End", "End", 35);
            case "pageup": return new KeyCode("PageUp", "PageUp", 33);
            case "pagedown": return new KeyCode("PageDown", "PageDown", 34);
            case "space": return new KeyCode(" ", "Space", 32, " ");
        }
        if (lowered.Length == 1)
        {
            var ch = lowered[0];
            if (ch is >= 'a' and <= 'z')
                return new KeyCode(shift ? key.ToUpperInvariant() : lowered, "Key" + char.ToUpperInvariant(ch),
                    char.ToUpperInvariant(ch), (modifiers & 2) != 0 ? "" : (shift ? key.ToUpperInvariant() : lowered));
            if (ch is >= '0' and <= '9')
                return new KeyCode(ch.ToString(), "Digit" + ch, ch);
        }
        return new KeyCode(key, key, 0);
    }
}
