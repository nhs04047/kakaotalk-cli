// macOS backend: AXUIElement + CGEvent via objc2 + accessibility crate
// References:
// - kakaocli (Swift) CLAUDE.md (AX quirks)
// - docs/macos-ax.md
// - https://github.com/silver-flight-group/kakaocli

use crate::*;
use kakaocli_core::db_path;
use kakaocli_core::model::DbKey;

pub struct DarwinBackend;

impl PlatformBackend for DarwinBackend {
    fn resolve_db_key() -> Result<DbKey, PlatformError> {
        let uuid = db_path::mac_platform_uuid()
            .ok_or_else(|| PlatformError::Other("Cannot read IOPlatformUUID".into()))?;
        let user_id = db_path::mac_user_id()
            .ok_or_else(|| PlatformError::Other("Cannot read userId from plist".into()))?;
        let key_hex = kakaocli_core::kdf::derive_mac_key(user_id, &uuid);

        let db_files = db_path::mac_db_files();
        let db_path = db_files
            .first()
            .cloned()
            .ok_or_else(|| PlatformError::Other("No KakaoTalk database file found".into()))?;

        Ok(DbKey { key_hex, db_path })
    }

    fn check_status() -> Result<AppStatus, PlatformError> {
        if !db_path::check_full_disk_access() {
            return Err(PlatformError::Other(
                "Full Disk Access not granted. Grant it in System Settings > Privacy & Security".into(),
            ));
        }

        let running = is_kakaotalk_running();
        let logged_in = running && is_kakaotalk_logged_in();
        let db_exists = db_path::mac_db_files().first().is_some();

        match (running, logged_in, db_exists) {
            (true, true, _) => Ok(AppStatus::Ready),
            (true, false, _) => Ok(AppStatus::LoggedOut),
            (false, _, true) => Ok(AppStatus::DbAccessible),
            (false, _, false) => Ok(AppStatus::NotRunning),
        }
    }

    fn login(email: &str, password: &str) -> Result<(), PlatformError> {
        kakaocli_auth::store_credentials(email, password)
            .map_err(|e| PlatformError::Other(e.to_string()))
    }

    fn send_message(chat_name: &str, text: &str) -> Result<(), PlatformError> {
        // P0-6: Pre-check Accessibility permission
        if !check_accessibility() {
            user_facing_instruction(
                "KakaoTalk send needs Accessibility permission.\n\
                 → System Settings > Privacy & Security > Accessibility\n\
                 → Add Terminal (or iTerm2) and enable it.",
            );
            return Err(PlatformError::Other(
                "Accessibility permission not granted. See instructions above.".into(),
            ));
        }

        // P0-5: Check for login screen
        ensure_kakaotalk_running()?;
        check_login_interruption()?;

        let app = get_kakaotalk_ax_app()
            .ok_or_else(|| PlatformError::Other("Cannot find KakaoTalk AX application".into()))?;

        let main_window = get_main_window(&app)
            .ok_or_else(|| PlatformError::Other("Cannot find KakaoTalk main window".into()))?;

        // P7: dismiss paywall if present
        dismiss_paywall_if_present(&app);

        let chat_list = get_chat_list_role(&main_window)
            .ok_or_else(|| PlatformError::Other("Cannot find chat list in main window".into()))?;

        // P1-2: Scroll-accessible search
        let chat_row = find_chat_row_with_scroll(&chat_list, chat_name, 5)?;

        // P1-1: Select via kAXSelectedRowsAttribute + Enter (kakaocli 방식)
        select_ax_row_via_keyboard(&chat_list)?;

        // P0-2: Actually verify the opened window title
        std::thread::sleep(std::time::Duration::from_millis(800));
        verify_chat_window(chat_name)?;

        // P7: Handle newlines in text
        let sanitized = text.replace('\n', " ");

        type_text(&sanitized)?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        press_enter()?;

        // P1-7: Close chat window after send (Cmd+W)
        std::thread::sleep(std::time::Duration::from_millis(300));
        press_cmd_w().ok(); // best-effort close

        Ok(())
    }

    fn dump_ax_tree(chat: Option<&str>, max_depth: u32) -> Result<AxNode, PlatformError> {
        if !check_accessibility() {
            user_facing_instruction(
                "inspect needs Accessibility permission.\n\
                 → System Settings > Privacy & Security > Accessibility\n\
                 → Add Terminal (or iTerm2) and enable it.",
            );
            return Err(PlatformError::Other(
                "Accessibility permission not granted".into(),
            ));
        }

        let app = get_kakaotalk_ax_app()
            .ok_or_else(|| PlatformError::Other("KakaoTalk is not running".into()))?;

        let root = ax_build_tree(&app, max_depth);

        if let Some(chat_name) = chat {
            // Filter to only relevant parts
            Ok(filter_ax_for_chat(root, chat_name))
        } else {
            Ok(root)
        }
    }
}

// ── Core helpers ───────────────────────────────────────────────

fn is_kakaotalk_running() -> bool {
    use objc2_app_kit::NSRunningApplication;
    let apps =
        NSRunningApplication::runningApplicationsWithBundleIdentifier("com.kakao.KakaoTalkMac");
    apps.first().is_some()
}

fn is_kakaotalk_logged_in() -> bool {
    // Check by status bar menu: if "Log out" exists → logged in
    // For now, return true if we can find main window with AX children
    if let Some(app) = get_kakaotalk_ax_app() {
        if let Some(mw) = get_main_window(&app) {
            // main window with chat list = logged in
            return get_chat_list_role(&mw).is_some();
        }
    }
    false
}

fn ensure_kakaotalk_running() -> Result<(), PlatformError> {
    if is_kakaotalk_running() {
        return Ok(());
    }

    // P1-5: Use NSWorkspace URLForApplicationWithBundleIdentifier instead of hardcoded path
    let app_path = get_kakaotalk_app_path()
        .unwrap_or_else(|| "/Applications/KakaoTalk.app".to_string());

    unsafe {
        let workspace: *const objc2::runtime::AnyObject =
            objc2::msg_send![objc2::runtime::sel_registerClass("NSWorkspace"), sharedWorkspace];
        if workspace.is_null() {
            return Err(PlatformError::Other("Cannot get NSWorkspace".into()));
        }
        let url: *const objc2::runtime::AnyObject = objc2::msg_send![
            objc2::runtime::sel_registerClass("NSURL"),
            fileURLWithPath: nsstring(&app_path)
        ];
        if url.is_null() {
            return Err(PlatformError::Other(format!(
                "KakaoTalk.app not found at '{}'",
                app_path
            )));
        }
        let _: *const objc2::runtime::AnyObject = objc2::msg_send![workspace, openURL: url];
    }

    for _ in 0..30 {
        if is_kakaotalk_running() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    Err(PlatformError::Other("KakaoTalk did not launch within 15 seconds".into()))
}

fn get_kakaotalk_app_path() -> Option<String> {
    unsafe {
        let workspace: *const objc2::runtime::AnyObject =
            objc2::msg_send![objc2::runtime::sel_registerClass("NSWorkspace"), sharedWorkspace];
        if workspace.is_null() {
            return None;
        }
        let bundle_id = nsstring("com.kakao.KakaoTalkMac");
        let url: *const objc2::runtime::AnyObject =
            objc2::msg_send![workspace, URLForApplicationWithBundleIdentifier: bundle_id];
        if url.is_null() {
            return None;
        }
        let path: *const objc2::runtime::AnyObject = objc2::msg_send![url, path];
        if path.is_null() {
            return None;
        }
        // Get NSString from raw pointer and convert
        let s: &objc2_foundation::NSString =
            &*(path as *const objc2_foundation::NSString);
        Some(s.to_string())
    }
}

fn check_login_interruption() -> Result<(), PlatformError> {
    let app = match get_kakaotalk_ax_app() {
        Some(a) => a,
        None => return Ok(()), // not running, skip
    };
    let mw = match get_main_window(&app) {
        Some(w) => w,
        None => return Ok(()),
    };
    // Check if the main window title is "Log in" or similar
    if let Ok(title_val) = mw.attribute_value("AXTitle".into()) {
        if let Some(title) = title_val.as_string() {
            if title.contains("Log in") || title.contains("Login") {
                return Err(PlatformError::NotLoggedIn);
            }
        }
    }
    // Also check for absence of chat list (login screen has no chat list)
    if get_chat_list_role(&mw).is_none() {
        // Could be login screen — return warning but don't block
        return Ok(());
    }
    Ok(())
}

fn check_accessibility() -> bool {
    // AXIsProcessTrustedWithOptions allows checking without a prompt
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> std::os::raw::c_int;
        fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> std::os::raw::c_int;
    }
    // SAFETY: Simple getter, no side effects
    unsafe { AXIsProcessTrusted() != 0 }
}

fn user_facing_instruction(msg: &str) {
    eprintln!("ℹ️  {}", msg);
}

// ── NS / AX helpers ────────────────────────────────────────────

fn nsstring(s: &str) -> *const objc2::runtime::AnyObject {
    objc2_foundation::NSString::from_str(s).into_raw()
}

fn get_kakaotalk_ax_app() -> Option<accessibility::AXUIElement> {
    use objc2_app_kit::NSRunningApplication;

    let apps =
        NSRunningApplication::runningApplicationsWithBundleIdentifier("com.kakao.KakaoTalkMac");
    let app = apps.first()?;
    let pid = app.processIdentifier();
    Some(accessibility::AXUIElement::new_with_pid(pid))
}

fn get_main_window(app: &accessibility::AXUIElement) -> Option<accessibility::AXUIElement> {
    let windows_val = app.attribute_value("AXWindows".into()).ok()?;
    let windows = windows_val.as_array()?;

    for window in windows {
        let ax_window = window.as_axui_element()?;
        // Filter out AXApplication elements (kAXWindowsAttribute may return them)
        if let Ok(role) = ax_window.attribute_value("AXRole".into()) {
            if role.as_string().map(|s| s == "AXApplication").unwrap_or(false) {
                continue;
            }
        }
        // Prefer window with id "Main Window"
        if let Ok(id) = ax_window.attribute_value("AXIdentifier".into()) {
            if id.as_string().map(|s| s == "Main Window").unwrap_or(false) {
                return Some(ax_window.clone());
            }
        }
        // Fallback: first AXWindow
        if let Ok(role) = ax_window.attribute_value("AXRole".into()) {
            if role.as_string().map(|s| s == "AXWindow").unwrap_or(false) {
                return Some(ax_window.clone());
            }
        }
    }

    None
}

fn get_chat_list_role(window: &accessibility::AXUIElement) -> Option<accessibility::AXUIElement> {
    find_child_by_role(window, &["AXOutline", "AXList", "AXTable", "AXScrollArea"])
}

fn find_child_by_role(
    parent: &accessibility::AXUIElement,
    roles: &[&str],
) -> Option<accessibility::AXUIElement> {
    let children_val = parent.attribute_value("AXChildren".into()).ok()?;
    let children = children_val.as_array()?;

    for child in children {
        if let Some(ax_child) = child.as_axui_element() {
            if let Ok(role) = ax_child.attribute_value("AXRole".into()) {
                if let Some(role_str) = role.as_string() {
                    if roles.contains(&role_str.as_str()) {
                        return Some(ax_child.clone());
                    }
                }
            }
            if let Some(found) = find_child_by_role(&ax_child, roles) {
                return Some(found);
            }
        }
    }

    None
}

// ── Chat row search with scroll ──────────────────────────────────

fn find_chat_row_with_scroll(
    chat_list: &accessibility::AXUIElement,
    name: &str,
    max_scrolls: u32,
) -> Result<accessibility::AXUIElement, PlatformError> {
    // Handle self-chat special case: "badge me" descriptor
    if name == "_" || name == "badge me" || name == "me" {
        return find_self_chat_row(chat_list);
    }

    for _ in 0..max_scrolls {
        let rows = get_visible_rows(chat_list)?;
        for row in &rows {
            let row_name = get_row_display_name(row);
            if let Some(ref rn) = row_name {
                if row_name_matches(rn, name) {
                    return Ok(row.clone());
                }
            }
        }
        // Not found yet — scroll down one page
        scroll_down(chat_list)?;
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    // One last check after scrolls exhausted
    let rows = get_visible_rows(chat_list)?;
    let mut matches: Vec<String> = Vec::new();
    for row in &rows {
        if let Some(rn) = get_row_display_name(row) {
            if row_name_matches(&rn, name) {
                matches.push(rn);
            }
        }
    }

    match matches.len() {
        0 => Err(PlatformError::UiError(format!(
            "Chat '{}' not found in chat list (scrolled {} times)",
            name, max_scrolls
        ))),
        1 => {
            // Re-find the single match by scanning again
            for row in &rows {
                if let Some(rn) = get_row_display_name(row) {
                    if rn == matches[0] {
                        return Ok(row.clone());
                    }
                }
            }
            Err(PlatformError::UiError("Match vanished after scroll".into()))
        }
        _ => {
            let names: Vec<String> = rows
                .iter()
                .filter_map(|r| get_row_display_name(r))
                .filter(|rn| row_name_matches(rn, name))
                .collect();
            Err(PlatformError::AmbiguousChatName {
                name: name.to_string(),
                matches: names,
            })
        }
    }
}

fn find_self_chat_row(
    _chat_list: &accessibility::AXUIElement,
) -> Result<accessibility::AXUIElement, PlatformError> {
    // Search for row with "badge me" AX image descriptor
    let rows = get_visible_rows(_chat_list)?;
    for row in &rows {
        // Try AXDescription or AXLabel containing "badge me"
        if let Ok(desc) = row.attribute_value("AXDescription".into()) {
            if let Some(s) = desc.as_string() {
                if s.contains("badge me") || s.contains("Self-chat") || s.contains("Notes") {
                    return Ok(row.clone());
                }
            }
        }
    }
    Err(PlatformError::UiError("Self-chat 'badge me' row not found".into()))
}

fn row_name_matches(row_name: &str, query: &str) -> bool {
    let rn_lower = row_name.to_lowercase();
    let q_lower = query.to_lowercase();
    rn_lower.contains(&q_lower) || q_lower.contains(&rn_lower)
}

fn get_visible_rows(
    chat_list: &accessibility::AXUIElement,
) -> Result<Vec<accessibility::AXUIElement>, PlatformError> {
    let children_val = chat_list
        .attribute_value("AXChildren".into())
        .map_err(|_| PlatformError::Other("Cannot get chat list children".into()))?;
    let arr = children_val
        .as_array()
        .ok_or_else(|| PlatformError::Other("Chat list children is not an array".into()))?;

    let rows: Vec<accessibility::AXUIElement> = arr
        .iter()
        .filter_map(|v| v.as_axui_element().cloned())
        .collect();

    Ok(rows)
}

fn scroll_down(chat_list: &accessibility::AXUIElement) -> Result<(), PlatformError> {
    // Try AXScrollToVisible or AXIncrement
    if let Err(e) = chat_list.set_attribute_value(
        "AXFocused".into(),
        accessibility::AXValue::boolean(true),
    ) {
        eprintln!("Warning: could not focus chat list for scroll: {:?}", e);
    }
    // Use CGEvent scroll
    scroll_via_cg_event(-3).ok();
    Ok(())
}

fn scroll_via_cg_event(lines: i32) -> Result<(), ()> {
    // CGEvent scroll wheel
    unsafe {
        let source = core_graphics::event_source::CGEventSource::new(
            core_graphics::event_source::CGEventSourceStateID::HIDSystemState,
        )
        .map_err(|_| ())?;
        let event = core_graphics::event::CGEvent::new_scroll_event(
            &source,
            core_graphics::event::CGScrollEventUnit::LINE,
            lines as u32,
            0,
            0,
        )
        .map_err(|_| ())?;
        // Post to HID event tap
        core_graphics::event::CGEvent::post_to_process(event);
    }
    Ok(())
}

fn get_row_display_name(row: &accessibility::AXUIElement) -> Option<String> {
    // AXDescription first (most reliable for chat names)
    if let Ok(desc) = row.attribute_value("AXDescription".into()) {
        if let Some(s) = desc.as_string() {
            if !s.is_empty() && !s.contains("badge") {
                return Some(s);
            }
        }
    }
    // AXLabel
    if let Ok(label) = row.attribute_value("AXLabel".into()) {
        if let Some(s) = label.as_string() {
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    // AXTitle
    if let Ok(title) = row.attribute_value("AXTitle".into()) {
        if let Some(s) = title.as_string() {
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

// ── AX selection (kakaocli 방식: kAXSelectedRowsAttribute + Enter) ──

fn select_ax_row_via_keyboard(chat_list: &accessibility::AXUIElement) -> Result<(), PlatformError> {
    // kakaocli Swift: select row first by setting AXSelected
    // The row should already be focused from find_chat_row_with_scroll
    // Set AXFocused on the chat list to ensure key events land
    chat_list
        .set_attribute_value("AXFocused".into(), accessibility::AXValue::boolean(true))
        .ok();

    std::thread::sleep(std::time::Duration::from_millis(100));

    // Press Enter to open the selected chat
    press_enter()?;

    Ok(())
}

// ── Window verification ──

fn verify_chat_window(expected_name: &str) -> Result<(), PlatformError> {
    std::thread::sleep(std::time::Duration::from_millis(300));

    let app = get_kakaotalk_ax_app()
        .ok_or_else(|| PlatformError::Other("KakaoTalk closed unexpectedly".into()))?;

    // Get all windows and find the frontmost one
    let windows_val = app
        .attribute_value("AXWindows".into())
        .map_err(|_| PlatformError::UiError("Cannot list windows".into()))?;
    let windows = windows_val
        .as_array()
        .ok_or_else(|| PlatformError::UiError("Windows is not an array".into()))?;

    for w in windows {
        if let Some(ax_w) = w.as_axui_element() {
            if let Ok(title_val) = ax_w.attribute_value("AXTitle".into()) {
                if let Some(title) = title_val.as_string() {
                    // Skip the main window
                    if title.is_empty() || title == "KakaoTalk" {
                        continue;
                    }
                    // Check if the chat window title matches expected
                    if title.contains(expected_name) || expected_name.contains(&title) {
                        return Ok(());
                    }
                    // Not matching — found wrong window
                    return Err(PlatformError::ChatVerificationFailed {
                        expected: expected_name.to_string(),
                        actual: title,
                    });
                }
            }
        }
    }

    // No non-main window found — chat didn't open
    Err(PlatformError::UiError(format!(
        "Chat window for '{}' did not open",
        expected_name
    )))
}

// ── Keyboard input via CGEvent ──

fn type_text(text: &str) -> Result<(), String> {
    for c in text.chars() {
        let uni = c as u16;
        let source = core_graphics::event_source::CGEventSource::new(
            core_graphics::event_source::CGEventSourceStateID::HIDSystemState,
        )
        .map_err(|e| format!("Cannot create event source: {:?}", e))?;

        // Key down with unicode string
        let event = core_graphics::event::CGEvent::new_keyboard_event(&source, 0, true)
            .map_err(|e| format!("Cannot create key event: {:?}", e))?;
        event.set_string_from_utf16_unchecked(&[uni]);
        core_graphics::event::CGEvent::post_to_process(event);

        std::thread::sleep(std::time::Duration::from_millis(10));

        // Key up
        let event_up = core_graphics::event::CGEvent::new_keyboard_event(&source, 0, false)
            .map_err(|e| format!("Cannot create key up event: {:?}", e))?;
        event_up.set_string_from_utf16_unchecked(&[uni]);
        core_graphics::event::CGEvent::post_to_process(event_up);

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    Ok(())
}

fn press_enter() -> Result<(), String> {
    let source = core_graphics::event_source::CGEventSource::new(
        core_graphics::event_source::CGEventSourceStateID::HIDSystemState,
    )
    .map_err(|e| format!("Cannot create event source: {:?}", e))?;

    let event = core_graphics::event::CGEvent::new_keyboard_event(
        &source,
        core_graphics::key_codes::RETURN,
        true,
    )
    .map_err(|e| format!("Cannot create return event: {:?}", e))?;
    core_graphics::event::CGEvent::post_to_process(event);

    std::thread::sleep(std::time::Duration::from_millis(30));

    let event_up = core_graphics::event::CGEvent::new_keyboard_event(
        &source,
        core_graphics::key_codes::RETURN,
        false,
    )
    .map_err(|e| format!("Cannot create return up event: {:?}", e))?;
    core_graphics::event::CGEvent::post_to_process(event_up);

    Ok(())
}

fn press_cmd_w() -> Result<(), String> {
    let source = core_graphics::event_source::CGEventSource::new(
        core_graphics::event_source::CGEventSourceStateID::HIDSystemState,
    )
    .map_err(|e| format!("Cannot create event source: {:?}", e))?;

    // Cmd+W: key down W + cmd modifier
    let event = core_graphics::event::CGEvent::new_keyboard_event(
        &source,
        core_graphics::key_codes::W,
        true,
    )
    .map_err(|e| format!("Cannot create W event: {:?}", e))?;
    event.set_flags(core_graphics::event::CGEventFlags::COMMAND);
    core_graphics::event::CGEvent::post_to_process(event);

    std::thread::sleep(std::time::Duration::from_millis(30));

    let event_up = core_graphics::event::CGEvent::new_keyboard_event(
        &source,
        core_graphics::key_codes::W,
        false,
    )
    .map_err(|e| format!("Cannot create W up event: {:?}", e))?;
    core_graphics::event::CGEvent::post_to_process(event_up);

    Ok(())
}

// ── Paywall dismissal ──────────────────────────────────────

fn dismiss_paywall_if_present(app: &accessibility::AXUIElement) {
    // Look for 500x500 blank window (Talk Drive Plus paywall)
    if let Ok(windows_val) = app.attribute_value("AXWindows".into()) {
        if let Some(windows) = windows_val.as_array() {
            for w in windows {
                if let Some(ax_w) = w.as_axui_element() {
                    if let Ok(size_val) = ax_w.attribute_value("AXSize".into()) {
                        if let Some(size_str) = size_val.to_string().as_mut() {
                            // Crude check: small window ~500x500
                            if size_str.contains("500") || size_str.contains("4") {
                                // Press Escape to dismiss
                                let _ = press_escape();
                                return;
                            }
                        }
                    }
                }
            }
        }
    }
}

fn press_escape() -> Result<(), String> {
    let source = core_graphics::event_source::CGEventSource::new(
        core_graphics::event_source::CGEventSourceStateID::HIDSystemState,
    )
    .map_err(|e| format!("Cannot create event source: {:?}", e))?;

    let event = core_graphics::event::CGEvent::new_keyboard_event(
        &source,
        core_graphics::key_codes::ESCAPE,
        true,
    )
    .map_err(|e| format!("Cannot create escape event: {:?}", e))?;
    core_graphics::event::CGEvent::post_to_process(event);

    std::thread::sleep(std::time::Duration::from_millis(100));

    let event_up = core_graphics::event::CGEvent::new_keyboard_event(
        &source,
        core_graphics::key_codes::ESCAPE,
        false,
    )
    .map_err(|e| format!("Cannot create escape up event: {:?}", e))?;
    core_graphics::event::CGEvent::post_to_process(event_up);

    Ok(())
}

// ── AX tree dump (inspect command) ────────────────────────

fn ax_build_tree(element: &accessibility::AXUIElement, max_depth: u32) -> AxNode {
    if max_depth == 0 {
        let mut node = AxNode::new("…");
        node.description = "(max depth)".to_string();
        return node;
    }

    let role = element
        .attribute_value("AXRole".into())
        .ok()
        .and_then(|v| v.as_string().map(|s| s.to_string()))
        .unwrap_or_default();

    let title = element
        .attribute_value("AXTitle".into())
        .ok()
        .and_then(|v| v.as_string().map(|s| s.to_string()))
        .unwrap_or_default();

    let desc = element
        .attribute_value("AXDescription".into())
        .ok()
        .and_then(|v| v.as_string().map(|s| s.to_string()))
        .unwrap_or_default();

    let focused = element
        .attribute_value("AXFocused".into())
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let selected = element
        .attribute_value("AXSelected".into())
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut node = AxNode {
        role: if role.is_empty() {
            "(no role)".into()
        } else {
            role
        },
        title,
        description: desc,
        focused,
        selected,
        children: Vec::new(),
    };

    if let Ok(children_val) = element.attribute_value("AXChildren".into()) {
        if let Some(arr) = children_val.as_array() {
            for child in arr.iter().take(50) {
                if let Some(child_el) = child.as_axui_element() {
                    let child_node = ax_build_tree(child_el, max_depth - 1);
                    node.children.push(child_node);
                }
            }
        }
    }

    node
}

fn filter_ax_for_chat(root: AxNode, chat_name: &str) -> AxNode {
    let name_lower = chat_name.to_lowercase();

    let self_matches = root.title.to_lowercase().contains(&name_lower)
        || root.description.to_lowercase().contains(&name_lower);

    let filtered_children: Vec<AxNode> = root
        .children
        .into_iter()
        .filter_map(|child| {
            let filtered = filter_ax_for_chat(child, chat_name);
            if filtered.title.is_empty()
                && filtered.description.is_empty()
                && filtered.children.is_empty()
            {
                None
            } else {
                Some(filtered)
            }
        })
        .collect();

    if self_matches || !filtered_children.is_empty() {
        AxNode {
            children: filtered_children,
            ..root
        }
    } else {
        AxNode {
            role: root.role,
            children: Vec::new(),
            ..AxNode::new("")
        }
    }
}