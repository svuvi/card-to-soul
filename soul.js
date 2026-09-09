#!/usr/bin/env node
// card-to-soul — turn RP character cards into SOUL.md personas.
// Zero dependencies, Node stdlib only. `node soul.js help` for usage.
"use strict";
const fs = require("fs");
const path = require("path");
const http = require("http");
const zlib = require("zlib");
const crypto = require("crypto");

// ---------- shared ----------

function fail(msg) {
  console.error("error: " + msg);
  process.exit(1);
}

function readInput(p) {
  if (!fs.existsSync(p)) fail("file not found: " + p);
  return fs.readFileSync(p);
}

function isPng(buf) {
  return buf.length > 8 && buf.readUInt32BE(0) === 0x89504e47 && buf.readUInt32BE(4) === 0x0d0a1a0a;
}

// ---------- parse ----------

// Extract embedded chara_card_v2 JSON docs from PNG text chunks.
// Returns array of parsed card objects (usually 1; some files carry 2).
function extractPngCards(buf) {
  const cards = [];
  let off = 8;
  while (off + 8 <= buf.length) {
    const len = buf.readUInt32BE(off);
    const type = buf.toString("ascii", off + 4, off + 8);
    if (off + 12 + len > buf.length) break;
    const chunk = buf.slice(off + 8, off + 8 + len);
    if (type === "tEXt" || type === "iTXt" || type === "zTXt") {
      const nul = chunk.indexOf(0);
      const keyword = chunk.slice(0, nul === -1 ? chunk.length : nul).toString("latin1");
      if (keyword === "chara") {
        let text;
        if (type === "zTXt") {
          text = zlib.inflateSync(chunk.slice(nul + 2)).toString("utf8"); // skip \0 + method byte
        } else if (type === "iTXt") {
          // keyword \0 compFlag compMethod lang\0 translated\0 text
          let p = nul + 3;
          p = chunk.indexOf(0, p) + 1; // lang
          p = chunk.indexOf(0, p) + 1; // translated keyword
          const compFlag = chunk[nul + 1] === 1;
          const raw = chunk.slice(p);
          text = compFlag ? zlib.inflateSync(raw).toString("utf8") : raw.toString("utf8");
        } else {
          text = chunk.slice(nul + 1).toString("latin1");
        }
        try {
          cards.push(JSON.parse(Buffer.from(text.trim(), "base64").toString("utf8")));
        } catch (e) {
          console.error("warn: skipping unreadable chara chunk (" + e.message + ")");
        }
      }
    }
    if (type === "IEND") break;
    off += 12 + len;
  }
  return cards;
}

// Normalize a Tavern V2 card (or {data,...} wrapper) into a flat card.json.
function normalizeCard(raw) {
  const d = raw.data || raw;
  const pick = (v) => (typeof v === "string" ? v : "");
  return {
    spec: raw.spec || "chara_card_v2",
    name: d.name || "Unknown",
    description: pick(d.description),
    personality: pick(d.personality),
    scenario: pick(d.scenario),
    first_mes: pick(d.first_mes),
    mes_example: pick(d.mes_example),
    system_prompt: pick(d.system_prompt),
    tags: Array.isArray(d.tags) ? d.tags : [],
    creator: d.creator || "",
    avatar: d.avatar || "",
    alternate_greetings: Array.isArray(d.alternate_greetings) ? d.alternate_greetings : [],
    character_book: d.character_book || null,
    extensions: d.extensions || {},
  };
}

function cmdParse(args) {
  const input = args[0];
  if (!input) fail("usage: node soul.js parse <card.json|card.png> [--out card.json] [--index N]");
  const outIdx = args.indexOf("--out");
  const out = outIdx === -1 ? null : args[outIdx + 1];
  const idxFlag = args.indexOf("--index");
  const idx = idxFlag === -1 ? 0 : parseInt(args[idxFlag + 1], 10);

  const buf = readInput(input);
  let raw;
  if (isPng(buf)) {
    const cards = extractPngCards(buf);
    if (!cards.length) fail("no embedded chara data found in PNG");
    if (cards.length > 1 && idxFlag === -1)
      console.error("note: PNG holds " + cards.length + " cards, using [0] (pass --index N to pick)");
    raw = cards[idx] || fail("no card at index " + idx);
  } else {
    raw = JSON.parse(buf.toString("utf8"));
  }
  const card = normalizeCard(raw);
  const sum = {
    name: card.name,
    description_len: card.description.length,
    personality_len: card.personality.length,
    scenario_len: card.scenario.length,
    first_mes_len: card.first_mes.length,
    mes_example_len: card.mes_example.length,
    tags: card.tags,
    creator: card.creator,
    has_lorebook: !!card.character_book,
  };
  console.log(JSON.stringify(sum, null, 1));
  if (out) {
    fs.writeFileSync(out, JSON.stringify(card, null, 1));
    console.log("wrote " + out);
  } else {
    console.log("---");
    console.log("(pass --out card.json to save the normalized card)");
  }
}

// ---------- proxy ----------

// Minimal OpenAI-compatible catcher: JanitorAI (or any frontend) posts its
// assembled prompt here, we store the full body and answer with a short
// mock pointing at a retrievable URL — no token-heavy echo in chat.
function cmdProxy(args) {
  const opt = (name, def) => {
    const i = args.indexOf(name);
    return i === -1 ? def : args[i + 1];
  };
  const port = parseInt(opt("--port", process.env.PORT || "3000"), 10);
  const dir = opt("--dir", "./store");
  const ttlH = parseFloat(opt("--ttl-hours", "72"));
  const publicUrl = (opt("--public-url", "") || "").replace(/\/$/, "");
  // Any API key and model name are accepted (frontends require the fields).
  fs.mkdirSync(dir, { recursive: true });

  // TTL sweep on boot.
  const sweep = () => {
    const cutoff = Date.now() - ttlH * 3600 * 1000;
    for (const f of fs.readdirSync(dir)) {
      if (!f.endsWith(".json")) continue;
      const p = path.join(dir, f);
      try {
        if (fs.statSync(p).mtimeMs < cutoff) fs.unlinkSync(p);
      } catch (e) { /* gone */ }
    }
  };
  sweep();
  setInterval(sweep, 3600 * 1000).unref();

  // Character <Tag> names in system messages (multi-char cards carry several).
  function charactersOf(messages) {
    const names = [];
    for (const m of messages || []) {
      if (m.role !== "system" || typeof m.content !== "string") continue;
      const body = m.content.replace(/<system>[\s\S]*?<\/system>/g, "");
      for (const mt of body.matchAll(/<([^>\/][^>]*)>([\s\S]*?)<\/\1>/g)) {
        const tag = mt[1].trim();
        // JanitorAI wraps the user's persona in <UserPersona> — not a character.
        if (["scenario", "example_dialogs", "roleplay_guidelines", "userpersona"].includes(tag.toLowerCase())) continue;
        if (mt[2].length > 200 && !names.includes(tag)) names.push(tag);
      }
    }
    return names;
  }

  const server = http.createServer((req, res) => {
    const send = (code, obj, type) => {
      const body = typeof obj === "string" ? obj : JSON.stringify(obj);
      res.writeHead(code, { "content-type": type || "application/json", "access-control-allow-origin": "*" });
      res.end(body);
    };
    if (req.method === "OPTIONS") {
      res.writeHead(204, {
        "access-control-allow-origin": "*",
        "access-control-allow-headers": "authorization, content-type",
        "access-control-allow-methods": "GET, POST, OPTIONS",
      });
      return res.end();
    }
    if (req.method === "GET" && req.url === "/v1/models")
      return send(200, { object: "list", data: [{ id: "mock-model-1", object: "model", created: Date.now(), owned_by: "card-to-soul" }] });
    if (req.method === "GET" && req.url.startsWith("/r/")) {
      const id = req.url.slice(3).split(/[?/]/)[0];
      if (!/^[A-Za-z0-9_-]{8,64}$/.test(id)) return send(404, { error: "bad id" });
      const p = path.join(dir, id + ".json");
      if (!fs.existsSync(p)) return send(404, { error: "expired or unknown id" });
      res.writeHead(200, { "content-type": "application/json" });
      return fs.createReadStream(p).pipe(res);
    }
    if (req.method === "POST" && req.url === "/v1/chat/completions") {
      let raw = "";
      req.on("data", (c) => { raw += c; if (raw.length > 25 * 1024 * 1024) req.destroy(); });
      req.on("end", () => {
        let body;
        try { body = JSON.parse(raw); } catch (e) { return send(400, { error: "bad JSON" }); }
        if (!Array.isArray(body.messages)) return send(400, { error: "messages[] required" });
        const id = crypto.randomBytes(9).toString("base64url");
        const chars = charactersOf(body.messages);
        fs.writeFileSync(path.join(dir, id + ".json"), JSON.stringify({ id, at: new Date().toISOString(), characters: chars, body }, null, 1));
        const base = publicUrl || "http://" + (req.headers.host || ("localhost:" + port));
        console.log("captured " + id + " characters=[" + chars.join(", ") + "]");
        const link = base + "/r/" + id;
        // JanitorAI sends stream:true and requires an SSE body, otherwise the UI
        // reports PROXY ERROR even on HTTP 200 with valid JSON.
        if (body.stream === true) {
          const created = Math.floor(Date.now() / 1000);
          const head = { id: "mock-" + id, object: "chat.completion.chunk", created, model: "mock-model-1", choices: [{ index: 0, delta: { role: "assistant", content: "Definition captured: " + link }, finish_reason: null }] };
          const tail = { id: "mock-" + id, object: "chat.completion.chunk", created, model: "mock-model-1", choices: [{ index: 0, delta: {}, finish_reason: "stop" }] };
          res.writeHead(200, { "content-type": "text/event-stream", "access-control-allow-origin": "*" });
          return res.end("data: " + JSON.stringify(head) + "\n\ndata: " + JSON.stringify(tail) + "\n\ndata: [DONE]\n\n");
        }
        return send(200, {
          id: "mock-" + id, object: "chat.completion", created: Math.floor(Date.now() / 1000), model: "mock-model-1",
          choices: [{ index: 0, message: { role: "assistant", content: "Definition captured: " + link }, finish_reason: "stop" }],
          usage: { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 },
        });
      });
      return;
    }
    return send(404, { error: "not found" });
  });

  server.listen(port, () => console.log("proxy on :" + port + " store=" + dir + " ttl=" + ttlH + "h (any key/model accepted)"));
}

// Proxy self-test: boot on an ephemeral port, POST a Janitor-style payload,
// follow the returned link, assert the definition round-trips. One check.
function cmdSelfTest() {
  const dir = fs.mkdtempSync(path.join(fs.realpathSync("/tmp"), "cts-"));
  const server = http.createServer(() => {});
  server.listen(0, () => {
    const port = server.address().port;
    server.close();
    const child = require("child_process").spawn(process.execPath, [__filename, "proxy", "--port", String(port), "--dir", dir], { stdio: ["ignore", "pipe", "pipe"] });
    const payload = JSON.stringify({ messages: [{ role: "system", content: "<Vivi>" + "x".repeat(300) + "</Vivi>" }, { role: "user", content: "hi" }] });
    const assert = require("assert");
    let started = false;
    child.stdout.on("data", () => {
      if (started) return;
      started = true;
      const post = http.request({ port, path: "/v1/chat/completions", method: "POST", headers: { authorization: "Bearer custom-key", "content-type": "application/json" } },
        (r1) => {
          let b = "";
          r1.on("data", (c) => (b += c));
          r1.on("end", () => {
            const link = JSON.parse(b).choices[0].message.content.split(" ").pop();
            http.get(link, (r2) => {
              let b2 = "";
              r2.on("data", (c) => (b2 += c));
              r2.on("end", () => {
                try {
                  const got = JSON.parse(b2);
                  assert.deepStrictEqual(got.characters, ["Vivi"]);
                  assert.strictEqual(got.body.messages[1].content, "hi");
                  console.log("PASS proxy round-trip " + link.replace(/:\d+/, ":PORT"));
                  // stream:true must answer SSE or Janitor UI reports PROXY ERROR
                  const sp = http.request({ port, path: "/v1/chat/completions", method: "POST", headers: { "content-type": "application/json" } },
                    (r3) => {
                      let b3 = "";
                      r3.on("data", (c) => (b3 += c));
                      r3.on("end", () => {
                        try {
                          assert.strictEqual(r3.headers["content-type"], "text/event-stream");
                          assert.ok(b3.includes("data: [DONE]"));
                          assert.ok(b3.includes("/r/"));
                          console.log("PASS proxy sse");
                        } catch (e) { console.error("FAIL sse " + e.message); process.exitCode = 1; }
                        child.kill();
                        fs.rmSync(dir, { recursive: true, force: true });
                      });
                    });
                  sp.end(JSON.stringify({ stream: true, messages: [{ role: "system", content: "<Vivi>" + "x".repeat(300) + "</Vivi>" }] }));
                } catch (e) { console.error("FAIL " + e.message); process.exitCode = 1; child.kill(); fs.rmSync(dir, { recursive: true, force: true }); }
              });
            });
          });
        });
      post.end(payload);
    });
    setTimeout(() => { console.error("FAIL timeout"); child.kill(); process.exit(1); }, 15000).unref();
  });
}

// ---------- to-soul ----------

// Near-verbatim transform: card text is carried over as-is, third person kept,
// only {{char}}/{{user}} macros are resolved and RP scaffolding is stripped.
// Editorial judgment (what to cut, sensitive content) belongs to the agent,
// not to this command — see SKILL.md.
function resolveMacros(s, name) {
  return s.split("{{char}}").join(name).split("{{user}}").join("you");
}

function stripImageLines(s) {
  return s.split("\n").filter((l) => !/^\s*!\[.*?\]\(.*?\)\s*$/.test(l)).join("\n").trim();
}

function cmdToSoul(args) {
  const input = args[0];
  if (!input) fail("usage: node soul.js to-soul <card.json> [--out SOUL.md]");
  const outIdx = args.indexOf("--out");
  const out = outIdx === -1 ? null : args[outIdx + 1];
  const card = normalizeCard(JSON.parse(readInput(input).toString("utf8")));
  const name = card.name;

  const parts = [];
  parts.push("# Identity");
  parts.push("You are " + name + ".");
  if (card.description.trim()) parts.push(resolveMacros(card.description.trim(), name));
  if (card.personality.trim()) parts.push(resolveMacros(card.personality.trim(), name));
  if (card.scenario.trim()) parts.push("\n# Scenario\n" + resolveMacros(card.scenario.trim(), name));
  if (card.first_mes.trim()) {
    const sample = stripImageLines(resolveMacros(card.first_mes.trim(), name));
    if (sample) parts.push("\n# How " + name + " talks (verbatim sample)\n" + sample);
  }
  if (card.mes_example.trim()) parts.push("\n# Dialogue examples (verbatim)\n" + resolveMacros(card.mes_example.trim(), name));
  const soul = parts.join("\n\n") + "\n";

  if (out) {
    fs.writeFileSync(out, soul);
    console.log("wrote " + out + " (" + soul.length + " chars)");
  } else {
    process.stdout.write(soul);
  }
  if (!card.description.trim() && !card.personality.trim()) console.error("note: card has no description/personality — nothing to transfer");
}

// ---------- cli ----------

const [cmd, ...rest] = process.argv.slice(2);
if (cmd === "parse") cmdParse(rest);
else if (cmd === "proxy") cmdProxy(rest);
else if (cmd === "proxy-self-test") cmdSelfTest();
else if (cmd === "to-soul") cmdToSoul(rest);
else {
  console.log(`card-to-soul — RP character cards -> SOUL.md (zero deps)
usage:
  node soul.js parse <card.json|card.png> [--out card.json] [--index N]
  node soul.js proxy [--port 3000] [--dir ./store] [--ttl-hours 72] [--public-url https://host]
  node soul.js proxy-self-test
  node soul.js to-soul <card.json> [--out SOUL.md]
workflow:
  chubAI:      export the card (JSON or PNG) from the character page, then: parse -> to-soul
  janitorAI:   point the character chat at this proxy as a custom OpenAI endpoint,
               send one message, open the returned /r/<id> link, save it, then: to-soul`);
}
