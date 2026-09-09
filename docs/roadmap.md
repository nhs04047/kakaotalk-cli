# 로드맵

## Phase 1: macOS 읽기 MVP (예상: 1주)

### 작업 목록

- [ ] Cargo workspace 설정 (5개 크레이트)
- [ ] `kakaocli-core`: Chat/Message 모델, KDF, DB 경로
- [ ] `kakaocli-db`: rusqlite + bundled-sqlcipher 연결
- [ ] `kakaocli-cli`: auth, status, chats, messages, search, query
- [ ] macOS 실제 테스트: DB 복호화 + 채팅 읽기 검증

### 검증 기준
```
kakaocli auth       → "Database opened successfully! Tables found: N"
kakaocli chats      → 채팅방 목록 출력
kakaocli messages   → 특정 채팅 메시지 출력
kakaocli search     → 검색 결과 출력
kakaocli query      → raw SQL 결과 출력
```

## Phase 2: macOS 전송 (예상: 3-4일)

- [ ] `kakaocli-platform/darwin.rs`: objc2 AXUIElement 래퍼
- [ ] `kakaocli-auth`: keyring macOS Keychain
- [ ] CLI: send, login, inspect
- [ ] macOS 실제 테스트: 전송 성공

### 검증 기준
```
kakaocli send --me _ "hello"  → self-chat에 메시지 도착
kakaocli send "이름" "안녕!"  → 실제 채팅에 전송
kakaocli login --status       → loggedIn
kakaocli inspect              → AX 트리 덤프
```

## Phase 3: Windows MVP (예상: 1-2주)

- [ ] `kakaocli-platform/windows.rs`: DEK 스캐너
- [ ] `kakaocli-db`: DEK → DB 연결
- [ ] CLI: Windows에서 auth/status 동작
- [ ] Windows UIAutomation 전송
- [ ] Windows 실제 테스트: 읽기/전송

### 검증 기준
```
Windows에서 kakaocli auth    → DEK 발견, DB 오픈 성공
Windows에서 kakaocli chats   → 채팅방 목록
Windows에서 kakaocli send    → 전송 성공
```

## Phase 4: 안정화 + sync (예상: 1주)

- [~] sync: 증분 폴링 스트림 구현 완료(검증 대기) — 사람친화 기본 + --json NDJSON, --chat 필터, --webhook, --since 백필, ~/.kakaocli/sync.json 체크포인트. 설계: docs/superpowers/specs/2026-09-09-sync-design.md
- [ ] 에러 처리 강화
- [ ] 플랫폼별 crash report 처리
- [ ] 문서화 (README, AGENTS.md)
- [ ] GitHub Actions CI 설정 (macOS + Windows 빌드)

## Phase 5: 배포 (예상: 2-3일)

- [ ] GitHub Releases + 바이너리 업로드
- [ ] Homebrew formula (macOS)
- [ ] Scoop manifest (Windows)
- [ ] 릴리즈 노트