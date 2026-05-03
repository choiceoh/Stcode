//! 헤드리스 GUI bridge 라이브 테스트 — 멀티 세션 버전.
//!
//! `cargo run -p stcode-codex --example livetest_bridge -- "프롬프트"`
//!   → 단일 세션 + 단일 prompt + 자동 Decline (승인 round-trip 검증)
//!
//! `STCODE_LIVETEST_PARALLEL=1 cargo run --example livetest_bridge`
//!   → 두 세션 동시 prompt — 진짜 병렬 polling 검증.
//!     prompt는 ENV `STCODE_PROMPT_A` / `STCODE_PROMPT_B` 또는 default.

use std::env;
use std::path::PathBuf;
use std::time::Duration;

use stcode_codex::bridge::{ApprovalDecision, Bridge, SessionId, UiCommand, UiEvent};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,stcode_codex=info".into()),
        )
        .init();

    let parallel = env::var_os("STCODE_LIVETEST_PARALLEL").is_some();
    if parallel {
        run_parallel().await
    } else {
        run_single().await
    }
}

async fn run_single() -> anyhow::Result<()> {
    let user_text = env::args()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    let user_text = if user_text.is_empty() {
        "한 줄로만 답해. 너 누구야?".to_string()
    } else {
        user_text
    };

    let cwd: PathBuf = env::current_dir()?;

    println!("== Stcode bridge livetest (단일 세션) ==");
    println!("  cwd        : {}", cwd.display());
    println!("  prompt     : {user_text}");

    let Bridge {
        cmd_tx,
        mut evt_rx,
    } = Bridge::spawn();

    let sid: SessionId = "s1".into();
    cmd_tx.send(UiCommand::NewSession {
        session_id: sid.clone(),
        path: cwd,
    })?;

    let timeout = Duration::from_secs(
        env::var("STCODE_LIVETEST_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(180),
    );
    let deadline = tokio::time::Instant::now() + timeout;

    // 세션 시작 대기
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, evt_rx.recv()).await {
            Ok(Some(UiEvent::SessionStarted { session_id, thread_id, .. }))
                if session_id == sid =>
            {
                println!("  Started: {thread_id}");
                break;
            }
            Ok(Some(UiEvent::SessionFailed { session_id, error })) if session_id == sid => {
                println!("  Failed: {error}");
                return Ok(());
            }
            Ok(Some(other)) => println!("  (waiting Started, got {other:?})"),
            Ok(None) => return Ok(()),
            Err(_) => {
                println!("⏱ TIMEOUT");
                return Ok(());
            }
        }
    }

    cmd_tx.send(UiCommand::SendText {
        session_id: sid.clone(),
        text: user_text,
    })?;

    let mut accumulated = String::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            println!("⏱ TIMEOUT");
            break;
        }
        match tokio::time::timeout(remaining, evt_rx.recv()).await {
            Ok(Some(UiEvent::AgentDelta { text, .. })) => accumulated.push_str(&text),
            Ok(Some(UiEvent::ApprovalRequested {
                session_id,
                request_id,
                ..
            })) => {
                println!("  ApprovalRequested(id={request_id}) → auto-Decline");
                cmd_tx.send(UiCommand::ApprovalDecision {
                    session_id,
                    request_id,
                    decision: ApprovalDecision::Decline,
                })?;
            }
            Ok(Some(UiEvent::TurnCommitted {
                commit_oid, summary, revert_to, ..
            })) => {
                println!(
                    "  TurnCommitted: oid={}… summary={summary:?} revert_to={revert_to:?}",
                    commit_oid.chars().take(7).collect::<String>()
                );
            }
            Ok(Some(UiEvent::Reverted { ok, .. })) => println!("  Reverted: ok={ok}"),
            Ok(Some(UiEvent::TurnDone { ok, error_text, .. })) => {
                println!("  TurnDone: ok={ok} err={error_text:?}");
                break;
            }
            Ok(Some(UiEvent::Error(text))) => {
                println!("  Error: {text}");
                break;
            }
            Ok(Some(_)) => {} // 노이즈
            Ok(None) => break,
            Err(_) => {
                println!("⏱ TIMEOUT");
                break;
            }
        }
    }

    println!("=== summary ===");
    println!("  accumulated text : {} chars", accumulated.len());
    let preview: String = accumulated.chars().take(200).collect();
    if !preview.is_empty() {
        println!("  text_preview     : {preview:?}");
    }
    cmd_tx.send(UiCommand::Shutdown)?;
    Ok(())
}

async fn run_parallel() -> anyhow::Result<()> {
    let prompt_a = env::var("STCODE_PROMPT_A")
        .unwrap_or_else(|_| "1부터 5까지 세어. 한 줄로.".into());
    let prompt_b = env::var("STCODE_PROMPT_B")
        .unwrap_or_else(|_| "오늘 기분이 어때? 한 줄로.".into());
    let cwd_a: PathBuf = env::var("STCODE_CWD_A")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().expect("cwd"));
    let cwd_b: PathBuf = env::var("STCODE_CWD_B")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().expect("cwd"));

    println!("== Stcode bridge livetest (병렬 2 세션) ==");
    println!("  s1 cwd    : {}", cwd_a.display());
    println!("  s1 prompt : {prompt_a}");
    println!("  s2 cwd    : {}", cwd_b.display());
    println!("  s2 prompt : {prompt_b}");

    let Bridge {
        cmd_tx,
        mut evt_rx,
    } = Bridge::spawn();
    let sid_a: SessionId = "s1".into();
    let sid_b: SessionId = "s2".into();

    cmd_tx.send(UiCommand::NewSession {
        session_id: sid_a.clone(),
        path: cwd_a,
    })?;
    cmd_tx.send(UiCommand::NewSession {
        session_id: sid_b.clone(),
        path: cwd_b,
    })?;

    let timeout = Duration::from_secs(
        env::var("STCODE_LIVETEST_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(180),
    );
    let deadline = tokio::time::Instant::now() + timeout;

    let mut started_a = false;
    let mut started_b = false;
    let mut sent_a = false;
    let mut sent_b = false;
    let mut done_a = false;
    let mut done_b = false;

    while !(done_a && done_b) {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            println!("⏱ TIMEOUT — done_a={done_a}, done_b={done_b}");
            break;
        }
        let ev = match tokio::time::timeout(remaining, evt_rx.recv()).await {
            Ok(Some(ev)) => ev,
            Ok(None) => break,
            Err(_) => {
                println!("⏱ TIMEOUT");
                break;
            }
        };
        match &ev {
            UiEvent::SessionStarted { session_id, thread_id, .. } => {
                println!("  [{session_id}] Started: {thread_id}");
                if session_id == &sid_a {
                    started_a = true;
                }
                if session_id == &sid_b {
                    started_b = true;
                }
            }
            UiEvent::AgentDelta { session_id, text } => {
                println!("  [{session_id}] +{} chars", text.len());
            }
            UiEvent::TurnCommitted { session_id, summary, .. } => {
                println!("  [{session_id}] Committed: {summary:?}");
            }
            UiEvent::TurnDone { session_id, ok, .. } => {
                println!("  [{session_id}] TurnDone ok={ok}");
                if session_id == &sid_a {
                    done_a = true;
                }
                if session_id == &sid_b {
                    done_b = true;
                }
            }
            UiEvent::SessionFailed { session_id, error } => {
                println!("  [{session_id}] Failed: {error}");
                if session_id == &sid_a {
                    done_a = true;
                }
                if session_id == &sid_b {
                    done_b = true;
                }
            }
            UiEvent::Error(t) => println!("  Error: {t}"),
            _ => {}
        }
        // 양쪽 다 시작됐으면 두 prompt를 같은 시점에 발사 — 진짜 병렬 진행 검증.
        if started_a && started_b && !(sent_a && sent_b) {
            cmd_tx.send(UiCommand::SendText {
                session_id: sid_a.clone(),
                text: prompt_a.clone(),
            })?;
            cmd_tx.send(UiCommand::SendText {
                session_id: sid_b.clone(),
                text: prompt_b.clone(),
            })?;
            sent_a = true;
            sent_b = true;
            println!("  → 양쪽 prompt 발사");
        }
    }

    println!("=== summary === done_a={done_a} done_b={done_b}");
    cmd_tx.send(UiCommand::Shutdown)?;
    Ok(())
}
