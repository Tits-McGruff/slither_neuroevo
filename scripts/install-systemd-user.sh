#!/bin/sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
TEMPLATE="$SCRIPT_DIR/slither-neuroevo.service.in"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
UNIT_PATH="$UNIT_DIR/slither-neuroevo.service"
LOGIN_NAME=$(id -un)

command -v systemctl >/dev/null 2>&1 || {
  echo "[ERROR] systemctl is required." >&2
  exit 1
}
[ -f "$TEMPLATE" ] || {
  echo "[ERROR] Missing service template: $TEMPLATE" >&2
  exit 1
}

case "$REPO_ROOT" in
  *'|'*|*'&'*|*'\'*|*'"'*|*'%'*|*'$'*|*'
'*|*"$(printf '\r')"*)
    echo "[ERROR] Repository path contains characters unsupported by the service installer: $REPO_ROOT" >&2
    exit 1
    ;;
esac

mkdir -p "$UNIT_DIR"
temporary="${UNIT_PATH}.tmp.$$"
trap 'rm -f "$temporary"' EXIT HUP INT TERM
sed "s|@REPO_ROOT@|$REPO_ROOT|g" "$TEMPLATE" >"$temporary"
chmod 0644 "$temporary"
mv "$temporary" "$UNIT_PATH"
trap - EXIT HUP INT TERM

systemctl --user daemon-reload
systemctl --user enable slither-neuroevo.service

echo "[OK] Installed and enabled $UNIT_PATH"
echo "[INFO] Review server/systemd.env if this host needs overrides."
echo "[INFO] Start now: systemctl --user start slither-neuroevo.service"
echo "[INFO] Logs:      journalctl --user -u slither-neuroevo.service -f"

linger=$(loginctl show-user "$LOGIN_NAME" -p Linger --value 2>/dev/null || true)
if [ "$linger" != "yes" ]; then
  echo "[WARN] User lingering is disabled; this service will stop after the last login session ends."
  echo "[WARN] An administrator must run: sudo loginctl enable-linger $LOGIN_NAME"
fi
