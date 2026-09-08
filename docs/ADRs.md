# Architecture Decision Records (ADRs)

## ADR-001: Rust over Node.js/Swift

**Status:** Accepted
**Context:** kakaocli는 Swift로 작성되어 macOS만 지원. Windows 지원과 단일 바이너리 배포 필요.
**Decision:** Rust 선택
- `windows-rs`로 Windows DEK 스캔을 네이티브 내장 가능 (Node.js는 C++ addon 불가피)
- `bundled-sqlcipher`로 SQLCipher 정적 링킹 (brew/node-gyp 불필요)
- 단일 바이너리 → brew/scoop/winget 배포 용이
- `objc2`로 macOS AX 제어 가능
**Consequences:** macOS AX 부분이 Swift보다 boilerplate 많음

---

## ADR-002: macOS 키 유도 — kakaocli 방식 계승

**Status:** Accepted  
**Context:** kakaocli는 blluv의 PBKDF2-SHA256 100k iterations 알고리즘 사용. 이미 검증됨.  
**Decision:** 동일한 알고리즘을 Rust로 이식  
- `ring` crate의 PBKDF2 사용 (CCKeyDerivationPBKDF 대체)  
- 동일한 hawawa 문자열 조합 방식 유지  
- cipher compatibility mode 3 → 4 fallback 유지  
**Consequences:** 기존 kakaocli 사용자의 DB와 100% 호환

---

## ADR-003: Windows DEK 스캔 — MoniKa 접근법 참고

**Status:** Proposed  
**Context:** Windows KakaoTalk의 .edb 파일은 SQLCipher이지만 키가 프로세스 메모리에만 존재.  
**Decision:** 프로세스 메모리 스캔 방식 채택  
- windows-rs의 ReadProcessMemory로 KakaoTalk.exe 힙 스캔  
- BCrypt AES-256 ECB로 page-1 oracle 검증  
- MoniKa의 접근법 참고하되 C++ 대신 Rust로 직접 구현  
**Consequences:** KakaoTalk.exe 실행 중이어야 함. 업데이트 시 DEK 위치 변경 가능

---

## ADR-004: 크레이트 분할 — core/db/platform/auth/cli

**Status:** Accepted  
**Context:** 플랫폼 특화 코드와 공통 로직의 분리가 필요.  
**Decision:** 5개 크레이트로 분할  
- core: 플랫폼 무관 (순수 계산 + 모델) → Linux에서도 테스트 가능  
- db: SQLCipher 읽기 → macOS/Windows 동일 바이너리  
- platform: cfg 분기 → 각 OS별 특화 코드만 컴파일  
- auth: keyring → OS 키체인 추상화  
- cli: clap → 순수 CLI  
**Consequences:** 의존성 그래프가 명확하고 테스트 가능한 단위로 분리

---

## ADR-005: cfg 조건부 컴파일 — 런타임 분기보다 컴파일타임 분기

**Status:** Accepted  
**Context:** macOS와 Windows의 백엔드가 완전히 다름.  
**Decision:** `#[cfg(target_os = "...")]`으로 컴파일타임 분기  
- 공통 trait 정의 → 각 OS가 impl  
- Linux에서는 컴파일 가능하나 런타임 panic (dev 전용)  
- 런타임 if-else 분기 없음 → dead code 없음  
**Consequences:** 크로스 컴파일 필요 (CI에서 macOS/Windows 각각 빌드)

---

## ADR-006: bundled-sqlcipher-vendored-openssl 사용

**Status:** Accepted  
**Context:** SQLCipher 연결 방식 결정. macOS는 Security.framework, Windows는 OpenSSL 필요.  
**Decision:** `bundled-sqlcipher-vendored-openssl` feature 사용  
- macOS: Security.framework 자동 감지 (별도 설정 불필요)  
- Windows: OpenSSL을 정적 링킹 (사용자 PC에 OpenSSL 설치 불필요)  
- brew나 vcpkg 없이 cargo build 하나로 완료  
**Consequences:** 첫 빌드 시간이 약간 길지만 (SQLCipher + OpenSSL 소스 컴파일), 배포가 단순해짐

---

## ADR-007: JSON 출력 기본값

**Status:** Accepted  
**Context:** 사람이 읽는 출력과 AI 에이전트가 읽는 출력을 모두 지원.  
**Decision:** `--json` 플래그가 없으면 사람용 pretty print, 있으면 JSON  
- 사람용: 컬러 + 정렬된 테이블  
- JSON: serde_json::to_string_pretty  
- 모든 명령어가 JSON 출력 가능  
**Consequences:** 약간의 코드 중복 (pretty vs json 분기) 있지만 AI 연동 대비 필수

---

## ADR-008: sync 명령어 — 폴링 방식

**Status:** Proposed  
**Context:** 실시간 새 메시지 모니터링 필요.  
**Decision:** DB 폴링 방식 (NDJSON 스트림)  
- 일정 간격(기본 2초)으로 DB에서 가장 최근 메시지 조회  
- 이전 max_log_id 이후의 새 메시지만 출력  
- `--follow`: 지속적 스트리밍, `--webhook`: HTTP POST  
- macOS: DB 읽기 (앱 실행 불필요)  
- Windows: DEK 캐시 + DB 읽기 (앱 필요)  
**Consequences:** 진정한 실시간은 아니나 (폴링 딜레이), 가장 안정적인 방식