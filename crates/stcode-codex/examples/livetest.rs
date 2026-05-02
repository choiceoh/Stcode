//! 헤드리스 라이브 테스트 — GUI 없이 codex 통합 흐름 검증.
//!
//! `cargo run -p stcode-codex --example livetest -- "프롬프트"`
//!
//! GUI bridge와 동일한 ENV/SpawnOptions로 codex spawn → thread/start → turn/start →
//! 모든 ThreadEvent를 method 이름과 짧은 payload preview로 stdout에 출력 →
//! turn/completed 또는 timeout까지 대기.
//!
//! Stcode 개발 시 GUI 띄우지 않고 codex 응답 흐름을 빠르게 검증하기 위한 도구.

use std::env;
use std::time::Duration;

use stcode_codex::{
    ApprovalPolicy, ClientInfo, SandboxMode, SpawnOptions, ThreadEvent, ThreadSession,
    ThreadStartParams,
};

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

    let cwd = env::current_dir()?.to_string_lossy().into_owned();
    let provider = env::var("STCODE_PROVIDER").unwrap_or_else(|_| "local-vllm".into());
    let model = env::var("STCODE_MODEL").unwrap_or_else(|_| "qwen3.6-35b-a3b".into());

    println!("== Stcode livetest ==");
    println!("  cwd        : {cwd}");
    println!("  provider   : {provider}");
    println!("  model      : {model}");
    println!("  prompt     : {user_text}");
    println!();

    // GUI bridge와 동일한 spawn options
    let mut opts = SpawnOptions::with_provider_model(&provider, &model);
    if provider == "local-vllm" {
        opts = opts
            .with_env("STCODE_VLLM_COMPAT", "1")
            .push("model_reasoning_effort", "minimal")
            .push("model_providers.local-vllm.supports_websockets", "false");
        if env::var_os("VLLM_API_KEY").is_none() {
            opts = opts.with_env("VLLM_API_KEY", "dummy");
        }
    }

    let mut session = ThreadSession::start_with(
        ClientInfo::default(),
        ThreadStartParams {
            cwd: Some(cwd),
            approval_policy: Some(ApprovalPolicy::Never),
            sandbox: Some(SandboxMode::ReadOnly),
            ..Default::default()
        },
        opts,
    )
    .await?;

    println!("thread_id  : {}", session.thread_id);
    let turn_id = session.send_user_text(user_text).await?;
    println!("turn_id    : {turn_id}");
    println!();
    println!("=== events ===");

    let mut event_counts = std::collections::BTreeMap::<String, usize>::new();
    let mut accumulated_message = String::new();
    let timeout = Duration::from_secs(
        env::var("STCODE_LIVETEST_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120),
    );
    let deadline = tokio::time::Instant::now() + timeout;
    let mut completed = false;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            println!("⏱ TIMEOUT after {:?}", timeout);
            break;
        }
        let ev = match tokio::time::timeout(remaining, session.next_event()).await {
            Ok(Some(ev)) => ev,
            Ok(None) => {
                println!("⛔ codex disconnected (channel closed)");
                break;
            }
            Err(_) => {
                println!("⏱ TIMEOUT (no event for {:?})", timeout);
                break;
            }
        };
        match &ev {
            ThreadEvent::AgentMessageDelta(d) => {
                *event_counts.entry("AgentMessageDelta".into()).or_default() += 1;
                accumulated_message.push_str(&d.delta);
                println!("  AgentMessageDelta: +{} chars", d.delta.len());
            }
            ThreadEvent::TurnCompleted { turn } => {
                *event_counts.entry("TurnCompleted".into()).or_default() += 1;
                println!("  TurnCompleted: status={:?}", turn.status);
                completed = true;
                break;
            }
            ThreadEvent::TurnStarted { turn } => {
                *event_counts.entry("TurnStarted".into()).or_default() += 1;
                println!("  TurnStarted: id={}", turn.id);
            }
            ThreadEvent::ThreadStarted { thread_id } => {
                println!("  ThreadStarted: {thread_id}");
            }
            ThreadEvent::CommandOutputDelta(d) => {
                *event_counts.entry("CommandOutputDelta".into()).or_default() += 1;
                println!(
                    "  CommandOutputDelta[{:?}]: +{} chars",
                    d.stream,
                    d.delta.len()
                );
            }
            ThreadEvent::Other { method, params } => {
                *event_counts.entry(format!("Other:{method}")).or_default() += 1;
                let preview = serde_json::to_string(params).unwrap_or_default();
                let preview = preview.chars().take(120).collect::<String>();
                println!("  Other[{method}]: {preview}");
            }
            ThreadEvent::InboundRequest { method, .. } => {
                *event_counts
                    .entry(format!("InboundRequest:{method}"))
                    .or_default() += 1;
                println!("  InboundRequest: {method}");
            }
        }
    }

    println!();
    println!("=== summary ===");
    println!("  completed         : {completed}");
    println!("  accumulated_text  : {} chars", accumulated_message.len());
    if !accumulated_message.is_empty() {
        let preview: String = accumulated_message.chars().take(200).collect();
        println!("  text_preview      : {preview:?}");
    }
    println!("  event counts:");
    for (k, v) in &event_counts {
        println!("    {k:60} : {v}");
    }

    session.shutdown().await;
    Ok(())
}
