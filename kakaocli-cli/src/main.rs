use clap::{Parser, Subcommand};
use kakaocli_core::model::*;
use kakaocli_db::Database;

mod display;

/// kakaocli-rs: Cross-platform KakaoTalk CLI (macOS + Windows)
#[derive(Parser)]
#[command(name = "kakaocli", version, about, aliases = ["kakaocli-rs"])]
struct Cli {
    /// Enable JSON output for supported commands
    #[arg(global = true, long)]
    json: bool,

    /// Verbose output
    #[arg(global = true, long, short)]
    verbose: bool,

    /// Override database path (auto-detect by default)
    #[arg(global = true, long)]
    db_path: Option<String>,

    /// Override database key (auto-derive by default)
    #[arg(global = true, long)]
    key: Option<String>,

    /// Override user ID (auto-detect by default)
    #[arg(global = true, long)]
    user_id: Option<u64>,

    /// Override device UUID (auto-detect by default)
    #[arg(global = true, long)]
    uuid: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 앱/권한/DB 상태 확인
    #[command(aliases = ["status"])]
    Check,

    /// DB 복호화 검증
    Auth {
        /// Show derived key/DB name for debugging
        #[arg(long)]
        verbose: bool,
    },

    /// 채팅방 목록
    #[command(aliases = ["rooms"])]
    Chats {
        #[arg(long, default_value = "50")]
        limit: u32,
    },

    /// 메시지 조회
    #[command(aliases = ["messages"])]
    Msg {
        /// 채팅방 이름 (부분일치)
        #[arg(long)]
        chat: String,

        /// 시간 범위 (예: 1h, 30m, 7d, 2026-09-07)
        #[arg(long)]
        since: Option<String>,

        #[arg(long, default_value = "50")]
        limit: u32,
    },

    /// 메시지 검색 (LIKE %keyword%)
    #[command(aliases = ["search"])]
    Find {
        /// 검색어
        keyword: String,

        /// 정확히 일치 (= keyword)
        #[arg(long)]
        exact: bool,

        /// 정규식 검색 (REGEXP)
        #[arg(long)]
        regex: bool,

        /// 채팅방 이름 검색
        #[arg(long)]
        rooms: bool,

        /// 친구 이름 검색
        #[arg(long)]
        friends: bool,

        /// 통합 검색 (메시지+방+친구)
        #[arg(long)]
        all: bool,
    },

    /// Raw SQL 쿼리 (읽기 전용)
    Query {
        sql: String,
    },

    /// 메시지 전송 (Phase 2)
    #[command(aliases = ["say"])]
    Send {
        chat: String,
        message: String,
        /// 나와의 채팅으로 전송
        #[arg(long)]
        me: bool,
        /// 미리보기 (전송 안 함)
        #[arg(long)]
        dry_run: bool,
    },

    /// 새 메시지 모니터링 (Phase 4)
    #[command(aliases = ["tail"])]
    Sync {
        /// 실시간 스트리밍
        #[arg(long)]
        follow: bool,
        /// 폴링 간격 (초)
        #[arg(long, default_value = "2")]
        interval: u32,
    },

    /// AX/UIA 트리 덤프 (Phase 2)
    Inspect {
        #[arg(long)]
        chat: Option<String>,
    },

    /// 로그인/인증
    Login {
        #[arg(long)]
        email: Option<String>,

        #[arg(long)]
        password: Option<String>,

        /// 저장된 로그인 상태 확인
        #[arg(long)]
        status: bool,

        /// 저장된 인증 정보 삭제
        #[arg(long)]
        clear: bool,
    },
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    let result = match &cli.command {
        Command::Check { .. } => cmd_check(&cli),
        Command::Auth { verbose } => cmd_auth(&cli, *verbose),
        Command::Chats { limit } => cmd_chats(&cli, *limit),
        Command::Msg { chat, since, limit } => cmd_messages(&cli, chat, since.as_deref(), *limit),
        Command::Find { keyword, exact, regex, rooms, friends, all } => {
            cmd_find(&cli, keyword, *exact, *regex, *rooms, *friends, *all)
        }
        Command::Query { sql } => cmd_query(&cli, sql),
        Command::Send { chat , message , me , dry_run  } => {
            cmd_send(&cli, chat, message, *me, *dry_run)
        }
        Command::Sync { .. } => cmd_not_implemented("sync (Phase 4)"),
        Command::Inspect { .. } => cmd_not_implemented("inspect (Phase 2)"),
        Command::Login { email, password, status, clear } => {
            cmd_login(&cli, email.as_deref(), password.as_deref(), *status, *clear)
        }
    };

    if let Err(err) = result {
        eprintln!("Error: {}", err);
        std::process::exit(1);
    }
}

// ── Helpers ────────────────────────────────────────────────

fn not_implemented_yet(feature: &str) -> ! {
    eprintln!("{} is not yet implemented (see roadmap in docs/roadmap.md)", feature);
    std::process::exit(1);
}

fn open_db(cli: &Cli) -> Result<Database, String> {
    use kakaocli_platform::PlatformBackend;

    // Resolve DB key
    let db_key = if let (Some(path), Some(hex)) = (&cli.db_path, &cli.key) {
        // Manual override — useful for testing on Linux
        kakaocli_core::model::DbKey {
            key_hex: hex.clone(),
            db_path: std::path::PathBuf::from(path),
        }
    } else {
        kakaocli_platform::Platform::resolve_db_key()
            .map_err(|e| format!("Cannot resolve DB key: {}", e))?
    };

    let my_uid = if let Some(uid) = cli.user_id {
        uid as i64
    } else {
        // Try to get from platform, fallback to 0
        kakaocli_core::db_path::mac_user_id()
            .map(|id| id as i64)
            .unwrap_or(0)
    };

    let db = Database::open(&db_key.db_path, &db_key.key_hex, my_uid)
        .map_err(|e| format!("Failed to open database: {}", e))?;

    Ok(db)
}

// ── Commands ───────────────────────────────────────────────

fn cmd_check(cli: &Cli) -> Result<(), String> {
    use kakaocli_platform::PlatformBackend;

    if cli.verbose {
        println!("kakaocli-rs v{}", env!("CARGO_PKG_VERSION"));
        println!("Platform: {}", std::env::consts::OS);
        println!();
    }

    match kakaocli_platform::Platform::check_status() {
        Ok(status) => {
            let status_str = match &status {
                kakaocli_platform::AppStatus::Ready => "✅ Ready (app running + DB accessible)",
                kakaocli_platform::AppStatus::LoggedOut => "⚠️  App running but not logged in",
                kakaocli_platform::AppStatus::NotRunning => "⚠️  KakaoTalk not running",
                kakaocli_platform::AppStatus::DbAccessible => "✅ DB accessible (app not running)",
            };
            println!("{}", status_str);

            if cli.verbose {
                let db_path = kakaocli_core::db_path::mac_container_path();
                println!("  Container: {}", db_path.display());
                println!("  Full Disk Access: {}", kakaocli_core::db_path::check_full_disk_access());
            }

            Ok(())
        }
        Err(e) => Err(format!("Status check failed: {}", e)),
    }
}

fn cmd_auth(cli: &Cli, verbose: bool) -> Result<(), String> {

    // Verify DB can be opened
    let db = open_db(cli)?;
    let tables = db.verify_tables().map_err(|e| format!("Verify failed: {}", e))?;

    println!("✅ Database opened successfully!");
    println!("   Tables found: {}", tables.len());

    if verbose || cli.verbose {
        for t in &tables {
            println!("   - {}", t);
        }
    }

    Ok(())
}

fn cmd_chats(cli: &Cli, limit: u32) -> Result<(), String> {
    let db = open_db(cli)?;
    let chats = db.list_chats(limit).map_err(|e| format!("Failed to list chats: {}", e))?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&chats).map_err(|e| e.to_string())?);
    } else {
        display::print_chats(&chats);
    }

    Ok(())
}

fn cmd_messages(cli: &Cli, chat: &str, since: Option<&str>, limit: u32) -> Result<(), String> {
    let db = open_db(cli)?;

    // Resolve chat name to chat ID
    let (chat_id, chat_name) = db
        .resolve_chat_id(chat)
        .map_err(|e| format!("Chat lookup failed: {}", e))?
        .ok_or_else(|| format!("Chat '{}' not found", chat))?;

    // Parse since
    let since_ts = parse_since(since);

    let messages = db
        .get_messages(chat_id, since_ts, limit)
        .map_err(|e| format!("Failed to get messages: {}", e))?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&messages).map_err(|e| e.to_string())?);
    } else {
        display::print_messages(&messages, &chat_name);
    }

    Ok(())
}

fn cmd_find(cli: &Cli, keyword: &str, exact: bool, regex: bool, rooms: bool, friends: bool, all: bool) -> Result<(), String> {
    let db = open_db(cli)?;

    if all {
        let results = db.search_all(keyword, 50).map_err(|e| format!("Search failed: {}", e))?;
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&results).map_err(|e| e.to_string())?);
        } else {
            display::print_search_all(&results, keyword);
        }
        return Ok(());
    }

    if rooms {
        let results = db.search_rooms(keyword, 50).map_err(|e| format!("Room search failed: {}", e))?;
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&results).map_err(|e| e.to_string())?);
        } else {
            display::print_chats(&results);
        }
        return Ok(());
    }

    if friends {
        let results = db.search_friends(keyword, 50).map_err(|e| format!("Friend search failed: {}", e))?;
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&results).map_err(|e| e.to_string())?);
        } else {
            display::print_friends(&results);
        }
        return Ok(());
    }

    // Message search (default)
    if exact {
        // Use LIKE superset + filter in Rust
        let results = db.search_messages(keyword, 200).map_err(|e| format!("Search failed: {}", e))?;
        let exact_results: Vec<&Message> = results.iter().filter(|m| {
            m.text.as_deref() == Some(keyword)
        }).collect();

        if cli.json {
            println!("{}", serde_json::to_string_pretty(&exact_results).map_err(|e| e.to_string())?);
        } else {
            display::print_messages_slice(&exact_results, &format!("Exact: \"{}\"", keyword));
        }
        return Ok(());
    }

    if regex {
        let re = regex::Regex::new(keyword).map_err(|e| format!("Invalid regex: {}", e))?;
        let results = db.search_messages("", 200).map_err(|e| format!("Search failed: {}", e))?;
        let regex_results: Vec<&Message> = results.iter().filter(|m| {
            m.text.as_deref().map(|t| re.is_match(t)).unwrap_or(false)
        }).collect();

        if cli.json {
            println!("{}", serde_json::to_string_pretty(&regex_results).map_err(|e| e.to_string())?);
        } else {
            display::print_messages_slice(&regex_results, &format!("Regex: /{}/", keyword));
        }
        return Ok(());
    }

    // Default: LIKE search
    let results = db.search_messages(keyword, 50).map_err(|e| format!("Search failed: {}", e))?;
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&results).map_err(|e| e.to_string())?);
    } else {
        display::print_search_messages(&results, keyword);
    }

    Ok(())
}

fn cmd_query(cli: &Cli, sql: &str) -> Result<(), String> {
    let db = open_db(cli)?;
    let result = db.raw_query(sql).map_err(|e| format!("Query failed: {}", e))?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?);
    } else {
        println!("{}", serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?);
    }

    Ok(())
}

fn cmd_send(cli: &Cli, chat_name: &str, text: &str, me: bool, dry_run: bool) -> Result<(), String> {
    let target_name = if me { "_" } else { chat_name };

    if dry_run {
        println!("🔍 Dry-run: would send to '{}': {}", target_name, text);
        return Ok(());
    }

    kakaocli_platform::Platform::send_message(target_name, text)
        .map_err(|e| format!("Send failed: {}", e))?;

    println!("✅ Message sent to '{}'", target_name);
    Ok(())
}

fn cmd_login(_cli: &Cli, email: Option<&str>, password: Option<&str>, status: bool, clear: bool) -> Result<(), String> {
    if clear {
        kakaocli_auth::clear_credentials().map_err(|e| format!("Failed to clear: {}", e))?;
        println!("Credentials cleared.");
        return Ok(());
    }

    if status {
        if kakaocli_auth::has_credentials() {
            let (email, _) = kakaocli_auth::get_credentials().map_err(|e| format!("Failed to read: {}", e))?;
            println!("✅ Credentials stored for: {}", email);
        } else {
            println!("⚠️  No credentials stored. Use `kakaocli login --email ... --password ...`");
        }
        return Ok(());
    }

    if let (Some(e), Some(p)) = (email, password) {
        kakaocli_auth::store_credentials(e, p).map_err(|e| format!("Failed to store: {}", e))?;
        println!("✅ Credentials stored for: {}", e);
        return Ok(());
    }

    not_implemented_yet("interactive login (use --email + --password)");
}

fn cmd_not_implemented(feature: &str) -> Result<(), String> {
    Err(format!("Not implemented: {}. See docs/roadmap.md for timeline.", feature))
}

// ── Since parser ────────────────────────────────────────────

/// Parse `--since` argument to Unix timestamp (seconds).
/// Supports: 30m, 2h, 7d, 2026-09-07, ISO 8601
fn parse_since(since: Option<&str>) -> Option<i64> {
    let s = since?;

    // Relative formats: 30m, 2h, 7d
    if let Some(n) = s.strip_suffix('m') {
        if let Ok(min) = n.parse::<i64>() {
            return Some(chrono::Utc::now().timestamp() - min * 60);
        }
    }
    if let Some(n) = s.strip_suffix('h') {
        if let Ok(hours) = n.parse::<i64>() {
            return Some(chrono::Utc::now().timestamp() - hours * 3600);
        }
    }
    if let Some(n) = s.strip_suffix('d') {
        if let Ok(days) = n.parse::<i64>() {
            return Some(chrono::Utc::now().timestamp() - days * 86400);
        }
    }

    // Date format: 2026-09-07
    if let Ok(dt) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(dt.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp());
    }

    // ISO 8601: 2026-09-07T10:30:00Z
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp());
    }

    eprintln!("Warning: Could not parse --since '{}'. Using all messages.", s);
    None
}