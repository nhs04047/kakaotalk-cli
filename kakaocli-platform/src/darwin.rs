// macOS backend: AXUIElement + CGEvent via objc2 + accessibility crate
// References:
// - kakaocli (Swift) CLAUDE.md (AX quirks)
// - docs/macos-ax.md
// - https://github.com/silver-flight-group/kakaocli

use crate::*;
use kakaocli_core::db_path;
use kakaocli_core::model::DbKey;

use accessibility::AXAttribute;
use core_foundation::boolean::CFBoolean;
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
    // AXAttribute::title() returns CFString → .to_string()
    if let Ok(title) = mw.attribute(&AXAttribute::title()) {
        let title_str: String = title.to_string();
        if title_str.contains("Log in") || title_str.contains("Login") {
            return Err(PlatformError::NotLoggedIn);
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
    // AXAttribute::windows() returns CFArray<AXUIElement> → can iterate directly
    let windows = app.attribute(&AXAttribute::windows()).ok()?;

    for window in windows.iter() {
        // AXAttribute::role() returns CFString → .to_string()
        if let Ok(role) = window.attribute(&AXAttribute::role()) {
            let role_str: String = role.to_string();
            if role_str == "AXApplication" {
                continue;
            }
        }
        // Custom attribute: AXIdentifier (returns CFType → must downcast)
        let ident_attr = AXAttribute::new(&CFString::new("AXIdentifier"));
        if let Ok(id_val) = window.attribute(&ident_attr) {
            if let Some(id_cfstr) = id_val.downcast::<CFString>() {
                let id_str: String = id_cfstr.to_string();
                if id_str == "Main Window" {
                    return Some(window.clone());
                }
            }
        }
        // Fallback: first AXWindow
        if let Ok(role) = window.attribute(&AXAttribute::role()) {
            let role_str: String = role.to_string();
            if role_str == "AXWindow" {
                return Some(window.clone());
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
    // AXAttribute::children() returns CFArray<AXUIElement>
    let children = parent.attribute(&AXAttribute::children()).ok()?;

    for child in children.iter() {
        if let Ok(role) = child.attribute(&AXAttribute::role()) {
            let role_str: String = role.to_string();
            if roles.contains(&role_str.as_str()) {
                return Some(child.clone());
            }
        }
        if let Some(found) = find_child_by_role(&child, roles) {
            return Some(found);
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
        // AXAttribute::description() returns CFString
        if let Ok(desc) = row.attribute(&AXAttribute::description()) {
            let desc_str: String = desc.to_string();
            if desc_str.contains("badge me") || desc_str.contains("Self-chat") || desc_str.contains("Notes") {
                return Ok(row.clone());
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
    let children = chat_list
        .attribute(&AXAttribute::children())
        .map_err(|_| PlatformError::Other("Cannot get chat list children".into()))?;

    let rows: Vec<accessibility::AXUIElement> = children.iter().map(|r| r.clone()).collect();

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
        let s: String = desc.to_string();
        if !s.is_empty() && !s.contains("badge") {
            return Some(s);
        }
    }
    // Custom attribute: AXLabel (CFType → downcast)
    let label_attr = AXAttribute::new(&CFString::new("AXLabel"));
    if let Ok(label) = row.attribute(&label_attr) {
        if let Some(label_cfstr) = label.downcast::<CFString>() {
            let s: String = label_cfstr.to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    if let Ok(title) = row.attribute(&AXAttribute::title()) {
        let s: String = title.to_string();
        if !s.is_empty() {
            return Some(s);
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

    let windows = app
        .attribute(&AXAttribute::windows())
        .map_err(|_| PlatformError::UiError("Cannot list windows".into()))?;

    for w in windows.iter() {
        if let Ok(title) = w.attribute(&AXAttribute::title()) {
            let title_str: String = title.to_string();
            if title_str.is_empty() || title_str == "KakaoTalk" {
                continue;
            }
            if title_str.contains(expected_name) || expected_name.contains(&title_str) {
                return Ok(());
            }
            return Err(PlatformError::ChatVerificationFailed {
                expected: expected_name.to_string(),
                actual: title_str,
            });
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

// ── AX tree dump (inspect command) ──────────────────────────

fn ax_build_tree(element: &accessibility::AXUIElement, max_depth: u32) -> AxNode {
    if max_depth == 0 {
        return AxNode {
            role: "…".to_string(),
            description: "(max depth)".to_string(),
            ..AxNode::new("")
        };
    }

    let role: String = element
        .attribute(&AXAttribute::role())
        .ok()
        .map(|v: CFString| v.to_string())
        .unwrap_or_default();

    let title: String = element
        .attribute(&AXAttribute::title())
        .ok()
        .map(|v: CFString| v.to_string())
        .unwrap_or_default();

    let desc: String = element
        .attribute(&AXAttribute::description())
        .ok()
        .map(|v: CFString| v.to_string())
        .unwrap_or_default();

    let focused: bool = element
        .attribute(&AXAttribute::focused())
        .ok()
        .map(|v: CFBoolean| bool::from(v))
        .unwrap_or(false);

    // Custom attribute: AXSelected (returns CFType → downcast)
    let selected = element
        .attribute(&AXAttribute::new(&CFString::new("AXSelected")))
        .ok()
        .and_then(|v| v.downcast::<CFBoolean>())
        .map(|v| bool::from(v))
        .unwrap_or(false);

    let mut node = AxNode {
        role: if role.is_empty() { "(no role)".into() } else { role },
        title,
        description: desc,
        focused,
        selected,
        children: Vec::new(),
    };

    if let Ok(children) = element.attribute(&AXAttribute::children()) {
        for child in children.iter().take(50) {
            node.children.push(ax_build_tree(&child, max_depth - 1));
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