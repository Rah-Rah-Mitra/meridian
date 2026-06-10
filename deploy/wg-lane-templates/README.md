# WireGuard region-lane templates (SPEC §12.3) — Phase 4

Each `region:<id>` lane = one operator-managed host WireGuard interface with its own
source IP. `meridiand` (host-network `regions` profile) binds outbound sockets to
that source IP via reqwest `local_address`; Linux policy routing steers the traffic.

**Meridian never auto-mutates host routing.** This directory will generate scripts
the operator reviews and runs. Shape of the recipe per region:

```sh
# 1. Operator's wg interface (they bring their own endpoint + keys):
#    wg-quick up /etc/wireguard/eu-west.conf        # iface wg-eu-west, src 10.77.1.2

# 2. Policy routing: traffic FROM the lane's source IP uses the lane's table:
#    ip route add default dev wg-eu-west table 110
#    ip rule  add from 10.77.1.2 lookup 110

# 3. meridian.toml:
#    [[lanes.region]]
#    id = "eu-west"
#    source_ip = "10.77.1.2"     # meridiand needs ONLY this — never the private key
```

On bring-up, the lane fetches an IP-echo endpoint and asserts the observed egress
IP/region matches expectation (GeoLite2 if present); mismatch => Degraded. Region
lanes are for lawful region-vantaged retrieval; robots.txt + shared per-domain rate
limits apply identically (SPEC §12.5).

Keys stay in the operator's wg config under /etc/wireguard — outside Meridian's
/data volume and outside this repo.
