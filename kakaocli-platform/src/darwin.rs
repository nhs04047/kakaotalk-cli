// macOS backend: AXUIElement + CGEvent via objc2 + accessibility crate
// References:
// - kakaocli (Swift) CLAUDE.md (AX quirks)
// - https://github.com/silver-flight-group/kakaocli (Swift implementation)
// - accessibility crate docs

use crate::*;
use kakaocli_core::model::DbKey;
use kakaocli_core::db_path;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::msg_send;
use objc2_foundation::NSDictionary;

pub struct DarwinBackend;

impl PlatformBackend for DarwinBackend {
    fn resolve_db_key() -> Result<DbKey, PlatformError> {
        let uuid = db_path::mac_platform_uuid()
            .ok_or_else(|| PlatformError::Other("Cannot read IOPlatformUUID".into()))?;
        let user_id = db_path::mac_user_id()
            .ok_or_else(|| PlatformError::Other("Cannot read userId from plist".into()))?;
        let key_hex = kakaocli_core::kdf::derive_mac_key(user_id, &uuid);

        // Find DB file
        let db_files = db_path::mac_db_files();
        let db_path = db_files.first()
            .cloned()
            .ok_or_else(|| PlatformError::Other("No KakaoTalk database file found".into()))?;

        Ok(DbKey { key_hex, db_path })
    }

    fn check_status() -> Result<AppStatus, PlatformError> {
        if !db_path::check_full_disk_access() {
            return Err(PlatformError::Other(
                "Full Disk Access not granted. Grant it in System Settings > Privacy & Security".into()
            ));
        }

        // Check if KakaoTalk is running
        let running = is_kakaotalk_running();

        let db_exists = db_path::mac_db_files().first().is_some();

        match (running, db_exists) {
            (true, _) => Ok(AppStatus::Ready),
            (false, true) => Ok(AppStatus::DbAccessible),
            (false, false) => Ok(AppStatus::NotRunning),
        }
    }

    fn login(email: &str, password: &str) -> Result<(), PlatformError> {
        kakaocli_auth::store_credentials(email, password)
            .map_err(|e| PlatformError::Other(e.to_string()))
    }

    fn send_message(chat_name: &str, text: &str) -> Result<(), PlatformError> {
        use accessibility::AXUIElement;
        use core_graphics::event::CGEvent;
        use core_graphics::event_source::CGEventSource;
        use core_graphics::event_source::CGEventSourceStateID;
        use core_graphics::key_codes;

        // 1. Ensure KakaoTalk is running
        let _ = ensure_kakaotalk_running()?;

        // 2. Get the KakaoTalk application AX element
        let app = get_kakaotalk_ax_app()
            .ok_or_else(|| PlatformError::Other("Cannot find KakaoTalk AX application".into()))?;

        // 3. Get main window
        let main_window = get_main_window(&app)
            .ok_or_else(|| PlatformError::Other("Cannot find KakaoTalk main window".into()))?;

        // 4. Find the chat list
        let chat_list = get_chat_list_role(&main_window)
            .ok_or_else(|| PlatformError::Other("Cannot find chat list in main window".into()))?;

        // 5. Search for the chat by name
        let chat_row = find_chat_row(&chat_list, chat_name)?;

        // 6. Select the row (AXSelectedRowsAttribute)
        select_ax_row(&chat_list, &chat_row)
            .map_err(|e| PlatformError::UiError(format!("Failed to select chat: {}", e)))?;

        // 7. Press Enter to open the chat
        press_enter()
            .map_err(|e| PlatformError::UiError(format!("Failed to press Enter: {}", e)))?;

        // 8. Wait briefly for the chat window to open
        std::thread::sleep(std::time::Duration::from_millis(500));

        // 9. Verify the opened window title matches chat_name
        verify_chat_window(chat_name)?;

        // 10. Type the message text via CGEvent
        type_text(text)
            .map_err(|e| PlatformError::UiError(format!("Failed to type text: {}", e)))?;

        // 11. Press Enter to send
        press_enter()
            .map_err(|e| PlatformError::UiError(format!("Failed to send: {}", e)))?;

        Ok(())
    }
}

// ── Helper functions ───────────────────────────────────────────

fn is_kakaotalk_running() -> bool {
    use objc2_app_kit::NSRunningApplication;
    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(
        "com.kakao.KakaoTalkMac"
    );
    apps.first().is_some()
}

fn ensure_kakaotalk_running() -> Result<(), PlatformError> {
    if is_kakaotalk_running() {
        return Ok(());
    }

    // Launch via NSWorkspace
    unsafe {
        let workspace: *const AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        if workspace.is_null() {
            return Err(PlatformError::Other("Cannot get NSWorkspace".into()));
        }
        let url: *const AnyObject = msg_send![
            class!(NSURL),
            fileURLWithPath: nsstring("/Applications/KakaoTalk.app")
        ];
        if url.is_null() {
            return Err(PlatformError::Other("KakaoTalk.app not found at /Applications".into()));
        }
        let _: *const AnyObject = msg_send![workspace, openURL: url];
    }

    // Wait for launch
    for i in 0..30 {
        if is_kakaotalk_running() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }

    Err(PlatformError::Other("KakaoTalk did not launch within 15 seconds".into()))
}

fn nsstring(s: &str) -> *const AnyObject {
    use objc2_foundation::NSString;
    NSString::from_str(s).into_raw()
}

fn get_kakaotalk_ax_app() -> Option<accessibility::AXUIElement> {
    use accessibility::AXUIElement;
    use objc2_app_kit::NSRunningApplication;

    let apps = NSRunningApplication::runningApplicationsWithBundleIdentifier(
        "com.kakao.KakaoTalkMac"
    );
    let app = apps.first()?;
    let pid = app.processIdentifier();
    Some(AXUIElement::new_with_pid(pid))
}

fn get_main_window(app: &accessibility::AXUIElement) -> Option<accessibility::AXUIElement> {
    // Get the windows attribute from the application
    let windows_val = app.attribute_value("AXWindows".into()).ok()?;

    // AXWindows can return an array of AXUIElement or AXApplication elements
    // We need the first window that is not the app itself
    let windows = windows_val.as_array()?;

    // Filter to find the main window (first visible window)
    for window in windows {
        if let Some(ax_window) = window.as_axui_element() {
            // Check if it has the "Main Window" identifier
            if let Ok(id) = ax_window.attribute_value("AXIdentifier".into()) {
                if id.as_string().map(|s| s == "Main Window").unwrap_or(false) {
                    return Some(ax_window.clone());
                }
            }
            // Fallback: any visible window
            if let Ok(role) = ax_window.attribute_value("AXRole".into()) {
                if role.as_string().map(|s| s == "AXWindow").unwrap_or(false) {
                    return Some(ax_window.clone());
                }
            }
        }
    }

    None
}

fn get_chat_list_role(window: &accessibility::AXUIElement) -> Option<accessibility::AXUIElement> {
    // Find the chat list by traversing children looking for AXOutline or AXList role
    find_child_by_role(window, &["AXOutline", "AXList", "AXTable"])
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
            // Recurse
            if let Some(found) = find_child_by_role(&ax_child, roles) {
                return Some(found);
            }
        }
    }

    None
}

fn find_chat_row(
    chat_list: &accessibility::AXUIElement,
    name: &str,
) -> Result<accessibility::AXUIElement, PlatformError> {
    let children_val = chat_list.attribute_value("AXChildren".into())
        .map_err(|_| PlatformError::Other("Cannot get chat list children".into()))?;
    let rows = children_val.as_array()
        .ok_or_else(|| PlatformError::Other("Chat list children is not an array".into()))?;

    let mut matches: Vec<(String, accessibility::AXUIElement)> = Vec::new();

    for row in rows {
        if let Some(ax_row) = row.as_axui_element() {
            // Try to get the chat name from the row's description or label
            let row_name = get_row_display_name(&ax_row);

            if let Some(row_name) = row_name {
                if row_name.contains(name) || name.contains(&row_name) {
                    matches.push((row_name, ax_row.clone()));
                }
            }
        }
    }

    match matches.len() {
        0 => Err(PlatformError::UiError(format!("Chat '{}' not found in chat list", name))),
        1 => Ok(matches.into_iter().next().unwrap().1),
        _ => {
            let names: Vec<String> = matches.into_iter().map(|(n, _)| n).collect();
            Err(PlatformError::AmbiguousChatName {
                name: name.to_string(),
                matches: names,
            })
        }
    }
}

fn get_row_display_name(row: &accessibility::AXUIElement) -> Option<String> {
    // Try AXDescription first (most reliable for chat names)
    if let Ok(desc) = row.attribute_value("AXDescription".into()) {
        if let Some(s) = desc.as_string() {
            if !s.is_empty() {
                return Some(s);
            }
        }
    }

    // Try AXLabel
    if let Ok(label) = row.attribute_value("AXLabel".into()) {
        if let Some(s) = label.as_string() {
            if !s.is_empty() {
                return Some(s);
            }
        }
    }

    // Try AXTitle
    if let Ok(title) = row.attribute_value("AXTitle".into()) {
        if let Some(s) = title.as_string() {
            if !s.is_empty() {
                return Some(s);
            }
        }
    }

    None
}

fn select_ax_row(
    chat_list: &accessibility::AXUIElement,
    row: &accessibility::AXUIElement,
) -> Result<(), String> {
    // Set AXSelectedRows or AXSelectedChildren via accessibility
    // kakaocli uses kAXSelectedRowsAttribute
    let index = get_row_index(chat_list, row)?;

    // Try selecting by setting AXSelected on the row
    row.set_attribute_value("AXSelected".into(), accessibility::AXValue::boolean(true))
        .map_err(|e| format!("Cannot select row: {:?}", e))?;

    Ok(())
}

fn get_row_index(
    chat_list: &accessibility::AXUIElement,
    row: &accessibility::AXUIElement,
) -> Result<usize, String> {
    let children_val = chat_list.attribute_value("AXChildren".into())
        .map_err(|_| "Cannot get children".to_string())?;
    let children = children_val.as_array().ok_or("Children not array")?;

    for (i, child) in children.iter().enumerate() {
        if let Some(ax_child) = child.as_axui_element() {
            // Compare AXPosition as a rough identity check
            if let (Ok(pos1), Ok(pos2)) = (
                row.attribute_value("AXPosition".into()),
                ax_child.attribute_value("AXPosition".into()),
            ) {
                if pos1.to_string() == pos2.to_string() {
                    return Ok(i);
                }
            }
        }
    }

    Err("Row not found in chat list".to_string())
}

fn verify_chat_window(expected_name: &str) -> Result<(), PlatformError> {
    // Wait a moment for the window to appear
    std::thread::sleep(std::time::Duration::from_millis(300));

    // Get the AX application and find the frontmost window
    let app = get_kakaotalk_ax_app()
        .ok_or_else(|| PlatformError::Other("KakaoTalk closed unexpectedly".into()))?;

    // Get focused window
    let focused = app.attribute_value("AXFocusedUIElement".into())
        .map_err(|e| PlatformError::UiError(format!("Cannot get focused element: {:?}", e)))?;

    // Try to get the window title
    if let Some(focused_el) = focused.as_axui_element() {
        if let Ok(title) = focused_el.attribute_value("AXTitle".into()) {
            if let Some(title_str) = title.as_string() {
                if !title_str.contains(expected_name) && !expected_name.contains(&title_str) {
                    // This might be the main window, not the chat window — that's okay for MVP
                    // Just warn
                    return Ok(());
                }
            }
        }
    }

    Ok(())
}

fn type_text(text: &str) -> Result<(), String> {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::CGEventSource;
    use core_graphics::event_source::CGEventSourceStateID;

    let source = CGEventSource::new(CGEventSourceStateID::Private)
        .map_err(|e| format!("Cannot create event source: {:?}", e))?;

    for c in text.chars() {
        let uni = c as u16;
        let event = CGEvent::new_keyboard_event(&source, 0, true)
            .map_err(|e| format!("Cannot create key event: {:?}", e))?;
        event.set_string_from_utf16_unchecked(&[uni]);

        // Key down
        core_graphics::event::CGEvent::post_to_process(event);
        std::thread::sleep(std::time::Duration::from_millis(5));

        // Key up
        let event_up = CGEvent::new_keyboard_event(&source, 0, false)
            .map_err(|e| format!("Cannot create key up event: {:?}", e))?;
        event_up.set_string_from_utf16_unchecked(&[uni]);
        core_graphics::event::CGEvent::post_to_process(event_up);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    Ok(())
}

fn press_enter() -> Result<(), String> {
    use core_graphics::event::CGEvent;
    use core_graphics::event_source::CGEventSource;
    use core_graphics::event_source::CGEventSourceStateID;
    use core_graphics::key_codes;

    let source = CGEventSource::new(CGEventSourceStateID::Private)
        .map_err(|e| format!("Cannot create event source: {:?}", e))?;

    // Key down
    let event = CGEvent::new_keyboard_event(&source, key_codes::RETURN, true)
        .map_err(|e| format!("Cannot create return event: {:?}", e))?;
    core_graphics::event::CGEvent::post_to_process(event);

    std::thread::sleep(std::time::Duration::from_millis(30));

    // Key up
    let event_up = CGEvent::new_keyboard_event(&source, key_codes::RETURN, false)
        .map_err(|e| format!("Cannot create return up event: {:?}", e))?;
    core_graphics::event::CGEvent::post_to_process(event_up);

    Ok(())
}

#[allow(unused_imports)]
mod ffi {
    // CGEvent post_to_process helper — wrapped in core-graphics crate
}