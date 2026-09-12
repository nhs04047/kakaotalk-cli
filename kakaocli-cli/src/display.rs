//! Display helpers for kakaocli-cli human-readable output.

use kakaocli_core::model::*;
use kakaocli_platform::AxNode;

use comfy_table::{
    modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, Attribute, Cell, Color, ContentArrangement,
    Table,
};
use owo_colors::OwoColorize;
use std::fmt::Write as FmtWrite;
use std::io::IsTerminal;

// ── Color / style helpers ─────────────────────────────────

/// Whether ANSI colors should be emitted: respects `NO_COLOR` and only
/// colors when stdout is an actual terminal (not a pipe/file).
pub(crate) fn color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

pub(crate) fn style_header(s: &str) -> String {
    if color_enabled() {
        s.cyan().bold().to_string()
    } else {
        s.to_string()
    }
}

pub(crate) fn style_success(s: &str) -> String {
    if color_enabled() {
        s.green().bold().to_string()
    } else {
        s.to_string()
    }
}

pub(crate) fn style_warn(s: &str) -> String {
    if color_enabled() {
        s.yellow().to_string()
    } else {
        s.to_string()
    }
}

pub(crate) fn style_dim(s: &str) -> String {
    if color_enabled() {
        s.dimmed().to_string()
    } else {
        s.to_string()
    }
}

pub(crate) fn style_err(s: &str) -> String {
    if color_enabled() {
        s.red().bold().to_string()
    } else {
        s.to_string()
    }
}

// ── Table helpers ──────────────────────────────────────────

fn header_cell(text: &str) -> Cell {
    let cell = Cell::new(text);
    if color_enabled() {
        cell.fg(Color::Cyan).add_attribute(Attribute::Bold)
    } else {
        cell
    }
}

fn new_table(headers: &[&str]) -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic);
    table.set_header(headers.iter().map(|h| header_cell(h)));
    table
}

fn time_cell(text: impl std::fmt::Display) -> Cell {
    let cell = Cell::new(text.to_string());
    if color_enabled() {
        cell.fg(Color::DarkGrey)
    } else {
        cell
    }
}

fn sender_cell(msg: &Message) -> Cell {
    let name = msg.sender_name.as_deref().unwrap_or("(알수없음)");
    let label = if msg.is_from_me {
        format!("→ {}", name)
    } else {
        name.to_string()
    };
    let cell = Cell::new(label);
    if !color_enabled() {
        return cell;
    }
    if msg.is_from_me {
        cell.fg(Color::Green).add_attribute(Attribute::Bold)
    } else {
        cell.fg(Color::Blue)
    }
}

fn message_cell(text: &str, highlight: bool) -> Cell {
    let cell = Cell::new(truncate(text, 120));
    if highlight && color_enabled() {
        cell.fg(Color::Yellow).add_attribute(Attribute::Bold)
    } else {
        cell
    }
}

fn unread_cell(count: i32) -> Cell {
    if count <= 0 {
        return Cell::new("-");
    }
    let cell = Cell::new(count.to_string());
    if color_enabled() {
        cell.fg(Color::Yellow).add_attribute(Attribute::Bold)
    } else {
        cell
    }
}

fn chat_type_label(t: &ChatType) -> String {
    match t {
        ChatType::Direct => "1:1".to_string(),
        ChatType::Group => "그룹".to_string(),
        ChatType::Open => "오픈채팅".to_string(),
        ChatType::SelfChat => "나와의채팅".to_string(),
        ChatType::Unknown(n) => format!("기타({})", n),
    }
}

/// Truncate to at most `max_chars` characters, appending `…` if cut.
fn truncate(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}…", head)
    } else {
        head
    }
}

// ── Chats ───────────────────────────────────────────────────

/// Print chat rooms in a table.
pub fn print_chats(chats: &[Chat]) {
    if chats.is_empty() {
        println!("{}", style_warn("채팅방이 없습니다."));
        return;
    }

    let mut table = new_table(&["#", "채팅방", "유형", "인원", "안읽음", "최근 메시지"]);

    for (i, chat) in chats.iter().enumerate() {
        let last_msg = chat
            .last_message_at
            .map_or("-".to_string(), format_timestamp);

        table.add_row(vec![
            Cell::new((i + 1).to_string()),
            Cell::new(&chat.display_name),
            Cell::new(chat_type_label(&chat.chat_type)),
            Cell::new(chat.member_count.to_string()),
            unread_cell(chat.unread_count),
            time_cell(last_msg),
        ]);
    }

    println!("{table}");
    println!("{}", style_dim(&format!("채팅방 {}개", chats.len())));
}

// ── Messages ────────────────────────────────────────────────

/// Print messages in a readable table.
pub fn print_messages(messages: &[Message], chat_name: &str) {
    if messages.is_empty() {
        println!(
            "{}",
            style_warn(&format!("'{}' 채팅방에 메시지가 없습니다.", chat_name))
        );
        return;
    }

    println!("{}", style_header(&format!("─── {} ───", chat_name)));

    let mut table = new_table(&["시간", "발신자", "메시지"]);
    for msg in messages.iter().rev() {
        let text = msg.text.as_deref().unwrap_or("");
        table.add_row(vec![
            time_cell(format_timestamp(msg.created_at)),
            sender_cell(msg),
            message_cell(text, false),
        ]);
    }
    println!("{table}");
    println!("{}", style_dim(&format!("메시지 {}개", messages.len())));
}

/// Print a slice of messages with a title.
#[cfg_attr(windows, allow(dead_code))]
pub fn print_messages_slice(messages: &[&Message], title: &str) {
    if messages.is_empty() {
        println!("{}", style_warn(&format!("'{}'에 대한 결과가 없습니다.", title)));
        return;
    }

    println!("{}", style_header(&format!("─── {} ───", title)));

    let mut table = new_table(&["시간", "발신자", "메시지"]);
    for msg in messages.iter().rev() {
        let text = msg.text.as_deref().unwrap_or("");
        table.add_row(vec![
            time_cell(format_timestamp(msg.created_at)),
            sender_cell(msg),
            message_cell(text, false),
        ]);
    }
    println!("{table}");
    println!("{}", style_dim(&format!("결과 {}건", messages.len())));
}

/// Print search results for messages, highlighting keyword hits.
pub fn print_search_messages(messages: &[Message], keyword: &str) {
    if messages.is_empty() {
        println!(
            "{}",
            style_warn(&format!("'{}'에 대한 메시지가 없습니다.", keyword))
        );
        return;
    }

    println!("{}", style_header(&format!("검색: \"{}\"", keyword)));

    let needle = keyword.to_lowercase();
    let mut table = new_table(&["시간", "발신자", "메시지"]);
    for msg in messages.iter() {
        let text = msg.text.as_deref().unwrap_or("");
        let hit = !needle.is_empty() && text.to_lowercase().contains(&needle);
        table.add_row(vec![
            time_cell(format_timestamp(msg.created_at)),
            sender_cell(msg),
            message_cell(text, hit),
        ]);
    }
    println!("{table}");
    println!("{}", style_dim(&format!("결과 {}건", messages.len())));
}

/// Print combined search results.
#[cfg_attr(windows, allow(dead_code))]
pub fn print_search_all(results: &SearchResults, keyword: &str) {
    let mut any = false;

    if !results.messages.is_empty() {
        any = true;
        println!("{}", style_header("메시지"));
        let mut table = new_table(&["시간", "발신자", "메시지"]);
        for msg in results.messages.iter().take(10) {
            let text = msg.text.as_deref().unwrap_or("");
            table.add_row(vec![
                time_cell(format_timestamp(msg.created_at)),
                sender_cell(msg),
                message_cell(text, false),
            ]);
        }
        println!("{table}");
        if results.messages.len() > 10 {
            println!(
                "{}",
                style_dim(&format!("... 외 {}건", results.messages.len() - 10))
            );
        }
    }

    if !results.rooms.is_empty() {
        any = true;
        println!("{}", style_header("채팅방"));
        print_chats(&results.rooms);
    }

    if !results.friends.is_empty() {
        any = true;
        println!("{}", style_header("친구"));
        print_friends(&results.friends);
    }

    if !any {
        println!("{}", style_warn(&format!("'{}'에 대한 결과가 없습니다.", keyword)));
    }
}

// ── Friends ─────────────────────────────────────────────────

/// Print friends list.
pub fn print_friends(friends: &[Friend]) {
    if friends.is_empty() {
        println!("{}", style_warn("친구가 없습니다."));
        return;
    }

    let mut table = new_table(&["#", "이름"]);
    for (i, f) in friends.iter().enumerate() {
        let name = f
            .display_name
            .as_deref()
            .or(f.friend_nick_name.as_deref())
            .or(f.nick_name.as_deref())
            .unwrap_or("(알수없음)");
        table.add_row(vec![Cell::new((i + 1).to_string()), Cell::new(name)]);
    }
    println!("{table}");
    println!("{}", style_dim(&format!("친구 {}명", friends.len())));
}

/// Print one message as a human-friendly sync line: `MM-DD HH:MM · [room] sender: text`.
/// In single-room mode (`--chat`) the `[room]` prefix is omitted.
pub fn print_sync_message(msg: &Message, room: &str, single_room: bool) {
    let time = format_local_hm(msg.created_at);
    let sender = msg.sender_name.as_deref().unwrap_or("(알수없음)");
    let text = msg.text.as_deref().unwrap_or("");
    let room_prefix = if single_room {
        String::new()
    } else {
        format!("[{}] ", room)
    };

    if color_enabled() {
        let time_s = time.dimmed().to_string();
        let room_s = room_prefix.dimmed().to_string();
        let sender_s = if msg.is_from_me {
            sender.green().bold().to_string()
        } else {
            sender.blue().to_string()
        };
        println!("{} · {}{}: {}", time_s, room_s, sender_s, text);
    } else {
        println!("{} · {}{}: {}", time, room_prefix, sender, text);
    }
}

/// A Unix timestamp as `MM-DD HH:MM` in the local timezone (sync stream lines).
fn format_local_hm(ts: i64) -> String {
    match chrono::DateTime::from_timestamp(ts, 0) {
        Some(dt) => dt
            .with_timezone(&chrono::Local)
            .format("%m-%d %H:%M")
            .to_string(),
        None => ts.to_string(),
    }
}

/// Format a Unix timestamp to a human-readable time string in the local timezone.
fn format_timestamp(ts: i64) -> String {
    match chrono::DateTime::from_timestamp(ts, 0) {
        Some(dt) => dt
            .with_timezone(&chrono::Local)
            .format("%m-%d %H:%M")
            .to_string(),
        None => ts.to_string(),
    }
}

/// Render a `query` result: a table when it's a non-empty array of objects,
/// otherwise pretty-printed JSON. Used for human (non-`--json`) output.
pub fn print_query_result(value: &serde_json::Value) {
    let rows = match value.as_array() {
        Some(rows) if !rows.is_empty() && rows.iter().all(|r| r.is_object()) => rows,
        _ => {
            println!(
                "{}",
                serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
            );
            return;
        }
    };

    // Column order = keys of the first row, then any extra keys appended.
    let mut columns: Vec<String> = Vec::new();
    for row in rows {
        if let Some(obj) = row.as_object() {
            for key in obj.keys() {
                if !columns.iter().any(|c| c == key) {
                    columns.push(key.clone());
                }
            }
        }
    }

    let header_refs: Vec<&str> = columns.iter().map(String::as_str).collect();
    let mut table = new_table(&header_refs);
    for row in rows {
        let obj = row.as_object();
        let cells: Vec<Cell> = columns
            .iter()
            .map(|col| {
                let text = obj
                    .and_then(|o| o.get(col))
                    .map(json_cell_text)
                    .unwrap_or_default();
                Cell::new(truncate(&text, 120))
            })
            .collect();
        table.add_row(cells);
    }

    println!("{table}");
    println!("{}", style_dim(&format!("{}행", rows.len())));
}

/// A scalar JSON value as a compact cell string (objects/arrays fall back to JSON).
fn json_cell_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "-".to_string(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// Print AX tree for inspect command
pub fn print_ax_tree(node: &AxNode, depth: u32) {
    if depth > 0
        && node.role.is_empty()
        && node.title.is_empty()
        && node.description.is_empty()
        && node.children.is_empty()
    {
        return;
    }

    let prefix = if depth == 0 {
        String::new()
    } else {
        let indent = "  ".repeat(depth as usize - 1);
        format!("{}├── ", indent)
    };

    let mut meta = String::new();
    if !node.title.is_empty() {
        let _ = write!(meta, " title=\"{}\"", node.title);
    }
    if !node.description.is_empty() {
        let _ = write!(meta, " description=\"{}\"", node.description);
    }
    if node.focused {
        let _ = write!(meta, " {}", style_warn("focused=1"));
    }
    if node.selected {
        let _ = write!(meta, " {}", style_success("selected=1"));
    }

    println!("{}[{}]{}", prefix, style_header(&node.role), meta);

    // If at max depth, show [...] marker
    if depth > 0 && node.children.is_empty() && !node.role.is_empty() {
        let indent = "  ".repeat(depth as usize) + "  ";
        // Check for truncation hint
        if node.role == "…" && node.description == "(max depth)" {
            println!("{}{}", indent, style_dim("└── …(max depth)"));
        }
    }

    for child in &node.children {
        print_ax_tree(child, depth + 1);
    }
}
