#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ -n "${RIX_IN_LIMITED_SCOPE:-}" ]] || ! command -v systemd-run >/dev/null; then
    exec cargo "$@"
fi
exec systemd-run --user --scope --quiet --same-dir \
    -p MemoryMax=10G -p CPUQuota=600% \
    env RIX_IN_LIMITED_SCOPE=1 cargo "$@"
