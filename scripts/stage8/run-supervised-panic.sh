#!/bin/sh
# Disposable Debian unit using the checked-in supervision policy and real server fixture.
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
case "$root" in
  */codex-supervision-*) ;;
  *) echo 'Run only from a dedicated codex-supervision-* checkout.' >&2; exit 1 ;;
esac
report=${1:?Supply the report path to preserve before checkout cleanup.}
cd "$root"
test ! -e "$root/panic.sqlite"
test ! -e "$root/panic.sqlite.checkpoints"
available=$(df -PB1 "$root" | awk 'NR == 2 { print $4 }')
test "$available" -ge 2415919104 || {
  echo 'Fixture filesystem must retain at least 2.25 GiB free before startup.' >&2
  exit 1
}
unit="codex-slither-caught-panic-$(git rev-parse --short=7 HEAD).service"
unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
unit_path="$unit_dir/$unit"
node_path=$(command -v node)
test ! -e "$unit_path"
mkdir -p "$unit_dir"
cleanup() {
  systemctl --user stop "$unit" || true
  rm -f -- "$unit_path"
  systemctl --user daemon-reload
}
trap cleanup EXIT HUP INT TERM
# Preserve Restart, backoff, start limits and termination policy from the real unit.
sed -e "s|@REPO_ROOT@|$root|g" \
  -e "s|^ExecStart=.*|ExecStart=$node_path --import tsx $root/scripts/stage8/supervised-panic-server.ts|" \
  scripts/slither-neuroevo.service.in >"$unit_path"
printf '\n[Service]\nEnvironment=SLITHER_PANIC_FIXTURE_DB=%s/panic.sqlite\nEnvironment=SLITHER_PANIC_FIXTURE_PORT=5181\n' "$root" >>"$unit_path"
systemctl --user daemon-reload
node --import tsx scripts/stage8/supervised-panic-evidence.ts \
  --unit "$unit" --url http://127.0.0.1:5181 --db-path "$root/panic.sqlite" --report "$report"
