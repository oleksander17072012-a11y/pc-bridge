// The plan board: a short list of steps that the window shows above the journal.
using System.Text.Json.Nodes;

namespace PcBridge.Core;

public sealed record PlanStep(int Number, string Text, string State); // done | current | pending

/// A plan is created by the assistant itself for tasks with several steps. Small jobs
/// (create a folder, open a page) get none: an empty board draws nothing at all.
public sealed class PlanBoard
{
    private readonly object _lock = new();
    private List<string> _steps = new();
    private int _current;
    private string? _title;
    private DateTime _finishedAt = DateTime.MinValue;

    public event Action? Changed;

    public bool Visible
    {
        get { lock (_lock) return _steps.Count > 0; }
    }

    public bool Finished
    {
        get { lock (_lock) return _steps.Count > 0 && _current >= _steps.Count; }
    }

    public DateTime FinishedAt { get { lock (_lock) return _finishedAt; } }

    public void Set(IEnumerable<string> steps, int current, string? title)
    {
        var list = steps.Select(step => (step ?? "").Trim()).Where(step => step.Length > 0).Take(40).ToList();
        lock (_lock)
        {
            _steps = list;
            _current = Math.Clamp(current, 0, list.Count);
            _title = string.IsNullOrWhiteSpace(title) ? null : title.Trim();
            _finishedAt = _current >= list.Count ? DateTime.UtcNow : DateTime.MinValue;
        }
        Changed?.Invoke();
    }

    public void Clear()
    {
        lock (_lock)
        {
            _steps = new List<string>();
            _current = 0;
            _title = null;
            _finishedAt = DateTime.MinValue;
        }
        Changed?.Invoke();
    }

    /// The finished board lingers ten seconds, then goes away by itself.
    public bool Expired() => Visible && Finished &&
        (DateTime.UtcNow - FinishedAt).TotalSeconds > 10;

    public string Counter
    {
        get
        {
            lock (_lock)
            {
                if (_steps.Count == 0) return "";
                return _current >= _steps.Count ? $"{_steps.Count}/{_steps.Count}" : $"{_current}/{_steps.Count}";
            }
        }
    }

    public string Headline
    {
        get
        {
            lock (_lock)
            {
                if (_steps.Count == 0) return "";
                if (_current >= _steps.Count) return "готово";
                var now = _steps[Math.Clamp(_current, 0, _steps.Count - 1)];
                return _title is null ? now : $"{_title} · {now}";
            }
        }
    }

    public List<PlanStep> Steps()
    {
        lock (_lock)
        {
            var result = new List<PlanStep>(_steps.Count);
            for (var index = 0; index < _steps.Count; index++)
            {
                var state = index < _current ? "done" : index == _current ? "current" : "pending";
                result.Add(new PlanStep(index + 1, _steps[index], state));
            }
            return result;
        }
    }

    public JsonObject Snapshot()
    {
        lock (_lock)
        {
            var steps = new JsonArray();
            foreach (var step in Steps()) steps.Add(new JsonObject { ["n"] = step.Number, ["text"] = step.Text, ["state"] = step.State });
            return new JsonObject
            {
                ["visible"] = _steps.Count > 0,
                ["title"] = _title,
                ["current"] = _current,
                ["total"] = _steps.Count,
                ["counter"] = Counter,
                ["finished"] = _current >= _steps.Count && _steps.Count > 0,
                ["steps"] = steps,
            };
        }
    }
}
