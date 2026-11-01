# PC-Bridge 3.0.0 — C#-версия

Тот же мост, что и Rust-версия, но целиком на C#/WPF: один exe, без установки .NET на машине.

## Что внутри

- `src/PcBridge/Core/Core.cs` — пути в `%LOCALAPPDATA%\PC-Bridge`, ключ (32 hex), журнал (кольцо 2000 + `pc_agent_log.txt`), декодирование консоли (UTF-8 → CP866/CP1251 по счёту), фоновые задачи.
- `Core/Win32.cs` — скриншот через GDI + StretchBlt, список окон, закругление окна, настоящий курсор и клавиатура.
- `Core/Shell.cs` — запуск `cmd.exe`/`powershell`, UTF-8, вывод из двух потоков, таймаут с убийством дерева процессов, команды длиннее 6000 символов уходят во временный `.ps1`/`.bat`.
- `Core/Relay.cs` — HTTP на 127.0.0.1 случайного порта, Bearer-токен, те же ручки, что в 2.6.0.
- `Core/Tunnel.cs` — cloudflared: скачивание, проверка подписи, разбор URL из лога.
- `Core/State.cs`, `Core/Input.cs` — состояние моста и `gui`-действия.
- `MainWindow.xaml` — окно 940×760: блок доступа, три кнопки, строка состояния, «Логги».

## Сборка на своём ПК

Нужен только Windows SDK (`.NET 8 SDK`). В PowerShell **от администратора**, из этой папки:

```powershell
.\build.ps1
```

Результат — `out\pc-bridge.exe` (self-contained, ~63 МБ). Рядом печатается SHA-256.

## API

Один в один с 2.6.0:

```
GET  /health                                    без токена
GET  /ping /status /board /windows /journal /jobs
POST /note      {"text":"…","kind":"note|stage|step|result|error"}
POST /cmd       {"command":"…","shell":"cmd|powershell","timeout":60}
                {"type":"batch","commands":[…]}
                {"type":"screenshot","width":1100,"quality":65}
                {"type":"gui","actions":[…]}
                {"…","background":true} → {"job_id":"…","poll":"/job/<id>"}
GET  /job/{id}                                  {"status":"running|done","output","exit_code","took_ms"}
POST /files     {"files":[{"path":"…","text":"…"|"data":"base64"}]}
GET  /file?path=…&mode=text|base64
GET  /shot?width=&quality=                      raw JPEG
POST /wait      {"port"|"process"|"window"|"file", "timeout":≤120} → {"ok","waited_ms"}, код 124 по таймауту
```

Пауза в окне приложения отвечает `503` на `/cmd` — снять её может только человек за компьютером.
