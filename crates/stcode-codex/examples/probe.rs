//! `cargo run -p stcode-codex --example probe -- [질문]`
//!
//! GPUI 없이 codex를 end-to-end 검증한다.
//!  1. spawn → initialize → initialized
//!  2. thread/start (read-only sandbox + never approval — 안전)
//!  3. turn/start (사용자 입력)
//!  4. agentMessage/delta 스트림을 한 글자씩 stdout으로 흘림
//!  5. turn/completed → 종료
//!
//! 인자가 없으면 "안녕" 한 마디만 보낸다.

use std::env;
use std::io::Write;

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
        "안녕".to_string()
    } else {
        user_text
    };

    let cwd = env::current_dir()?
        .to_string_lossy()
        .into_owned();

    // 환경변수로 프로바이더/모델 override 가능. 기본은 사용자의 local-vllm.
    let provider = env::var("STCODE_PROVIDER").unwrap_or_else(|_| "local-vllm".into());
    let model = env::var("STCODE_MODEL").unwrap_or_else(|_| "qwen3.6-35b-a3b".into());

    println!("== Stcode codex probe ==");
    println!("  cwd        : {cwd}");
    println!("  provider   : {provider}");
    println!("  model      : {model}");
    println!("  사용자 입력: {user_text}");
    println!();

    // local-vllm은 인증 안 받지만 codex가 env_key 빈 값 검증을 하므로 dummy 채움.
    // 주의: codex 0.128부터 wire_api="chat" 미지원 → vLLM이 Responses API + `developer`
    // role을 받아야 한다. qwen3.6-35b-a3b의 chat template이 `developer`를 모르면
    // 서버에서 chat template 패치 필요 (Stcode 측에선 우회 불가).
    let mut spawn_opts = SpawnOptions::with_provider_model(&provider, &model);
    if provider == "local-vllm" && env::var_os("VLLM_API_KEY").is_none() {
        spawn_opts = spawn_opts.with_env("VLLM_API_KEY", "dummy");
    }

    let mut session = ThreadSession::start_with(
        ClientInfo::default(),
        ThreadStartParams {
            cwd: Some(cwd),
            // M1 안전 데모: read-only + never. 도구 실행/승인 없음.
            approval_policy: Some(ApprovalPolicy::Never),
            sandbox: Some(SandboxMode::ReadOnly),
            ..Default::default()
        },
        spawn_opts,
    )
    .await?;

    println!("thread_id: {}", session.thread_id);
    let turn_id = session.send_user_text(user_text).await?;
    println!("turn_id  : {turn_id}");
    println!();
    print!("🤖 ");
    std::io::stdout().flush().ok();

    let mut completed = false;
    let mut stdout = std::io::stdout().lock();
    while let Some(ev) = session.next_event().await {
        match ev {
            ThreadEvent::AgentMessageDelta(d) => {
                let _ = write!(stdout, "{}", d.delta);
                let _ = stdout.flush();
            }
            ThreadEvent::TurnCompleted { turn } => {
                let _ = writeln!(stdout);
                let _ = writeln!(stdout, "\n== turn 종료: status={:?} ==", turn.status);
                if let Some(err) = turn.error {
                    let _ = writeln!(stdout, "에러: {err}");
                }
                completed = true;
                break;
            }
            ThreadEvent::InboundRequest { id, method, .. } => {
                tracing::warn!("승인 요청이 와버림 (M1 데모는 never 정책): id={id} method={method}");
            }
            ThreadEvent::Other { method, .. } => {
                tracing::debug!("기타 노티: {method}");
            }
            _ => {}
        }
    }

    if !completed {
        let _ = writeln!(stdout, "\n== 비정상 종료 (turn/completed 미수신) ==");
    }

    session.shutdown().await;
    Ok(())
}
