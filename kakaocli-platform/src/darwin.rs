// macOS backend: AXUIElement + CGEvent via objc2 + accessibility crate
// References:
// - kakaocli (Swift) CLAUDE.md (AX quirks)
// - docs/macos-ax.md
// - https://github.com/silver-flight-group/kakaocli

use crate::*;
use kakaocli_core::db_path;
use kakaocli_core::model::DbKey;

use accessibility::AXAttribute;
use accessibility::AXUIElement;

use core_foundation::string::CFString;

use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, KeyCode};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

use objc2::rc::Retained;
use objc2_app_kit::NSRunningApplication;
use objc2_app_kit::NSWorkspace;
use objc2_foundation::NSString;

/// Mac virtual key code for 'W' (Q=12, W=13, E=14, R=15)
const KEY_W: u16 = 13;

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

        ensure_kakaotalk_running()?;
        check_login_interruption()?;

        let app = get_kakaotalk_ax_app()
            .ok_or_else(|| PlatformError::Other("Cannot find KakaoTalk AX application".into()))?;

        let main_window = get_main_window(&app)
            .ok_or_else(|| PlatformError::Other("Cannot find KakaoTalk main window".into()))?;

        // Paywall dismissal deferred to Phase 2 (AXSize not in accessibility 0.2)

        let chat_list = get_chat_list_role(&main_window)
            .ok_or_else(|| PlatformError::Other("Cannot find chat list in main window".into()))?;

        let _chat_row = find_chat_row_with_scroll(&chat_list, chat_name, 5)?;

        select_ax_row_via_keyboard(&chat_list)?;

        std::thread::sleep(std::time::Duration::from_millis(800));
        verify_chat_window(chat_name)?;

        let sanitized = text.replace('\n', " ");

        type_text(&sanitized)?;
        std::thread::sleep(std::time::Duration::from_millis(100));
        press_enter()?;

        std::thread::sleep(std::time::Duration::from_millis(300));
        press_cmd_w().ok();

        Ok(())
    }

    fn dump_ax_tree(chat: Option<&str>, max_depth: u32) -> Result<AxNode, PlatformError> {
        if !check_accessibility() {
            user_facing_instruction(
                "inspect needs Accessibility permission.\n\
                 → System Settings > Privacy & Security > Accessibility\n\
                 → Add Terminal (or iTerm2) and enable it.",
            );
            return Err(PlatformError::Other("Accessibility permission not granted".into()));
        }

        let app = get_kakaotalk_ax_app()
            .ok_or_else(|| PlatformError::Other("KakaoTalk is not running".into()))?;

        let root = ax_build_tree(&app, max_depth);

        if let Some(chat_name) = chat {
            Ok(filter_ax_for_chat(root, chat_name))
        } else {
            Ok(root)
        }
    }
}

// ── Core helpers ──────────────────────────────────────────────

fn nsstring(s: &str) -> Retained<NSString> {
    NSString::from_str(s)
}

fn is_kakaotalk_running() -> bool {
    let bundle_id = nsstring("com.kakao.KakaoTalkMac");
    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(&bundle_id);
    apps.count() > 0
}

fn is_kakaotalk_logged_in() -> bool {
    if let Some(app) = get_kakaotalk_ax_app() {
        if let Some(mw) = get_main_window(&app) {
            return get_chat_list_role(&mw).is_some();
        }
    }
    false
}

fn ensure_kakaotalk_running() -> Result<(), PlatformError> {
    if is_kakaotalk_running() {
        return Ok(());
    }

    let workspace = NSWorkspace::sharedWorkspace();
    if let Some(url) = workspace.URLForApplicationWithBundleIdentifier(&nsstring("com.kakao.KakaoTalkMac")) {
        workspace.openURL(&url);
    } else {
        let path = nsstring("/Applications/KakaoTalk.app");
        let url = objc2_foundation::NSURL::fileURLWithPath(&path);
        workspace.openURL(&url);
    }

    for _ in 0..30 {
        if is_kakaotalk_running() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    Err(PlatformError::Other("KakaoTalk did not launch within 15 seconds".into()))
}

fn check_login_interruption() -> Result<(), PlatformError> {
    let app = match get_kakaotalk_ax_app() {
        Some(a) => a,
        None => return Ok(()),
    };
    let mw = match get_main_window(&app) {
        Some(w) => w,
        None => return Ok(()),
    };
    if let Ok(title_val) = mw.attribute(&AXAttribute::title()) {
        if let Some(title) = title_val.as_string() {
            if title.contains("Log in") || title.contains("Login") {
                return Err(PlatformError::NotLoggedIn);
            }
        }
    }
    Ok(())
}

fn check_accessibility() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> std::os::raw::c_int;
    }
    unsafe { AXIsProcessTrusted() != 0 }
}

fn user_facing_instruction(msg: &str) {
    eprintln!("ℹ️  {}", msg);
}

// ── AX helpers ────────────────────────────────────────────────

fn get_kakaotalk_ax_app() -> Option<accessibility::AXUIElement> {
    let bundle_id = nsstring("com.kakao.KakaoTalkMac");
    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(&bundle_id);
    if apps.count() > 0 {
        let app = apps.objectAtIndex(0);
        let pid: i32 = unsafe { objc2::msg_send![&*app, processIdentifier] };
        Some(accessibility::AXUIElement::application(pid))
    } else {
        None
    }
}

fn get_main_window(app: &accessibility::AXUIElement) -> Option<accessibility::AXUIElement> {
    let windows_val = app.attribute(&AXAttribute::windows()).ok()?;
    let windows = windows_val.as_array()?;

    for window in windows {
        let ax_window = window.as_axui_element()?;
        if let Ok(role) = ax_window.attribute(&AXAttribute::role()) {
            if role.as_string().map(|s| s == "AXApplication").unwrap_or(false) {
                continue;
            }
        }
        let ident_attr = AXAttribute::new(&CFString::new("AXIdentifier"));
        if let Ok(id_val) = ax_window.attribute(&ident_attr) {
            if id_val.as_string().map(|s| s == "Main Window").unwrap_or(false) {
                return Some(ax_window.clone());
            }
        }
        if let Ok(role) = ax_window.attribute(&AXAttribute::role()) {
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
    let children_val = parent.attribute(&AXAttribute::children()).ok()?;
    let children = children_val.as_array()?;

    for child in children {
        if let Some(ax_child) = child.as_axui_element() {
            if let Ok(role) = ax_child.attribute(&AXAttribute::role()) {
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

// ── Chat row search with scroll ──────────────────────────────

fn find_chat_row_with_scroll(
    chat_list: &accessibility::AXUIElement,
    name: &str,
    max_scrolls: u32,
) -> Result<accessibility::AXUIElement, PlatformError> {
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
        scroll_down_cg()?;
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    let rows = get_visible_rows(chat_list)?;
    for row in &rows {
        if let Some(rn) = get_row_display_name(row) {
            if row_name_matches(&rn, name) {
                return Ok(row.clone());
            }
        }
    }

    Err(PlatformError::UiError(format!(
        "Chat '{}' not found in chat list (scrolled {} times)", name, max_scrolls
    )))
}

fn find_self_chat_row(
    chat_list: &accessibility::AXUIElement,
) -> Result<accessibility::AXUIElement, PlatformError> {
    let rows = get_visible_rows(chat_list)?;
    for row in &rows {
        if let Ok(desc) = row.attribute(&AXAttribute::description()) {
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
        .attribute(&AXAttribute::children())
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

fn scroll_down_cg() -> Result<(), PlatformError> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|e| PlatformError::UiError(format!("Cannot create source: {:?}", e)))?;

    let event = CGEvent::new_keyboard_event(source.clone(), KeyCode::PAGE_DOWN, true)
        .map_err(|e| PlatformError::UiError(format!("Cannot create event: {:?}", e)))?;
    event.post(CGEventTapLocation::HID);

    std::thread::sleep(std::time::Duration::from_millis(50));

    let source2 = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|e| PlatformError::UiError(format!("Cannot create source: {:?}", e)))?;
    let event_up = CGEvent::new_keyboard_event(source2, KeyCode::PAGE_DOWN, false)
        .map_err(|e| PlatformError::UiError(format!("Cannot create event: {:?}", e)))?;
    event_up.post(CGEventTapLocation::HID);

    Ok(())
}

fn get_row_display_name(row: &accessibility::AXUIElement) -> Option<String> {
    if let Ok(desc) = row.attribute(&AXAttribute::description()) {
        if let Some(s) = desc.as_string() {
            if !s.is_empty() && !s.contains("badge") {
                return Some(s);
            }
        }
    }
    let label_attr = AXAttribute::new(&CFString::new("AXLabel"));
    if let Ok(label) = row.attribute(&label_attr) {
        if let Some(s) = label.as_string() {
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    if let Ok(title) = row.attribute(&AXAttribute::title()) {
        if let Some(s) = title.as_string() {
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

// ── AX selection (kakaocli 방식) ─────────────────────────────

fn select_ax_row_via_keyboard(chat_list: &accessibility::AXUIElement) -> Result<(), PlatformError> {
    let focused_attr = AXAttribute::focused();
    let _ = chat_list.set_attribute(&focused_attr, true);

    std::thread::sleep(std::time::Duration::from_millis(100));
    press_enter()?;
    Ok(())
}

// ── Window verification ──────────────────────────────────────

fn verify_chat_window(expected_name: &str) -> Result<(), PlatformError> {
    std::thread::sleep(std::time::Duration::from_millis(300));

    let app = get_kakaotalk_ax_app()
        .ok_or_else(|| PlatformError::Other("KakaoTalk closed unexpectedly".into()))?;

    let windows_val = app
        .attribute(&AXAttribute::windows())
        .map_err(|_| PlatformError::UiError("Cannot list windows".into()))?;
    let windows = windows_val
        .as_array()
        .ok_or_else(|| PlatformError::UiError("Windows is not an array".into()))?;

    for w in windows {
        if let Some(ax_w) = w.as_axui_element() {
            if let Ok(title_val) = ax_w.attribute(&AXAttribute::title()) {
                if let Some(title) = title_val.as_string() {
                    if title.is_empty() || title == "KakaoTalk" {
                        continue;
                    }
                    if title.contains(expected_name) || expected_name.contains(&title) {
                        return Ok(());
                    }
                    return Err(PlatformError::ChatVerificationFailed {
                        expected: expected_name.to_string(),
                        actual: title,
                    });
                }
            }
        }
    }

    Err(PlatformError::UiError(format!(
        "Chat window for '{}' did not open", expected_name
    )))
}

// ── Keyboard input via CGEvent ──────────────────────────────

fn type_text(text: &str) -> Result<(), PlatformError> {
    for c in text.chars() {
        let uni = c as u16;
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|e| PlatformError::UiError(format!("Cannot create source: {:?}", e)))?;

        let event = CGEvent::new_keyboard_event(source.clone(), 0, true)
            .map_err(|e| PlatformError::UiError(format!("Cannot create event: {:?}", e)))?;
        event.set_string_from_utf16_unchecked(&[uni]);
        event.post(CGEventTapLocation::HID);

        std::thread::sleep(std::time::Duration::from_millis(10));

        let source2 = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|e| PlatformError::UiError(format!("Cannot create source: {:?}", e)))?;
        let event_up = CGEvent::new_keyboard_event(source2, 0, false)
            .map_err(|e| PlatformError::UiError(format!("Cannot create event: {:?}", e)))?;
        event_up.set_string_from_utf16_unchecked(&[uni]);
        event_up.post(CGEventTapLocation::HID);

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    Ok(())
}

fn press_enter() -> Result<(), PlatformError> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|e| PlatformError::UiError(format!("Cannot create source: {:?}", e)))?;

    let event = CGEvent::new_keyboard_event(source.clone(), KeyCode::RETURN, true)
        .map_err(|e| PlatformError::UiError(format!("Cannot create event: {:?}", e)))?;
    event.post(CGEventTapLocation::HID);

    std::thread::sleep(std::time::Duration::from_millis(30));

    let source2 = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|e| PlatformError::UiError(format!("Cannot create source: {:?}", e)))?;
    let event_up = CGEvent::new_keyboard_event(source2, KeyCode::RETURN, false)
        .map_err(|e| PlatformError::UiError(format!("Cannot create event: {:?}", e)))?;
    event_up.post(CGEventTapLocation::HID);

    Ok(())
}

fn press_cmd_w() -> Result<(), PlatformError> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|e| PlatformError::UiError(format!("Cannot create source: {:?}", e)))?;

    let event = CGEvent::new_keyboard_event(source.clone(), KEY_W, true)
        .map_err(|e| PlatformError::UiError(format!("Cannot create event: {:?}", e)))?;
    event.set_flags(CGEventFlags::CGEventFlagCommand);
    event.post(CGEventTapLocation::HID);

    std::thread::sleep(std::time::Duration::from_millis(30));

    let source2 = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|e| PlatformError::UiError(format!("Cannot create source: {:?}", e)))?;
    let event_up = CGEvent::new_keyboard_event(source2, KEY_W, false)
        .map_err(|e| PlatformError::UiError(format!("Cannot create event: {:?}", e)))?;
    event_up.set_flags(CGEventFlags::CGEventFlagCommand);
    event_up.post(CGEventTapLocation::HID);

    Ok(())
}

fn press_escape() -> Result<(), PlatformError> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|e| PlatformError::UiError(format!("Cannot create source: {:?}", e)))?;

    let event = CGEvent::new_keyboard_event(source.clone(), KeyCode::ESCAPE, true)
        .map_err(|e| PlatformError::UiError(format!("Cannot create event: {:?}", e)))?;
    event.post(CGEventTapLocation::HID);

    std::thread::sleep(std::time::Duration::from_millis(100));

    let source2 = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|e| PlatformError::UiError(format!("Cannot create source: {:?}", e)))?;
    let event_up = CGEvent::new_keyboard_event(source2, KeyCode::ESCAPE, false)
        .map_err(|e| PlatformError::UiError(format!("Cannot create event: {:?}", e)))?;
    event_up.post(CGEventTapLocation::HID);

    Ok(())
}

// ── AX tree dump (inspect command) ──────────────────────────

fn ax_build_tree(element: &accessibility::AXUIElement, max_depth: u32) -> AxNode {
    if max_depth == 0 {
        return AxNode {
            role: "…".to_string(),
            description: "(max depth)".to_string(),
            ..AxNode::new("")
        };
    }

    let role = element
        .attribute(&AXAttribute::role())
        .ok()
        .and_then(|v| v.as_string().map(|s| s.to_string()))
        .unwrap_or_default();

    let title = element
        .attribute(&AXAttribute::title())
        .ok()
        .and_then(|v| v.as_string().map(|s| s.to_string()))
        .unwrap_or_default();

    let desc = element
        .attribute(&AXAttribute::description())
        .ok()
        .and_then(|v| v.as_string().map(|s| s.to_string()))
        .unwrap_or_default();

    let focused = element
        .attribute(&AXAttribute::focused())
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let selected_attr = AXAttribute::new(&CFString::new("AXSelected"));
    let selected = element
        .attribute(&selected_attr)
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut node = AxNode {
        role: if role.is_empty() { "(no role)".into() } else { role },
        title,
        description: desc,
        focused,
        selected,
        children: Vec::new(),
    };

    if let Ok(children_val) = element.attribute(&AXAttribute::children()) {
        if let Some(arr) = children_val.as_array() {
            for child in arr.iter().take(50) {
                if let Some(child_el) = child.as_axui_element() {
                    node.children.push(ax_build_tree(child_el, max_depth - 1));
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
            if filtered.title.is_empty() && filtered.description.is_empty() && filtered.children.is_empty() {
                None
            } else {
                Some(filtered)
            }
        })
        .collect();

    if self_matches || !filtered_children.is_empty() {
        AxNode { children: filtered_children, ..root }
    } else {
        AxNode { role: root.role, children: Vec::new(), ..AxNode::new("") }
    }
}