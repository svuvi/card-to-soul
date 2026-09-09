# instruction.md — read this when the user wants a soul.md from an RP character

The user wants to turn a roleplay character definition (ChubAI, JanitorAI,
or pasted text) into a SOUL.md persona file. SOUL.md is the agent's identity
slot: who it is, how it speaks, what it avoids. Your job: guide the user
through fetching the definition, then draft the soul.

Start by telling the user what THEY need to do (step 1 or 2 below).
Do the rest yourself. If requests to the hosted endpoints below fail,
fall back to section 5 (self-hosted) instead of asking the user to fix it.

## 1. ChubAI links (`chub.ai/...`)

Tell the user:
- open the character page in their own browser,
- use Export / download the card (JSON or the PNG image — both work),
- send you the file.

When you get the file, normalize it — hosted, nothing to install:
`curl -X POST https://soul.svuvi.ch/proxy/api/parse --data-binary @<file> -o card.json`
(local alternative: `card2soul parse <file> --out card.json`, single binary
from the repo releases). If it is a PNG, it usually embeds the full card JSON
(`chara` text chunks) — the endpoint extracts it. Normalize to the Tavern V2
fields (`name, description, personality, scenario, first_mes, mes_example, tags`).
Chub authors often leave `personality`/`scenario` empty and pack everything
into `description` — parse that instead of assuming the fields exist.

## 2. JanitorAI links (`janitorai.com/...`)

Public pages show lore text only; hidden definitions never appear on the page.
If the definition is visible, the user can paste it. Otherwise use the proxy:

1. Tell the user: open the character chat, switch the API to a custom /
   OpenAI-compatible proxy endpoint:
   `https://soul.svuvi.ch/proxy/v1/chat/completions`
   — any model name and any key (e.g. `x` / `x`) — save, then send one
   message like "hi".
2. The reply contains a link like
   `https://soul.svuvi.ch/proxy/r/<id>`. Ask the user to paste the link.
3. Download the JSON from the link YOURSELF, save it, continue below.
   (The user only copy-pastes; you do the fetching.)

## 3. Multiple characters in one card

Some cards define several characters. If you detect more than one, STOP and
ask the user which one they want in SOUL.md. Never pick silently.

## 4. Drafting SOUL.md

Transfer is near-verbatim: carry the card's text over, do not summarize it
into generic guidelines ("be warm / be concise" is filler, not persona).
Third person; drop the pronoun/subject where the context is clear.
Keep specifics — name, age, looks, habits, backstory, speech patterns,
contradictions, side characters. A soul full of specifics beats a soul full
of advice. Keep the character's own speech sample (first message) as the
voice reference.

Scene setting, side plots, and system instructions are RP scaffolding, not
soul material by default — but the call is the user's. Explain the tradeoff:
every kept paragraph rides along in every session (context bloat, tokens,
a more rigid agent). If the user wants the scene in, it goes in.

After drafting, re-read the soul and verify each paragraph traces back to
the card. No invented traits.

## 5. Self-hosted mode (no hosted service involved)

If the user chose the self-hosted path, or hosted requests fail: fetch the
`card2soul` binary yourself (releases on GitHub, pick the user's platform)
or build it from source (`cargo build --release` at the repo root) — do not
ask the user to download or build anything. Then use it in place of the curl
endpoints above:
- `card2soul parse <file> --out card.json` replaces `POST /api/parse`,
- `card2soul to-soul card.json` replaces `POST /api/to-soul`,
- `card2soul serve --port 3000` plus an https tunnel
  (e.g. `cloudflared tunnel --url http://localhost:3000`) replaces the
  hosted proxy URL — the user puts the tunnel URL into JanitorAI instead.
The rest of this instruction (multi-character check, verbatim draft,
tradeoff, sensitive content) applies unchanged.

## 6. Sensitive content

Character cards can contain sexual, traumatic, self-harm, or otherwise heavy
material, including traits that would be destructive "in character" for an
agent with machine access. Before adding such material to a soul, tell the
user what you found and confirm explicitly. Never silently drop it, never
silently keep it.
