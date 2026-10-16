//! kakaocli-mcp: a minimal stdio MCP server exposing kakaocli to AI agents.
//!
//! Transport: newline-delimited JSON-RPC 2.0 over stdin/stdout (MCP stdio).
//! Each tool call shells out to the `kakaocli` binary with `--json` and returns
//! its output, so all platform logic (Windows DEK scan, macOS AX, schema
//! dispatch) is reused verbatim — one source of truth.
//!
//! Safety: the `send` tool only delivers when `confirm: true` is passed;
//! otherwise it runs `--dry-run` (preview). It never shells an interactive
//! prompt (always `--yes` on a confirmed send) so it can't hang headless.

use serde_json::{json, Value};
use std::io::{BufRead, Write};

const PROTOCOL_VERSION: &str = "2024-11-05";

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    let reader = stdin.lock();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[kakaocli-mcp] parse error: {e}");
                continue;
            }
        };

        let id = req.get("id").cloned();
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");

        // Notifications (no id) get no response.
        if id.is_none() {
            continue;
        }
        let id = id.unwrap();

        let response = match method {
            "initialize" => reply(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "kakaocli-mcp", "version": env!("CARGO_PKG_VERSION") }
                }),
            ),
            "ping" => reply(id, json!({})),
            "tools/list" => reply(id, json!({ "tools": tool_specs() })),
            "tools/call" => handle_tool_call(id, req.get("params")),
            other => error(id, -32601, &format!("method not found: {other}")),
        };

        let _ = writeln!(out, "{response}");
        let _ = out.flush();
    }
}

fn reply(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Build a text-content tool result. `is_error` marks a failed call.
fn tool_result(id: Value, text: String, is_error: bool) -> Value {
    reply(
        id,
        json!({
            "content": [ { "type": "text", "text": text } ],
            "isError": is_error
        }),
    )
}

fn tool_specs() -> Value {
    json!([
        {
            "name": "chats",
            "description": "채팅방 목록을 조회한다 (JSON).",
            "inputSchema": {
                "type": "object",
                "properties": { "limit": { "type": "integer", "description": "최대 개수 (기본 50)" } }
            }
        },
        {
            "name": "messages",
            "description": "한 채팅방의 메시지를 조회한다. chat(부분일치) 또는 me(나와의 채팅) 중 하나를 지정.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "chat": { "type": "string", "description": "채팅방 이름 (부분일치)" },
                    "me": { "type": "boolean", "description": "나와의 채팅" },
                    "since": { "type": "string", "description": "시간 범위 (예: 1h, 7d, 2026-09-01)" },
                    "limit": { "type": "integer", "description": "최대 개수 (기본 50)" }
                }
            }
        },
        {
            "name": "find",
            "description": "메시지/방/친구를 검색한다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "keyword": { "type": "string" },
                    "rooms": { "type": "boolean", "description": "방 이름 검색" },
                    "friends": { "type": "boolean", "description": "친구 이름 검색" },
                    "all": { "type": "boolean", "description": "통합 검색" }
                },
                "required": ["keyword"]
            }
        },
        {
            "name": "stats",
            "description": "메시지 통계(방별·날짜별·시간대별)를 집계한다. Windows는 앱에서 연 방만 집계될 수 있다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "since": { "type": "string", "description": "시간 범위 (예: 7d, 30d)" },
                    "top": { "type": "integer", "description": "방별 상위 N (기본 20)" }
                }
            }
        },
        {
            "name": "check",
            "description": "앱/권한/DB 상태를 점검한다.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "send",
            "description": "메시지를 전송한다. 안전을 위해 confirm=true 일 때만 실제 전송하고, 그 외에는 미리보기(dry-run)만 수행한다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "chat": { "type": "string", "description": "채팅방 이름. me=true면 생략" },
                    "me": { "type": "boolean", "description": "나와의 채팅으로 전송" },
                    "message": { "type": "string" },
                    "confirm": { "type": "boolean", "description": "true여야 실제 전송. 기본 false(미리보기)" }
                },
                "required": ["message"]
            }
        }
    ])
}

fn handle_tool_call(id: Value, params: Option<&Value>) -> Value {
    let params = match params {
        Some(p) => p,
        None => return error(id, -32602, "missing params"),
    };
    let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    let argv = match build_argv(name, &args) {
        Ok(a) => a,
        Err(e) => return tool_result(id, e, true),
    };

    match run_kakaocli(&argv) {
        Ok(out) => tool_result(id, out, false),
        Err(e) => tool_result(id, format!("kakaocli 실행 실패: {e}"), true),
    }
}

/// Translate a tool call into kakaocli argv. Read tools always append `--json`.
fn build_argv(name: &str, args: &Value) -> Result<Vec<String>, String> {
    let s = |k: &str| args.get(k).and_then(|v| v.as_str()).map(|v| v.to_string());
    let b = |k: &str| args.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
    let i = |k: &str| args.get(k).and_then(|v| v.as_i64());

    let mut a: Vec<String> = Vec::new();
    match name {
        "chats" => {
            a.push("chats".into());
            if let Some(n) = i("limit") {
                a.push("--limit".into());
                a.push(n.to_string());
            }
            a.push("--json".into());
        }
        "messages" => {
            a.push("msg".into());
            if b("me") {
                a.push("--me".into());
            } else if let Some(c) = s("chat") {
                a.push("--chat".into());
                a.push(c);
            }
            if let Some(since) = s("since") {
                a.push("--since".into());
                a.push(since);
            }
            if let Some(n) = i("limit") {
                a.push("--limit".into());
                a.push(n.to_string());
            }
            a.push("--json".into());
        }
        "find" => {
            let kw = s("keyword").ok_or("keyword가 필요합니다")?;
            a.push("find".into());
            a.push(kw);
            if b("rooms") {
                a.push("--rooms".into());
            }
            if b("friends") {
                a.push("--friends".into());
            }
            if b("all") {
                a.push("--all".into());
            }
            a.push("--json".into());
        }
        "stats" => {
            a.push("stats".into());
            if let Some(since) = s("since") {
                a.push("--since".into());
                a.push(since);
            }
            if let Some(n) = i("top") {
                a.push("--top".into());
                a.push(n.to_string());
            }
            a.push("--json".into());
        }
        "check" => {
            a.push("check".into());
            a.push("--json".into());
        }
        "send" => {
            let msg = s("message").ok_or("message가 필요합니다")?;
            a.push("send".into());
            if b("me") {
                a.push("--me".into());
            } else if let Some(c) = s("chat") {
                a.push(c);
            } else {
                return Err("chat 또는 me가 필요합니다".into());
            }
            a.push(msg);
            if b("confirm") {
                a.push("--yes".into()); // 실제 전송 (비대화형)
            } else {
                a.push("--dry-run".into()); // 기본: 미리보기만
            }
        }
        other => return Err(format!("알 수 없는 도구: {other}")),
    }
    Ok(a)
}

/// Locate the kakaocli binary next to this one; fall back to PATH.
fn kakaocli_path() -> std::path::PathBuf {
    let exe_name = if cfg!(windows) {
        "kakaocli.exe"
    } else {
        "kakaocli"
    };
    if let Ok(cur) = std::env::current_exe() {
        if let Some(dir) = cur.parent() {
            let sibling = dir.join(exe_name);
            if sibling.exists() {
                return sibling;
            }
        }
    }
    std::path::PathBuf::from("kakaocli")
}

fn run_kakaocli(argv: &[String]) -> Result<String, String> {
    let output = std::process::Command::new(kakaocli_path())
        .args(argv)
        .output()
        .map_err(|e| e.to_string())?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        if !err.trim().is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&err);
        }
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn send_defaults_to_dry_run() {
        let a = build_argv("send", &json!({ "chat": "짱정하", "message": "hi" })).unwrap();
        assert_eq!(a, vec!["send", "짱정하", "hi", "--dry-run"]);
    }

    #[test]
    fn send_confirm_true_actually_sends_noninteractive() {
        let a = build_argv(
            "send",
            &json!({ "me": true, "message": "hi", "confirm": true }),
        )
        .unwrap();
        assert_eq!(a, vec!["send", "--me", "hi", "--yes"]);
    }

    #[test]
    fn send_requires_target() {
        assert!(build_argv("send", &json!({ "message": "hi" })).is_err());
    }

    #[test]
    fn messages_me_takes_precedence_over_chat() {
        let a = build_argv("messages", &json!({ "me": true, "chat": "x", "limit": 10 })).unwrap();
        assert_eq!(a, vec!["msg", "--me", "--limit", "10", "--json"]);
    }

    #[test]
    fn read_tools_append_json() {
        let a = build_argv("chats", &json!({ "limit": 5 })).unwrap();
        assert_eq!(a, vec!["chats", "--limit", "5", "--json"]);
    }

    #[test]
    fn find_requires_keyword() {
        assert!(build_argv("find", &json!({})).is_err());
    }
}
