//! `codex app-server`를 자식 프로세스로 spawn 하고 JSON-RPC 2.0(stdio)로 통신한다.
//!
//! 레이어:
//! - [`rpc`] — raw JSON-RPC 2.0 클라이언트 + 인바운드 채널
//! - [`protocol`] — codex 프로토콜의 minimal serde 타입 (자체 정의)
//! - [`session`] — 타입 안전 thread/turn 추상화 ([`ThreadSession`])

pub mod protocol;
mod rpc;
pub mod session;
mod spawn;

pub use protocol::{
    method, AccountReadParams, AccountReadResponse, AgentMessageDelta, ApprovalPolicy,
    CommandOutputDelta, CommandStream, SandboxMode, Thread, ThreadStartParams,
    ThreadStartResponse, TurnInfo, TurnStartParams, TurnStartResponse, TurnStatus, UserInput,
};
pub use rpc::{InboundMessage, RpcClient, RpcError};
pub use session::{ThreadEvent, ThreadSession};
pub use spawn::{find_codex_binary, CodexBinary, SpawnError, SpawnOptions};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct ClientInfo {
    pub name: String,
    pub title: String,
    pub version: String,
}

impl Default for ClientInfo {
    fn default() -> Self {
        Self {
            name: "stcode".into(),
            title: "Stcode".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Capabilities {
    #[serde(rename = "experimentalApi")]
    pub experimental_api: bool,
    #[serde(rename = "optOutNotificationMethods")]
    pub opt_out_notification_methods: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct InitializeParams {
    #[serde(rename = "clientInfo")]
    pub client_info: ClientInfo,
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub user_agent: Option<String>,
    pub codex_home: Option<String>,
    pub platform_family: Option<String>,
    pub platform_os: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// M0 검증 헬퍼 — codex 핸드셰이크만 보고 종료한다.
pub async fn probe_initialize() -> anyhow::Result<InitializeResult> {
    let bin = find_codex_binary()?;
    tracing::info!("codex 바이너리: {}", bin.path.display());
    let (rpc, _inbound) = RpcClient::spawn_app_server(&bin).await?;
    let result: InitializeResult = rpc
        .request(method::INITIALIZE, InitializeParams::default())
        .await?;
    rpc.notify(method::INITIALIZED, serde_json::json!({})).await?;
    rpc.shutdown().await;
    Ok(result)
}
