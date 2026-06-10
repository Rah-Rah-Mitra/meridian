#!/bin/sh
# Generate the policy-routing helper scripts for ONE region lane (SPEC §12.3).
# Meridian NEVER mutates host routing — this generator emits scripts for the
# operator to review and run themselves.
#
# Usage:   ./make-region.sh <id> <wg-iface> <source-ip> <table>
# Example: ./make-region.sh eu-west wg-eu-west 10.77.1.2 110
#
# Prerequisite: the operator's own WireGuard endpoint, already configured via
# wg-quick (keys stay in /etc/wireguard — Meridian only ever sees <source-ip>).

set -eu

if [ "$#" -ne 4 ]; then
    echo "usage: $0 <id> <wg-iface> <source-ip> <table>" >&2
    exit 64
fi

ID="$1"
IFACE="$2"
SRC="$3"
TABLE="$4"
UP="region-${ID}-up.sh"
DOWN="region-${ID}-down.sh"

cat >"$UP" <<EOF
#!/bin/sh
# Policy routing for Meridian region lane "${ID}" — REVIEW BEFORE RUNNING (root).
# Traffic FROM ${SRC} (meridiand's bound source address) uses table ${TABLE},
# whose default route is the WireGuard interface ${IFACE}.
set -eux
ip route replace default dev ${IFACE} table ${TABLE}
ip rule add from ${SRC} lookup ${TABLE} priority 15${TABLE}
EOF

cat >"$DOWN" <<EOF
#!/bin/sh
# Tear down policy routing for Meridian region lane "${ID}" (root).
set -eux
ip rule del from ${SRC} lookup ${TABLE} priority 15${TABLE}
ip route flush table ${TABLE}
EOF

chmod +x "$UP" "$DOWN"

cat <<EOF
Wrote ${UP} and ${DOWN} — review them, then run ${UP} as root.

meridian config for this lane (toml):
    [lanes]
    regions_enabled = true
    [lanes.regions.${ID}]
    source_ip   = "${SRC}"
    # expected_ip = "<the wg endpoint's public egress IP>"   # strongly recommended
    # verify_url  = "https://checkip.amazonaws.com/"

Bring-up check: meridiand fetches verify_url through this lane and refuses to
serve it until the observed egress IP matches expected_ip (GET /v1/lanes shows
the verification state).
EOF
