using System.Collections.ObjectModel;
using System.Windows;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Threading;
using PcBridge.Core;

namespace PcBridge;

public sealed class LogRow
{
    public string Line { get; init; } = "";
    public Brush Tone { get; init; } = Brushes.Gray;
}

public partial class MainWindow : Window
{
    private readonly ObservableCollection<LogRow> _rows = new();
    private readonly DispatcherTimer _ticker;
    private readonly Brush _plain = Freeze("#C3C9D6");
    private readonly Brush _dim = Freeze("#7E8698");
    private readonly Brush _alert = Freeze("#FF7A6B");
    private readonly Brush _ok = Freeze("#6FD08C");

    private static Brush Freeze(string hex)
    {
        var brush = new SolidColorBrush((Color)ColorConverter.ConvertFromString(hex));
        brush.Freeze();
        return brush;
    }

    public MainWindow()
    {
        InitializeComponent();
        LogList.ItemsSource = _rows;

        // Fill from the journal that already exists, then follow new entries.
        foreach (var entry in App.State.Journal.Recent(80)) Append(entry);
        App.State.Journal.Added += entry => Dispatcher.BeginInvoke(() => Append(entry));
        // A repeated operation is folded into one line with a counter: rewrite it in place.
        App.State.Journal.Updated += entry => Dispatcher.BeginInvoke(() =>
        {
            if (_rows.Count > 0) _rows.RemoveAt(_rows.Count - 1);
            Append(entry);
        });

        _ticker = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(700) };
        _ticker.Tick += (_, _) => Refresh();
        _ticker.Start();

        Loaded += (_, _) =>
        {
            ApplyRounding();
            Refresh();
        };
        SizeChanged += (_, _) => ApplyRounding();
    }

    private void ApplyRounding()
    {
        var handle = new WindowInteropHelper(this).Handle;
        if (handle == IntPtr.Zero) return;
        var scale = VisualTreeHelper.GetDpi(this).DpiScaleX;
        Win32.Round(handle, ActualWidth, ActualHeight, scale, 14);
    }

    private void Append(JournalEntry entry)
    {
        var tone = entry.Kind switch
        {
            "error" => _alert,
            "shell" => _plain,
            "ui" => _ok,
            _ => _dim,
        };
        _rows.Add(new LogRow { Line = $"{entry.Time[11..]}  {entry.Text}", Tone = tone });
        while (_rows.Count > 400) _rows.RemoveAt(0);
        // Scroll only after the new line has been measured, otherwise the offset lands past the end.
        Dispatcher.BeginInvoke(new Action(() => LogScroll.ScrollToEnd()), DispatcherPriority.Background);
    }

    private void Refresh()
    {
        var state = App.State;
        PairingBox.Text = state.PairingBlock();

        var active = state.Active;
        StateLine.Text = active
            ? state.Tunnel.Url is null ? "туннель поднимается" : "на связи"
            : "пауза";
        StateLine.Foreground = active ? (state.Tunnel.Url is null ? _dim : _ok) : _alert;
        PauseButton.Content = active ? "Пауза" : "Продолжить";

        var snapshot = state.Snapshot();
        CountLine.Text = $"команд: {snapshot["commands"]} · заметок: {snapshot["notes"]}";
        var uptime = TimeSpan.FromSeconds(state.UptimeSeconds);
        UptimeLine.Text = $"работает: {uptime.Hours:00}:{uptime.Minutes:00}:{uptime.Seconds:00} · {state.Hostname}";
    }

    // ---------- window chrome ----------
    private void TitleBar_MouseDown(object sender, MouseButtonEventArgs e)
    {
        if (e.ClickCount == 2) return; // no maximize
        if (e.ButtonState == MouseButtonState.Pressed) DragMove();
    }

    private void Minimize_Click(object sender, RoutedEventArgs e) => WindowState = WindowState.Minimized;

    private void Close_Click(object sender, RoutedEventArgs e) => Close();

    // ---------- actions ----------
    /// Большая кнопка: блок доступа, разделитель и инструкции — вставлять одним куском.
    private void Copy_Click(object sender, RoutedEventArgs e)
        => Copy(App.State.PairingBundle(), $"скопировано: ключ + промпт ({Prompt.Bytes / 1000.0:0.0} тыс. знаков)");

    /// Иконка у карточки: только сам блок — когда промпт уже отправлен раньше.
    private void CopyBlock_Click(object sender, RoutedEventArgs e)
        => Copy(App.State.PairingBlock(), "скопирован блок доступа");

    private void Data_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            Process.Start(new ProcessStartInfo("explorer.exe") { ArgumentList = { Paths.DataDir }, UseShellExecute = true });
            App.State.Journal.Push("ui", "открыта папка данных");
        }
        catch (Exception error)
        {
            App.State.Journal.Push("error", "проводник не открылся: " + error.Message);
        }
    }

    private void Log_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            if (!File.Exists(Paths.LogFile)) File.WriteAllText(Paths.LogFile, "");
            Process.Start(new ProcessStartInfo(Paths.LogFile) { UseShellExecute = true });
            App.State.Journal.Push("ui", "журнал открыт в блокноте");
        }
        catch (Exception error)
        {
            App.State.Journal.Push("error", "журнал не открылся: " + error.Message);
        }
    }

    /// Локальная проверка: релей отвечает по своему порту, туннель поднят, браузерный мост виден.
    private async void Check_Click(object sender, RoutedEventArgs e)
    {
        var state = App.State;
        App.State.Journal.Push("step", "проверяю связь");
        CheckButton.IsEnabled = false;
        try
        {
            using var client = new HttpClient(new HttpClientHandler { UseProxy = false }) { Timeout = TimeSpan.FromSeconds(5) };
            client.DefaultRequestHeaders.Add("Authorization", "Bearer " + state.Key);
            var answer = await client.GetStringAsync($"http://127.0.0.1:{Relay.Port}/ping");
            var ok = answer.Contains("\"ok\":true", StringComparison.OrdinalIgnoreCase);
            state.Journal.Push(ok ? "result" : "error", ok
                ? $"релей отвечает на 127.0.0.1:{Relay.Port}"
                : "релей ответил неожиданно: " + Trim(answer, (int)120));
        }
        catch (Exception error)
        {
            state.Journal.Push("error", "релей не отвечает: " + error.Message);
        }

        var url = state.Tunnel.Url;
        state.Journal.Push(url is null ? "error" : "result", url is null
            ? "туннеля нет — нажмите «Новый туннель»"
            : "туннель: " + url);

        var cloudflared = Process.GetProcesses().Any(p =>
        {
            try { return p.ProcessName.Contains("cloudflared", StringComparison.OrdinalIgnoreCase); }
            catch { return false; }
        });
        state.Journal.Push(cloudflared ? "result" : "error", cloudflared
            ? "cloudflared запущен"
            : "cloudflared не найден — «Новый туннель» перезапустит его");

        CheckButton.IsEnabled = true;
    }

    private void Copy(string text, string message)
    {
        try
        {
            Clipboard.SetText(text);
            App.State.Journal.Push("ui", message);
        }
        catch (Exception error)
        {
            App.State.Journal.Push("error", "буфер обмена занят: " + error.Message);
        }
    }

    private static string Trim(string text, int limit)
        => text.Length <= limit ? text : text[..limit] + "…";

    private void Tunnel_Click(object sender, RoutedEventArgs e) => App.State.Tunnel.Restart();

    private void Pause_Click(object sender, RoutedEventArgs e)
    {
        var next = !App.State.Active;
        App.State.SetActive(next);
        App.State.Journal.Push("ui", next ? "приём команд возобновлён" : "приём команд приостановлен");
        Refresh();
    }
}
