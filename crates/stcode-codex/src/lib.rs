//! `codex app-server`를 자식 프로세스로 spawn 하고 JSON-RPC 2.0(stdio)로 통신한다.
//!
//! M0 범위: spawn → `initialize` → `initialized` notification 까지 검증.
//! M1+: thread/start, turn/start, item delta 스트림 + 승인 요청 라우팅.

mod rpc;
mod spawn;

pub use rpc::{RpcClient, RpcError};
pub use spawn::{CodexBinary, SpawnError, find_codex_binary};

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

/// M0 검증 헬퍼: codex 바이너리를 찾아 spawn 하고 initialize 한 뒤 결과를 반환.
/// `codex` 미설치 시 친화적 에러로 변환.
pub async fn probe_initialize() -> anyhow::Result<InitializeResult> {
    let bin = find_codex_binary()?;
    tracing::info!("codex 바이너리 발견: {}", bin.path.display());

    let mut client = RpcClient::spawn_app_server(&bin).await?;
    let result = client
        .request::<_, InitializeResult>("initialize", InitializeParams::default())
        .await?;
    client.notify("initialized", serde_json::json!({})).await?;

    // M0에선 핸드셰이크 확인 후 곧바로 종료.
    client.shutdown().await;
    Ok(result)
}
