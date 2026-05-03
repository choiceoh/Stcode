# 백로그 — v0.1.0 이후 안정화 로드맵

현재(v0.1.0)는 사내 첫 배포 골든 패스가 끝까지 동작하는 상태다. 이제 우선순위는
큰 기능 확장보다 **검증 루프, 자동 작업 안전망, 긴 작업 제어, 배포 유지보수**다.

## 제품 방향

Stcode는 코드를 1도 모르는 작업자가 10개 안팎의 에이전트를 병렬 자동으로 수시간
연속으로 굴리는 멀티 에이전트 바이브코딩 콘솔이다. IDE가 아니며, 코드를 읽고 판단하는
개발자용 화면을 만들지 않는다. 백로그 우선순위는 코드 독해 편의가 아니라 다중 세션
운영성, 상태 표시, 중단/되돌리기, 친화적 결과 요약을 기준으로 정한다. 변경 확인도
diff/patch 검토가 아니라 결과 요약과 turn 단위 되돌리기로 해결한다.
병렬 작업 특성상 GitHub를 많이 쓰지만, 사용자는 Git을 몰라도 된다. 세션 시작/종료에
맞춰 작업용 워크트리와 브랜치를 시스템이 만들고 정리해야 한다.
메인 에이전트와 서브 에이전트는 시스템 레벨에서 서로 다른 모델을 지정할 수 있어야 하며,
사용자에게 매번 모델 선택을 요구하지 않고 역할별 기본값으로 자동 라우팅해야 한다.
GUI 완성도는 Codex Desktop 수준을 기준으로 삼는다. 즉 어두운 개발자 도구가 아니라,
왼쪽 작업 내비게이션, 넓고 읽기 쉬운 대화 캔버스, 하단 composer, 조용한 상태/권한
표시가 있는 데스크탑 작업 콘솔이어야 한다.

## 완료된 기반

- 자동 모드: `ApprovalPolicy::Never` + `SandboxMode::DangerFullAccess`.
- inbound command/file approval request 자동 `AcceptForSession`.
- 병렬 멀티 세션: 세션별 tokio task + 사이드바 라우팅.
- Tool Cards: `commandExecution`, `fileChange`, `mcpToolCall`, `webSearch`.
- Reasoning 패널과 본 답변 bubble 분리.
- 기본 Markdown 표시: heading, list, plain preformatted block.
- turn 단위 git 안전망: 자동 init, HEAD snapshot, 자동 commit, 되돌리기.
- friendly 에러 메시지 매핑.
- provider/model 설정 저장.
- macOS `.app` 번들 + ad-hoc codesign.
- `livetest` / `livetest_bridge` 기반 헤드리스 검증.

## P0 — 자동 워크트리/브랜치 관리

- [x] 세션 worktree/branch 생성과 안전 정리 기반 API 추가.
  - 원본 repo HEAD에서 `stcode/<session>` branch와 격리 worktree 생성.
  - 변경 없는 세션은 worktree와 branch를 함께 정리.
  - 미커밋 변경이 있거나 세션 commit이 남은 branch는 삭제하지 않음.
- [x] 세션 시작 시 프로젝트 원본 폴더를 건드리지 않고 작업용 워크트리를 자동 생성.
- [x] 세션마다 추적 가능한 작업 브랜치를 자동 생성하고 세션 id와 연결.
- [x] 세션 종료 시 완료/폐기/중단 상태에 맞춰 사용하지 않는 워크트리 자동 정리.
- [x] PR merge 또는 작업 폐기 후 더 이상 필요 없는 로컬 브랜치 자동 정리.
  - `stcode/*` branch가 base에 병합됐거나 upstream 삭제로 종료 확인되면 정리.
  - 다른 작업공간에서 checkout 중이거나 병합/폐기 확인 전이면 보관.
- [x] 사용자는 Git 명령, 브랜치 이름, 워크트리 경로를 몰라도 되게 하고, 화면에는 "작업공간 준비됨", "정리됨", "되돌릴 수 있음"처럼 친화적으로 표시.

## P1 — 시스템 모델 라우팅

- [x] 메인 에이전트 기본 모델과 서브 에이전트 기본 모델을 별도로 저장.
- [x] 세션 시작 시 시스템 설정에 따라 조율용 메인 모델을 자동 주입.
- [x] 서브 에이전트 spawn 시 시스템 설정의 작업 모델을 자동 주입.
  - 세션 시작 때 작업 모델용 기본 agent role 파일을 자동 생성하고 Codex 설정 override로 연결.
  - 사용자가 모델을 고르지 않아도 기본 서브 에이전트가 작업 모델을 쓰게 한다.
- [ ] 작업 유형별 override를 허용하되, 사용자가 매번 모델을 고르게 하지 않는다.
- [x] 화면에는 모델명을 기술 설정처럼 과하게 노출하지 않고, 필요할 때만 "조율 모델", "작업 모델" 정도로 확인 가능하게 표시.

## P0 — 검증 루프와 안전망

- [x] `cargo test --workspace`가 통과하도록 `livetest` 이벤트 매칭 갱신.
- [x] `stcode-vibe::git_safety` temp repo 단위 테스트 추가.
  - non-git 폴더 자동 init.
  - 변경 없음이면 commit skip.
  - untracked/modified 파일 자동 commit.
  - 직전 HEAD로 hard reset 되돌리기.
  - 첫 turn 이전 되돌리기 실패 메시지.
- [x] `livetest_bridge`를 기본 smoke command로 문서화.
- [x] `scripts/build-app.sh --debug --no-codesign` smoke를 릴리즈 전 체크리스트에 추가.

## P1 — 긴 작업 제어

- [x] 진행 중 turn 중단 버튼을 GUI에 노출.
  - bridge `UiCommand::InterruptTurn` → `ThreadSession::interrupt()` 연결.
  - 헤더에 "중단" 버튼 표시, 클릭 후 "중단 요청됨" 상태로 전환.
  - turn 완료 이벤트가 오면 `turn_in_flight=false`로 정리.
- [x] 세션 close 시 진행 중 task 종료/interrupt 동작을 명시적으로 검증.
  - `CloseSession`/전체 shutdown이 세션 task에 interrupt-aware shutdown을 보냄.
  - 세션 task는 shutdown 직전 `turn/interrupt`를 먼저 시도한 뒤 codex process를 정리.
  - 내부 단위 테스트로 interrupt flag 전달을 잠금.
- [x] 무한 reasoning 탐지용 상태 표시 개선.
  - reasoning delta는 오는데 답변 delta가 없으면 `생각 중`으로 표시.
  - reasoning이 길어지면 `생각이 길어지는 중`으로 바뀌어 중단 판단을 돕는다.
  - 답변 delta가 시작되면 `답변 작성 중`으로 전환.

## P1 — 실사용 UX 안정화

- [x] Codex Desktop 스타일의 1차 shell polish.
  - 라이트 테마, 넓은 사이드바, 상단 세션 헤더, 중앙 대화 캔버스, 하단 composer card 적용.
  - 자동 권한/모델 상태를 composer와 헤더에 조용한 chip 형태로 표시.
- [x] 최근 프로젝트 목록.
  - 프로젝트를 열면 설정에 최근 경로를 저장하고, 첫 화면/사이드바에서 바로 새 작업으로 재개.
  - 첫 화면도 좌측 내비게이션이 유지되는 데스크탑 앱 shell로 정리.
- [ ] 최근 세션 목록.
- [ ] 검색/플러그인/자동화 내비게이션 실제 기능 연결.
- [ ] 세션별 요약/결과/작업 트리 패널을 코드 보기 없이 제공.

- [x] 한글 IME edge case 수동 재현 체크리스트 작성.
  - `docs/ime-checklist.md`에 조합, 편집, selection, clipboard, Enter 전송 경계를 정리.
- [x] multi-line selection 드래그 경계 테스트 보강.
  - `SelectableText` drag range/reversed 전환 단위 테스트 추가.
  - `docs/selection-checklist.md`에 실제 multi-line drag/복사 수동 QA 정리.
- [x] inline Markdown marker 정리(bold, link, monospace text).
  - `SelectableText`에 inline span 기반 TextRun 스타일링 추가.
  - backtick, `**bold**`, `[label](url)` markers를 turn 완료 후 표시 텍스트/스타일로 변환.
  - 선택/복사는 marker가 제거된 표시 텍스트 기준으로 유지.

## P2 — 설정과 배포 유지보수

- [ ] Reasoning effort 설정화.
- [ ] Sandbox/approval policy는 고급 설정으로만 노출 여부 결정.
- [ ] OpenAI provider 사용 시 API 키 입력/저장 흐름.
- [ ] macOS Keychain 저장 검토.
- [ ] codex fork 추적 정책 문서화.
- [ ] fork patch를 별도 repo/submodule로 관리할지 결정.
- [ ] release zip 생성 자동화.

## 구현하지 않을 것

- 파일 트리, diff/patch viewer, 코드 에디터 패널.
- 코드블록 syntax highlight.
- AST/semantic code navigation, LSP.
- 터미널 패널, 디버거.

## P3 — 나중에 미룰 것

- Windows/Linux 지원.
- 다중 사용자, 플러그인, 마켓플레이스.

## 기본 검증 순서

```bash
cargo test --workspace
cargo run -p stcode-codex --example livetest_bridge -- "한 줄로 답해"
bash scripts/build-app.sh --debug --no-codesign
```

로컬 vLLM/codex fork가 필요한 검증은 환경이 준비된 머신에서만 수행한다. 순수 단위 테스트는
항상 먼저 통과시킨다.
