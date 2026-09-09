# card-to-soul

Turn RP character cards (ChubAI, JanitorAI) into SOUL.md personas for agents
(Hermes and others). Near-verbatim transfer: the character's own words carry
over, no generic-guideline summaries.

Two ways to use it. Recommended: the hosted API — plain curl, nothing to
install. Alternative: run the single binary locally, no requests leave
your machine.

## Recommended: hosted API (`https://soul.svuvi.ch/proxy`)

```bash
# ChubAI card (JSON export or PNG image) -> normalized card.json
curl -X POST https://soul.svuvi.ch/proxy/api/parse \
  --data-binary @card.png -o card.json

# card.json -> SOUL.md draft
curl -X POST https://soul.svuvi.ch/proxy/api/to-soul \
  --data-binary @card.json -o SOUL.md
```

Hidden JanitorAI definitions go through the proxy catcher: point the
character chat at `https://soul.svuvi.ch/proxy/v1/chat/completions` as a
custom OpenAI endpoint (any model name and key), send one message, open the
returned `…/r/<id>` link. See `SKILL.md` / the site for the full flow.

## Local: single binary, no server involved

Download `card-proxy-<platform>` from
[releases](https://github.com/svuvi/card-to-soul/releases), then:

```bash
card-proxy parse <card.json|card.png> --out card.json
card-proxy to-soul card.json --out SOUL.md
# one-off catcher instead of the hosted proxy:
card-proxy proxy --port 3000   # + an https tunnel for JanitorAI to reach
```

Build from source: `cd proxy-rs && cargo build --release` (Rust 1.70+).

## Layout

- `proxy-rs/` — the whole tool: `parse` | `to-soul` | `proxy` server
  (`/v1/*` catcher, `/r/<id>`, `/api/parse`, `/api/to-soul`)
- `SKILL.md` — instructions for agents doing the conversion
- `site/` — static page + agent instruction file (served at soul.svuvi.ch)
- `deploy/` — Caddy snippet + systemd unit for the hosted instance
- `store/` — local proxy captures (gitignored, TTL-expire)

## License

MIT.
