//! Windows inspect: KakaoTalk UIAutomation 트리를 AxNode로 덤프 (읽기 전용).
//! macOS의 AX inspect에 대응. 커스텀 EVA UI라 대부분 Document로 노출되지만
//! 커스텀 이름/AutomationId로 구조 파악 가능 (send 등 자동화 디버깅용).

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTreeWalker,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
};

use crate::{AxNode, PlatformError};

pub fn dump_tree(chat: Option<&str>, max_depth: u32) -> Result<AxNode, PlatformError> {
    let pid = crate::dek::find_kakao_pid().ok_or(PlatformError::AppNotAvailable)?;
    let hwnd = find_window(pid, chat).ok_or_else(|| {
        PlatformError::Other(match chat {
            Some(c) => format!("'{}' 채팅창을 찾지 못했습니다 (먼저 열어주세요).", c),
            None => "KakaoTalk 메인 창을 찾지 못했습니다.".to_string(),
        })
    })?;

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let auto: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| PlatformError::UiError(format!("CUIAutomation: {e}")))?;
        let root = auto
            .ElementFromHandle(hwnd)
            .map_err(|e| PlatformError::UiError(format!("ElementFromHandle: {e}")))?;
        let walker = auto
            .ControlViewWalker()
            .map_err(|e| PlatformError::UiError(format!("ControlViewWalker: {e}")))?;
        Ok(build(&walker, &root, max_depth))
    }
}

fn find_window(pid: u32, chat: Option<&str>) -> Option<HWND> {
    struct Ctx {
        pid: u32,
        out: Vec<(HWND, String)>,
    }
    let mut ctx = Ctx {
        pid,
        out: Vec::new(),
    };

    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        let ctx = &mut *(l.0 as *mut Ctx);
        let mut wpid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut wpid));
        if wpid == ctx.pid && class_name(h) == "EVA_Window_Dblclk" {
            let t = window_text(h);
            if !t.is_empty() {
                ctx.out.push((h, t));
            }
        }
        BOOL(1)
    }

    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut ctx as *mut Ctx as isize));
    }
    match chat {
        Some(c) => ctx
            .out
            .into_iter()
            .find(|(_, t)| t.contains(c) || c.contains(t.as_str()))
            .map(|(h, _)| h),
        None => ctx
            .out
            .iter()
            .find(|(_, t)| t == "카카오톡")
            .or_else(|| ctx.out.first())
            .map(|(h, _)| *h),
    }
}

unsafe fn build(walker: &IUIAutomationTreeWalker, el: &IUIAutomationElement, depth: u32) -> AxNode {
    let mut node = AxNode::new(control_type_name(
        el.CurrentControlType().map(|c| c.0).unwrap_or(0),
    ));
    node.title = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
    node.description = el
        .CurrentAutomationId()
        .map(|b| b.to_string())
        .unwrap_or_default();

    if depth > 0 {
        if let Ok(first) = walker.GetFirstChildElement(el) {
            let mut cur = first;
            loop {
                node.children.push(build(walker, &cur, depth - 1));
                match walker.GetNextSiblingElement(&cur) {
                    Ok(n) => cur = n,
                    Err(_) => break,
                }
            }
        }
    }
    node
}

fn window_text(h: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(h);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(h, &mut buf);
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n as usize])
}

fn control_type_name(ct: i32) -> &'static str {
    match ct {
        50000 => "Button",
        50004 => "Edit",
        50008 => "List",
        50007 => "ListItem",
        50011 => "Text",
        50020 => "Pane",
        50026 => "Group",
        50032 => "Window",
        50033 => "Document",
        50025 => "TabItem",
        50002 => "CheckBox",
        50019 => "MenuItem",
        _ => "Element",
    }
}
