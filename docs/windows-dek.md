# Windows 백엔드: DEK 스캔 + UIAutomation

## 개요

Windows KakaoTalk은 `.edb` 파일을 SQLCipher로 암호화하지만,
키 유도가 macOS와 달라 **프로세스 메모리에서 DEK를 직접 포착**해야 함.

## DEK (Database Encryption Key) 스캔

### 배경

- macOS: plist + PBKDF2로 키 계산 가능 (공개된 알고리즘)
- Windows: **공개된 키 유도 알고리즘이 더 이상 동작하지 않음**
- 해결책: 실행 중인 KakaoTalk.exe의 메모리에서 실제 SQLCipher DEK 후보를 찾아 검증

### MoniKa 접근법 (참고)

MoniKa는 C++ Windows 앱으로 DEK를 포착:
1. ETW 이벤트로 KakaoTalk.exe 시작 감지
2. Toolhelp로 실행 중인 프로세스 찾기
3. 프로세스 메모리에서 0x88 바이트 시그니처로 DEK 후보 스캔
4. BCrypt AES-256 ECB로 `.edb` page-1 복호화 검증
5. 검증된 DEK 로컬 캐시에 저장

### Rust 구현

```rust
use windows::Win32::System::Threading::*;
use windows::Win32::System::Diagnostics::ToolHelp::*;
use windows::Win32::Security::*;

/// DEK 스캐너
pub struct DekScanner;

impl DekScanner {
    /// KakaoTalk.exe PID 찾기
    pub fn find_kakao_process() -> Option<u32> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
            // PROCESSENTRY32 순회 → "KakaoTalk.exe" 매칭
        }
    }

    /// 프로세스 메모리에서 32바이트 DEK 후보 스캔
    pub fn scan_dek_candidates(pid: u32) -> Vec<Vec<u8>> {
        unsafe {
            let process = OpenProcess(PROCESS_VM_READ, false, pid).ok()?;
            // 힙/데이터 영역 메모리 읽기
            // 32바이트 연속 데이터 후보 수집
            // 시그니처 기반 필터링
        }
    }

    /// BCrypt AES-256 page-1 oracle로 DEK 검증
    pub fn verify_dek(dek: &[u8], db_path: &Path) -> bool {
        // 1. .edb 파일의 첫 4096바이트 읽기
        // 2. BCrypt AES-256 ECB로 dek 복호화
        // 3. 결과에 SQLCipher header magic 확인
    }
}
```

### 검증 알고리즘 (page-1 oracle)

SQLCipher DB의 첫 페이지는 다음과 같은 구조:

```
Page 1:
  [0..15]   : salt (16바이트, PRAGMA key를 KDF할 때 사용)
  [16..47]  : AES-256 encrypted first page data
  [48..79]  : HMAC (integrity check)
  ...
```

검증:
1. `.edb` 파일에서 첫 4096바이트 읽기
2. 후보 DEK로 page-1을 AES-256 ECB 복호화 시도
3. 복호화된 데이터가 SQLite header magic (`"SQLite format 3\x00"`)로 시작하면 성공
4. 또는 `PRAGMA key` 후 `SELECT count(*) FROM sqlite_master` 성공

### 캐싱

```rust
// Windows의 경우 DEK를 %LOCALAPPDATA%/kakaocli-rs/cache.db에 캐시
// KakaoTalk 재시작 시마다 DEK가 바뀔 수 있으므로,
// 캐시된 DEK로 DB 오픈 실패하면 재스캔
```

## UIAutomation 전송

### 개요

Windows Accessibility API (UIAutomation)을 통해 KakaoTalk Windows 클라이언트를 자동화.

```rust
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// 메시지 전송
pub fn send_message(chat_name: &str, text: &str) -> Result<()> {
    // 1. 메인 윈도우 찾기
    let hwnd = find_main_window()?;

    // 2. IUIAutomation Tree 탐색
    let automation: IUIAutomation = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)? };

    // 3. 채팅방 리스트에서 chat_name 검색
    let condition = automation.CreatePropertyCondition(UIA_NamePropertyId, &BSTR::from(chat_name))?;
    let element = automation.FindFirst(TreeScope_Descendants, &condition)?;

    // 4. 채팅방 열기 (InvokePattern)
    let invoke_pattern = element.GetCurrentPattern(UIA_InvokePatternId)?;
    // ...

    // 5. 텍스트 입력
    let edit = find_edit_element(&automation)?;
    // SetFocus → SendKeys (or ValuePattern.SetValue)

    // 6. 전송 버튼 클릭
    let send_button = find_send_button(&automation)?;
    let invoke = send_button.GetCurrentPattern(UIA_InvokePatternId)?;
    // ...
}
```

### Win32 SendKeys로 텍스트 입력

```rust
fn send_keys(text: &str) {
    for ch in text.chars() {
        // VkKeyScanEx로 가상키 매핑
        // SendInput으로 키 다운/업 전송
        // shift 상태 처리 (한영 전환)
    }
    // Enter 전송
}
```

## 주의사항

- **카카오톡 Windows 버전별 UI 트리 차이가 큼** — 실제 분석 필수
- **UIAutomation은 권한에 따라 동작이 다름** (관리자/일반)
- **DEK 스캔은 KakaoTalk.exe 실행 중에만 가능** (읽기 명령어에도 앱 필요)
- **64비트 전용** (Windows KakaoTalk은 64비트)
- **Windows 10/11 x64만 지원**

## DEK 스캔 실패 시 대안

1. 사전에 MoniKa를 실행해 DEK를 캐시하고 kakaocli-rs가 읽도록
2. N-API addon으로 C++ DEK 스캐너를 만들어 Node.js용으로도 재사용
3. 사용자에게 수동으로 DEK 입력 요청 (비추)