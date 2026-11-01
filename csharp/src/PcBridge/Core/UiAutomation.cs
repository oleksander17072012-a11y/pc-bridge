// UI Automation: click and type by name instead of by pixel — no cursor, no focus stealing,
// and it reaches controls that posted messages cannot (Chromium, WPF, UWP).
using System.Windows;
using System.Windows.Automation;
using Condition = System.Windows.Automation.Condition;

namespace PcBridge.Core;

public static class Ui
{
    /// UIA wants a single-threaded apartment, and the app already has one.
    private static T OnUi<T>(Func<T> work)
    {
        var app = Application.Current;
        if (app is null || app.Dispatcher.CheckAccess()) return work();
        return app.Dispatcher.Invoke(work);
    }

    private static string Kind(AutomationElement element) => element.Current.ControlType.ProgrammaticName.Replace("ControlType.", "");

    private static JsonObject Describe(AutomationElement element)
    {
        var rect = element.Current.BoundingRectangle;
        var patterns = new JsonArray();
        foreach (var pair in new (string Name, AutomationPattern Pattern)[]
                 {
                     ("invoke", InvokePattern.Pattern),
                     ("setValue", ValuePattern.Pattern),
                     ("toggle", TogglePattern.Pattern),
                     ("select", SelectionItemPattern.Pattern),
                     ("expand", ExpandCollapsePattern.Pattern),
                     ("text", TextPattern.Pattern),
                 })
        {
            if (element.TryGetCurrentPattern(pair.Pattern, out _)) patterns.Add(pair.Name);
        }

        return new JsonObject
        {
            ["name"] = element.Current.Name ?? "",
            ["automation_id"] = element.Current.AutomationId ?? "",
            ["kind"] = Kind(element),
            ["class"] = element.Current.ClassName ?? "",
            ["pid"] = element.Current.ProcessId,
            ["hwnd"] = element.Current.NativeWindowHandle,
            ["enabled"] = element.Current.IsEnabled,
            ["rect"] = new JsonArray
            {
                (int)rect.X, (int)rect.Y, (int)rect.Width, (int)rect.Height,
            },
            ["patterns"] = patterns,
        };
    }

    private static AutomationElement? Window(string? title, string? process, long? hwnd)
    {
        if (hwnd is > 0)
        {
            try { return AutomationElement.FromHandle((IntPtr)hwnd.Value); } catch { return null; }
        }

        foreach (var child in AutomationElement.RootElement.FindAll(TreeScope.Children, Condition.TrueCondition).Cast<AutomationElement>())
        {
            var name = child.Current.Name ?? "";
            if (!string.IsNullOrWhiteSpace(title) && name.Contains(title, StringComparison.OrdinalIgnoreCase)) return child;
            if (!string.IsNullOrWhiteSpace(process))
            {
                try
                {
                    if (System.Diagnostics.Process.GetProcessById(child.Current.ProcessId).ProcessName
                        .Contains(process, StringComparison.OrdinalIgnoreCase)) return child;
                }
                catch { }
            }
        }
        return null;
    }

    private static IEnumerable<AutomationElement> Walk(AutomationElement root, int depth, int limit)
    {
        var queue = new Queue<(AutomationElement Element, int Level)>();
        queue.Enqueue((root, 0));
        var seen = 0;
        while (queue.Count > 0 && seen < limit)
        {
            var (element, level) = queue.Dequeue();
            seen++;
            yield return element;
            if (level >= depth) continue;
            AutomationElementCollection children;
            try { children = element.FindAll(TreeScope.Children, Condition.TrueCondition); }
            catch { continue; }
            foreach (var child in children.Cast<AutomationElement>()) queue.Enqueue((child, level + 1));
        }
    }

    private static AutomationElement? Find(AutomationElement root, string? name, string? automationId, string? kind)
    {
        foreach (var element in Walk(root, 12, 4000).Skip(1))
        {
            var elementName = element.Current.Name ?? "";
            var elementId = element.Current.AutomationId ?? "";
            if (!string.IsNullOrWhiteSpace(kind) &&
                !Kind(element).Equals(kind, StringComparison.OrdinalIgnoreCase)) continue;
            if (!string.IsNullOrWhiteSpace(automationId) &&
                !elementId.Equals(automationId, StringComparison.OrdinalIgnoreCase)) continue;
            if (!string.IsNullOrWhiteSpace(name) &&
                !elementName.Contains(name, StringComparison.OrdinalIgnoreCase) && !elementName.Equals(name, StringComparison.OrdinalIgnoreCase)) continue;
            return element;
        }
        return null;
    }

    public static JsonNode Tree(string? title, string? process, long? hwnd, int depth, int limit) => OnUi(() =>
    {
        var root = Window(title, process, hwnd);
        if (root is null) return (JsonNode)new JsonObject { ["error"] = "окно не найдено", ["exit_code"] = 404 };
        var items = new JsonArray();
        foreach (var element in Walk(root, Math.Clamp(depth, 1, 12), Math.Clamp(limit, 1, 3000))) items.Add(Describe(element));
        return new JsonObject { ["window"] = root.Current.Name ?? "", ["count"] = items.Count, ["elements"] = items };
    });

    /// Tries the patterns in the order a human would: press the button, flip the switch,
    /// select the row, open the node, invoke the legacy default action.
    public static JsonNode Invoke(string? title, string? process, long? hwnd, string? name, string? automationId, string? kind) => OnUi(() =>
    {
        var root = Window(title, process, hwnd);
        if (root is null) return (JsonNode)new JsonObject { ["error"] = "окно не найдено", ["exit_code"] = 404 };
        var element = Find(root, name, automationId, kind);
        if (element is null) return new JsonObject { ["error"] = "элемент не найден", ["exit_code"] = 404, ["name"] = name, ["automation_id"] = automationId };

        var tried = new List<string>();
        foreach (var pattern in new[] { InvokePattern.Pattern, TogglePattern.Pattern, SelectionItemPattern.Pattern, ExpandCollapsePattern.Pattern })
        {
            if (!element.TryGetCurrentPattern(pattern, out var handler)) continue;
            tried.Add(pattern.ProgrammaticName);
            try
            {
                switch (handler)
                {
                    case InvokePattern invoke: invoke.Invoke(); break;
                    case TogglePattern toggle: toggle.Toggle(); break;
                    case SelectionItemPattern select: select.Select(); break;
                    case ExpandCollapsePattern expand:
                        if (expand.Current.ExpandCollapseState == ExpandCollapseState.Collapsed) expand.Expand();
                        else expand.Collapse();
                        break;
                }
                return new JsonObject { ["ok"] = true, ["via"] = pattern.ProgrammaticName, ["element"] = Describe(element) };
            }
            catch (Exception error) { tried.Add("ошибка: " + error.Message); }
        }

        return new JsonObject
        {
            ["ok"] = false,
            ["exit_code"] = 1,
            ["error"] = "элемент не принимает нажатие",
            ["tried"] = new JsonArray(tried.Select(t => (JsonNode)t!).ToArray()),
            ["element"] = Describe(element),
        };
    });

    public static JsonNode SetValue(string? title, string? process, long? hwnd, string? name, string? automationId, string value) => OnUi(() =>
    {
        var root = Window(title, process, hwnd);
        if (root is null) return (JsonNode)new JsonObject { ["error"] = "окно не найдено", ["exit_code"] = 404 };
        AutomationElement? element = null;
        if (string.IsNullOrWhiteSpace(name) && string.IsNullOrWhiteSpace(automationId))
        {
            // No name given: take the first field that actually accepts a value.
            foreach (var candidate in Walk(root, 8, 800).Skip(1))
            {
                if (!candidate.TryGetCurrentPattern(ValuePattern.Pattern, out var raw) || raw is not ValuePattern probe) continue;
                if (probe.Current.IsReadOnly) continue;
                element = candidate;
                break;
            }
        }
        else
        {
            element = Find(root, name, automationId, null);
            if (element is not null && !element.TryGetCurrentPattern(ValuePattern.Pattern, out _))
            {
                // Название совпало с обёрткой — ищем поле с этим именем рядом.
                foreach (var candidate in Walk(element, 4, 200))
                {
                    if (candidate.TryGetCurrentPattern(ValuePattern.Pattern, out var raw) && raw is ValuePattern probe && !probe.Current.IsReadOnly)
                    {
                        element = candidate;
                        break;
                    }
                }
            }
        }
        if (element is null) return new JsonObject
        {
            ["error"] = "поле с ValuePattern не найдено",
            ["exit_code"] = 404,
            ["use_instead"] = "gui с background:true (WM_CHAR) для классических приложений",
        };

        if (element.TryGetCurrentPattern(ValuePattern.Pattern, out var pattern) && pattern is ValuePattern valuePattern)
        {
            try
            {
                if (valuePattern.Current.IsReadOnly)
                    return new JsonObject { ["ok"] = false, ["exit_code"] = 1, ["error"] = "поле только для чтения" };
                valuePattern.SetValue(value);
                return new JsonObject { ["ok"] = true, ["via"] = "ValuePattern", ["value"] = value };
            }
            catch (Exception error)
            {
                return new JsonObject { ["ok"] = false, ["exit_code"] = 1, ["error"] = error.Message };
            }
        }
        return new JsonObject
        {
            ["ok"] = false,
            ["exit_code"] = 1,
            ["error"] = "у элемента нет ValuePattern (Notepad, игры, часть Win32-приложений)",
            ["use_instead"] = "gui с background:true (WM_CHAR) — текст вводится посимвольно, без курсора",
        };
    });

    public static JsonNode Read(string? title, string? process, long? hwnd, string? name, string? automationId) => OnUi(() =>
    {
        var root = Window(title, process, hwnd);
        if (root is null) return (JsonNode)new JsonObject { ["error"] = "окно не найдено", ["exit_code"] = 404 };
        var element = string.IsNullOrWhiteSpace(name) && string.IsNullOrWhiteSpace(automationId) ? root : Find(root, name, automationId, null);
        if (element is null) return new JsonObject { ["error"] = "элемент не найден", ["exit_code"] = 404 };

        // No element named: return the first thing that actually carries text, not the title bar.
        if (element == root)
        {
            foreach (var candidate in Walk(root, 6, 400).Skip(1))
            {
                if (candidate.TryGetCurrentPattern(ValuePattern.Pattern, out var valueRaw) &&
                    valueRaw is ValuePattern direct && !direct.Current.IsReadOnly)
                    return new JsonObject { ["ok"] = true, ["text"] = direct.Current.Value, ["via"] = "ValuePattern", ["element"] = Kind(candidate) };
                if (candidate.TryGetCurrentPattern(TextPattern.Pattern, out var textRaw) && textRaw is TextPattern text &&
                    !string.IsNullOrWhiteSpace(text.DocumentRange.GetText(-1)) && Kind(candidate) is "Document" or "Edit" or "Text")
                    return new JsonObject { ["ok"] = true, ["text"] = text.DocumentRange.GetText(-1), ["via"] = "TextPattern", ["element"] = Kind(candidate) };
            }
        }

        if (element.TryGetCurrentPattern(TextPattern.Pattern, out var pattern) && pattern is TextPattern textPattern)
        {
            try { return new JsonObject { ["ok"] = true, ["text"] = textPattern.DocumentRange.GetText(-1) }; }
            catch { }
        }
        if (element.TryGetCurrentPattern(ValuePattern.Pattern, out var valuePatternRaw) && valuePatternRaw is ValuePattern valuePattern)
        {
            try { return new JsonObject { ["ok"] = true, ["text"] = valuePattern.Current.Value }; } catch { }
        }
        return new JsonObject { ["ok"] = true, ["text"] = element.Current.Name ?? "" };
    });
}
