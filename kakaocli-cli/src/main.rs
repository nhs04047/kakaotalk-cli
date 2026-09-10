use clap::{Parser, Subcommand};
use kakaocli_core::model::*;
use kakaocli_db::Database;
use std::collections::HashMap;
use std::io::IsTerminal;
use std::time::Duration;

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
    Auth,

    /// 채팅방 목록
    #[command(aliases = ["rooms"])]
    Chats {
        #[arg(long, default_value = "50")]
        limit: u32,
    },

    /// 메시지 조회
    #[command(aliases = ["messages"])]
    Msg {
        /// 채팅방 이름 (부분일치, 생략 시 인터랙티브 선택)
        #[arg(long)]
        chat: Option<String>,

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
        /// 채팅방 이름 (생략 시 인터랙티브 선택)
        chat: Option<String>,
        /// 보낼 메시지 (생략 시 인터랙티브 입력)
        message: Option<String>,
        /// 나와의 채팅으로 전송
        #[arg(long)]
        me: bool,
        /// 미리보기 (전송 안 함)
        #[arg(long)]
        dry_run: bool,
        /// 확인 프롬프트 없이 즉시 전송
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// AX 트리 덤프 (디버깅용, macOS only)
    Inspect {
        /// 채팅방 이름 필터
        #[arg(long)]
        chat: Option<String>,

        /// 트리 깊이 제한 (기본 3)
        #[arg(long, default_value = "3")]
        depth: u32,
    },

    /// 새 메시지 모니터링 (증분 폴링 스트림)
    #[command(aliases = ["tail"])]
    Sync {
        /// 지속 폴링 (생략 시 1회 캐치업 후 종료)
        #[arg(long)]
        follow: bool,
        /// 폴링 간격 (초)
        #[arg(long, default_value = "2")]
        interval: u32,
        /// 특정 채팅방만 (부분일치, 생략 시 전체 방)
        #[arg(long)]
        chat: Option<String>,
        /// 새 메시지 배치를 이 URL로 HTTP POST
        #[arg(long)]
        webhook: Option<String>,
        /// 최초 기준선을 이 시점으로 백필 (예: 1h, 30m, 2026-09-07)
        #[arg(long)]
        since: Option<String>,
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
        Command::Auth => cmd_auth(&cli),
        Command::Chats { limit } => cmd_chats(&cli, *limit),
        Command::Msg { chat, since, limit } => {
            cmd_messages(&cli, chat.as_deref(), since.as_deref(), *limit)
        }
        Command::Find { keyword, exact, regex, rooms, friends, all } => {
            cmd_find(&cli, keyword, *exact, *regex, *rooms, *friends, *all)
        }
        Command::Query { sql } => cmd_query(&cli, sql),
        Command::Send { chat, message, me, dry_run, yes } => {
            cmd_send(&cli, chat.as_deref(), message.as_deref(), *me, *dry_run, *yes)
        }
        Command::Sync { follow, interval, chat, webhook, since } => cmd_sync(
            &cli,
            *follow,
            *interval,
            chat.as_deref(),
            webhook.as_deref(),
            since.as_deref(),
        ),
        Command::Inspect { chat, depth } => cmd_inspect(&cli, chat.as_deref(), *depth),
        Command::Login { email, password, status, clear } => {
            cmd_login(&cli, email.as_deref(), password.as_deref(), *status, *clear)
        }
    };

    if let Err(err) = result {
        eprintln!("{}", display::style_err(&format!("오류: {}", err)));
        std::process::exit(1);
    }
}

// ── Helpers ────────────────────────────────────────────────

fn not_implemented_yet(feature: &str) -> ! {
    eprintln!("{}: 아직 구현되지 않았습니다 (docs/roadmap.md 참고)", feature);
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
        let hint = "힌트: userId를 모르면 `kakaocli --user-id <본인_카카오_id> auth` 실행 (1회 입력, 이후 ~/.kakaocli/user_id 캐시)";
        kakaocli_platform::Platform::resolve_db_key(cli.user_id)
            .map_err(|e| format!("DB 키를 확인할 수 없습니다: {}\n\n{}", e, hint))?
    };

    let my_uid = cli
        .user_id
        .or_else(kakaocli_core::db_path::read_cached_user_id)
        .unwrap_or(0) as i64;

    let db = Database::open(&db_key.db_path, &db_key.key_hex, my_uid)
        .map_err(|e| format!("데이터베이스 열기 실패: {}", e))?;

    Ok(db)
}

/// Run `f` while showing an indicatif spinner on stderr (only when attached
/// to a real terminal — e.g. userId SHA-512 역산 can take a minute or two).
fn with_spinner<T>(message: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let pb = std::io::stderr().is_terminal().then(|| {
        let pb = indicatif::ProgressBar::new_spinner();
        let style = indicatif::ProgressStyle::with_template("{spinner:.cyan} {msg}")
            .unwrap_or_else(|_| indicatif::ProgressStyle::default_spinner());
        pb.set_style(style);
        pb.enable_steady_tick(Duration::from_millis(100));
        pb.set_message(message.to_string());
        pb
    });

    let result = f();

    if let Some(pb) = pb {
        pb.finish_and_clear();
    }

    result
}

/// A chat wrapped for display in an `inquire::Select` prompt.
struct ChatChoice(Chat);

impl std::fmt::Display for ChatChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.0.unread_count > 0 {
            write!(f, "{} ({})", self.0.display_name, self.0.unread_count)
        } else {
            write!(f, "{}", self.0.display_name)
        }
    }
}

/// Interactively pick a chat room from the DB's chat list.
fn pick_chat_interactive(db: &Database) -> Result<(i64, String), String> {
    let chats = db
        .list_chats(200)
        .map_err(|e| format!("채팅방 목록 조회 실패: {}", e))?;

    if chats.is_empty() {
        return Err("표시할 채팅방이 없습니다.".to_string());
    }

    let choices: Vec<ChatChoice> = chats.into_iter().map(ChatChoice).collect();
    let selected = inquire::Select::new("채팅방을 선택하세요:", choices)
        .prompt()
        .map_err(|e| format!("채팅방 선택이 취소되었습니다: {}", e))?;

    Ok((selected.0.id, selected.0.display_name))
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
                kakaocli_platform::AppStatus::Ready => {
                    display::style_success("✅ 준비됨 (앱 실행 중 + DB 접근 가능)")
                }
                kakaocli_platform::AppStatus::LoggedOut => {
                    display::style_warn("⚠️ 앱은 실행 중이지만 로그인되지 않았습니다")
                }
                kakaocli_platform::AppStatus::NotRunning => {
                    display::style_warn("⚠️ 카카오톡이 실행 중이 아닙니다")
                }
                kakaocli_platform::AppStatus::DbAccessible => {
                    display::style_success("✅ DB 접근 가능 (앱 미실행)")
                }
            };
            println!("{}", status_str);

            if cli.verbose {
                let db_path = kakaocli_core::db_path::mac_container_path();
                println!("  컨테이너: {}", db_path.display());
                println!("  전체 디스크 접근: {}", kakaocli_core::db_path::check_full_disk_access());
            }

            Ok(())
        }
        Err(e) => Err(format!("상태 확인 실패: {}", e)),
    }
}

fn cmd_auth(cli: &Cli) -> Result<(), String> {
    // Verify DB can be opened (may trigger a slow userId SHA-512 역산 on macOS)
    let db = with_spinner(
        "DB 키 확인 중... (userId 역산이 필요하면 최대 1~2분 소요될 수 있습니다)",
        || open_db(cli),
    )?;
    let tables = db.verify_tables().map_err(|e| format!("테이블 확인 실패: {}", e))?;

    println!("{}", display::style_success("✅ 데이터베이스 열기 성공!"));
    println!("   테이블 {}개 발견", tables.len());

    if cli.verbose {
        for t in &tables {
            println!("   - {}", t);
        }
    }

    Ok(())
}

fn cmd_chats(cli: &Cli, limit: u32) -> Result<(), String> {
    #[cfg(windows)]
    {
        return cmd_chats_windows(cli, limit);
    }
    #[cfg(not(windows))]
    {
        let db = open_db(cli)?;
        let chats = db.list_chats(limit).map_err(|e| format!("채팅방 목록 조회 실패: {}", e))?;

        if cli.json {
            println!("{}", serde_json::to_string_pretty(&chats).map_err(|e| e.to_string())?);
        } else {
            display::print_chats(&chats);
        }

        Ok(())
    }
}

/// Windows read path: DEK 스캔 → chatListInfo.edb 복사·복호화 → chatRoomList.
#[cfg(windows)]
fn windows_my_uid(cli: &Cli) -> i64 {
    cli.user_id.map(|u| u as i64).unwrap_or(0)
}

#[cfg(windows)]
fn cmd_chats_windows(cli: &Cli, limit: u32) -> Result<(), String> {
    let chat_data = kakaocli_core::db_path::windows_chat_data_path();
    let uid = windows_my_uid(cli);
    let db = kakaocli_platform::dek::open_edb(&chat_data, "chatListInfo.edb", uid)
        .map_err(|e| format!("채팅방 목록을 열 수 없습니다: {}", e))?;
    let chats = db
        .list_chats_windows(limit)
        .map_err(|e| format!("채팅방 목록 조회 실패: {}", e))?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&chats).map_err(|e| e.to_string())?);
    } else {
        display::print_chats(&chats);
    }
    Ok(())
}

#[cfg(windows)]
fn cmd_messages_windows(
    cli: &Cli,
    chat: Option<&str>,
    since: Option<&str>,
    limit: u32,
) -> Result<(), String> {
    use kakaocli_platform::dek;

    let user_dir = kakaocli_core::db_path::windows_user_dir()
        .ok_or_else(|| "KakaoTalk 사용자 디렉터리를 찾지 못했습니다 (로그인 상태 확인).".to_string())?;
    let chat_data = user_dir.join("chat_data");
    let uid = windows_my_uid(cli);

    // Scan every resident DEK once (covers chatListInfo, TalkUserDB, chatLogs).
    let deks = dek::scan_deks(&user_dir).map_err(|e| format!("DEK 스캔 실패: {}", e))?;

    // Resolve chat name → chatId via chatListInfo.
    let room_db = dek::open_edb_from(&deks, &chat_data.join("chatListInfo.edb"), uid)
        .map_err(|e| format!("채팅방 목록을 열 수 없습니다: {}", e))?;
    let chats = room_db
        .list_chats_windows(999)
        .map_err(|e| format!("채팅방 목록 조회 실패: {}", e))?;

    let (chat_id, chat_name) = match chat {
        Some(name) => {
            let m = chats
                .iter()
                .find(|c| c.display_name.contains(name))
                .ok_or_else(|| {
                    format!("'{}' 채팅방을 찾지 못했습니다. 이름 없이 실행하면 목록에서 고를 수 있어요.", name)
                })?;
            (m.id, m.display_name.clone())
        }
        None => {
            let choices: Vec<ChatChoice> = chats.into_iter().map(ChatChoice).collect();
            let sel = inquire::Select::new("채팅방을 선택하세요:", choices)
                .prompt()
                .map_err(|e| format!("채팅방 선택이 취소되었습니다: {}", e))?;
            (sel.0.id, sel.0.display_name)
        }
    };

    // My own userId (for is_from_me): --user-id override, else derive from
    // chatMembers (I'm in every room → most-frequent member).
    let own_uid = cli
        .user_id
        .map(|u| u as i64)
        .or_else(|| room_db.windows_own_user_id().ok().flatten())
        .unwrap_or(0);

    // Contact names (best-effort): TalkUserDB.talkUser → authorId map.
    let mut names = dek::open_edb_from(&deks, &user_dir.join("TalkUserDB.edb"), uid)
        .ok()
        .and_then(|db| db.talk_user_names().ok())
        .unwrap_or_default();
    if own_uid != 0 {
        names.entry(own_uid).or_insert_with(|| "나".to_string());
    }

    // Open that chat's per-file message DB (its DEK must be resident).
    let log_file = format!("chatLogs_{}.edb", chat_id);
    let db = dek::open_edb_from(&deks, &chat_data.join(&log_file), own_uid)
        .map_err(|e| format!("메시지 DB를 열 수 없습니다: {}", e))?;
    let mut msgs = db
        .messages_windows(chat_id, parse_since(since), limit)
        .map_err(|e| format!("메시지 조회 실패: {}", e))?;

    // Fill sender names from the contact map.
    for m in &mut msgs {
        if m.sender_name.is_none() {
            m.sender_name = names.get(&m.sender_id).cloned();
        }
    }

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&msgs).map_err(|e| e.to_string())?);
    } else {
        display::print_messages(&msgs, &chat_name);
    }
    Ok(())
}

fn cmd_messages(cli: &Cli, chat: Option<&str>, since: Option<&str>, limit: u32) -> Result<(), String> {
    #[cfg(windows)]
    {
        return cmd_messages_windows(cli, chat, since, limit);
    }
    #[cfg(not(windows))]
    {
        cmd_messages_unix(cli, chat, since, limit)
    }
}

#[cfg(not(windows))]
fn cmd_messages_unix(cli: &Cli, chat: Option<&str>, since: Option<&str>, limit: u32) -> Result<(), String> {
    let db = open_db(cli)?;

    // Resolve chat name to chat ID (interactive pick when omitted)
    let (chat_id, chat_name) = match chat {
        Some(name) => db
            .resolve_chat_id(name)
            .map_err(|e| format!("채팅방 조회 실패: {}", e))?
            .ok_or_else(|| format!("'{}' 채팅방을 찾지 못했습니다. 이름 없이 실행하면 목록에서 고를 수 있어요 (kakaocli msg)", name))?,
        None => pick_chat_interactive(&db)?,
    };

    // Parse since
    let since_ts = parse_since(since);

    let messages = db
        .get_messages(chat_id, since_ts, limit)
        .map_err(|e| format!("메시지 조회 실패: {}", e))?;

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
        let results = db.search_all(keyword, 50).map_err(|e| format!("검색 실패: {}", e))?;
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&results).map_err(|e| e.to_string())?);
        } else {
            display::print_search_all(&results, keyword);
        }
        return Ok(());
    }

    if rooms {
        let results = db.search_rooms(keyword, 50).map_err(|e| format!("채팅방 검색 실패: {}", e))?;
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&results).map_err(|e| e.to_string())?);
        } else {
            display::print_chats(&results);
        }
        return Ok(());
    }

    if friends {
        let results = db.search_friends(keyword, 50).map_err(|e| format!("친구 검색 실패: {}", e))?;
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
        let results = db.search_messages(keyword, 200).map_err(|e| format!("검색 실패: {}", e))?;
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
        let re = regex::Regex::new(keyword).map_err(|e| format!("잘못된 정규식: {}", e))?;
        let results = db.search_messages("", 200).map_err(|e| format!("검색 실패: {}", e))?;
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
    let results = db.search_messages(keyword, 50).map_err(|e| format!("검색 실패: {}", e))?;
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&results).map_err(|e| e.to_string())?);
    } else {
        display::print_search_messages(&results, keyword);
    }

    Ok(())
}

fn cmd_query(cli: &Cli, sql: &str) -> Result<(), String> {
    let db = open_db(cli)?;
    let result = db.raw_query(sql).map_err(|e| format!("쿼리 실패: {}", e))?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?);
    } else {
        display::print_query_result(&result);
    }

    Ok(())
}

// ── Sync ────────────────────────────────────────────────────

fn cmd_sync(
    cli: &Cli,
    follow: bool,
    interval: u32,
    chat: Option<&str>,
    webhook: Option<&str>,
    since: Option<&str>,
) -> Result<(), String> {
    use kakaocli_core::sync_state;

    let db = open_db(cli)?;

    // Resolve optional chat filter to a chat_id.
    let chat_id: Option<i64> = match chat {
        Some(name) => Some(
            db.resolve_chat_id(name)
                .map_err(|e| format!("채팅방 조회 실패: {}", e))?
                .ok_or_else(|| {
                    format!("'{}' 채팅방을 찾지 못했습니다. 이름 없이 실행하면 전체 방을 따라갑니다.", name)
                })?
                .0,
        ),
        None => None,
    };
    let single_room = chat_id.is_some();

    // Room-name map for display (all-rooms mode only).
    let chat_names: HashMap<i64, String> = if single_room {
        HashMap::new()
    } else {
        db.list_chats(500)
            .map_err(|e| format!("채팅방 목록 조회 실패: {}", e))?
            .into_iter()
            .map(|c| (c.id, c.display_name))
            .collect()
    };

    // Establish the baseline (last processed logId).
    let mut last = match sync_state::load(chat_id) {
        Some(v) => v,
        None => match parse_since(since) {
            Some(ts) => match db
                .log_id_before_since(chat_id, ts)
                .map_err(|e| format!("기준선 조회 실패: {}", e))?
            {
                Some(base) => base,
                None => db.max_log_id(chat_id).map_err(|e| format!("기준선 조회 실패: {}", e))?,
            },
            None => db.max_log_id(chat_id).map_err(|e| format!("기준선 조회 실패: {}", e))?,
        },
    };

    let webhook_client = webhook.map(|_| reqwest::blocking::Client::new());
    const LIMIT: u32 = 500;

    loop {
        // Drain any backlog fully within this tick (bounded batches).
        loop {
            let batch = db
                .messages_after(last, chat_id, LIMIT)
                .map_err(|e| format!("메시지 조회 실패: {}", e))?;
            if batch.is_empty() {
                break;
            }

            for m in &batch {
                emit_sync_message(cli, m, &chat_names, single_room);
            }
            if let (Some(client), Some(url)) = (webhook_client.as_ref(), webhook) {
                let payload: Vec<serde_json::Value> =
                    batch.iter().map(|m| sync_message_json(m, &chat_names)).collect();
                post_webhook(client, url, &payload);
            }

            last = batch.last().map(|m| m.id).unwrap_or(last);
            let _ = sync_state::save(chat_id, last);

            if batch.len() < LIMIT as usize {
                break;
            }
        }

        if !follow {
            break;
        }
        std::thread::sleep(Duration::from_secs(interval.max(1) as u64));
    }

    Ok(())
}

/// Room display name for a chat id; falls back to the numeric id.
fn room_name_for(chat_names: &HashMap<i64, String>, chat_id: i64) -> String {
    chat_names
        .get(&chat_id)
        .cloned()
        .unwrap_or_else(|| chat_id.to_string())
}

fn message_type_str(t: &MessageType) -> &'static str {
    match t {
        MessageType::Text => "text",
        MessageType::Photo => "photo",
        MessageType::Video => "video",
        MessageType::Unknown(_) => "unknown",
    }
}

/// A message's Unix timestamp as RFC 3339 in the local timezone.
fn format_rfc3339_local(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.with_timezone(&chrono::Local).to_rfc3339())
        .unwrap_or_else(|| ts.to_string())
}

/// The NDJSON object for one message (also the webhook array element).
fn sync_message_json(m: &Message, chat_names: &HashMap<i64, String>) -> serde_json::Value {
    serde_json::json!({
        "type": "message",
        "logId": m.id,
        "chatId": m.chat_id,
        "chat": room_name_for(chat_names, m.chat_id),
        "sender": m.sender_name,
        "fromMe": m.is_from_me,
        "text": m.text,
        "time": format_rfc3339_local(m.created_at),
        "msgType": message_type_str(&m.message_type),
    })
}

/// Emit one message to stdout: NDJSON with `--json`, else a human line. Flushes
/// so the stream is visible immediately when piped.
fn emit_sync_message(cli: &Cli, m: &Message, chat_names: &HashMap<i64, String>, single_room: bool) {
    use std::io::Write;
    if cli.json {
        println!("{}", sync_message_json(m, chat_names));
    } else {
        let room = room_name_for(chat_names, m.chat_id);
        display::print_sync_message(m, &room, single_room);
    }
    let _ = std::io::stdout().flush();
}

/// POST a batch as a JSON array. Timeout 5s, one retry; on final failure warn to
/// stderr and continue (the stdout stream must not stall on a bad webhook).
fn post_webhook(client: &reqwest::blocking::Client, url: &str, payload: &[serde_json::Value]) {
    for attempt in 0..2 {
        let result = client
            .post(url)
            .timeout(Duration::from_secs(5))
            .json(payload)
            .send();
        match result {
            Ok(resp) if resp.status().is_success() => return,
            Ok(resp) if attempt == 1 => {
                eprintln!(
                    "{}",
                    display::style_warn(&format!("웹훅 응답 오류 {} → {}", resp.status(), url))
                );
            }
            Err(e) if attempt == 1 => {
                eprintln!(
                    "{}",
                    display::style_warn(&format!("웹훅 전송 실패: {} → {}", e, url))
                );
            }
            _ => {}
        }
    }
}

fn cmd_inspect(cli: &Cli, chat: Option<&str>, depth: u32) -> Result<(), String> {
    use kakaocli_platform::PlatformBackend;
    let tree = kakaocli_platform::Platform::dump_ax_tree(chat, depth)
        .map_err(|e| format!("Inspect 실패: {}", e))?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&tree).map_err(|e| e.to_string())?);
    } else {
        display::print_ax_tree(&tree, 0);
    }

    Ok(())
}

fn cmd_send(
    cli: &Cli,
    chat_name: Option<&str>,
    text: Option<&str>,
    me: bool,
    dry_run: bool,
    yes: bool,
) -> Result<(), String> {
    // Resolve target chat (interactive pick when omitted and not sending to self)
    let target_name = if me {
        "_".to_string()
    } else {
        match chat_name {
            Some(name) => name.to_string(),
            None => {
                let db = open_db(cli)?;
                pick_chat_interactive(&db)?.1
            }
        }
    };

    let text = match text {
        Some(t) => t.to_string(),
        None => inquire::Text::new("보낼 메시지:")
            .prompt()
            .map_err(|e| format!("메시지 입력이 취소되었습니다: {}", e))?,
    };

    if dry_run {
        println!(
            "{}",
            display::style_dim(&format!(
                "🔍 미리보기: '{}'에 전송 예정 → {}",
                target_name, text
            ))
        );
        return Ok(());
    }

    if !yes {
        if !std::io::stdin().is_terminal() {
            return Err("비대화형 환경에서는 --yes(-y) 플래그가 필요합니다.".to_string());
        }
        let proceed = inquire::Confirm::new(&format!(
            "'{}'에 메시지를 보낼까요?\n> {}",
            target_name, text
        ))
        .with_default(false)
        .prompt()
        .map_err(|e| format!("전송 확인이 취소되었습니다: {}", e))?;

        if !proceed {
            println!("{}", display::style_warn("전송이 취소되었습니다."));
            return Ok(());
        }
    }

    use kakaocli_platform::PlatformBackend;

    kakaocli_platform::Platform::send_message(&target_name, &text)
        .map_err(|e| format!("전송 실패: {}", e))?;

    println!(
        "{}",
        display::style_success(&format!("✅ '{}'에 메시지를 전송했습니다", target_name))
    );
    Ok(())
}

fn cmd_login(_cli: &Cli, email: Option<&str>, password: Option<&str>, status: bool, clear: bool) -> Result<(), String> {
    if clear {
        kakaocli_auth::clear_credentials().map_err(|e| format!("삭제 실패: {}", e))?;
        println!("저장된 인증 정보를 삭제했습니다.");
        return Ok(());
    }

    if status {
        if kakaocli_auth::has_credentials() {
            let (email, _) = kakaocli_auth::get_credentials().map_err(|e| format!("읽기 실패: {}", e))?;
            println!(
                "{}",
                display::style_success(&format!("✅ 저장된 계정: {}", email))
            );
        } else {
            println!(
                "{}",
                display::style_warn("⚠️ 저장된 인증 정보가 없습니다. `kakaocli login --email ... --password ...`로 저장하세요.")
            );
        }
        return Ok(());
    }

    if let (Some(e), Some(p)) = (email, password) {
        kakaocli_auth::store_credentials(e, p).map_err(|e| format!("저장 실패: {}", e))?;
        println!(
            "{}",
            display::style_success(&format!("✅ 인증 정보를 저장했습니다: {}", e))
        );
        return Ok(());
    }

    not_implemented_yet("대화형 로그인 (--email + --password 사용)");
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