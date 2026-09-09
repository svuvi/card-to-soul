---
name: card-to-soul
description: "Turn RP character cards (ChubAI, JanitorAI) into SOUL.md personas."
version: 0.1.0
author: Svuvich
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [soul, persona, character-card, chubai, janitorai, roleplay]
    related_skills: []
---

# card-to-soul agent skill

Turn an RP character card (ChubAI, JanitorAI) into a SOUL.md persona.
Transfer is near-verbatim: carry the card's text over, do not summarize it
into generic guidelines. The service does mechanics; judgment is yours.

You need nothing installed: the hosted API at `https://soul.svuvi.ch/proxy`
covers parse, draft, and capture via plain curl. If the user prefers that no
data leaves their machine, the same tool runs locally as a single binary
(`card-proxy`, downloadable from the repo releases, or `cargo build --release`
in `proxy-rs/`): `card-proxy parse …`, `card-proxy to-soul …`,
`card-proxy proxy …`. Prefer the hosted API unless the user asks for local.

## Workflow

1. User asks for a soul from a character and gives a link.
2. Identify the site, reply with the matching fetch instruction below.
3. User hands back a file (Chub) or a `/r/<id>` link + saved body (Janitor).
4. Run the CLI to normalize, then draft SOUL.md, then confirm open questions
   with the user before writing the final file.

## Per-site fetch instructions

### ChubAI
Tell the user:
- open the character page in their own browser,
- use Export / download the card (JSON or the PNG image — both work),
- send you the file.
Then: normalize it into `card.json` — hosted:
`curl -X POST https://soul.svuvi.ch/proxy/api/parse --data-binary @<file> -o card.json`
(local alternative: `card-proxy parse <file> --out card.json`).
PNG files often embed the full card JSON in `chara` text chunks.

### JanitorAI
Public pages show lore text only; hidden definitions never appear on the page.
If the definition is visible, the user can paste it. Otherwise use the proxy
catcher (hosted — nothing to run; a local `card-proxy proxy` works the same
with an https tunnel, e.g. `cloudflared tunnel --url http://localhost:3000`):
- tell the user: open the character chat, switch the API to proxy/custom,
  endpoint `<base-url>/v1/chat/completions`, any model and key (e.g. `x` / `x`),
  save, send one message like "hi",
- the reply contains a `…/r/<id>` link; ask the user to paste the link,
  download the JSON from it yourself, save it, and run `to-soul`.
  (One less step for the user: they only copy-paste, you do the fetching.)
Capture files land in `./store/<id>.json` and expire after `--ttl-hours` (72h default).
Any API key and model name are accepted — tell the user to type whatever,
e.g. key `x`, model `mock-model-1`.

## Multi-character cards

Some cards define several characters in one page/request. If you detect more
than one (multiple `chara` chunks, several `<Name>` tags, several names in
the captured body), STOP and ask the user which one they want in SOUL.md.
Never pick silently.

## Drafting SOUL.md

- Base draft, hosted:
  `curl -X POST https://soul.svuvi.ch/proxy/api/to-soul --data-binary @card.json`
  (local alternative: `card-proxy to-soul card.json`). Macros resolved,
  scaffolding stripped.
- Adapt by hand from there: third person, drop the pronoun/subject where the
  context is clear. Keep specifics — name, age, looks, habits, backstory,
  speech patterns, contradictions. A soul full of specifics beats a soul full
  of "be warm / be concise".
- Scene setting, side plots, and system instructions are RP scaffolding, not
  soul material by default — but the call is the user's. Explain the tradeoff:
  every kept paragraph rides along in every session (context bloat, tokens,
  a more rigid agent). If the user wants the scene in, it goes in.
- Harness check: after drafting, re-read the soul and verify each paragraph
  traces back to the card. No invented traits.

## Sensitive content

Character cards can contain sexual, traumatic, self-harm, or otherwise
heavy material, including traits that would be destructive "in character"
for an agent with machine access. The CLI deliberately does NOT filter or
flag content. Before adding such material to a soul, tell the user what you
found and confirm explicitly. Never silently drop it, never silently keep it.
