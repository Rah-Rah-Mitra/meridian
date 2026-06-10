#!/usr/bin/env bash
# Meridian Pi 5 setup — SPEC §7.3 / §14. Idempotent; safe to re-run.
#
# Phase-0 SKELETON: every mutating step is gated behind --apply and is a no-op until
# Phase 1 fills it in. Run without flags for a read-only readiness report.
#
# SERVICE-SAFETY NOTE: on shared devices (e.g. a Pi that also runs other services),
# this script must never touch anything outside Meridian's own scope. It only ever:
# tunes documented sysctls, configures zram, caps journald, and prepares the Meridian
# data directory. It does NOT stop/disable other units, delete caches, or modify
# routing (region-lane routing is operator-reviewed, see wg-lane-templates/).

set -euo pipefail

APPLY=0
[[ "${1:-}" == "--apply" ]] && APPLY=1

note() { printf '>> %s\n' "$*"; }
todo() { printf 'TODO(phase-1) %s\n' "$*"; }

note "Meridian pi-setup (skeleton) — apply=${APPLY}"

# ---------------------------------------------------------------- diagnostics
note "kernel: $(uname -r)  page size: $(getconf PAGESIZE)"
if [[ "$(getconf PAGESIZE)" != "4096" ]]; then
  # SPEC §2 kernel page-size decision — see docs/plan/00-adr.md ADR-01.
  # Current supported configuration is the Pi OS default 16K kernel; all 16K-risky
  # dependencies (mimalloc, usearch mmap view) are smoke-tested on-device in Phase 0.
  note "16K-page kernel detected: this is the supported configuration (ADR-01)."
  note "To switch to 4K instead: add 'kernel=kernel8.img' to /boot/firmware/config.txt and reboot."
fi

free -h | sed 's/^/   /'
df -h / | sed 's/^/   /'

command -v docker >/dev/null || { echo "docker missing — install docker first"; exit 1; }
docker compose version >/dev/null || { echo "docker compose v2 missing"; exit 1; }

# ------------------------------------------------------------------- sysctls
# SPEC §7.3: vm.swappiness=100 (zram-friendly), vm.page-cluster=0,
# dirty_background_ratio=5, dirty_ratio=15 — written to /etc/sysctl.d/90-meridian.conf
todo "write /etc/sysctl.d/90-meridian.conf + sysctl --system"

# ---------------------------------------------------------------------- zram
# SPEC §7.3: 2GB zstd zram; disable SD swapfile. Skipped if zram is already active
# (do not fight an existing configuration on a shared device).
todo "configure zram-tools (2GB, zstd); disable dphys-swapfile"

# ------------------------------------------------------------------ journald
# SPEC §6.1 log budget: journald SystemMaxUse=150M via /etc/systemd/journald.conf.d/
todo "cap journald to 150M (drop-in, not editing the main config)"

# ----------------------------------------------------------------- data dirs
# Single mutable volume (SPEC §9.7) — everything Meridian persists lives here.
todo "create meridian data volume / directory with non-root UID"

# -------------------------------------------------------------------- secrets
todo "generate searxng secret_key + meridiand bearer token into 0600 env files"

note "skeleton complete — nothing was modified${APPLY:+ (apply mode pending Phase 1)}"
