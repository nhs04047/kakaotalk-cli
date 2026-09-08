# 기능별 흐름도

---

## 1. 상태 확인 (`kakaocli check`)

```mermaid
sequenceDiagram
    participant T as Terminal
    participant C as kakaocli-cli
    participant P as kakaocli-platform
    participant DB as File System

    T->>C: kakaocli check
    C->>P: check_status()

    alt macOS
        P->>DB: Full Disk Access 확인 (Container 접근)
        alt 권한 없음
            DB-->>P: ❌ 접근 불가
            P-->>C: Error("Full Disk Access 필요")
            C-->>T: ⛔ 에러 출력
        end
        P->>DB: KakaoTalk.app 실행 확인
        P->>DB: DB 파일 존재 확인
        P-->>C: AppStatus (Ready/NotRunning/DbAccessible)
    else Windows
        P->>DB: chat_data/*.edb 존재 확인
        P->>DB: KakaoTalk.exe 프로세스 확인
        P-->>C: AppStatus
    end

    C-->>T: 상태 출력 (앱/DB/권한)
```

---

## 2. DB 복호화 (`kakaocli auth`)

### macOS 흐름

```mermaid
sequenceDiagram
    participant T as Terminal
    participant C as CLI
    participant Core as kakaocli-core
    participant DB as File System

    T->>C: kakaocli auth
    C->>Core: mac_platform_uuid()
    Core->>DB: ioreg IOPlatformUUID 읽기
    DB-->>Core: "XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX"
    
    C->>Core: mac_user_id()
    Core->>DB: ~/Library/Preferences/...plist
    Note over Core: FSChatWindowTransparency 파싱
    DB-->>Core: userId (예: 12345678)

    C->>Core: derive_mac_key(userId, uuid)
    Note over Core: PBKDF2-SHA256, 100k iterations
    Note over Core: → 128바이트 출력 → 앞 32바이트 → hex
    
    C->>Core: mac_db_files()
    Core->>DB: Container 디렉토리 탐색
    DB-->>Core: DB 파일 경로

    C->>C: open_sqlcipher(db, key)
    Note over C: PRAGMA cipher_compatibility = 3
    Note over C: PRAGMA key = "x'<hex>'..."
    Note over C: 실패 → compat = 4 재시도 (새 커넥션)

    alt 성공
        C-->>T: ✅ Tables: NTChatRoom, NTChatMessage, NTUser...
    else 실패
        C-->>T: ❌ 복호화 실패
    end
```

### Windows 흐름

```mermaid
sequenceDiagram
    participant T as Terminal
    participant C as CLI
    participant P as kakaocli-platform
    participant Proc as KakaoTalk.exe

    T->>C: kakaocli auth
    C->>C: windows_edb_files()
    Note over C: %LOCALAPPDATA%\Kakao\KakaoTalk\chat_data\*.edb

    C->>P: DEK 스캔 시작
    
    P->>Proc: CreateToolhelp32Snapshot → PID 찾기
    P->>Proc: OpenProcess(PROCESS_VM_READ)
    P->>Proc: ReadProcessMemory → 힙 영역 스캔
    
    Note over P: 32바이트 DEK 후보 수집
    
    loop 각 후보 검증
        P->>P: BCrypt AES-256 ECB decrypt(page-1)
        alt SQLite header 매직 확인
            P-->>C: ✅ DEK 발견
        else 실패
            P->>Proc: 다음 후보 스캔
        end
    end

    alt DEK 발견
        C->>C: open_sqlcipher(edb, dek_hex)
        C-->>T: ✅ 복호화 성공
    else 실패
        C-->>T: ❌ 카카오톡을 먼저 실행해주세요
    end
```

---

## 3. 채팅방 목록 (`kakaocli chats`)

```mermaid
sequenceDiagram
    participant T as Terminal
    participant C as CLI
    participant DB as SQLCipher DB

    T->>C: kakaocli chats --json --limit 20
    
    C->>DB: SELECT FROM NTChatRoom LEFT JOIN NTUser
    Note over C: 최근 활동순 정렬
    
    DB-->>C: [{id, type, display_name, member_count, unread_count, last_message_at}]
    
    alt --json
        C-->>T: JSON 배열 출력
    else 기본
        C-->>T: 표 형태 출력 (이름 | 안읽음 | 최근메시지)
    end
```

---

## 4. 메시지 조회 (`kakaocli msg`)

```mermaid
sequenceDiagram
    participant T as Terminal
    participant C as CLI
    participant DB as SQLCipher DB

    T->>C: kakaocli msg --chat "지수" --since 1h

    Note over C: 1. chats()로 채팅방 검색
    C->>DB: NTChatRoom JOIN NTUser WHERE name LIKE '%지수%'
    DB-->>C: chatId = 12345
    
    Note over C: 2. 메시지 조회
    C->>DB: NTChatMessage JOIN NTUser WHERE chatId=12345 AND sentAt >= now-1h
    
    DB-->>C: [{sender, text, type, sent_at, is_from_me}]
    
    alt --json
        C-->>T: JSON
    else 기본
        C-->>T: "지수: 안녕!" (시간 / 내메시지 구분)
    end
```

---

## 5. 검색 (`kakaocli find`)

```mermaid
stateDiagram-v2
    [*] --> 기본검색: --exact, --regex 없음
    [*] --> 정확검색: --exact
    [*] --> 정규식검색: --regex
    [*] --> 방검색: --rooms
    [*] --> 친구검색: --friends
    [*] --> 통합검색: --all

    기본검색 --> 결과: LIKE '%키워드%'
    정확검색 --> 결과: = '키워드'
    정규식검색 --> 결과: REGEXP '패턴'

    방검색 --> 결과: NTChatRoom.chatName LIKE
    친구검색 --> 결과: NTUser.nickName LIKE
    통합검색 --> 결과: 메시지+방+친구 병합
```

---

## 6. 메시지 전송 (`kakaocli send`) — macOS

```mermaid
sequenceDiagram
    participant T as Terminal
    participant C as CLI
    participant P as kakaocli-platform
    participant OS as macOS AX
    participant KT as KakaoTalk.app

    T->>C: kakaocli send "지수" "안녕!"

    alt --dry-run
        Note over C: 미리보기 모드 → 전송 안 함
    end

    C->>P: send_message("지수", "안녕!")
    
    alt 앱이 꺼져 있음
        P->>OS: NSWorkspace → KakaoTalk 실행
        P->>OS: 자동 로그인 (Keychain → 크리덴셜)
    end

    P->>OS: 메인 윈도우 찾기 (id="Main Window")
    
    P->>OS: 채팅방 리스트 스캔
    OS->>KT: AXChildren → 각 리스트 아이템
    KT-->>OS: kAXDescription으로 "지수" 매칭

    P->>OS: AXSelectedRowsAttribute 선택 + Enter(CGEvent)
    Note over P: ❌ double-click 금지 (오프스크린 좌표)

    P->>OS: 열린 창 제목 읽기
    Note over P: 🔒 대상 방 검증

    alt 방 제목 불일치
        P-->>C: Error("ChatVerificationFailed")
        C-->>T: ❌ 실제 열린 방과 다른 방입니다
    end

    P->>OS: CGEventKeyboard → 유니코드 입력
    P->>OS: Enter → 전송

    Note over P: ⏱ Rate limit: 2초 대기

    alt --dry-run
        C-->>T: ✅ 전송될 메시지: "지수"에게 "안녕!" (미리보기)
    else 실제 전송
        C-->>T: ✅ 전송 완료
    end
```

---

## 7. 메시지 전송 (`kakaocli send`) — Windows

```mermaid
sequenceDiagram
    participant T as Terminal
    participant C as CLI
    participant P as kakaocli-platform
    participant OS as Windows UIA
    participant KT as KakaoTalk.exe

    T->>C: kakaocli send "지수" "안녕!"
    C->>P: send_message("지수", "안녕!")
    
    alt 앱이 꺼져 있음
        P->>OS: CreateProcess → KakaoTalk 실행
        P->>OS: 자동 로그인 (Credential Manager)
    end

    P->>OS: FindWindow → 메인 윈도우
    P->>OS: IUIAutomation Tree → ListItem 검색
    P->>OS: Condition(name="지수") → InvokePattern

    Note over P: 🔒 대상 방 검증 (제목 확인)

    P->>OS: Edit 컨트롤 → SetFocus → SendKeys(유니코드)
    P->>OS: 전송 버튼 → InvokePattern

    alt --dry-run
        C-->>T: ✅ 미리보기
    else 실제 전송
        C-->>T: ✅ 전송 완료
    end
```

---

## 8. 🔄 새 메시지 동기화 흐름 (`kakaocli sync --follow`)

**이게 너가 말한 "채팅 동기화" 흐름이야.**

```mermaid
sequenceDiagram
    participant T as Terminal
    participant C as CLI
    participant DB as SQLCipher DB
    participant Web as Webhook Server (옵션)

    T->>C: kakaocli sync --follow --interval 2

    Note over C: 최대 logId 확인 (초기값 = 0)
    C->>DB: SELECT MAX(logId) FROM NTChatMessage
    DB-->>C: maxLogId = 1500

    loop 폴링 (매 2초)
        Note over C: ✅ 이전보다 큰 logId만 조회
        C->>DB: SELECT ... WHERE logId > 1500 ORDER BY logId ASC
        DB-->>C: [{logId: 1501, chat: "지수", text: "밥 먹었어?"}, ...]

        alt 새 메시지 있음
            C->>C: maxLogId 업데이트 = 1505
            C-->>T: NDJSON 출력 (줄바꿈 구분)
            Note over T: {"type":"message","chat":"지수","sender":"지수","text":"밥 먹었어?","time":"..."}

            alt --webhook 설정됨
                C->>Web: POST [json_array] → http://localhost:8080/kakao
                Web-->>C: 200 OK
            end
        end

        Note over C: ⏱ interval=2s 만큼 대기
    end
```

### 동기화 관련 기술적 포인트

| 항목 | 설명 |
|------|------|
| **트리거 방식** | ❌ 푸시 불가 (DB만 있음). **폴링** 사용 |
| **증분 조회** | `WHERE logId > :last_max` — 이미 읽은 메시지는 다시 안 가져옴 |
| **출력 형식** | NDJSON (JSON Stream) — `sync \| jq` 조합 가능 |
| **--webhook** | 새 메시지를 HTTP POST로 전달 → 내 서버에서 처리 |
| **macOS** | 앱 없이 DB만으로 동기화 가능 ✅ |
| **Windows** | KakaoTalk.exe 실행 중이어야 DEK 있음 (DB 접근) ⚠️ |
| **DEK 캐싱** | Windows는 한 번 검증된 DEK를 로컬 캐시에 저장 → 재시작 없으면 지속 사용 |
| **Rate limit** | --interval 기본 2초. 너무 빠르면 DB 부하 |

---

## 9. 전체 기능 요약 맵

```mermaid
flowchart TB
    CLI[kakaocli-rs CLI]
    
    CLI --> CHECK[check]
    CLI --> AUTH[auth]
    CLI --> CHATS[chats]
    CLI --> MSG[msg]
    CLI --> FIND[find]
    CLI --> QUERY[query]
    CLI --> SEND[send]
    CLI --> SYNC[sync]
    CLI --> INSPECT[inspect]
    CLI --> LOGIN[login]

    AUTH -->|macOS| KDF[PBKDF2 키 유도]
    AUTH -->|Windows| DEK[프로세스 메모리 DEK 스캔]
    KDF --> DB[(SQLCipher DB)]
    DEK --> DB

    CHATS --> DB
    MSG --> DB
    FIND --> DB
    QUERY --> DB
    
    SEND -->|macOS| AX[AXUIElement 자동화]
    SEND -->|Windows| UIA[UIAutomation 자동화]
    
    SYNC --> DB
    SYNC -->|--webhook| WEB[HTTP POST]
    SYNC -->|stdout| NDJSON[NDJSON 스트림]

    INSPECT -->|macOS| AX
    INSPECT -->|Windows| UIA

    LOGIN --> KEY[keyring → OS 키체인]