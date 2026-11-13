#!/bin/sh
# Install Muxshed on the Aquario STV-2000 (S905W) from a release directory.
# Run ON the box as root, from a directory containing:
#   muxshed-api   (aarch64 release binary, cross-built with cargo-zigbuild)
#   web/          (static frontend build, `npm run build` output)
# Usage: ./install.sh
set -eu

BIN_SRC="./muxshed-api"
WEB_SRC="./web"

[ -x "$BIN_SRC" ] || { echo "missing executable $BIN_SRC" >&2; exit 1; }
[ -d "$WEB_SRC" ] || { echo "missing $WEB_SRC directory" >&2; exit 1; }
command -v ffmpeg >/dev/null || { echo "ffmpeg is required (apt install ffmpeg)" >&2; exit 1; }

systemctl stop muxshed 2>/dev/null || true

install -Dm755 "$BIN_SRC" /usr/local/bin/muxshed-api
mkdir -p /usr/local/share/muxshed
rm -rf /usr/local/share/muxshed/web
cp -r "$WEB_SRC" /usr/local/share/muxshed/web
mkdir -p /var/lib/muxshed/data

# systemd unit lives next to this script in the repo (deploy/aquario/).
if [ -f ./muxshed.service ]; then
    install -Dm644 ./muxshed.service /etc/systemd/system/muxshed.service
fi
systemctl daemon-reload
systemctl enable --now muxshed

echo "Muxshed installed. Web UI: http://$(hostname -I | awk '{print $1}'):8090"
echo "First run: open the web UI to complete setup (admin API key)."
