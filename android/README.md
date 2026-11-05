# Nebula Client для Android

Приложение для телефона: вход в сеть Nebula по инвайту из панели
[Nebula Control](https://github.com/Arseniy07955/nebula-control), продление сертификата и VPN-туннель.

## Как устроено

- `core/` — Go-обёртка над `github.com/slackhq/nebula` v1.9.3 (та же версия, что у десктопа). Собирается gomobile в
  `app/libs/nebulacore.aar`. Nebula работает внутри процесса приложения, адаптер ей выдаёт `VpnService`.
- `app/` — Kotlin и Jetpack Compose: вход по инвайту (`/v1/enroll`), `/v1/sync`, `/v1/peers`, `/v1/leave`, окно, служба VPN.
- Ключ устройства создаёт Go-ядро (X25519, PEM как у `nebula-cert keygen`), он остаётся в закрытой папке приложения.
- Сертификат продлевается через WorkManager раз в 6 часов и при подключении, если до конца срока меньше суток.
- DNS сети (например, `10.0.10.7` для `*.neb`) ставится DNS-сервером VPN на время подключения; выключатель в окне.
  Android не умеет слать в VPN-DNS только отдельные зоны, поэтому сервер отвечает за все имена (сервер сети рекурсивный).
  Строгий «Частный DNS» (DoT на имя хоста) ломает DNS VPN: до хоста из VPN-сети нет маршрута и перестают резолвиться все имена.
  Поэтому при строгом режиме приложение DNS в VPN не ставит. Если хост «Частного DNS» резолвится в адрес DNS-сервера сети
  (например, `dns.catpaws.ru` = `217.60.10.228`), `.neb` отвечает он сам и ничего менять не нужно. Если это другой сервер,
  приложение предупредит: выберите «Автоматически» или «Отключено».
- Имена маяков из `static_host_map` ядро резолвит само по DNS активной сети без VPN (в Android у Go нет `/etc/resolv.conf`).

Пока одна сеть на устройство (Android держит один активный VPN).

## Сборка

Нужны JDK 17+ (подойдёт `jbr` из Android Studio), Android SDK 35 и NDK 27, Go и gomobile.

```bash
# 1. ядро
cd android/core
gomobile bind -target=android/arm64,android/arm -androidapi 24 -javapkg=ru.catpaws.nebula -ldflags="-s -w" -o ../app/libs/nebulacore.aar .

# 2. приложение
cd ..
./gradlew assembleRelease     # app/build/outputs/apk/release/app-release.apk
```

## Подпись

Release-APK подписывается ключом из `android/keystore.properties` (`storeFile`, `storeAlias`, `storePassword`,
`keyPassword`); сам файл ключа и `keystore.properties` в git не попадают. **Храните ключ и пароль**: обновление поверх
установленного приложения принимается только с тем же ключом. Без `keystore.properties` release соберётся неподписанным.

Подписи v1, v2 и v3 включены. Приложение ставится вручную («неизвестное приложение»), Google Play Protect может показать
обычное предупреждение о приложении неизвестного разработчика; это не пометка о вредоносности.

## Установка

```bash
adb install -r app/build/outputs/apk/release/app-release.apk
```
