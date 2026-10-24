# Nebula Client

Подключение ПК к сетям [Nebula](https://github.com/slackhq/nebula) из панели
[Nebula Control](https://github.com/Arseniy07955/nebula-control). Вроде Hamachi для своих: поставил,
нажал «Подключиться» и оказался в общей виртуальной сети с друзьями. Можно быть сразу в нескольких сетях,
например в основной `10.0.10.0/24` и у друзей `10.77.0.0/24`.

*English: see [below](#english).*

Есть два варианта:

- **Программа** (Rust): служба, значок в трее и окно. Windows 10/11 и Linux.
- **PowerShell-скрипт** `nebula-join.ps1`: одна команда без установки, только Windows и одна сеть, описан ниже.

## Программа

### Установка в Windows

Запустите `nebula-client-<версия>-setup.exe` из [релизов](https://github.com/Arseniy07955/nebula-client/releases).
Установщик спросит права администратора, положит программу в `C:\Program Files\Nebula Client`, поставит службу
`NebulaClient`, ярлык в «Пуск» и (по желанию) автозапуск значка в трее. Nebula 1.9.3 уже внутри, GitHub для
первого подключения не нужен. Поверх версии 0.2 ставится без удаления, подключение сохранится.

Удаление через «Параметры → Приложения». Установщик спросит, стереть ли ключи: если да, устройство сначала выйдет
из своих сетей на панели.

### Установка в Linux

Нужны systemd и glibc 2.28+ (Debian 10, Ubuntu 20.04, Fedora, Arch и новее). Окно работает в X11 и Wayland,
значка в трее в Linux нет.

```bash
tar xf nebula-client-<версия>-linux-x86_64.tar.gz
cd nebula-client-<версия>-linux-x86_64
sudo ./install.sh
```

Скрипт кладёт программу в `/usr/local/bin`, ставит службу `nebula-client` и ярлык в меню, создаёт группу
`nebula-client` и добавляет в неё вас: управлять службой из окна могут только её участники (перезайдите в систему).
Если в `PATH` есть `nebula` и `nebula-cert`, служба возьмёт их, иначе скачает официальный релиз и сверит SHA256.
Удаление: `sudo ./uninstall.sh [--purge]`.

### NixOS

```nix
# flake.nix системы
inputs.nebula-client.url = "github:Arseniy07955/nebula-client";

# configuration.nix
imports = [ inputs.nebula-client.nixosModules.default ];
services.nebula-client = {
  enable = true;
  users = [ "alice" ];                             # кто может управлять из окна
  trustedInterfaces = [ "nebula1" "nb-friends" ];  # пустить пиров мимо брандмауэра NixOS
};
```

Nebula берётся из nixpkgs (`services.nebula-client.nebulaPackage`), данные лежат в `/var/lib/nebula-client`.
Сборка через Nix пока не проверялась на живой системе; если что-то не так, пишите в issues.

### Несколько сетей

У каждой сети свой ключ устройства, сертификат и свой процесс `nebula` со своим адаптером (`nebula1`, `nb-friends`
и т. п.; при совпадении имён программа переименует адаптер). Сети включаются и выключаются отдельно: в окне
переключателем сверху, в трее галочками. «+ Добавить сеть» спрашивает имя устройства и, если нужно, инвайт; если на
панели несколько сетей со свободной регистрацией, нужную можно выбрать там же. «Выйти из сети» стирает ключ этой сети
и отзывает сертификат на панели.

Язык окна, трея и сообщений панели берётся из системы (русский или английский). Задать вручную:
`NEBULA_CLIENT_LANG=en` или `ru`.

### Пинг

У участников, которые сейчас в сети, рядом с адресом видна задержка с этого устройства по туннелю Nebula:
зелёная до 60 мс, жёлтая до 150, дальше красная. Служба пингует только пока открыто окно и только тех, кто онлайн
(Windows: `IcmpSendEcho`, Linux: ICMP-сокет без внешнего `ping`).

### DNS сети

Если в панели у сети задан DNS-сервер и зоны (например, `10.0.10.7` и `neb`), клиент отправляет запросы к
`*.neb` на этот сервер, пока сеть подключена. Остальные имена резолвятся как обычно, поэтому если туннель
упал, ломаются только имена сети. Сами записи (`site.neb` → `10.0.10.21`) заводятся на сервере, например
перезаписями в AdGuard Home.

Серверов может быть до четырёх: адрес внутри сети и публичный запасной, например `10.0.10.7, 217.60.10.228`.
Если одна зона есть в нескольких сетях, на Windows получается одно правило со всеми их серверами, адреса внутри
туннелей стоят первыми.

- Windows: правило NRPT (видно в `Get-DnsClientNrptPolicy`), убирается при отключении сети и остановке службы.
- Linux: `resolvectl dns` и `domain ~neb` на адаптере сети, нужен systemd-resolved.
- Выключить для сети: «Настройки» → «DNS сетей» или `nebula-client-service dns СЕТЬ off`.

### Обновления

Раз в сутки служба смотрит [релизы](https://github.com/Arseniy07955/nebula-client/releases). Настраивается в окне
(«Настройки») или командой `update`:

- **Ставить самому** (по умолчанию, Windows): служба скачивает `setup.exe`, сверяет его с `SHA256SUMS.txt` того же
  релиза и ставит тихо. Связь пропадает на несколько секунд, значок в трее возвращается сам.
- **Только сообщать**: в окне и в трее появится «Доступна версия X» с кнопкой «Установить».
- **Не проверять.**
- «Получать тестовые версии»: учитывать pre-release (включено, пока все релизы тестовые).

В Linux служба только сообщает о новой версии: обновление ставится тем же `install.sh` или через Nix.
Установщики не подписаны, поэтому защита держится на SHA256 и на том, что релизы выкладываются только из этого
репозитория.

### Как устроено

| Часть | Что делает | Память |
|---|---|---|
| `nebula-client-service` | служба (SYSTEM в Windows, root в Linux): вход в сети, `nebula` для каждой, продление сертификатов раз в 6 часов, брандмауэр Windows | ~6 МБ |
| `nebula-client` (Windows) | значок в трее: общий цвет состояния, сети галочками, автозапуск | ~9 МБ |
| `nebula-client --window` (в Linux просто `nebula-client`) | окно на egui, отдельный процесс: закрыли окно — процесс завершился | ~60 МБ, пока открыто |

- Ключ устройства создаётся на ПК, приватная часть никуда не уходит. Архив Nebula сверяется по SHA256.
- `nebula` сидит в job-объекте службы (Windows) или получает сигнал при её смерти (Linux): упала служба — завершилась и Nebula.
- Окно и трей говорят со службой по JSON: в Windows через канал `\\.\pipe\nebula-client` (SYSTEM, администраторы,
  вошедшие пользователи, без удалённых подключений), в Linux через сокет `/run/nebula-client/nebula-client.sock`
  (root и группа `nebula-client`).
- Данные: `C:\ProgramData\NebulaClient\networks\<сеть>` или `/var/lib/nebula-client/networks/<сеть>`, доступ только
  у администраторов. Устройство, подключённое PowerShell-скриптом или версией 0.2, переносится само.

Команды службы:

```
nebula-client-service status                    сети и их состояние
nebula-client-service open [--server URL]       какие сети пускают без инвайта
nebula-client-service join --name ИМЯ [--invite КОД] [--net СЕТЬ] [--server URL]
nebula-client-service connect|disconnect|peers|leave СЕТЬ
nebula-client-service sync [СЕТЬ] | logs
nebula-client-service dns СЕТЬ on|off              имена сети через её DNS
nebula-client-service update [check|install]    проверить / поставить найденное обновление
nebula-client-service update auto|notify|off [--pre yes|no]
nebula-client-service install | uninstall [--purge]   (Windows)
nebula-client-service run                       в консоли или под systemd
```

### Сборка

Нужен Rust stable. Свой адрес панели по умолчанию: переменная `NEBULA_CLIENT_SERVER` при сборке.

- **Windows** (`x86_64-pc-windows-gnu`, в `PATH` MinGW: `gcc`, `dlltool`, `windres`): `cargo build --release`.
  Установщик: [Inno Setup 6](https://jrsoftware.org/isinfo.php), затем
  `powershell -File dist\windows\build.ps1 -Iscc путь\ISCC.exe` → `dist/out/nebula-client-<версия>-setup.exe`.
- **Linux**: `cargo build --release`. Из Windows или macOS через [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild):
  `cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.28` (и `aarch64-…`), потом
  `python dist/linux/package.py` → архивы в `dist/out`.
- **Nix**: `nix build`.

## PowerShell-скрипт

### Как подключиться

1. Админ создаёт инвайт в панели на вкладке «Инвайты» и отправляет команду вида
   ```powershell
   & ([scriptblock]::Create((irm 'https://example.org/nebula/v1/join.ps1'))) -Invite NF-XXXX-XXXX-XXXX
   ```
2. Друг открывает PowerShell (лучше от имени администратора, иначе скрипт сам попросит UAC) и вставляет её.
3. Скрипт спросит имя устройства латиницей (по умолчанию имя компьютера) и всё настроит.

Если в сети включена свободная регистрация, `-Invite` не нужен: команда без кода сразу подключает к сети,
адрес выдаётся случайно. Если свободной регистрации нет, скрипт сам спросит код.

### Что делает скрипт

- Скачивает Nebula 1.9.3 для Windows с GitHub и сверяет SHA256 архива с зашитым в скрипт.
- Создаёт ключ устройства (`nebula-cert keygen`). Приватный ключ остаётся на ПК, в панель уходит только публичная часть.
- Отправляет инвайт и публичный ключ на `/v1/enroll`, получает сертификат, `ca.crt`, `config.yml` и токен устройства.
- Кладёт всё в `C:\ProgramData\NebulaClient`, доступ к папке только у SYSTEM и администраторов.
- Ставит службу `Nebula` (`nebula.exe -service install`) с автозапуском и перезапуском при падении.
- Добавляет правила брандмауэра (группа `Nebula Client`): входящие для `nebula.exe` и ping из подсети Nebula,
  а адаптер Nebula переводит в «частную» сеть.
- Регистрирует задачу планировщика `Nebula Client Sync` (при старте и каждые 6 часов, от SYSTEM): она зовёт `/v1/sync`,
  продлевает сертификат и подтягивает свежий конфиг (lighthouse, релеи, список отозванных). Служба перезапускается
  только если что-то поменялось.

### Команды

```powershell
$nc = "$env:ProgramData\NebulaClient\nebula-join.ps1"
& $nc -Status      # адрес, служба, срок сертификата, кто из участников в сети
& $nc -Sync        # синхронизировать сейчас (обычно делает планировщик)
& $nc -Uninstall   # остановить и удалить службу, задачу, правила и папку
```

Для проверки без установки службы: `-NoService -DataDir <папка>` создаёт ключ, получает сертификат и конфиг
и проверяет его через `nebula -test`, ничего не меняя в системе.

### Как это устроено на сервере

Панель поднимает отдельный HTTP-listener только с `/v1/*` (переменные `NP_ENROLL_BIND`, `NP_ENROLL_PORT`,
`NP_ENROLL_URL`), его ставят за reverse proxy с HTTPS. Скрипт панель отдаёт по `/v1/join.ps1` из файла
`NP_CLIENT_PS1`, подставляя свой публичный адрес вместо `__SERVER__`.

| Метод | Что делает |
|---|---|
| `POST /v1/enroll` | `{invite?, name, public_key, conf_dir}` → сертификат, ca.crt, config.yml, токен. Без инвайта только в сеть со свободной регистрацией |
| `POST /v1/sync` | по токену: продление (когда осталось меньше трети срока), свежий конфиг и DNS сети (`dns: {server, domains}` или `null`) |
| `GET /v1/peers` | по токену: участники сети и кто из них в сети |
| `POST /v1/leave` | по токену: устройство выходит само, сертификат отзывается |

Если клиент прислал `Accept-Language: en`, ошибки приходят по-английски.

Сертификаты по инвайту короткие (по умолчанию 30 дней) и продлеваются сами. Отзыв в панели значит, что устройство
больше не продлится, а его отпечаток попадёт в blocklist остальных при их синхронизации.

Если в настройках сети включено «Устройства по инвайтам тоже агенты», в конфиг добавляется отладочный SSH Nebula
на nebula-адресе устройства. Через него панель видит туннели устройства на живой карте. Это только команды Nebula
(список туннелей), доступа к системе он не даёт.

## Ограничения

- Nebula работает на уровне IP, broadcast не проходит. Игры, которые ищут друг друга только «в локальной сети»,
  не увидят друг друга, подключаться нужно по IP.
- PowerShell-скрипт держит одну сеть и ставит службу `Nebula`, рядом с другой установкой Nebula как службы
  он не встанет (проверит и остановится). У программы своя служба и сколько угодно сетей.
- Windows 10/11 x64 (скрипт ещё и ARM64), Linux x86_64 и aarch64 с systemd.
- Бинарники не подписаны, SmartScreen и антивирусы могут ругаться на установщик, `nebula-client*.exe`,
  `nebula.exe` и драйвер Wintun.

## Версия Nebula

Версия и SHA256 зашиты в `crates/service/src/nebula.rs`, `dist/windows/build.ps1` и в начале скрипта
(`$NebulaVersion`, `$NebulaBuilds`). Суммы берутся из `SHASUM256.txt` релиза. Менять версию стоит осознанно, вместе с lighthouse и релеями сети.

## Лицензия

[GNU GPL v3.0](LICENSE). Можно использовать, менять и распространять, но производные версии тоже должны
выходить под GPL-3.0 с открытым кодом.

Программа поставляется «КАК ЕСТЬ» (AS IS), без каких-либо гарантий, явных или подразумеваемых, в том числе
гарантий пригодности для продажи или для конкретной цели. Авторы не отвечают за любой ущерб от её использования,
включая потерю данных, сбои сети или системы. Подробно — разделы 15 и 16 лицензии.

Nebula скачивается с официальных релизов [slackhq/nebula](https://github.com/slackhq/nebula) (MIT),
драйвер Wintun идёт в её архиве под своей лицензией.

## English

Nebula Client joins a PC to [Nebula](https://github.com/slackhq/nebula) overlay networks managed by
[Nebula Control](https://github.com/Arseniy07955/nebula-control), Hamachi-style: install, press Connect, and you share
a virtual LAN with your friends. One device can be in several networks at once, each with its own key, certificate,
`nebula` process and adapter.

- **Windows 10/11**: run `nebula-client-<version>-setup.exe` from the releases. It installs the `NebulaClient`
  service, a Start menu shortcut and an optional tray icon at sign-in. Nebula 1.9.3 is bundled.
- **Linux** (systemd, glibc 2.28+, X11 or Wayland): unpack `nebula-client-<version>-linux-<arch>.tar.gz` and run
  `sudo ./install.sh`. Members of the `nebula-client` group can control the service from the window.
- **NixOS**: add the flake, import `nixosModules.default` and set `services.nebula-client.enable = true;` and
  `users = [ "you" ];` (all options are in the NixOS section above).

The UI follows the system language (Russian or English); override it with `NEBULA_CLIENT_LANG=en`. The service CLI
(`nebula-client-service status | open | join | connect | disconnect | peers | sync | leave | logs`) prints help when
run without arguments. Leaving a network deletes its key and revokes the certificate on the panel.

The member list shows the ping from this device to online peers over the Nebula tunnel. Once a day the service
checks GitHub releases; on Windows it downloads `setup.exe`, verifies it against the release's `SHA256SUMS.txt`
and installs it silently (Settings → Updates, or `nebula-client-service update auto|notify|off`). On Linux it only
notifies.

If the panel sets a DNS server and zones for a network (say `10.0.10.7` and `neb`), the client sends queries for
`*.neb` to that server while the network is connected; everything else resolves as before. Windows uses an NRPT
rule, Linux uses systemd-resolved (`resolvectl domain <tun> ~neb`). Turn it off per network in Settings or with
`nebula-client-service dns NET off`.

Licensed under GPL-3.0 and provided AS IS, without any warranty (sections 15 and 16 of the license).
