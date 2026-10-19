# Windows DEK 스캐너 설계 (DRAFT — 미승인)

- 작성일: 2026-09-09
- 상태: **초안 (사용자 브레인스토밍/승인 대기)** — 아래 "열린 질문"이 확정돼야 구현 진입
- 범위: `kakaocli-platform/src/windows.rs`의 `resolve_db_key` / `check_status` 실구현
- 레퍼런스: `docs/windows-dek.md`, MoniKa(https://github.com/maxswjeon/MoniKa)
- 선행: 이 기능이 완료돼야 Windows에서 auth/chats/msg/find/query/sync가 열림

> ⚠️ 이 문서는 자율 진행 중 작성된 **초안**이다. 프로세스 메모리 레이아웃·시그니처는
> 실제 KakaoTalk.exe를 디버거로 분석해야 확정되며, 여기 수치는 레퍼런스 기반 가설이다.
> "열린 질문" 절의 결정 없이는 구현을 시작하지 않는다.

## 1. 문제

- macOS는 plist + PBKDF2로 DB 키를 계산할 수 있다(공개 알고리즘).
- Windows는 공개 키유도 알고리즘이 더 이상 동작하지 않는다.
- 따라서 실행 중인 `KakaoTalk.exe` **프로세스 메모리에서 SQLCipher DEK(원시 키)를
  직접 포착**해 검증해야 한다.
- 함의: Windows는 **읽기 명령(auth/chats/…)에도 KakaoTalk.exe 실행이 필요**하다
  (DEK가 메모리에만 있으므로). macOS와 다른 제약.

## 2. 파이프라인

```
find_kakao_pid()            KakaoTalk.exe PID (Toolhelp32Snapshot)
   → open_process(VM_READ)  핸들 (PROCESS_QUERY_INFORMATION | PROCESS_VM_READ)
   → enumerate_regions()    VirtualQueryEx로 커밋된 읽기가능 영역 열거
   → scan_candidates()      영역을 읽어 DEK 후보(32바이트) 수집·필터
   → verify_dek(candidate)  .edb page-1 oracle로 검증 (AES-256)
   → cache_dek()            검증된 DEK를 로컬 캐시에 저장
   → DbKey { key_hex, db_path }
```

### 2.1 PID 탐색
`CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS)` → `Process32FirstW/NextW` 순회,
`szExeFile == "KakaoTalk.exe"`. 여러 개면 첫 번째(또는 가장 큰 워킹셋). 64비트 전용.

### 2.2 메모리 영역 열거
`VirtualQueryEx`로 `MEMORY_BASIC_INFORMATION`을 훑어 다음만 스캔 대상:
- `State == MEM_COMMIT`
- `Protect ∈ { PAGE_READWRITE, PAGE_WRITECOPY }` (힙/데이터; 실행영역 제외)
- `Type == MEM_PRIVATE` (이미지/매핑 파일 제외로 노이즈 감소)

각 영역을 `ReadProcessMemory`로 청크 단위(예: 1MB)로 읽는다.

### 2.3 후보 수집·필터 (⚠️ 시그니처 미확정)
레퍼런스는 "0x88 바이트 시그니처"와 "32바이트 연속" 후보를 언급. 확정 필요:
- 후보 = 32바이트(AES-256 키) 윈도우. 무작정 전수 스캔은 비용이 큼 → 시그니처/정렬
  힌트로 좁힌다. **실제 시그니처는 디버거 분석으로 확정**(열린 질문 Q1).
- 엔트로피 필터: 키는 고엔트로피 → 반복/저엔트로피 32바이트 블록 제외로 후보 축소.

### 2.4 검증 오라클 (page-1) — **기술적으로 확정 가능한 핵심**
SQLCipher DB 첫 페이지 구조(compat별 상이). KakaoTalk은 macOS에서 v3(64000/SHA1)로
확인됨 → Windows도 동일 계열일 가능성이 높으나 **확정 필요**(Q2).

검증 절차(원시 DEK를 이미 가진 경우, passphrase-KDF 없이):
1. `.edb` 첫 4096바이트 읽기. 선두 16바이트 = salt.
2. 후보 DEK를 원시 키로 사용해 page-1 본문을 AES-256-CBC 복호화.
   - SQLCipher는 페이지별 IV(페이지 끝 16바이트)를 사용. compat별 HMAC 유무 상이.
3. 복호화 결과가 SQLite 헤더 매직 `"SQLite format 3\x00"`으로 시작하면 성공.
   - page-1은 salt 16바이트 뒤부터가 암호문이라, 복호화 후 매직은 오프셋 16이 아니라
     페이지 선두(무암호 salt 제외)에서 나타남 — SQLCipher 규칙에 맞춰 정확히 구현.
4. 더 견고한 대안: 후보로 `PRAGMA key`(raw key 형식 `x'<hex>'`) + `cipher_compatibility`
   설정 후 `SELECT count(*) FROM sqlite_master` 성공 여부. **기존 kakaocli-db 재사용
   가능** → 오라클을 자체 AES 구현 대신 rusqlite로 태울 수 있음(권장, Q3).

> 권장: 2.4의 자체 AES 오라클 대신 **rusqlite raw-key 오픈 오라클**을 1차로 쓴다.
> 이미 검증된 SQLCipher 스택을 재사용하므로 암호 구현 버그 위험이 없다. 원시 DEK를
> `PRAGMA key = "x'<64hex>'"` 형식으로 넘긴다(참고: macOS 경로는 passphrase-hex 방식과
> 다름 — Windows는 raw key). 이 형식 차이는 kakaocli-db open()에 raw-key 분기 추가 필요.

### 2.5 캐싱
- 위치: `%LOCALAPPDATA%\kakaocli\dek`(또는 기존 `config_dir()` 규약과 통일 — Q4).
- KakaoTalk 재시작 시 DEK가 바뀔 수 있음 → **캐시된 DEK로 오픈 실패하면 자동 재스캔**.
- 보안: DEK는 DB 평문 접근 키. 캐시 파일 권한/암호화 여부 결정 필요(Q5).
  - 옵션: DPAPI(CryptProtectData)로 사용자 바인딩 암호화 저장.

## 3. 통합 지점

- `WindowsBackend::resolve_db_key`: 캐시 로드 → 실패 시 스캔 → 검증 → 캐시 → `DbKey`.
  - `db_path`는 `db_path::windows_edb_files().first()` (이미 존재).
  - `key_hex`: Windows는 원시 32바이트 DEK → 64-hex. **db open의 키 형식 분기 필요**
    (macOS 256-hex passphrase vs Windows 64-hex raw key).
- `WindowsBackend::check_status`: KakaoTalk.exe 실행 여부 + `.edb` 존재로 상태 판정
  (현재 placeholder를 실제 프로세스 체크로 교체).

## 4. 테스트 전략 (컴파일/실행 가능한 부분)

- **오라클 단위테스트**: 알려진 `.edb` 픽스처 + 알려진 DEK로 `verify_dek` 성공/실패.
  (픽스처가 없으면 SQLCipher로 raw-key DB를 만들어 생성)
- **후보 필터 단위테스트**: 저엔트로피 블록 배제, 32바이트 정렬 등 순수 로직.
- **프로세스 메모리 스캔**: 통합 테스트 성격(실 KakaoTalk 필요) → 수동/실기 검증.
- rusqlite raw-key 오라클을 쓰면 오라클 테스트는 크로스플랫폼으로 돌릴 수 있음.

## 5. 리스크
- 프로세스 메모리 스캔은 KakaoTalk 버전마다 레이아웃/시그니처가 달라질 수 있음.
- 안티치트/보안 솔루션이 `PROCESS_VM_READ`를 차단할 수 있음(권한/관리자 이슈).
- 자체 AES 오라클은 SQLCipher IV/HMAC 규칙 오구현 위험 → rusqlite 오라클 권장.

## 6. 열린 질문 (사용자 결정 필요 — 이게 정해져야 구현 시작)

1. **Q1 DEK 시그니처/스캔 전략**: 디버거로 실제 시그니처를 먼저 분석할지, 아니면
   MoniKa 방식(0x88 시그니처)을 그대로 이식해 시작할지?
2. **Q2 SQLCipher compat**: Windows `.edb`가 v3(64000/SHA1)인지 실측 확인 필요.
   누가/언제 실 DB로 확인?
3. **Q3 오라클 방식**: rusqlite raw-key 오픈 오라클(권장, 안전) vs 자체 BCrypt/AES 구현?
4. **Q4 캐시 위치/포맷**: `config_dir()` 통일 vs `%LOCALAPPDATA%`, 단일 파일 포맷?
5. **Q5 DEK 캐시 보안**: 평문 저장 vs DPAPI 암호화? (DEK는 DB 평문 열쇠라 민감)
6. **Q6 MoniKa 의존**: 자체 Rust 스캐너로 완결 vs MoniKa를 사전 실행해 캐시만 읽기?
7. **Q7 키 형식 분기**: db open()에 Windows raw-key(`x'<hex>'`) 분기를 추가하는 설계 확정.

## 7. 비범위
- UIAutomation 전송(별도: Windows UIA send 설계)
- 관리자 권한 상승 처리
- 안티치트 우회(하지 않음)
