# Meridian skills

Portable, self-contained skills for driving a running Meridian appliance —
written so both **humans** (copy-paste the `curl` recipes) and **agents**
(load the `SKILL.md`, follow the procedure) can use one source of truth.

Each skill is a `SKILL.md` with YAML frontmatter + procedural markdown, in the
same format Hermes loads, so an agent platform can consume them unmodified.

| Skill | For | What it does |
|---|---|---|
| [`meridian-search/`](meridian-search/SKILL.md) | everyone | Query `/v1/search`: scope/lane semantics, extractive answer mode, evidence/corroboration, honesty caveats. |
| [`meridian-operate/`](meridian-operate/SKILL.md) | maintainers | Ingest/forget, decision-log + OPE ship-gate, health checks. Bearer required. |

## Integration recipe

Every skill reads two environment variables — set them once:

```sh
export MERIDIAN_URL="http://127.0.0.1:8080"     # the appliance base URL
export MERIDIAN_BEARER_TOKEN="…"                # value of MERIDIAN_BEARER_TOKEN in deploy/.env
```

`$MERIDIAN_URL` is the loopback default; point it at wherever the appliance
listens. `$MERIDIAN_BEARER_TOKEN` is the operator token — in this repo it is
the value of `MERIDIAN_BEARER_TOKEN` in `deploy/.env` (mode 0600). **Never**
hardcode, print, or commit the literal token; the skills only ever reference
the `$MERIDIAN_BEARER_TOKEN` placeholder.

### Humans

Set the two variables in your shell and run the `curl` recipes inside each
`SKILL.md`. Start with `meridian-search`.

### Agents (Hermes and other platforms)

Symlink the skill directory into the agent's skills directory so it loads
alongside the platform's own skills. For Hermes:

```sh
ln -s "$(pwd)/skills/meridian-search" ~/.hermes/skills/meridian-search
```

The agent reads `$MERIDIAN_URL` + `$MERIDIAN_BEARER_TOKEN` from its
environment exactly as a human shell would. The Hermes **Meridian provider
plugin** is the reference integration — see
[`docs/integrations.md`](../docs/integrations.md).

## See also

- [API reference](../docs/api.md) — the canonical, field-exact HTTP contract.
- [Integration guide](../docs/integrations.md) — agent + human integration,
  the SearXNG-format mismatch caveat, retry/degraded handling.
- [Operator manual](../docs/operator-manual.md) — deploy and run the appliance.
