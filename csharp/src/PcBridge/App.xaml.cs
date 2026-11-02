using System.Windows;
using PcBridge.Core;

namespace PcBridge;

public partial class App : Application
{
    public static AppState State { get; private set; } = null!;

    protected override void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);

        DispatcherUnhandledException += (_, args) =>
        {
            try { State?.Journal.Push("error", args.Exception.Message); } catch { }
            args.Handled = true;
        };

        var journal = new Journal(Paths.LogFile);
        journal.Push("boot", "PC-Bridge 3.4.4 стартует");

        var key = LoadOrCreateKey();
        var jobs = new Jobs();
        // The pairing file is rewritten as soon as the tunnel URL is known,
        // so the block on screen and the file never disagree.
        var tunnel = new Tunnel(journal, url =>
        {
            try
            {
                var text = $"PC-BRIDGE-PAIRING v1\nurl={url}\nkey={key}\ncreated={DateTime.Now:yyyy-MM-dd HH:mm:ss}";
                File.WriteAllText(Paths.KeyFile, text);
            }
            catch { }
        });
        State = new AppState { Journal = journal, Tunnel = tunnel, Jobs = jobs, Key = key };

        Relay.Start(State);
        State.WriteKeyFile();
        tunnel.Start();

        try
        {
            var window = new MainWindow();
            MainWindow = window;
            window.Show();
        }
        catch (Exception error)
        {
            // A broken XAML resource must not leave the agent running invisibly.
            journal.Push("error", "окно не открылось: " + error.Message);
            MessageBox.Show("окно не открылось: " + error.Message, "PC-Bridge", MessageBoxButton.OK, MessageBoxImage.Error);
            Shutdown(1);
        }
    }

    protected override void OnExit(ExitEventArgs e)
    {
        try { State?.Journal.Push("boot", "выход"); } catch { }
        try { State?.Tunnel.Stop(); } catch { }
        Relay.Stop();
        base.OnExit(e);
    }

    /// The key lives in %LOCALAPPDATA%\PC-Bridge\pairing.key and survives restarts.
    private static string LoadOrCreateKey()
    {
        try
        {
            if (File.Exists(Paths.KeyFile))
            {
                foreach (var line in File.ReadAllLines(Paths.KeyFile))
                {
                    if (!line.StartsWith("key=", StringComparison.OrdinalIgnoreCase)) continue;
                    var value = line[4..].Trim();
                    if (value.Length >= 16) return value;
                }
            }
        }
        catch { }
        var fresh = Keys.NewKey();
        try { File.WriteAllText(Paths.KeyFile, $"PC-BRIDGE-PAIRING v1\nurl=нет связи\nkey={fresh}"); } catch { }
        return fresh;
    }
}
