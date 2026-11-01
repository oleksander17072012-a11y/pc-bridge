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
    private void Copy_Click(object sender, RoutedEventArgs e)
        => Copy(App.State.PairingBundle(), "скопировано: ключ + промпт");

    private void CopyKey_Click(object sender, RoutedEventArgs e)
        => Copy(App.State.PairingBlock(), "скопирован блок ключа");

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

    private void Tunnel_Click(object sender, RoutedEventArgs e) => App.State.Tunnel.Restart();

    private void Pause_Click(object sender, RoutedEventArgs e)
    {
        var next = !App.State.Active;
        App.State.SetActive(next);
        App.State.Journal.Push("ui", next ? "приём команд возобновлён" : "приём команд приостановлен");
        Refresh();
    }
}
