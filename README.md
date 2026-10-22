# PC-Bridge

[![Build](https://github.com/oleksander17072012-a11y/pc-bridge/actions/workflows/windows.yml/badge.svg?branch=master)](https://github.com/oleksander17072012-a11y/pc-bridge/actions/workflows/windows.yml)

Мост удалённого управления **своим собственным ПК** из чата ассистента: один `.exe`,
без установки Python, без консольного окна, без фоновых служб.

Переписан с Python-версии 4.3.3 на **Rust + Tauri 2**. Сборка — целиком в GitHub
Actions: `windows-latest` собирает один самодостаточный `pc-bridge.exe` (бесплатный
раннер, никаких локальных компиляций).

---

## Что это делает

```
чат / ассистент ──HTTPS──▶ trycloudflare.com ──▶ локальный релей 127.0.0.1:<эфемерный порт>
                                                        │  Bearer <pairing key>
                                                        ▼
                                    ┌────────────────────────────────────────┐
                                    │ shell: cmd / PowerShell                │
                                    │ screenshot: GDI BitBlt → JPEG          │
                                    │ gui: PostMessage в фоновое окно        │
                                    │ windows: список / активация / закрытие │
                                    └────────────────────────────────────────┘
```

Ключ генерируется при каждом запуске из `BCryptGenRandom` (32 hex-символа) и живёт
только в памяти процесса + в `%LOCALAPPDATA%\PC-Bridge\pairing.key`.

## Отличия от Python-версии

| | Python 4.3.3 | этот репозиторий |
|---|---|---|
| Размер | Python 3.13 + `bin/cloudflared` (40 МБ) + pip-пакеты | один exe ~4 МБ, cloudflared докачивается при первом запуске |
| Рантайм | нужен установленный Python | ничего не нужно |
| GUI | customtkinter | WebView2 (встроен в Windows 10/11) |
| Задачи | очередь `/poll` + `/result` | синхронное выполнение `/cmd`, `/poll` оставлен для совместимости |
| Токен | `?token=` в query-строке | только `Authorization: Bearer` |
| cloudflared | скачивался без проверки | проверяется подпись Authenticode (Cloudflare) |
| Логи и ключ | рядом с `.py` | в `%LOCALAPPDATA%\PC-Bridge` |
| Зависимости | 4 пакета из PyPI без пиннинга | фиксированный `Cargo.lock`, без нативных UI-эффектов |

## Возможности

* `shell` — команды `cmd.exe /c` и `powershell -NoProfile -NonInteractive -Command`,
  таймаут с `taskkill /T`, денайлист на разрушительные команды.
* `screenshot` — снимок экрана через GDI, JPEG, `image_base64` в ответе.
* `gui` — `moveTo`, `click`, `doubleClick`, `rightClick`, `typewrite`, `hotkey`,
  `press`, `scroll`, `dragTo`, `wait`. Ввод уходит сообщениями `PostMessageW`
  в окно под **виртуальным** курсором: физическая мышь и клавиатура не двигаются.
* `windows` — видимые окна с процессом и PID; активация и мягкое закрытие.
* Виртуальный курсор — отдельное прозрачное always-on-top окно Tauri, click-through,
  голубое кольцо с пульсом на клике; включён всегда, переключателя нет.
* Журнал — рабочий лог, как у сборки: этапы (`stage`), шаги (`step`), результаты
  (`result`) и ошибки (`error`), с глифами и временем. Текущий этап виден в титлбаре.
  Дублируется в `%LOCALAPPDATA%\PC-Bridge\pc_agent_log.txt` и доступен через `GET /journal`.
* Трей, сворачивание по «✕», запрос UAC отдельной кнопкой.
* Кастомный титлбар без системной рамки и без кнопки разворачивания: только
  «─» (свернуть, жёлтая подсветка хитбокса) и «✕» (в трей). Никакого блюра и тени
  окна: DWM-эффекты и на Windows 10 тормозили перетаскивание, и закрашивали
  скруглённые углы прямоугольником. Углы задаются системно — `SetWindowRgn` на
  Windows 10, `DWMWA_WINDOW_CORNER_PREFERENCE` на Windows 11.
* Перетаскивание — нативный `startDragging()` одним вызовом Win32, без JS-цикла,
  поэтому окно не отстаёт от курсора.
* «Копировать ключ + промпт» раздвигает лаунчер на левую половину экрана и открывает
  рядом настоящее окно браузера с Arena (Agent Mode) — со своим профилем WebView2,
  поэтому вход в аккаунт сохраняется между запусками. Это отдельное окно, а не iframe:
  OAuth-провайдеры блокируют вход внутри embedded-фреймов. Кнопка «Закрыть чат»
  возвращает лаунчеру исходный размер, а страница чата не имеет никакого доступа к
  IPC моста (её нет ни в одной capability).
* Та же кнопка кладёт в буфер обмена одним текстом и блок
  `PC-BRIDGE-PAIRING`, и инструкцию для ассистента из `docs/PAIRING_PROMPT.txt`
  (она вкомпилирована в exe через `include_str!`, так что копируется всегда, даже
  если репозитория под рукой нет).

## HTTP API

Все эндпоинты, кроме `/health`, требуют `Authorization: Bearer <key>`.

| Метод | Путь | Назначение |
|---|---|---|
| GET | `/health` | `{"ok":true}` — проверка живости туннеля |
| GET | `/status` | состояние агента, хост, uptime, счётчики |
| GET | `/windows` | кэш списка окон |
| GET | `/journal` | последние 100 записей журнала |
| GET | `/poll` | всегда `{"job":null}` (совместимость) |
| POST | `/note` | `{"text":"…","kind":"stage\|step\|result\|error\|note"}` — запись в лог |
| POST | `/cmd` | см. ниже |
| POST | `/result` | no-op, оставлен для совместимости |

```bash
# команда
curl -s -X POST "$URL/cmd" -H "Authorization: Bearer $KEY" \
     -H 'content-type: application/json' \
     -d '{"command":"dir C:\\Users","shell":"cmd","timeout":60}'

# скриншот
curl -s -X POST "$URL/cmd" -H "Authorization: Bearer $KEY" \
     -d '{"type":"screenshot"}'

# мышь и клавиатура
curl -s -X POST "$URL/cmd" -H "Authorization: Bearer $KEY" -d '{
  "type":"gui",
  "actions":[
    {"action":"moveTo","x":640,"y":420,"duration":0.2},
    {"action":"click","x":640,"y":420},
    {"action":"typewrite","text":"hello","interval":0.02},
    {"action":"hotkey","keys":["ctrl","s"]}
  ]}'
```

Коды: `0` успех, `95` цель не принимает фоновый ввод, `124` таймаут,
`126` денайлист, `130` failsafe (виртуальный курсор в левом верхнем углу),
`400` некорректный запрос.

## Сборка

Локально не требуется ничего, кроме Rust (и то по желанию):

```bash
cargo build --release --manifest-path src-tauri/Cargo.toml
cargo test  --manifest-path src-tauri/Cargo.toml
```

Основной путь — CI: пуш в `master` собирает exe и выкладывает артефакт
`pc-bridge-windows-x64`. Пуш тега `v*` дополнительно прикрепляет exe к релизу.

```bash
git tag v1.0.0 && git push origin v1.0.0
```

Установщик NSIS (`bundle.targets`) можно собрать через `cargo tauri build`,
но для работы достаточно одного exe.

## Где что лежит

```
%LOCALAPPDATA%\PC-Bridge\
├── pairing.key           блок PC-BRIDGE-PAIRING (ключ + URL туннеля)
├── pc_agent_log.txt      журнал всех действий
└── bin\cloudflared.exe   туннель, скачивается один раз
```

## Безопасность

* Ключ = полный доступ к ПК. «🎲 Новый ключ» мгновенно отзывает старый.
* `gui`-действия не трогают физические мышь и клавиатуру; failsafe — левый верхний угол.
* Туннель публичный (quick tunnel Cloudflare): ключ — единственная защита,
  поэтому он передаётся только в заголовке и не попадает в логи.
* `cloudflared` проверяется по Authenticode-подписи; непроверенный файл удаляется.
* Приложение **всегда** запускается с правами администратора: в манифест
  (`src-tauri/windows-app-manifest.xml`) зашит `requestedExecutionLevel =
  requireAdministrator`, поэтому Windows сама показывает UAC при старте, а все
  команды с ключом выполняются уже с повышенными правами. Учитывайте это: ключ
  даёт доступ к админскому процессу.
* Денайлист (`format`, `diskpart`, `bcdedit`, …) — предохранитель, не песочница.

## Ограничения

* Только Windows x64.
* Фоновый ввод работает не везде: браузеры на canvas, игры, elevated-окна и UAC
  могут игнорировать `PostMessage`. Отсутствие эффекта возвращается как код `95`,
  тихого отката на системный курсор нет.
* Quick tunnel даёт новый адрес при каждом перезапуске — копируйте новый блок ключа.
