#!/bin/sh
set -eu

# Docker creates new host bind-mount directories as root on Linux.
# Prepare the database directory, then run the service without root privileges.
if [ "$(id -u)" = "0" ]; then
    mkdir -p /data/logs
    chown jellyalert:jellyalert /data /data/logs
    for file in /data/jelly-alert.db /data/jelly-alert.db-*; do
        if [ -f "$file" ]; then
            chown jellyalert:jellyalert "$file"
        fi
    done
    exec gosu jellyalert "$@"
fi

exec "$@"
