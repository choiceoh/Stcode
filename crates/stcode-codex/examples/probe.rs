//! `cargo run -p stcode-codex --example probe`
//!
//! GPUI 없이 codex initialize 핸드셰이크만 검증한다.
//! M0 검증: Xcode 미설치 환경에서도 codex 경로가 정상인지 확인.

use stcode_codex::probe_initialize;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,stcode_codex=debug".into()),
        )
        .init();

    let info = probe_initialize().await?;
    println!("== Codex initialize 성공 ==");
    println!("  userAgent      : {}", info.user_agent.as_deref().unwrap_or("<none>"));
    println!("  codexHome      : {}", info.codex_home.as_deref().unwrap_or("<none>"));
    println!("  platformFamily : {}", info.platform_family.as_deref().unwrap_or("<none>"));
    println!("  platformOs     : {}", info.platform_os.as_deref().unwrap_or("<none>"));
    if !info.extra.is_empty() {
        println!("  추가 필드      : {}", serde_json::to_string(&info.extra)?);
    }
    Ok(())
}
