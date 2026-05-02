# 백로그 — Claude Code Desktop 수준까지의 거리

현재(M1.x) 상태에서 production 데스크톱 코딩 에이전트(Claude Code Desktop / Codex Desktop) 수준까지 남은 작업.

## 즉시 (P0) — 응답 흐름 fix

- [ ] **`item/agentMessage/delta` 0건 문제 진단**
  - codex 자체의 SSE → 노티 변환 흐름 추적
  - vLLM이 `output_text.delta` 보내지만 codex가 message item으로 인식 못 함 (현재 가설)
  - reasoning vs message item 구분이 codex에서 어떻게 되는지 확인
  - 임시: `STCODE_REVEAL_REASONING=1` ENV로 reasoning을 본문 노출 (final answer가 그 끝에 포함되는 케이스)

## M2 — 도구 가시성 + 승인 (P1)

- [ ] `item/commandExecution/outputDelta` 처리 — UI에 "⚙ npm install 실행 중..." 박스
- [ ] `item/fileChange/*` 처리 — UI에 "📄 파일 수정됨" 한 줄 알림
- [ ] `item/commandExecution/requestApproval` server→client request 처리
  - 모달 다이얼로그 (한 번만/세션 내내/거절)
  - server에 응답 RPC
- [ ] `item/fileChange/requestApproval` 동일 처리
- [ ] `item/plan/*` — 체크리스트 표시

## M3 — 바이브 안전 레이어 (P1)

- [ ] turn 시작 시 git stash 체크포인트
- [ ] turn 완료 시 자동 git commit
- [ ] "되돌리기" 버튼 — 마지막 commit reset
- [ ] codex 에러 → 한국어 친화 메시지 매핑 테이블
  - `Unauthorized` / `RateLimitExceeded` / `BadRequest` 등
- [ ] API 키 macOS Keychain 저장 (`security-framework`)

## UX 다듬기 (P1)

- [ ] **인풋 multi-line 진짜 wrap** — 현재 단일 라인 base, 화면 밖 짤림. shape_text + WrappedLine 진짜 적용
- [ ] **메시지 본문 multi-line wrap** — SelectableText도 같은 작업
- [ ] **자동 scroll to bottom** — 메시지 추가 시 (이미 적용됨, 동작 검증)
- [ ] **drag selection 다듬기** — multi-line selection 한 번에 지나가는 케이스
- [ ] **friendly empty state** — Welcome 화면 풍부 (최근 프로젝트, 설명 등)
- [ ] **메시지 Markdown 렌더** — codex 응답이 마크다운인 경우 (현재는 plain text)
- [ ] **코드 블록 syntax highlight** — 응답 중 ```rust ... ``` 등

## 설정 화면 (P2)

- [ ] Provider 선택 (local-vllm / openai)
- [ ] Model 선택 (qwen3.6-35b-a3b 등 후보)
- [ ] Reasoning effort (minimal/low/medium/high)
- [ ] Sandbox mode (read-only / workspace-write / danger-full-access)
- [ ] Approval policy (untrusted / onFailure / onRequest / never)
- [ ] reveal reasoning toggle (현재 ENV 기반)

## 멀티 세션 (P2)

- [ ] 좌측 사이드바에 세션 리스트 (cmd+shift+P "최근 프로젝트")
- [ ] 세션 별 thread state 보존 (현재는 한 윈도우 한 thread)
- [ ] 세션 검색

## 인증 (P2)

- [ ] codex `account/read` 호출 — 인증 상태 확인
- [ ] `account/login/start` — ChatGPT OAuth 흐름 (브라우저 열기)
- [ ] API 키 직접 입력 폼

## 배포 (P3)

- [ ] macOS `.app` 번들 (cargo-bundle)
- [ ] ad-hoc 코드사이닝
- [ ] codex fork 자동 빌드/번들 (사용자가 수동 clone+build 안 해도 되게)
- [ ] release 자동화 (GitHub Actions)

## codex fork 유지보수 (P3)

- [ ] codex upstream 추적 정책 (어느 시점 pin? 주기적 rebase?)
- [ ] fork patch를 git submodule 또는 별도 fork repo로 (`choiceoh/codex`)
- [ ] CI: fork patch unit tests

## graphify 회귀 (P3)

- [ ] `graphify update .` 자동화 — code 변경 후 자동
- [ ] `.notify()` 같은 노드 dedup artifact 우회 (graphify upstream issue 또는 우리 측 ID 보강)

---

**Claude Code Desktop 수준까지 추정 기간**: 1~2주 풀타임 작업 (P0~P1 끝내고 P2 일부). M2 + M3 + UX 다듬기가 핵심.
