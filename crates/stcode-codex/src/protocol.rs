//! Codex app-server JSON-RPC 프로토콜의 minimal serde 타입.
//!
//! 정책: codex-app-server-protocol 크레이트 자체를 import 하면 의존성 트리가 너무
//! 무거워지므로(rmcp, schemars, ts-rs, codex-protocol 등 다수의 codex 워크스페이스
//! 크레이트를 git path로 끌어옴), Stcode가 **실제로 쓰는 메서드와 노티의 필드만**
//! 직접 정의한다.
//!
//! 출처(2026-04 시점): codex-rs/app-server-protocol/src/protocol/v2.rs

use serde::{Deserialize, Serialize};

// ─── 공통 enum ────────────────────────────────────────────────

/// `untrusted | onFailure | onRequest | never` (camelCase wire).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalPolicy {
    Untrusted,
    OnFailure,
    OnRequest,
    Never,
}

/// `read-only | workspace-write | danger-full-access` (kebab-case wire).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

/// turn 상태. `inProgress` 는 시작 직후, 나머지는 종료 사유.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TurnStatus {
    InProgress,
    Completed,
    Interrupted,
    Failed,
}

/// commandExecution 출력 스트림.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CommandStream {
    Stdout,
    Stderr,
}

// ─── thread/start ────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_policy: Option<ApprovalPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<SandboxMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ephemeral: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartResponse {
    pub thread: Thread,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
}

// ─── turn/start ──────────────────────────────────────────────

/// codex `UserInput` tagged union. M1에선 text만 사용.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum UserInput {
    Text { text: String },
    LocalImage { path: String },
    Image { url: String },
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartParams {
    pub thread_id: String,
    pub input: Vec<UserInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInfo {
    pub id: String,
    #[serde(default)]
    pub status: Option<TurnStatus>,
    #[serde(default)]
    pub error: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartResponse {
    pub turn: TurnInfo,
}

// ─── 스트리밍 노티 (server → client) ────────────────────────

/// `item/agentMessage/delta` params.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessageDelta {
    pub item_id: String,
    pub delta: String,
}

/// `item/reasoning/textDelta` params.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasoningTextDelta {
    pub item_id: String,
    pub delta: String,
}

/// `item/started` 또는 `item/completed` params 의 `item` 필드 일부 — type 분기용.
/// 우리는 commandExecution / fileChange / agentMessage / reasoning 만 신경 씀.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemLifecycle {
    pub item: serde_json::Value,
}

/// `item/commandExecution/outputDelta` params.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandOutputDelta {
    pub item_id: String,
    pub stream: CommandStream,
    pub delta: String,
}

/// `turn/completed` params 의 `turn` 필드.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnCompletedParams {
    pub turn: TurnInfo,
}

// ─── 인증 ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountReadParams {
    #[serde(default)]
    pub refresh_token: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountReadResponse {
    #[serde(default)]
    pub account: Option<serde_json::Value>,
    #[serde(default)]
    pub requires_openai_auth: bool,
}

// ─── 메서드 이름 상수 ────────────────────────────────────────

pub mod method {
    pub const INITIALIZE: &str = "initialize";
    pub const INITIALIZED: &str = "initialized";
    pub const ACCOUNT_READ: &str = "account/read";
    pub const THREAD_START: &str = "thread/start";
    pub const TURN_START: &str = "turn/start";
    pub const TURN_INTERRUPT: &str = "turn/interrupt";
    pub const SHUTDOWN: &str = "shutdown";

    // 서버→클라이언트 노티
    pub const NOTIF_THREAD_STARTED: &str = "thread/started";
    pub const NOTIF_TURN_STARTED: &str = "turn/started";
    pub const NOTIF_TURN_COMPLETED: &str = "turn/completed";
    pub const NOTIF_AGENT_MESSAGE_DELTA: &str = "item/agentMessage/delta";
    pub const NOTIF_REASONING_TEXT_DELTA: &str = "item/reasoning/textDelta";
    pub const NOTIF_REASONING_SUMMARY_TEXT_DELTA: &str = "item/reasoning/summaryTextDelta";
    pub const NOTIF_COMMAND_OUTPUT_DELTA: &str = "item/commandExecution/outputDelta";
    pub const NOTIF_ITEM_STARTED: &str = "item/started";
    pub const NOTIF_ITEM_COMPLETED: &str = "item/completed";
    pub const NOTIF_REMOTE_CONTROL_STATUS: &str = "remoteControl/status/changed";

    // 서버→클라이언트 리퀘스트 (승인)
    pub const REQ_COMMAND_APPROVAL: &str = "item/commandExecution/requestApproval";
    pub const REQ_FILE_CHANGE_APPROVAL: &str = "item/fileChange/requestApproval";
}
