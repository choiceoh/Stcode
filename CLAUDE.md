# Stcode — 다음 세션 가이드

이 문서는 다음 세션의 Claude(또는 본인)가 컨텍스트를 빨리 잡기 위한 메모.
README가 "어떻게 빌드/실행하는지"라면 여기는 "왜 이렇게 만들었는지 + 절대 헷갈리면 안 되는 의도".

---

## 1. 프로젝트 정체성 한 문장

**코드를 1도 모르는 "바이브 코더"용 macOS 데스크톱 자동 작업 에이전트.**
사용자가 자연어로 시키면 알아서 만들고, 알아서 실행하고, 잘못되면 한 번에 되돌릴 수 있게 한다.

타깃 사용자가 IDE를 안 쓰므로 IDE 흉내(파일트리, diff뷰어, 에디터 패널)를 일부러 안 만든다.
Claude Code Desktop / Codex Desktop / Zed Agent 정도의 시각적 완성도를 지향하되, 디테일은 최소화.

### 1-1. 핵심 워크플로우 — **병렬 멀티 에이전트 바이브코딩이 기본**

사용자가 명시한 "기본 작업 패턴": 한 사람이 동시에 여러 에이전트를 돌려놓고
바이브코딩한다. 즉, 사이드바에 세션 여러 개를 띄워두고 각각 다른 프로젝트/다른
작업을 동시에 진행. 한 세션이 reasoning 중일 때 다른 세션을 보거나 새 prompt를
줄 수 있어야 한다.

이는 단순한 UI 폴리시가 아니라 **모든 설계 결정을 다시 평가하게 하는 1순위 제약**:

- `Bridge::handler_loop`은 **세션 1개 가정 → 세션 N개**로 진화해야 함 (명령/이벤트에 session_id 라우팅)
- `MainView` 상태도 단일 `Screen::Chat` → 사이드바 + 활성 세션 + 백그라운드 세션 dict 구조
- 백그라운드 세션의 turn 진행은 GUI에 활성 세션이 아니어도 계속 — 사이드바 항목에 "⏳ 작업 중" 표식
- git auto-commit/되돌리기는 "세션이 곧 프로젝트"가 아닐 수 있다는 가능성 고려 (같은 폴더에 여러 세션? 보통은 1세션 1폴더가 자연스러움)
- 자동 에이전트 정책은 사용자 개입 없이 굴러야 하므로 **이 워크플로우를 가능하게 하는 전제조건**이다 — 승인 모달이 떠서 사용자를 잡아두면 멀티세션 의미 없음. 이 둘은 한 묶음.

v1엔 일단 단일 세션 UI로 시작했지만, 사이드바를 도입하는 시점부터 세션 list 구조로 짜고 multi-session 인프라(handler_loop·UiCommand·UiEvent의 session_id 라우팅)를 단계적으로 도입한다.

---

## 2. 시간순으로 굳어진 핵심 원칙 (헷갈리지 마라)

| 굳어진 결정 | 의도 |
|---|---|
| **자동 모드. 승인 모달 안 띄움.** | "전부 권한 건너뛰기에 완전 자동 작업 에이전트가 목적" — 사용자 명시 요구. `ApprovalPolicy::Never` + `SandboxMode::DangerFullAccess`. inbound approval request가 와도 bridge가 즉시 `AcceptForSession`으로 자동 응답. (모달 UI 코드는 inert dead-allow로 남겨둠 — 미래 옵션 대비) |
| **diff·raw command·파일 경로 등 기술 디테일 노출 금지** | 바이브 코더는 어차피 못 읽는다. Tool Card summary는 "완료"/"적용됨" 같이 친화적으로. 승인 모달이 살아있던 시기에도 친화적 한국어 제목 강조 + raw는 작은 보조. |
| **안전망은 사후 git 되돌리기로** | 자동 모드의 위험성을 사후로 회수. 폴더에 .git 없으면 자동 init. turn 단위로 자동 commit (`stcode: <prompt 첫 줄>`). 채팅 헤더에 "↶ 되돌리기" 칩. 사용자가 git을 의식조차 못하게. |
| **로컬 vLLM + codex fork** | OpenAI 본가 트래픽 영향 없게 ENV `STCODE_VLLM_COMPAT=1`로 게이팅. `~/Documents/GitHub/codex-fork/codex-rs/target/{debug,release}/codex` 자동 탐지. |
| **Reasoning은 별도 회색 패널 + final answer는 bubble** | qwen 같은 reasoning model은 사고가 답변보다 길다. 분리해서 사고는 접고 결론만 강조. |
| **GPUI는 main HEAD git rev 핀 (crates.io 0.2.x 안 씀)** | pre-1.0이라 어차피 잦은 breaking. 분기 단위로만 rev 업데이트. |
| **Zed 본체 코드 복사 절대 금지** | gpui crate는 Apache-2.0이지만 editor/ui/workspace는 GPL/AGPL. 한 줄도 가져오면 안 됨. UI 컴포넌트는 우리가 직접 짠다 (chat_input.rs / selectable_text.rs). |

---

## 3. 아키텍처 단위

```
GPUI App (stcode-app)
  ├─ Bridge (별 OS thread tokio runtime)
  │    cmd_tx: UiCommand  →  evt_rx: UiEvent
  │
  ├─ stcode-codex
  │    ├─ rpc.rs        raw JSON-RPC 2.0 (stdin/stdout)
  │    ├─ protocol.rs   serde 타입 (codex-app-server-protocol 자체정의 minimal)
  │    ├─ session.rs    typed thread/turn 추상
  │    └─ bridge.rs     handler_loop (tokio::select! cmd vs codex events)
  │
  └─ stcode-vibe
       └─ git_safety.rs  ensure_repo / current_head / auto_commit_turn / revert_to
```

`handler_loop`은 `tokio::select!`로 cmd_rx와 session.next_event()를 동시 polling.
turn 진행 중에도 사용자 명령(되돌리기 등)이 즉시 처리됨.

---

## 4. 개발 흐름 가이드

### 4-1. 빌드 환경 함정

`rustup`이 brew keg-only라 cargo가 PATH에 없을 수 있다. 안전한 호출:

```bash
env RUSTUP_HOME=/Users/ost/.rustup CARGO_HOME=/Users/ost/.cargo \
    PATH=/Users/ost/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH \
    /Users/ost/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo build ...
```

shell alias로 묶어두면 편함.

### 4-2. 라이브 테스트 (헤드리스)

GUI 안 띄우고 wire 검증:

- `cargo run -p stcode-codex --example livetest "프롬프트"` — 저수준 ThreadSession
- `cargo run -p stcode-codex --example livetest_bridge "프롬프트"` — Bridge layer (GUI가 쓰는 흐름과 동일)

`livetest_bridge`는 ApprovalRequested 자동 Decline (테스트용), TurnCommitted/Reverted 출력. 프로덕션 로직 변경 후엔 항상 이걸로 먼저 검증한 뒤 GUI를 띄운다.

### 4-3. PR 패턴

작업 묶음 단위로 별 브랜치 → 1 PR → squash merge. 사용자가 매번 "PR 후 머지"라고 명시함. base는 `main`. (gh CLI가 가끔 직전 PR base를 재사용하는 버그 있음 — `gh pr edit <n> --base main`로 교정.)

### 4-4. 사용자 톤 매칭

- "빨리" — 큰 묶음으로, 재확인 줄이고 진행
- "정독 후 진단 print 추가는 마지막 수단" — 추측 디버깅 금지. 코드 먼저 읽기
- "라이브테스트로 검증" — GUI 띄우는 건 부담. 헤드리스 우선
- 한국어로 응답. 코드 주석도 한국어 OK

---

## 5. 알려진 함정

| 함정 | 대응 |
|---|---|
| GPUI listener 안에서 sync `rfd::pick_folder` 호출 시 panic (RefCell double borrow) | `rfd::AsyncFileDialog` + `cx.spawn` 으로 분리 (open_folder 패턴) |
| `shape_line` 이 `\n` 만나면 panic | SelectableText는 multi-line `shape_text(wrap_width)` 사용 |
| 한글 IME char boundary panic | `offset_from_utf16` / `range_from_utf16` 변환 (chat_input.rs) |
| codex `app-server` 스키마 — `ApprovalPolicy` wire는 **kebab-case** (`on-request`), 다른 enum은 camelCase | `ApprovalPolicy`만 `#[serde(rename_all = "kebab-case")]` |
| `CommandExecutionApprovalDecision` 직렬화 wire는 camelCase (`accept`/`acceptForSession`/`decline`/`cancel`) | bridge.rs `as_wire()` 정확히 이 값들 |
| codex fork에서 reasoning model이 message 안 만들고 무한 reasoning | `STCODE_VLLM_COMPAT=1` + `model_reasoning_effort=minimal` (start_session에서 자동 set) |
| codex provider WebSocket 시도 → vLLM은 ws 미지원 | `model_providers.local-vllm.supports_websockets=false` (start_session에서 자동 set) |
| approval params 스키마는 **flat** (item wrapper 없음) — `command`, `cwd`, `reason` 직접 | approval_text 함수가 flat으로 파싱 |
| `tracing::warn!` 이 default filter에 막혀 안 보일 때 있음 | 절박할 땐 `eprintln!` |
| `git2`는 user.name/email 없으면 commit 실패 | `signature_for`에서 "Stcode <stcode@local>" fallback |

---

## 6. 백로그 (`docs/backlog.md`도 참고)

P1 — 가치 큰 다음 작업 후보:
- **Markdown 렌더 (코드블록·리스트)** — agent 답변에 ``` 코드블록 자주 나옴. ChatItem::Message가 단일 SelectableText라서 segments(text/code) 리스트로 리팩터 필요. mono font 코드 블록은 별도 component.
- **사이드바 + 세션 리스트** — 시각적 완성도 한 단계 ↑. 다중 세션은 그 다음 단계.
- **하단 chips (model / permission)** — Zed/Codex Desktop 시각 참고.
- **친화적 에러 메시지 매핑** (`stcode-vibe::friendly`) — codex rate limit/sandbox/network → 한국어 + 액션 버튼.

P3 — 미루는 것:
- macOS Keychain (API 키) — 로컬 vLLM 우선이라 v1엔 dummy로 충분
- 설정 화면 — provider/model 변경. 지금은 hardcoded.
- `.app` 번들 + 사내 배포

---

## 7. 의식적으로 안 만드는 것 (v1)

- 파일 트리, diff 뷰어, 코드 에디터 패널 — IDE 아님
- LSP, 터미널 패널 — 안 함
- Windows / Linux — macOS only
- 다중 사용자 / 플러그인 / 마켓플레이스 — 사내 도구 범위 밖
- codex 바이너리 자동 번들/업데이트 — 사용자가 brew/fork로 관리
- 승인 다이얼로그 정책 — 자동 모드가 목적 (인프라는 남기되 비활성)
