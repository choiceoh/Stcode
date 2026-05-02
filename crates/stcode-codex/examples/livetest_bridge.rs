//! 헤드리스 GUI bridge 라이브 테스트.
//!
//! `cargo run -p stcode-codex --example livetest_bridge -- "프롬프트"`
//!
//! GUI를 띄우지 않고 stcode-app과 동일한 `Bridge` (cmd_tx/evt_rx 채널)를 통해
//! StartProject → SendText → AgentDelta 누적 → TurnDone 흐름을 검증한다.
//!
//! `livetest`는 ThreadSession 직접(low-level codex)이고, 이 도구는 한 단계 위
//! Bridge layer (GUI가 실제 사용하는 흐름) 검증.

use std::env;
use std::path::PathBuf;
use std::time::Duration;

use stcode_codex::bridge::{Bridge, UiCommand, UiEvent};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,stcode_codex=info".into()),
        )
        .init();

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

    println!("== Stcode bridge livetest ==");
    println!("  cwd        : {}", cwd.display());
    println!("  prompt     : {user_text}");
    println!();

    let Bridge {
        cmd_tx,
        mut evt_rx,
    } = Bridge::spawn();

    // 1) StartProject — codex spawn + thread/start
    println!("→ StartProject");
    cmd_tx.send(UiCommand::StartProject { path: cwd })?;

    let timeout = Duration::from_secs(
        env::var("STCODE_LIVETEST_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(180),
    );
    let deadline = tokio::time::Instant::now() + timeout;

    // Wait for Started
    let mut started = false;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, evt_rx.recv()).await {
            Ok(Some(UiEvent::Started { thread_id })) => {
                println!("  Started: {thread_id}");
                started = true;
                break;
            }
            Ok(Some(UiEvent::Error(text))) => {
                println!("  Error: {text}");
                break;
            }
            Ok(Some(other)) => {
                println!("  (waiting Started, got {other:?})");
            }
            Ok(None) => {
                println!("⛔ bridge channel closed");
                break;
            }
            Err(_) => {
                println!("⏱ TIMEOUT waiting Started");
                break;
            }
        }
    }

    if !started {
        let _ = cmd_tx.send(UiCommand::Shutdown);
        return Ok(());
    }

    // 2) SendText
    println!("→ SendText({user_text:?})");
    cmd_tx.send(UiCommand::SendText(user_text))?;

    // 3) Loop on UiEvent — accumulate AgentDelta until TurnDone or Error
    let mut accumulated = String::new();
    let mut delta_count = 0usize;
    let mut turn_done: Option<(bool, Option<String>)> = None;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            println!("⏱ TIMEOUT after {timeout:?}");
            break;
        }
        match tokio::time::timeout(remaining, evt_rx.recv()).await {
            Ok(Some(UiEvent::AgentDelta(text))) => {
                delta_count += 1;
                accumulated.push_str(&text);
                println!("  AgentDelta #{delta_count}: +{} chars", text.len());
            }
            Ok(Some(UiEvent::TurnDone { ok, error_text })) => {
                println!("  TurnDone: ok={ok} err={error_text:?}");
                turn_done = Some((ok, error_text));
                break;
            }
            Ok(Some(UiEvent::Error(text))) => {
                println!("  Error: {text}");
                break;
            }
            Ok(Some(other)) => {
                println!("  (other event: {other:?})");
            }
            Ok(None) => {
                println!("⛔ bridge channel closed");
                break;
            }
            Err(_) => {
                println!("⏱ TIMEOUT after {timeout:?}");
                break;
            }
        }
    }

    println!();
    println!("=== summary ===");
    println!("  AgentDelta count : {delta_count}");
    println!("  accumulated text : {} chars", accumulated.len());
    if !accumulated.is_empty() {
        let preview: String = accumulated.chars().take(200).collect();
        println!("  text_preview     : {preview:?}");
    }
    println!("  TurnDone         : {turn_done:?}");

    let _ = cmd_tx.send(UiCommand::Shutdown);
    Ok(())
}
