# kakaocli-rs

**크로스플랫폼(macOS + Windows) KakaoTalk CLI** — Rust로 작성.

로컬에 설치된 KakaoTalk의 암호화된 대화 DB를 **읽기 전용**으로 복호화해 채팅방·메시지
조회/검색을 제공하고, 접근성/Win32 자동화로 메시지 전송(`send`)과 새 메시지 스트리밍
(`sync`)을 지원합니다.

> ⚠️ 이 도구는 **본인 기기의 본인 계정 데이터**에 한해 사용하는 것을 전제로 합니다.
> DB는 항상 `SQLITE_OPEN_READ_ONLY`로만 열며, KakaoTalk DB에 **절대 쓰지 않습니다**.

---

## 특징

- **크로스플랫폼**: macOS(14+)와 Windows(10/11 x64)를 하나의 CLI로.
- **읽기 전용 복호화**: SQLCipher v4(vendored OpenSSL) 기반. DB 파일은 읽기 전용으로만 오픈.
- **자동 키 획득**:
  - macOS — KDF로 DB 키 유도(userId/기기 UUID 자동 탐색).
  - Windows — 실행 중인 KakaoTalk 프로세스 메모리에서 **파일별 DEK** 추출 후 복호화.
- **사람 친화적 출력** + `--json`(스크립트 연동).
- **안전한 전송**: 전송 직전 대상 채팅방 제목을 재검증해 오발송을 차단.

## 지원 플랫폼별 방식

| | macOS | Windows |
|---|---|---|
| DB 키 | KDF 유도(단일 키) | 프로세스 메모리 스캔 → **파일별 DEK** |
| 자동화 백엔드 | Accessibility(AX) | Win32 컨트롤 직접 제어(RichEdit) + UIAutomation(inspect) |
| 전제 | KakaoTalk 로그인 | KakaoTalk **실행 중**(DEK가 메모리에만 상주) |

Windows는 DEK가 실행 중인 프로세스 메모리에만 있으므로, 읽기 명령도 **KakaoTalk이
실행 중이어야** 동작합니다. 또한 특정 방의 `chatLogs`는 해당 방을 앱에서 한 번
열어야 DEK가 상주합니다.

---

## 프로젝트 구조

Cargo workspace (5개 크레이트):

```
kakaotalk-cli/
├── kakaocli-core/       # 도메인 모델(Chat/Message), KDF, DB 경로 탐색, sync 상태/유틸
├── kakaocli-db/         # SQLCipher 리더(rusqlite + bundled-sqlcipher), 플랫폼별 쿼리
├── kakaocli-platform/   # 플랫폼 백엔드
│   ├── darwin.rs        #   macOS: AXUIElement 자동화(send/inspect), KDF 키 해석
│   ├── dek.rs           #   Windows: DEK 메모리 스캔 + .edb 오픈(+세션 캐시)
│   ├── winsend.rs       #   Windows: RichEdit 직접 제어 전송 + 검색창 자동 열기
│   └── winuia.rs        #   Windows: UIAutomation 트리 덤프(inspect)
├── kakaocli-auth/       # OS 키체인 크레덴셜 저장(keyring: apple-native/windows-native)
├── kakaocli-cli/        # clap 기반 CLI 엔트리포인트 + 출력 포매팅(display.rs)
└── docs/ (→ ../kakaotalk-cli-design)  # 설계 문서(DESIGN/ADR/스키마/플로우/roadmap)
```

---

## 빌드

### macOS

```bash
cargo build --release
```

`bundled-sqlcipher-vendored-openssl`을 사용하므로 별도 SQLCipher 설치가 필요 없습니다.

### Windows

vendored OpenSSL 소스 빌드를 위해 아래 툴체인이 필요합니다:

- **MSVC Build Tools 2022** (x64)
- **Strawberry Perl** (OpenSSL 빌드용)
- Rust(MSVC 툴체인)

VS DevShell 환경에서 빌드해야 합니다. 로컬 개발 편의 스크립트
(`winbuild.ps1` / `winrun.ps1`)는 gitignore되어 있으며 아래 형태로 동작합니다:

```powershell
# vcvars(DevShell) + Strawberry Perl PATH 설정 후 cargo 실행
powershell -NoProfile -ExecutionPolicy Bypass -File winbuild.ps1 build
```

---

## 사용법

```
kakaocli <COMMAND> [OPTIONS]
```

### 전역 옵션

| 플래그 | 설명 |
|---|---|
| `--json` | 지원 명령에서 JSON 출력 |
| `-v, --verbose` | 상세 로그 |
| `--db-path <PATH>` | DB 경로 수동 지정(기본: 자동 탐지) |
| `--key <HEX>` | DB 키 수동 지정(기본: 자동 유도/스캔) |
| `--user-id <ID>` | userId 수동 지정(macOS 키 유도용) |
| `--uuid <UUID>` | 기기 UUID 수동 지정(macOS) |

### 명령어

| 명령 | 별칭 | 설명 |
|---|---|---|
| `check` | `status` | 앱/권한/DB 상태 확인 |
| `auth` | | DB 복호화 검증 |
| `chats` | `rooms` | 채팅방 목록 (`--limit`) |
| `msg` | `messages` | 메시지 조회 (`--chat`, `--since`, `--limit`) |
| `find <KEYWORD>` | `search` | 메시지 검색 (`--exact`/`--regex`/`--rooms`/`--friends`/`--all`) |
| `query <SQL>` | | Raw SQL(읽기 전용) |
| `send [CHAT] [MSG]` | `say` | 메시지 전송 (`--me`, `--dry-run`, `-y`) |
| `sync` | `tail` | 새 메시지 증분 폴링 (`--follow`, `--interval`, `--chat`, `--since`, `--webhook`) |
| `inspect` | | 접근성 트리 덤프(디버깅, `--chat`, `--depth`) |
| `login` | | 크레덴셜 저장/확인/삭제 (`--email`, `--password`, `--status`, `--clear`) |

### 예시

```bash
# 상태 확인 / 복호화 검증
kakaocli check
kakaocli auth

# 채팅방 목록, 특정 방 최근 메시지
kakaocli chats --limit 20
kakaocli msg --chat "홍길동" --since 1h

# 검색
kakaocli find "회의" --all

# 나와의 채팅으로 전송 (미리보기 → 실제 전송)
kakaocli send --me "메모" --dry-run
kakaocli send --me "메모" -y

# 새 메시지 실시간 모니터링
kakaocli sync --follow --interval 2
kakaocli sync --follow --since 30m --webhook https://example.com/hook
```

> `--since`는 `1h`, `30m`, `7d`, `2026-09-07` 형식을 지원합니다.

---

## 보안 · 프라이버시

- **읽기 전용**: KakaoTalk DB는 `SQLITE_OPEN_READ_ONLY`로만 오픈. 원본 DB에 쓰지 않습니다.
- **로컬 전용**: 복호화·조회는 전부 로컬에서 수행됩니다. `sync --webhook`을 **명시적으로
  지정한 경우에만** 새 메시지가 외부로 전송됩니다.
- **크레덴셜**: 로그인 정보는 OS 키체인(macOS Keychain / Windows Credential Manager)에
  저장하며, 코드/로그에 시크릿을 남기지 않습니다.
- **전송 안전장치**: `send`는 전송 직전 대상 창 제목을 재검증하고, 동명이인/모호한
  방 이름은 거부합니다. 비대화형 환경에서는 `-y`가 없으면 전송하지 않습니다.

---

## 개발

```bash
cargo test         # 워크스페이스 전체 테스트
cargo build        # 디버그 빌드
```

설계 문서는 `../kakaotalk-cli-design`(DESIGN.md, docs/ADRs.md, docs/db-schema.md,
docs/flows.md, docs/windows-dek.md, docs/macos-ax.md, docs/roadmap.md)에 있습니다.

---

## 로드맵 / 상태

- ✅ macOS: DB 복호화 + chats/msg/find/query/send/inspect/sync
- ✅ Windows: DEK 스캔 + 복호화 + 10개 명령 전부 구현
- ⬚ CI(GitHub Actions, macOS+Windows 빌드), 릴리즈 바이너리 배포

자세한 진행 상황은 `../kakaotalk-cli-design/docs/roadmap.md` 참고.
