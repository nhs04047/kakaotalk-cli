//! Display helpers for kakaocli-cli human-readable output.

use kakaocli_core::model::*;
use kakaocli_platform::AxNode;

use std::fmt::Write as FmtWrite;

/// Print chat rooms in a table.
pub fn print_chats(chats: &[Chat]) {
    if chats.is_empty() {
        println!("No chats found.");
        return;
    }

    for chat in chats {
        let unread = if chat.unread_count > 0 {
            format!(" ({})", chat.unread_count)
        } else {
            String::new()
        };
        let last_msg = chat.last_message_at.map_or("".to_string(), |ts| {
            format!(" {}", format_timestamp(ts))
        });
        println!("  {}{}{}", chat.display_name, unread, last_msg);
    }
    println!();
    println!("{} chats", chats.len());
}

/// Print messages in a readable format.
pub fn print_messages(messages: &[Message], chat_name: &str) {
    if messages.is_empty() {
        println!("No messages in '{}'.", chat_name);
        return;
    }

    println!("--- {} ---", chat_name);

    for msg in messages.iter().rev() {
        let sender = msg.sender_name.as_deref().unwrap_or("(unknown)");
        let text = msg.text.as_deref().unwrap_or("");
        let ts = format_timestamp(msg.created_at);
        let me = if msg.is_from_me { "→" } else { " " };

        if text.len() > 80 {
            println!("{}{} {}: {}", me, ts, sender, &text[..77]);
        } else {
            println!("{}{} {}: {}", me, ts, sender, text);
        }
    }
}

/// Print a slice of messages with a title.
pub fn print_messages_slice(messages: &[&Message], title: &str) {
    if messages.is_empty() {
        println!("No results for {}.", title);
        return;
    }

    println!("--- {} ---", title);
    for msg in messages.iter().rev() {
        let sender = msg.sender_name.as_deref().unwrap_or("(unknown)");
        let text = msg.text.as_deref().unwrap_or("");
        let ts = format_timestamp(msg.created_at);
        let me = if msg.is_from_me { "→" } else { " " };
        println!("{}{} {}: {}", me, ts, sender, text);
    }
    println!("{} results", messages.len());
}

/// Print search results for messages.
pub fn print_search_messages(messages: &[Message], keyword: &str) {
    if messages.is_empty() {
        println!("No messages matching '{}'.", keyword);
        return;
    }

    println!("--- Messages matching '{}' ---", keyword);
    for msg in messages.iter() {
        let sender = msg.sender_name.as_deref().unwrap_or("(unknown)");
        let text = msg.text.as_deref().unwrap_or("");
        let ts = format_timestamp(msg.created_at);
        println!("{} {}: {}", ts, sender, text);
    }
    println!("{} results", messages.len());
}

/// Print combined search results.
pub fn print_search_all(results: &SearchResults, keyword: &str) {
    if !results.messages.is_empty() {
        println!("--- Messages ---");
        for msg in results.messages.iter().take(10) {
            let sender = msg.sender_name.as_deref().unwrap_or("(unknown)");
            let text = msg.text.as_deref().unwrap_or("");
            let ts = format_timestamp(msg.created_at);
            println!("  {} {}: {}", ts, sender, text);
        }
        if results.messages.len() > 10 {
            println!("  ... and {} more", results.messages.len() - 10);
        }
    }

    if !results.rooms.is_empty() {
        println!("--- Rooms ---");
        for room in &results.rooms {
            println!("  {}", room.display_name);
        }
    }

    if !results.friends.is_empty() {
        println!("--- Friends ---");
        for friend in &results.friends {
            let name = friend.display_name.as_deref()
                .or(friend.friend_nick_name.as_deref())
                .or(friend.nick_name.as_deref())
                .unwrap_or("(unknown)");
            println!("  {}", name);
        }
    }

    if results.messages.is_empty() && results.rooms.is_empty() && results.friends.is_empty() {
        println!("No results for '{}'.", keyword);
    }
}

/// Print friends list.
pub fn print_friends(friends: &[Friend]) {
    if friends.is_empty() {
        println!("No friends found.");
        return;
    }

    for f in friends {
        let name = f.display_name.as_deref()
            .or(f.friend_nick_name.as_deref())
            .or(f.nick_name.as_deref())
            .unwrap_or("(unknown)");
        println!("  {}", name);
    }
    println!("{} friends", friends.len());
}

/// Format a Unix timestamp to a human-readable time string.
fn format_timestamp(ts: i64) -> String {
    let dt = chrono::DateTime::from_timestamp(ts, 0);
    match dt {
        Some(dt) => dt.format("%m-%d %H:%M").to_string(),
        None => ts.to_string(),
    }
}

/// Print AX tree for inspect command
pub fn print_ax_tree(node: &AxNode, depth: u32) {
    if depth > 0 && node.role.is_empty() && node.title.is_empty() && node.description.is_empty() && node.children.is_empty() {
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
        meta.push_str(" focused=1");
    }
    if node.selected {
        meta.push_str(" selected=1");
    }

    println!("{}{}[{}]{}", prefix, "[", node.role, meta);

    // If at max depth, show [...] marker
    if depth > 0 && node.children.is_empty() && !node.role.is_empty() {
        let indent = "  ".repeat(depth as usize) + "  ";
        // Check for truncation hint
        if node.role == "…" && node.description == "(max depth)" {
            println!("{}  └── …(max depth)", indent);
        }
    }

    for child in &node.children {
        print_ax_tree(child, depth + 1);
    }
}