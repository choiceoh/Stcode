//! 타입 안전 thread/turn 세션 래퍼.
//!
//! `RpcClient`가 raw JSON-RPC라면, `ThreadSession`은 codex의 thread/turn 추상을
//! Rust 타입으로 노출한다. 서버→클라이언트 노티는 [`ThreadEvent`]로 정규화.

use anyhow::Result;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::protocol::{
    method, AgentMessageDelta, CommandOutputDelta, ReasoningTextDelta, ThreadStartParams,
    ThreadStartResponse, TurnCompletedParams, TurnInfo, TurnStartParams, TurnStartResponse,
    UserInput,
};
use crate::rpc::{InboundMessage, RpcClient};
use crate::spawn::{find_codex_binary, CodexBinary, SpawnOptions};
use crate::{Capabilities, ClientInfo, InitializeParams, InitializeResult};

#[derive(Debug, Clone)]
pub enum ThreadEvent {
    ThreadStarted {
        thread_id: String,
    },
    TurnStarted {
        turn: TurnInfo,
    },
    TurnCompleted {
        turn: TurnInfo,
    },
    AgentMessageDelta(AgentMessageDelta),
    ReasoningDelta(ReasoningTextDelta),
    CommandOutputDelta(CommandOutputDelta),
    /// `item/started` — type별로 start signal (commandExecution, fileChange 등).
    ItemStarted {
        item_id: String,
        item_type: String,
        params: serde_json::Value,
    },
    /// `item/completed` — type별 종료. exit_code/diff 같은 final state는 params에.
    ItemCompleted {
        item_id: String,
        item_type: String,
        params: serde_json::Value,
    },
    /// 아직 타입화하지 않은 노티. method/params 그대로 노출.
    Other {
        method: String,
        params: serde_json::Value,
    },
    /// 서버→클라이언트 리퀘스트 (승인 등). 호출자가 [`ThreadSession::respond_request`]로 답해야 함.
    InboundRequest {
        id: i64,
        method: String,
        params: serde_json::Value,
    },
}

pub struct ThreadSession {
    rpc: RpcClient,
    inbound: mpsc::Receiver<InboundMessage>,
    pub thread_id: String,
}

impl ThreadSession {
    /// codex spawn → initialize → initialized → thread/start까지 한 번에.
    pub async fn start(client_info: ClientInfo, params: ThreadStartParams) -> Result<Self> {
        Self::start_with(client_info, params, SpawnOptions::default()).await
    }

    /// `SpawnOptions`로 codex 설정 override (예: 로컬 vLLM)를 적용해 시작.
    pub async fn start_with(
        client_info: ClientInfo,
        params: ThreadStartParams,
        opts: SpawnOptions,
    ) -> Result<Self> {
        let bin = find_codex_binary()?;
        Self::start_with_binary(&bin, client_info, params, opts).await
    }

    pub async fn start_with_binary(
        bin: &CodexBinary,
        client_info: ClientInfo,
        params: ThreadStartParams,
        opts: SpawnOptions,
    ) -> Result<Self> {
        tracing::info!("codex 바이너리: {} (overrides={})", bin.path.display(), opts.config_overrides.len());
        let (rpc, inbound) = RpcClient::spawn_app_server_with(bin, &opts).await?;

        // 핸드셰이크
        let _: InitializeResult = rpc
            .request(
                method::INITIALIZE,
                InitializeParams {
                    client_info,
                    capabilities: Capabilities::default(),
                },
            )
            .await?;
        rpc.notify(method::INITIALIZED, serde_json::json!({})).await?;

        // thread/start
        let resp: ThreadStartResponse = rpc.request(method::THREAD_START, params).await?;
        let thread_id = resp.thread.id.clone();
        tracing::info!("thread 시작: {thread_id}");

        Ok(Self {
            rpc,
            inbound,
            thread_id,
        })
    }

    /// 사용자 텍스트 한 건을 turn으로 보낸다. 응답 turnId 반환.
    pub async fn send_user_text(&self, text: String) -> Result<String> {
        let resp: TurnStartResponse = self
            .rpc
            .request(
                method::TURN_START,
                TurnStartParams {
                    thread_id: self.thread_id.clone(),
                    input: vec![UserInput::Text { text }],
                    cwd: None,
                    model: None,
                },
            )
            .await?;
        Ok(resp.turn.id)
    }

    /// 다음 정규화된 이벤트. None이면 codex가 끊김(`Disconnected`도 한 번 emit 후 None).
    pub async fn next_event(&mut self) -> Option<ThreadEvent> {
        let raw = self.inbound.recv().await?;
        Some(match raw {
            InboundMessage::Notification { method, params } => parse_notification(method, params),
            InboundMessage::Request { id, method, params } => ThreadEvent::InboundRequest {
                id,
                method,
                params,
            },
        })
    }

    /// 인바운드 채널이 끊겼는지(=codex 종료) 폴링용 헬퍼.
    pub fn is_disconnected(&self) -> bool {
        self.inbound.is_closed()
    }

    /// 서버 리퀘스트에 응답. 승인 처리에 사용.
    pub async fn respond_request<R: serde::Serialize>(&self, id: i64, result: &R) -> Result<()> {
        self.rpc.respond(id, result).await?;
        Ok(())
    }

    /// 진행 중인 turn 인터럽트.
    pub async fn interrupt(&self) -> Result<()> {
        self.rpc
            .notify(
                method::TURN_INTERRUPT,
                serde_json::json!({ "threadId": self.thread_id }),
            )
            .await?;
        Ok(())
    }

    pub async fn shutdown(self) {
        self.rpc.shutdown().await;
    }
}

/// `&Value`는 `Deserializer`를 구현하므로 borrow-only 파싱 → Err 시 원본 보존,
/// hot path(agentMessage delta)에서 clone 없음.
fn parse_notification(method: String, params: serde_json::Value) -> ThreadEvent {
    #[derive(Deserialize)]
    struct ThreadStartedShape {
        thread: ThreadIdOnly,
    }
    #[derive(Deserialize)]
    struct ThreadIdOnly {
        id: String,
    }

    let parsed: Result<ThreadEvent, serde_json::Error> = match method.as_str() {
        method::NOTIF_THREAD_STARTED => ThreadStartedShape::deserialize(&params)
            .map(|s| ThreadEvent::ThreadStarted { thread_id: s.thread.id }),
        method::NOTIF_TURN_STARTED => TurnStartResponse::deserialize(&params)
            .map(|r| ThreadEvent::TurnStarted { turn: r.turn }),
        method::NOTIF_TURN_COMPLETED => TurnCompletedParams::deserialize(&params)
            .map(|p| ThreadEvent::TurnCompleted { turn: p.turn }),
        method::NOTIF_AGENT_MESSAGE_DELTA => {
            AgentMessageDelta::deserialize(&params).map(ThreadEvent::AgentMessageDelta)
        }
        method::NOTIF_REASONING_TEXT_DELTA | method::NOTIF_REASONING_SUMMARY_TEXT_DELTA => {
            ReasoningTextDelta::deserialize(&params).map(ThreadEvent::ReasoningDelta)
        }
        method::NOTIF_COMMAND_OUTPUT_DELTA => {
            CommandOutputDelta::deserialize(&params).map(ThreadEvent::CommandOutputDelta)
        }
        method::NOTIF_ITEM_STARTED | method::NOTIF_ITEM_COMPLETED => {
            return parse_item_lifecycle(method, params);
        }
        _ => return ThreadEvent::Other { method, params },
    };

    match parsed {
        Ok(ev) => ev,
        Err(e) => {
            tracing::warn!("노티 파싱 실패 ({method}): {e}");
            ThreadEvent::Other { method, params }
        }
    }
}

/// `item/started` / `item/completed` 의 `item.id` + `item.type`만 뽑아 lifecycle 이벤트 만든다.
fn parse_item_lifecycle(method: String, params: serde_json::Value) -> ThreadEvent {
    let item = match params.get("item") {
        Some(it) => it.clone(),
        None => return ThreadEvent::Other { method, params },
    };
    let item_id = item
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let item_type = item
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if method == method::NOTIF_ITEM_STARTED {
        ThreadEvent::ItemStarted {
            item_id,
            item_type,
            params,
        }
    } else {
        ThreadEvent::ItemCompleted {
            item_id,
            item_type,
            params,
        }
    }
}
