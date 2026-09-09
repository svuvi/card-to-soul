//! card2soul — standalone tool turning RP character cards into SOUL.md personas:
//! `parse` and `to-soul` run locally, `serve` runs the definition-catcher API.
//! Config via args or env: --port / PORT, --dir / STORE_DIR, --ttl-hours / TTL_HOURS,
//! --public-url / PUBLIC_URL. Any API key and model name are accepted.
use base64::Engine as _;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
struct Config {
    port: u16,
    dir: PathBuf,
    ttl_secs: u64,
    public_url: String,
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn load_config() -> Config {
    let args: Vec<String> = std::env::args().collect();
    let env = |k: &str| std::env::var(k).ok();
    Config {
        port: arg(&args, "--port")
            .or_else(|| env("PORT"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(3000),
        dir: arg(&args, "--dir")
            .or_else(|| env("STORE_DIR"))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("./store")),
        ttl_secs: (arg(&args, "--ttl-hours")
            .or_else(|| env("TTL_HOURS"))
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(72.0)
            * 3600.0) as u64,
        public_url: arg(&args, "--public-url")
            .or_else(|| env("PUBLIC_URL"))
            .map(|s| s.trim_end_matches('/').to_string())
            .unwrap_or_default(),
    }
}

// 12 random base64url chars: /dev/urandom, time+pid+counter hash fallback.
static CTR: AtomicU64 = AtomicU64::new(0);

fn new_id() -> String {
    let mut buf = [0u8; 9];
    if fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf).map(|_| ()))
        .is_err()
    {
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut h = DefaultHasher::new();
        (t, std::process::id(), CTR.fetch_add(1, Ordering::Relaxed)).hash(&mut h);
        let hb = h.finish().to_be_bytes();
        buf = [
            hb[0],
            hb[1],
            hb[2],
            hb[3],
            hb[4],
            hb[5],
            hb[6],
            hb[7],
            (t & 0xff) as u8,
        ];
    }
    const ALPH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut n: u128 = 0;
    for b in buf {
        n = (n << 8) | b as u128;
    }
    let mut out = String::with_capacity(12);
    for _ in 0..12 {
        out.push(ALPH[(n & 63) as usize] as char);
        n >>= 6;
    }
    out
}

// Character <Tag> names from system messages (multi-char cards carry several).
// Mirrors the JS proxy: skips system/scenario/example_dialogs/roleplay_guidelines,
// keeps tags whose body is longer than 200 chars.
fn characters_of(messages: &Value) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let skip = [
        "system",
        "scenario",
        "example_dialogs",
        "roleplay_guidelines",
        "userpersona",
    ];
    let arr = match messages.as_array() {
        Some(a) => a,
        None => return names,
    };
    for m in arr {
        if m.get("role").and_then(|r| r.as_str()) != Some("system") {
            continue;
        }
        let content = match m.get("content").and_then(|c| c.as_str()) {
            Some(c) => c.to_string(),
            None => continue,
        };
        // strip <system>...</system> to avoid matching it
        let mut body = content;
        while let Some(s) = body.find("<system>") {
            match body[s..].find("</system>") {
                Some(e) => body.replace_range(s..s + e + "</system>".len(), ""),
                None => break,
            }
        }
        let b = body.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] != b'<' {
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j < b.len() && b[j] != b'>' {
                j += 1;
            }
            if j >= b.len() {
                break;
            }
            let tag = String::from_utf8_lossy(&b[i + 1..j]).trim().to_string();
            if tag.is_empty() || tag.starts_with('/') || tag == "system" {
                i = j + 1;
                continue;
            }
            let close = format!("</{}>", tag);
            match body[j + 1..].find(&close) {
                Some(e) => {
                    let inner = &body[j + 1..j + 1 + e];
                    if inner.len() > 200
                        && !skip.contains(&tag.to_lowercase().as_str())
                        && !names.contains(&tag)
                    {
                        names.push(tag);
                    }
                    i = j + 1 + e + close.len();
                }
                None => i = j + 1,
            }
        }
    }
    names
}

fn sweep(dir: &std::path::Path, ttl_secs: u64) {
    let cutoff = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(u64::MAX)
        .saturating_sub(ttl_secs);
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() < cutoff)
            .unwrap_or(false);
        if old {
            let _ = fs::remove_file(&p);
        }
    }
}

fn valid_id(id: &str) -> bool {
    (8..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn cors(
    res: tiny_http::Response<std::io::Cursor<Vec<u8>>>,
) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    res.with_header(tiny_http::Header::from_bytes("Access-Control-Allow-Origin", "*").unwrap())
}

fn json_resp(code: u16, v: &Value) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    cors(
        tiny_http::Response::from_string(v.to_string())
            .with_status_code(code)
            .with_header(
                tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
            ),
    )
}

fn sse_capture(id: &str, link: &str) -> String {
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let head = json!({
        "id": format!("mock-{}", id), "object": "chat.completion.chunk",
        "created": created, "model": "mock-model-1",
        "choices": [{"index": 0, "delta": {"role": "assistant", "content": format!("Definition captured: {}", link)}, "finish_reason": null}]
    });
    let tail = json!({
        "id": format!("mock-{}", id), "object": "chat.completion.chunk",
        "created": created, "model": "mock-model-1",
        "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]
    });
    format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n", head, tail)
}

// ---------- parse ----------

fn is_png(buf: &[u8]) -> bool {
    buf.len() > 8 && buf[0..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
}

// Embedded chara_card_v2 docs from PNG text chunks (tEXt/iTXt, base64).
// Compressed zTXt is rejected — export JSON instead.
fn extract_png_cards(buf: &[u8]) -> Result<Vec<Value>, String> {
    let mut cards = Vec::new();
    let mut off = 8;
    while off + 8 <= buf.len() {
        let len = u32::from_be_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]) as usize;
        let typ = &buf[off + 4..off + 8];
        if off + 12 + len > buf.len() {
            break;
        }
        let chunk = &buf[off + 8..off + 8 + len];
        if typ == b"tEXt" || typ == b"iTXt" {
            if let Some(nul) = chunk.iter().position(|&b| b == 0) {
                let keyword = String::from_utf8_lossy(&chunk[..nul]);
                if keyword == "chara" {
                    // iTXt layout: keyword \0 compFlag compMethod lang\0 translated\0 text
                    let mut start = nul + 1;
                    if typ == b"iTXt" {
                        if chunk.get(nul + 1) == Some(&1) {
                            // compressed — skip
                            start = usize::MAX;
                        } else {
                            let mut p = nul + 3;
                            for _ in 0..2 {
                                match chunk[p..].iter().position(|&b| b == 0) {
                                    Some(e) => p += e + 1,
                                    None => {
                                        p = usize::MAX;
                                        break;
                                    }
                                }
                            }
                            start = p;
                        }
                    }
                    if start != usize::MAX && start <= chunk.len() {
                        let text = String::from_utf8_lossy(&chunk[start..]).trim().to_string();
                        let engine = base64::engine::general_purpose::STANDARD;
                        match engine
                            .decode(text.as_bytes())
                            .map_err(|e| e.to_string())
                            .and_then(|raw| {
                                serde_json::from_slice::<Value>(&raw).map_err(|e| e.to_string())
                            }) {
                            Ok(v) => cards.push(v),
                            Err(e) => eprintln!("warn: skipping unreadable chara chunk ({})", e),
                        }
                    }
                }
            }
        } else if typ == b"zTXt" {
            eprintln!("warn: zTXt chara chunk skipped (export JSON instead)");
        }
        if typ == b"IEND" {
            break;
        }
        off += 12 + len;
    }
    Ok(cards)
}

fn str_field(d: &Value, k: &str) -> String {
    d.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

// Flatten a Tavern V2 card (or {data,...} wrapper) into card.json.
fn normalize_card(raw: &Value) -> Value {
    let d = raw.get("data").unwrap_or(raw);
    json!({
        "spec": raw.get("spec").and_then(|s| s.as_str()).unwrap_or("chara_card_v2"),
        "name": d.get("name").and_then(|s| s.as_str()).unwrap_or("Unknown"),
        "description": str_field(d, "description"),
        "personality": str_field(d, "personality"),
        "scenario": str_field(d, "scenario"),
        "first_mes": str_field(d, "first_mes"),
        "mes_example": str_field(d, "mes_example"),
        "system_prompt": str_field(d, "system_prompt"),
        "tags": d.get("tags").cloned().unwrap_or(json!([])),
        "creator": d.get("creator").and_then(|s| s.as_str()).unwrap_or(""),
        "avatar": d.get("avatar").and_then(|s| s.as_str()).unwrap_or(""),
        "alternate_greetings": d.get("alternate_greetings").cloned().unwrap_or(json!([])),
        "character_book": d.get("character_book").cloned().unwrap_or(Value::Null),
        "extensions": d.get("extensions").cloned().unwrap_or(json!({})),
    })
}

fn card_summary(card: &Value) -> Value {
    let len = |k: &str| {
        card.get(k)
            .and_then(|v| v.as_str())
            .map(|s| s.len())
            .unwrap_or(0)
    };
    json!({
        "name": card.get("name"),
        "description_len": len("description"),
        "personality_len": len("personality"),
        "scenario_len": len("scenario"),
        "first_mes_len": len("first_mes"),
        "mes_example_len": len("mes_example"),
        "tags": card.get("tags"),
        "creator": card.get("creator"),
        "has_lorebook": !card.get("character_book").map(|v| v.is_null()).unwrap_or(true),
    })
}

// ---------- to-soul ----------

// Near-verbatim transform: card text carried over as-is, third person kept,
// only {{char}}/{{user}} macros resolved and image lines stripped.
// Editorial judgment belongs to the agent, not to this command.
fn resolve_macros(s: &str, name: &str) -> String {
    s.replace("{{char}}", name).replace("{{user}}", "you")
}

fn strip_image_lines(s: &str) -> String {
    s.lines()
        .filter(|l| {
            let t = l.trim();
            !(t.starts_with("![") && t.contains("](") && t.ends_with(')'))
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn to_soul(card: &Value) -> String {
    let name = card
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("Unknown");
    let get = |k: &str| {
        card.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let mut parts: Vec<String> = vec![format!("# Identity\n\nYou are {}.", name)];
    let desc = get("description");
    if !desc.is_empty() {
        parts.push(resolve_macros(&desc, name));
    }
    let pers = get("personality");
    if !pers.is_empty() {
        parts.push(resolve_macros(&pers, name));
    }
    let sc = get("scenario");
    if !sc.is_empty() {
        parts.push(format!("# Scenario\n{}", resolve_macros(&sc, name)));
    }
    let fm = strip_image_lines(&resolve_macros(&get("first_mes"), name));
    if !fm.is_empty() {
        parts.push(format!("# How {} talks (verbatim sample)\n{}", name, fm));
    }
    let me = get("mes_example");
    if !me.is_empty() {
        parts.push(format!(
            "# Dialogue examples (verbatim)\n{}",
            resolve_macros(&me, name)
        ));
    }
    parts.join("\n\n") + "\n"
}

fn read_body(req: &mut tiny_http::Request, cap: u64) -> Option<Vec<u8>> {
    let mut raw = Vec::new();
    req.as_reader().take(cap).read_to_end(&mut raw).ok()?;
    Some(raw)
}

fn is_png_body(req: &tiny_http::Request, raw: &[u8]) -> bool {
    req.headers().iter().any(|h| {
        h.field.to_string().eq_ignore_ascii_case("Content-Type")
            && h.value.as_str().to_ascii_lowercase().contains("png")
    }) || is_png(raw)
}
fn serve(cfg: &Config) {
    fs::create_dir_all(&cfg.dir).expect("cannot create store dir");
    sweep(&cfg.dir, cfg.ttl_secs);
    let cfg_sweep = cfg.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
        sweep(&cfg_sweep.dir, cfg_sweep.ttl_secs);
    });

    let server = tiny_http::Server::http(format!("0.0.0.0:{}", cfg.port)).expect("cannot bind");
    println!(
        "proxy on :{} store={} ttl={}h (any key/model accepted)",
        cfg.port,
        cfg.dir.display(),
        cfg.ttl_secs / 3600
    );
    for mut req in server.incoming_requests() {
        let url = req.url().to_string();
        let method = req.method().as_str().to_string();
        if method == "OPTIONS" {
            let _ = req.respond(cors(
                tiny_http::Response::from_data(Vec::new())
                    .with_status_code(204)
                    .with_header(
                        tiny_http::Header::from_bytes(
                            "Access-Control-Allow-Headers",
                            "authorization, content-type",
                        )
                        .unwrap(),
                    )
                    .with_header(
                        tiny_http::Header::from_bytes(
                            "Access-Control-Allow-Methods",
                            "GET, POST, OPTIONS",
                        )
                        .unwrap(),
                    ),
            ));
            continue;
        }
        if method == "GET" && url == "/v1/models" {
            let _ = req.respond(json_resp(200, &json!({"object":"list","data":[{"id":"mock-model-1","object":"model","created":0,"owned_by":"card-to-soul"}]})));
            continue;
        }
        if method == "GET" && url.starts_with("/r/") {
            let id = url[3..].split(['?', '/']).next().unwrap_or("");
            if !valid_id(id) || id.contains('.') || id.contains('/') {
                let _ = req.respond(json_resp(404, &json!({"error":"bad id"})));
                continue;
            }
            let p = cfg.dir.join(format!("{}.json", id));
            match fs::read(&p) {
                Ok(bytes) => {
                    let _ = req.respond(cors(tiny_http::Response::from_data(bytes).with_header(
                        tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
                    )));
                }
                Err(_) => {
                    let _ = req.respond(json_resp(404, &json!({"error":"expired or unknown id"})));
                }
            }
            continue;
        }
        if method == "POST" && url == "/v1/chat/completions" {
            let mut raw = Vec::new();
            if req
                .as_reader()
                .take(25 * 1024 * 1024)
                .read_to_end(&mut raw)
                .is_err()
            {
                let _ = req.respond(json_resp(400, &json!({"error":"unreadable body"})));
                continue;
            }
            let body: Value = match serde_json::from_slice(&raw) {
                Ok(b) => b,
                Err(_) => {
                    let _ = req.respond(json_resp(400, &json!({"error":"bad JSON"})));
                    continue;
                }
            };
            if body.get("messages").and_then(|m| m.as_array()).is_none() {
                let _ = req.respond(json_resp(400, &json!({"error":"messages[] required"})));
                continue;
            }
            let id = new_id();
            let chars = characters_of(&body["messages"]);
            let doc = json!({"id": id, "at": chrono_now(), "characters": chars, "body": body});
            if fs::write(
                cfg.dir.join(format!("{}.json", id)),
                serde_json::to_string_pretty(&doc).unwrap(),
            )
            .is_err()
            {
                let _ = req.respond(json_resp(500, &json!({"error":"cannot store"})));
                continue;
            }
            let host = req
                .headers()
                .iter()
                .find(|h| h.field.as_str() == "Host")
                .map(|h| h.value.as_str().to_string())
                .unwrap_or_else(|| format!("localhost:{}", cfg.port));
            let base = if cfg.public_url.is_empty() {
                format!("http://{}", host)
            } else {
                cfg.public_url.clone()
            };
            println!("captured {} characters=[{}]", id, chars.join(", "));
            let link = format!("{}/r/{}", base, id);
            // JanitorAI sends stream:true and requires an SSE body, otherwise
            // the UI reports PROXY ERROR even on HTTP 200 with valid JSON.
            if body
                .get("stream")
                .and_then(|s| s.as_bool())
                .unwrap_or(false)
            {
                let _ = req.respond(cors(
                    tiny_http::Response::from_string(sse_capture(&id, &link)).with_header(
                        tiny_http::Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
                    ),
                ));
                continue;
            }
            let _ = req.respond(json_resp(200, &json!({
                "id": format!("mock-{}", id), "object": "chat.completion",
                "created": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
                "model": "mock-model-1",
                "choices": [{"index": 0, "message": {"role": "assistant", "content": format!("Definition captured: {}", link)}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
            })));
            continue;
        }
        if method == "POST" && url == "/api/parse" {
            let raw = match read_body(&mut req, 25 * 1024 * 1024) {
                Some(b) => b,
                None => {
                    let _ = req.respond(json_resp(400, &json!({"error":"unreadable body"})));
                    continue;
                }
            };
            let parsed: Result<Value, String> = if is_png_body(&req, &raw) {
                match extract_png_cards(&raw) {
                    Ok(cards) if cards.is_empty() => {
                        Err("no embedded chara data found in PNG".to_string())
                    }
                    Ok(cards) => {
                        if cards.len() > 1 {
                            eprintln!("note: PNG holds {} cards, using [0]", cards.len());
                        }
                        Ok(cards.into_iter().next().unwrap())
                    }
                    Err(e) => Err(e),
                }
            } else {
                serde_json::from_slice(&raw).map_err(|e| e.to_string())
            };
            match parsed {
                Ok(raw_card) => {
                    let _ = req.respond(json_resp(200, &normalize_card(&raw_card)));
                }
                Err(e) => {
                    let _ = req.respond(json_resp(400, &json!({"error": e})));
                }
            }
            continue;
        }
        if method == "POST" && url == "/api/to-soul" {
            let raw = match read_body(&mut req, 25 * 1024 * 1024) {
                Some(b) => b,
                None => {
                    let _ = req.respond(json_resp(400, &json!({"error":"unreadable body"})));
                    continue;
                }
            };
            match serde_json::from_slice::<Value>(&raw) {
                Ok(raw_card) => {
                    let _ = req.respond(cors(
                        tiny_http::Response::from_string(to_soul(&normalize_card(&raw_card)))
                            .with_header(
                                tiny_http::Header::from_bytes(
                                    "Content-Type",
                                    "text/markdown; charset=utf-8",
                                )
                                .unwrap(),
                            ),
                    ));
                }
                Err(e) => {
                    let _ = req.respond(json_resp(400, &json!({"error": e.to_string()})));
                }
            }
            continue;
        }
        let _ = req.respond(json_resp(404, &json!({"error":"not found"})));
    }
}

fn chrono_now() -> String {
    // ISO-8601-ish UTC without pulling chrono: seconds since epoch + fixed shape.
    let s = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}s-since-epoch", s)
}

fn print_help() {
    println!(
        "card2soul — RP character cards -> SOUL.md\n\
         usage:\n  \
         card2soul [serve] [--port 3000] [--dir ./store] [--ttl-hours 72] [--public-url https://host]\n  \
         card2soul parse <card.json|card.png> [--out card.json] [--index N]\n  \
         card2soul to-soul <card.json> [--out SOUL.md]\n\
         api (same binary in server mode):\n  \
         POST /v1/chat/completions  definition catcher (proxy mode)\n  \
         GET  /r/<id>                captured definition\n  \
         POST /api/parse             card.json or card.png -> normalized card.json\n  \
         POST /api/to-soul           card.json -> soul.md draft (text/markdown)"
    );
}

fn opt_after(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn cmd_parse(argv: &[String]) {
    let input = argv.get(2).unwrap_or_else(|| {
        eprintln!("usage: card2soul parse <card.json|card.png> [--out card.json] [--index N]");
        std::process::exit(1);
    });
    let out = opt_after(argv, "--out");
    let idx: usize = opt_after(argv, "--index")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let buf = fs::read(input).unwrap_or_else(|_| {
        eprintln!("error: file not found: {}", input);
        std::process::exit(1);
    });
    let raw: Value = if is_png(&buf) {
        match extract_png_cards(&buf) {
            Ok(cards) if cards.is_empty() => {
                eprintln!("error: no embedded chara data found in PNG");
                std::process::exit(1);
            }
            Ok(cards) => {
                if cards.len() > 1 && opt_after(argv, "--index").is_none() {
                    eprintln!(
                        "note: PNG holds {} cards, using [0] (pass --index N)",
                        cards.len()
                    );
                }
                cards.into_iter().nth(idx).unwrap_or_else(|| {
                    eprintln!("error: no card at index {}", idx);
                    std::process::exit(1);
                })
            }
            Err(e) => {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        serde_json::from_slice(&buf).unwrap_or_else(|e| {
            eprintln!("error: bad JSON: {}", e);
            std::process::exit(1);
        })
    };
    let card = normalize_card(&raw);
    println!(
        "{}",
        serde_json::to_string_pretty(&card_summary(&card)).unwrap()
    );
    if let Some(o) = out {
        fs::write(&o, serde_json::to_string_pretty(&card).unwrap()).unwrap();
        println!("wrote {}", o);
    }
}

fn cmd_to_soul(argv: &[String]) {
    let input = argv.get(2).unwrap_or_else(|| {
        eprintln!("usage: card2soul to-soul <card.json> [--out SOUL.md]");
        std::process::exit(1);
    });
    let out = opt_after(argv, "--out");
    let buf = fs::read(input).unwrap_or_else(|_| {
        eprintln!("error: file not found: {}", input);
        std::process::exit(1);
    });
    let raw: Value = serde_json::from_slice(&buf).unwrap_or_else(|e| {
        eprintln!("error: bad JSON: {}", e);
        std::process::exit(1);
    });
    let card = normalize_card(&raw);
    let soul = to_soul(&card);
    match out {
        Some(o) => {
            fs::write(&o, &soul).unwrap();
            println!("wrote {} ({} chars)", o, soul.len());
        }
        None => print!("{}", soul),
    }
    if card
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .is_empty()
        && card
            .get("personality")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .is_empty()
    {
        eprintln!("note: card has no description/personality — nothing to transfer");
    }
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    match argv.get(1).map(|s| s.as_str()) {
        Some("parse") => cmd_parse(&argv),
        Some("to-soul") => cmd_to_soul(&argv),
        Some("serve") | None => serve(&load_config()),
        Some(h) if h == "help" || h == "-h" || h == "--help" => print_help(),
        Some(other) => {
            eprintln!("unknown command: {}\n", other);
            print_help();
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_character_tags() {
        let msgs = json!([
            {"role": "system", "content": "<system>rules</system><Vivi>Zloekzot".to_string() + &"x".repeat(300) + "</Vivi><scenario>short</scenario>"},
            {"role": "user", "content": "hi"}
        ]);
        assert_eq!(characters_of(&msgs), vec!["Vivi".to_string()]);
    }

    #[test]
    fn skips_non_character_tags() {
        let msgs = json!([{"role": "system", "content": "<scenario>tiny</scenario>"}]);
        assert!(characters_of(&msgs).is_empty());
    }

    #[test]
    fn parse_and_soul_helpers() {
        let raw = json!({"data": {
            "name": "Vivi",
            "description": "{{char}} is shy. Likes {{user}} quietly.",
            "first_mes": "![](http://x/y.png)\n\"h-hi,\" Vivi stutters.",
        }});
        let card = normalize_card(&raw);
        assert_eq!(card["name"], json!("Vivi"));
        let soul = to_soul(&card);
        assert!(soul.contains("You are Vivi."));
        assert!(soul.contains("Vivi is shy. Likes you quietly."));
        assert!(!soul.contains("{{char}}"));
        assert!(!soul.contains("![](http://x/y.png)"));
        assert!(soul.contains("# How Vivi talks (verbatim sample)"));
    }

    #[test]
    fn png_extract_round_trip() {
        // minimal PNG: sig + IHDR + tEXt(chara=base64 card) + IEND, valid CRCs
        fn crc(data: &[u8]) -> u32 {
            let mut c = 0xFFFF_FFFFu32;
            for b in data {
                c ^= *b as u32;
                for _ in 0..8 {
                    c = if c & 1 == 1 {
                        0xEDB8_8320 ^ (c >> 1)
                    } else {
                        c >> 1
                    };
                }
            }
            !c
        }
        fn chunk(typ: &[u8; 4], data: &[u8], out: &mut Vec<u8>) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            out.extend_from_slice(typ);
            out.extend_from_slice(data);
            let mut c = typ.to_vec();
            c.extend_from_slice(data);
            out.extend_from_slice(&crc(&c).to_be_bytes());
        }
        let card =
            json!({"spec": "chara_card_v2", "data": {"name": "Mia", "description": "lively"}});
        let engine = base64::engine::general_purpose::STANDARD;
        let mut text = b"chara\0".to_vec();
        text.extend_from_slice(engine.encode(serde_json::to_vec(&card).unwrap()).as_bytes());
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0], &mut png);
        chunk(b"tEXt", &text, &mut png);
        chunk(b"IEND", &[], &mut png);
        assert!(is_png(&png));
        let cards = extract_png_cards(&png).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(normalize_card(&cards[0])["name"], json!("Mia"));
    }
    #[test]
    fn sse_body_shape() {
        let b = sse_capture("abc123", "http://x/r/abc123");
        assert!(b.contains("chat.completion.chunk"));
        assert!(b.contains("Definition captured: http://x/r/abc123"));
        assert!(b.trim_end().ends_with("data: [DONE]"));
    }

    #[test]
    fn ids_look_right_and_unique() {
        let a = new_id();
        let b = new_id();
        assert_eq!(a.len(), 12);
        assert!(valid_id(&a));
        assert_ne!(a, b);
        assert!(!valid_id("../evil"));
    }

    // Full HTTP round-trip against a real socket. One check, per project habit.
    #[test]
    fn proxy_round_trip() {
        let dir = std::env::temp_dir().join(format!("cts-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            _ => panic!("unexpected listen addr"),
        };
        let cfg = Config {
            port,
            dir: dir.clone(),
            ttl_secs: 3600,
            public_url: String::new(),
        };
        let cfg2 = cfg.clone();
        std::thread::spawn(move || {
            for mut req in server.incoming_requests() {
                // minimal inline handler: reuse serve() logic via a nested server is overkill;
                // replicate the two endpoints directly.
                let url = req.url().to_string();
                if req.method().as_str() == "POST" && url == "/v1/chat/completions" {
                    let mut raw = Vec::new();
                    req.as_reader().read_to_end(&mut raw).unwrap();
                    let body: Value = serde_json::from_slice(&raw).unwrap();
                    let id = new_id();
                    let chars = characters_of(&body["messages"]);
                    fs::write(
                        cfg2.dir.join(format!("{}.json", id)),
                        serde_json::to_string(
                            &json!({"id": id, "characters": chars, "body": body}),
                        )
                        .unwrap(),
                    )
                    .unwrap();
                    let link = format!("http://127.0.0.1:{}/r/{}", port, id);
                    let _ = req.respond(json_resp(200, &json!({"choices":[{"message":{"content": format!("Definition captured: {}", link)}}]})));
                } else if req.method().as_str() == "GET" && url.starts_with("/r/") {
                    let bytes = fs::read(cfg2.dir.join(format!("{}.json", &url[3..]))).unwrap();
                    let _ = req.respond(cors(tiny_http::Response::from_data(bytes)));
                } else {
                    let _ = req.respond(json_resp(404, &json!({"error":"x"})));
                }
            }
        });
        let payload = serde_json::to_vec(&json!({"messages":[
            {"role":"system","content": format!("<Vivi>{}</Vivi>", "x".repeat(300))},
            {"role":"user","content":"hi"}]}))
        .unwrap();
        let out = std::process::Command::new("curl")
            .args([
                "-s",
                "-X",
                "POST",
                &format!("http://127.0.0.1:{}/v1/chat/completions", port),
                "-H",
                "content-type: application/json",
                "--data-binary",
                "@-",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.take().unwrap().write_all(&payload).unwrap();
                c.wait_with_output()
            })
            .unwrap();
        let resp: Value = serde_json::from_slice(&out.stdout).unwrap();
        let link = resp["choices"][0]["message"]["content"]
            .as_str()
            .unwrap()
            .split(' ')
            .last()
            .unwrap()
            .to_string();
        let got = std::process::Command::new("curl")
            .args(["-s", &link])
            .output()
            .unwrap();
        let doc: Value = serde_json::from_slice(&got.stdout).unwrap();
        assert_eq!(doc["characters"], json!(["Vivi"]));
        assert_eq!(doc["body"]["messages"][1]["content"], json!("hi"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
