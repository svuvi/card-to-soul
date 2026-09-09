# card-to-soul

Turn RP character cards (ChubAI, JanitorAI) into SOUL.md personas for agents
(Hermes and others). Near-verbatim transfer: the character's own words carry
over, no generic-guideline summaries.

No fetching from the CLI: both sites block datacenter IPs, so the user exports
(Chub) or captures via proxy (Janitor) in their own browser. This repo holds
the parser, the proxy, and the agent skill — nothing more.

## Install

Zero dependencies, Node 18+. Clone and run:

```bash
node soul.js help
```

## Usage

```bash
# Normalize a card (JSON export or PNG with embedded chara data)
node soul.js parse mia.json --out card.json
node soul.js parse vivi.png --out card.json

# Draft a soul (near-verbatim, third person; adapt by hand after)
node soul.js to-soul card.json --out SOUL.md

# Catch a hidden JanitorAI definition
node soul.js proxy --port 3000 --public-url https://your-host
# point the character chat at <public-url>/v1/chat/completions as a custom
# OpenAI endpoint (model mock-model-1, key custom-key), send "hi",
# open the returned /r/<id> link, save it, run to-soul on it.
```

See `SKILL.md` for the full agent workflow (per-site instructions,
multi-character disambiguation, token tradeoff, sensitive content).

## Layout

- `soul.js` — `parse` | `proxy` | `to-soul` (stdlib only)
- `SKILL.md` — instructions for agents doing the conversion
- `store/` — proxy captures (gitignored, TTL-expire)

## License

MIT.
