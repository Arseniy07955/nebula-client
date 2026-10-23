#!/bin/sh
# Nebula Client для Linux: программа в /usr/local/bin, служба systemd, ярлык в меню.
# Запуск из распакованного архива: sudo ./install.sh
set -eu
cd "$(dirname "$0")"

if [ "$(id -u)" != 0 ]; then
    exec sudo "$0" "$@"
fi
if ! command -v systemctl >/dev/null 2>&1; then
    echo "Нужен systemd / systemd is required" >&2
    exit 1
fi

install -m 0755 nebula-client nebula-client-service /usr/local/bin/
install -m 0644 nebula-client.service /etc/systemd/system/nebula-client.service
install -Dm 0644 nebula-client.desktop /usr/local/share/applications/nebula-client.desktop
install -Dm 0644 nebula-client.png /usr/local/share/icons/hicolor/256x256/apps/nebula-client.png

# управлять службой из окна могут участники группы nebula-client
getent group nebula-client >/dev/null || groupadd --system nebula-client
user="${SUDO_USER:-}"
if [ -n "$user" ] && [ "$user" != root ]; then
    usermod -aG nebula-client "$user"
    echo "Пользователь $user добавлен в группу nebula-client (перезайдите в систему) / $user added to the nebula-client group (log in again)"
fi

systemctl daemon-reload
systemctl enable nebula-client.service >/dev/null
systemctl restart nebula-client.service
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q /usr/local/share/applications || true

echo
echo "Готово / Done. nebula-client-service status"
echo "Окно / Window: nebula-client"
