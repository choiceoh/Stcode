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
use std::process;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

    let Bridge { cmd_tx, mut evt_rx } = Bridge::spawn();

    let sid = unique_session_id("single");
    cmd_tx.send(UiCommand::NewSession {
        session_id: sid.clone(),
        path: cwd,
        provider: "local-vllm".into(),
        main_model: "qwen3.6-35b-a3b".into(),
        sub_model: "qwen3.6-35b-a3b".into(),
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
            Ok(Some(UiEvent::SessionStarted {
                session_id,
                thread_id,
                ..
            })) if session_id == sid => {
                println!("  Started: {thread_id}");
                break;
            }
            Ok(Some(UiEvent::SessionFailed { session_id, error })) if session_id == sid => {
                println!("  Failed: {error}");
                anyhow::bail!("session start failed: {error}");
            }
            Ok(Some(other)) => println!("  (waiting Started, got {other:?})"),
            Ok(None) => anyhow::bail!("event stream closed before session start"),
            Err(_) => {
                println!("⏱ TIMEOUT");
                anyhow::bail!("session start timed out after {:?}", timeout);
            }
        }
    }

    cmd_tx.send(UiCommand::SendText {
        session_id: sid.clone(),
        text: user_text,
    })?;

    let mut accumulated = String::new();
    let mut turn_ok = false;
    let mut failure: Option<String> = None;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            println!("⏱ TIMEOUT");
            failure = Some(format!("turn timed out after {:?}", timeout));
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
                commit_oid,
                summary,
                revert_to,
                ..
            })) => {
                println!(
                    "  TurnCommitted: oid={}… summary={summary:?} revert_to={revert_to:?}",
                    commit_oid.chars().take(7).collect::<String>()
                );
            }
            Ok(Some(UiEvent::Reverted { ok, .. })) => println!("  Reverted: ok={ok}"),
            Ok(Some(UiEvent::TurnDone { ok, error_text, .. })) => {
                println!("  TurnDone: ok={ok} err={error_text:?}");
                turn_ok = ok;
                if !ok {
                    failure = Some(error_text.unwrap_or_else(|| "turn failed".into()));
                }
                break;
            }
            Ok(Some(UiEvent::Error(text))) => {
                println!("  Error: {text}");
                failure = Some(text);
                break;
            }
            Ok(Some(_)) => {} // 노이즈
            Ok(None) => {
                failure = Some("event stream closed before turn completed".into());
                break;
            }
            Err(_) => {
                println!("⏱ TIMEOUT");
                failure = Some(format!("turn timed out after {:?}", timeout));
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
    if !close_session_and_wait(
        &cmd_tx,
        &mut evt_rx,
        &sid,
        tokio::time::Instant::now() + Duration::from_secs(30),
    )
    .await?
    {
        failure = Some("session cleanup was not confirmed".into());
    }
    cmd_tx.send(UiCommand::Shutdown)?;
    if let Some(failure) = failure {
        anyhow::bail!("{failure}");
    }
    if !turn_ok {
        anyhow::bail!("turn did not complete successfully");
    }
    Ok(())
}

async fn run_parallel() -> anyhow::Result<()> {
    let prompt_a =
        env::var("STCODE_PROMPT_A").unwrap_or_else(|_| "1부터 5까지 세어. 한 줄로.".into());
    let prompt_b =
        env::var("STCODE_PROMPT_B").unwrap_or_else(|_| "오늘 기분이 어때? 한 줄로.".into());
    let cwd_a: PathBuf = env::var("STCODE_CWD_A")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().expect("cwd"));
    let cwd_b: PathBuf = env::var("STCODE_CWD_B")
        .map(PathBuf::from)
        .unwrap_or_else(|_| env::current_dir().expect("cwd"));

    println!("== Stcode bridge livetest (병렬 2 세션) ==");
    let Bridge { cmd_tx, mut evt_rx } = Bridge::spawn();
    let sid_a = unique_session_id("a");
    let sid_b = unique_session_id("b");

    println!("  {sid_a} cwd    : {}", cwd_a.display());
    println!("  {sid_a} prompt : {prompt_a}");
    println!("  {sid_b} cwd    : {}", cwd_b.display());
    println!("  {sid_b} prompt : {prompt_b}");

    cmd_tx.send(UiCommand::NewSession {
        session_id: sid_a.clone(),
        path: cwd_a,
        provider: "local-vllm".into(),
        main_model: "qwen3.6-35b-a3b".into(),
        sub_model: "qwen3.6-35b-a3b".into(),
    })?;
    cmd_tx.send(UiCommand::NewSession {
        session_id: sid_b.clone(),
        path: cwd_b,
        provider: "local-vllm".into(),
        main_model: "qwen3.6-35b-a3b".into(),
        sub_model: "qwen3.6-35b-a3b".into(),
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
    let mut ok_a = false;
    let mut ok_b = false;
    let mut failures = Vec::<String>::new();

    while !(done_a && done_b) {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            println!("⏱ TIMEOUT — done_a={done_a}, done_b={done_b}");
            failures.push(format!("parallel run timed out after {:?}", timeout));
            break;
        }
        let ev = match tokio::time::timeout(remaining, evt_rx.recv()).await {
            Ok(Some(ev)) => ev,
            Ok(None) => {
                failures.push("event stream closed before both sessions completed".into());
                break;
            }
            Err(_) => {
                println!("⏱ TIMEOUT");
                failures.push(format!("parallel run timed out after {:?}", timeout));
                break;
            }
        };
        match &ev {
            UiEvent::SessionStarted {
                session_id,
                thread_id,
                ..
            } => {
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
            UiEvent::TurnCommitted {
                session_id,
                summary,
                ..
            } => {
                println!("  [{session_id}] Committed: {summary:?}");
            }
            UiEvent::TurnDone { session_id, ok, .. } => {
                println!("  [{session_id}] TurnDone ok={ok}");
                if session_id == &sid_a {
                    done_a = true;
                    ok_a = *ok;
                    if !ok {
                        failures.push(format!("{sid_a} turn failed"));
                    }
                }
                if session_id == &sid_b {
                    done_b = true;
                    ok_b = *ok;
                    if !ok {
                        failures.push(format!("{sid_b} turn failed"));
                    }
                }
            }
            UiEvent::SessionFailed { session_id, error } => {
                println!("  [{session_id}] Failed: {error}");
                failures.push(format!("{session_id} session failed: {error}"));
                if session_id == &sid_a {
                    done_a = true;
                }
                if session_id == &sid_b {
                    done_b = true;
                }
            }
            UiEvent::Error(t) => {
                println!("  Error: {t}");
                failures.push(t.clone());
            }
            _ => {}
        }
        // 시작된 세션부터 prompt를 보낸다. 한쪽 준비 실패가 다른 세션 검증을 막으면 안 된다.
        if started_a && !sent_a {
            cmd_tx.send(UiCommand::SendText {
                session_id: sid_a.clone(),
                text: prompt_a.clone(),
            })?;
            sent_a = true;
            println!("  [{sid_a}] prompt 발사");
        }
        if started_b && !sent_b {
            cmd_tx.send(UiCommand::SendText {
                session_id: sid_b.clone(),
                text: prompt_b.clone(),
            })?;
            sent_b = true;
            println!("  [{sid_b}] prompt 발사");
        }
    }

    println!("=== summary === done_a={done_a} done_b={done_b}");
    let cleanup_deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    if started_a {
        if !close_session_and_wait(&cmd_tx, &mut evt_rx, &sid_a, cleanup_deadline).await? {
            failures.push(format!("{sid_a} cleanup was not confirmed"));
        }
    }
    if started_b {
        if !close_session_and_wait(&cmd_tx, &mut evt_rx, &sid_b, cleanup_deadline).await? {
            failures.push(format!("{sid_b} cleanup was not confirmed"));
        }
    }
    cmd_tx.send(UiCommand::Shutdown)?;
    if !done_a {
        failures.push(format!("{sid_a} did not complete"));
    }
    if !done_b {
        failures.push(format!("{sid_b} did not complete"));
    }
    if !ok_a {
        failures.push(format!("{sid_a} did not finish successfully"));
    }
    if !ok_b {
        failures.push(format!("{sid_b} did not finish successfully"));
    }
    if !failures.is_empty() {
        anyhow::bail!("livetest failed: {}", failures.join("; "));
    }
    Ok(())
}

fn unique_session_id(label: &str) -> SessionId {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    format!("lt-{label}-{:x}-{millis:x}", process::id()).into()
}

async fn close_session_and_wait(
    cmd_tx: &tokio::sync::mpsc::UnboundedSender<UiCommand>,
    evt_rx: &mut tokio::sync::mpsc::UnboundedReceiver<UiEvent>,
    sid: &SessionId,
    deadline: tokio::time::Instant,
) -> anyhow::Result<bool> {
    cmd_tx.send(UiCommand::CloseSession {
        session_id: sid.clone(),
    })?;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            println!("  [{sid}] close timeout");
            return Ok(false);
        }
        match tokio::time::timeout(remaining, evt_rx.recv()).await {
            Ok(Some(UiEvent::WorkspaceCleanup {
                session_id,
                message,
            })) if &session_id == sid => {
                println!("  [{sid}] Cleanup: {}", message.replace('\n', " / "));
            }
            Ok(Some(UiEvent::SessionClosed { session_id })) if &session_id == sid => {
                println!("  [{sid}] Closed");
                return Ok(true);
            }
            Ok(Some(UiEvent::Error(text))) => println!("  Error: {text}"),
            Ok(Some(_)) => {}
            Ok(None) => return Ok(false),
            Err(_) => {
                println!("  [{sid}] close timeout");
                return Ok(false);
            }
        }
    }
}
