#!/bin/sh
# Удаляет Nebula Client. С --purge стирает и ключи всех сетей (/var/lib/nebula-client).
set -eu
if [ "$(id -u)" != 0 ]; then
    exec sudo "$0" "$@"
fi

systemctl disable --now nebula-client.service 2>/dev/null || true
rm -f /etc/systemd/system/nebula-client.service
systemctl daemon-reload
rm -f /usr/local/bin/nebula-client /usr/local/bin/nebula-client-service
rm -f /usr/local/share/applications/nebula-client.desktop /usr/local/share/icons/hicolor/256x256/apps/nebula-client.png

if [ "${1:-}" = "--purge" ]; then
    rm -rf /var/lib/nebula-client
    getent group nebula-client >/dev/null && groupdel nebula-client || true
    echo "Удалено вместе с ключами / Removed together with the keys"
else
    echo "Удалено. Ключи остались в /var/lib/nebula-client / Removed. Keys are kept in /var/lib/nebula-client"
fi
