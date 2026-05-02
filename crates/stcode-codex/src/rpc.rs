use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot, Mutex};

use crate::spawn::{CodexBinary, SpawnOptions};

#[derive(Debug, Error)]
pub enum RpcError {
    #[error("프로세스 spawn 실패: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("표준 입출력 연결 실패")]
    NoStdio,
    #[error("JSON 직렬화 실패: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("I/O 오류: {0}")]
    Io(#[from] std::io::Error),
    #[error("응답 채널 닫힘")]
    Closed,
    #[error("서버 에러 ({code}): {message}")]
    Server { code: i64, message: String },
}

/// 서버 → 클라이언트 메시지 (노티 또는 리퀘스트).
/// 응답이 필요한 리퀘스트는 `id`가 Some.
#[derive(Debug, Clone)]
pub enum InboundMessage {
    Notification {
        method: String,
        params: serde_json::Value,
    },
    Request {
        id: i64,
        method: String,
        params: serde_json::Value,
    },
}

#[derive(Serialize)]
struct RpcRequest<'a, P: Serialize> {
    jsonrpc: &'static str,
    method: &'a str,
    id: i64,
    params: P,
}

#[derive(Serialize)]
struct RpcNotification<'a, P: Serialize> {
    jsonrpc: &'static str,
    method: &'a str,
    params: P,
}

/// 서버가 보내온 응답에 클라이언트가 답하는 형태.
#[derive(Serialize)]
struct RpcResultEnvelope<'a, R: Serialize> {
    jsonrpc: &'static str,
    id: i64,
    result: &'a R,
}

#[derive(Deserialize)]
struct RpcResponse {
    id: Option<i64>,
    method: Option<String>,
    result: Option<serde_json::Value>,
    error: Option<RpcServerError>,
    #[serde(default)]
    params: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct RpcServerError {
    code: i64,
    message: String,
}

type Pending = Arc<
    Mutex<std::collections::HashMap<i64, oneshot::Sender<Result<serde_json::Value, RpcError>>>>,
>;

pub struct RpcClient {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: AtomicI64,
    pending: Pending,
}

impl RpcClient {
    /// codex app-server를 spawn 하고 (client, inbound_rx)를 반환.
    /// inbound_rx는 서버→클라이언트 노티/리퀘스트 스트림. None이면 codex가 종료된 것.
    pub async fn spawn_app_server(
        bin: &CodexBinary,
    ) -> Result<(Self, mpsc::Receiver<InboundMessage>), RpcError> {
        Self::spawn_app_server_with(bin, &SpawnOptions::default()).await
    }

    /// `SpawnOptions`로 codex 설정 override를 전달하면서 spawn.
    pub async fn spawn_app_server_with(
        bin: &CodexBinary,
        opts: &SpawnOptions,
    ) -> Result<(Self, mpsc::Receiver<InboundMessage>), RpcError> {
        let mut cmd = Command::new(&bin.path);
        // `-c key=value`는 `app-server` 서브커맨드 *앞에* 와야 한다.
        for (k, v) in &opts.config_overrides {
            cmd.arg("-c").arg(format!("{k}={v}"));
        }
        cmd.arg("app-server").arg("--listen").arg("stdio://");
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }

        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(RpcError::Spawn)?;

        let stdin = child.stdin.take().ok_or(RpcError::NoStdio)?;
        let stdout = child.stdout.take().ok_or(RpcError::NoStdio)?;

        let pending: Pending = Arc::new(Mutex::new(std::collections::HashMap::new()));
        let (inbound_tx, inbound_rx) = mpsc::channel::<InboundMessage>(64);

        spawn_reader_loop(stdout, pending.clone(), inbound_tx);

        Ok((
            Self {
                child,
                stdin: Arc::new(Mutex::new(stdin)),
                next_id: AtomicI64::new(0),
                pending,
            },
            inbound_rx,
        ))
    }

    pub async fn request<P: Serialize, R: DeserializeOwned>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R, RpcError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let body = serde_json::to_vec(&RpcRequest {
            jsonrpc: "2.0",
            method,
            id,
            params,
        })?;
        self.write_line(&body).await?;

        let value = rx.await.map_err(|_| RpcError::Closed)??;
        Ok(serde_json::from_value(value)?)
    }

    pub async fn notify<P: Serialize>(&self, method: &str, params: P) -> Result<(), RpcError> {
        let body = serde_json::to_vec(&RpcNotification {
            jsonrpc: "2.0",
            method,
            params,
        })?;
        self.write_line(&body).await
    }

    /// 서버가 보낸 리퀘스트(승인 등)에 응답한다.
    pub async fn respond<R: Serialize>(&self, id: i64, result: &R) -> Result<(), RpcError> {
        let body = serde_json::to_vec(&RpcResultEnvelope {
            jsonrpc: "2.0",
            id,
            result,
        })?;
        self.write_line(&body).await
    }

    async fn write_line(&self, body: &[u8]) -> Result<(), RpcError> {
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(body).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    pub async fn shutdown(mut self) {
        let _ = self.notify("shutdown", serde_json::json!({})).await;
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
    }
}

fn spawn_reader_loop(
    stdout: ChildStdout,
    pending: Pending,
    inbound: mpsc::Sender<InboundMessage>,
) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        loop {
            match reader.next_line().await {
                Ok(Some(line)) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    let parsed: Result<RpcResponse, _> = serde_json::from_str(&line);
                    match parsed {
                        Ok(msg) => dispatch(msg, &pending, &inbound).await,
                        Err(e) => tracing::warn!("JSON 파싱 실패: {e}; 라인={line}"),
                    }
                }
                Ok(None) => {
                    tracing::info!("codex stdout EOF");
                    break;
                }
                Err(e) => {
                    tracing::warn!("codex stdout 읽기 오류: {e}");
                    break;
                }
            }
        }
        // sender drop → receiver는 None을 받으며 종료를 인지
    });
}

async fn dispatch(
    msg: RpcResponse,
    pending: &Pending,
    inbound: &mpsc::Sender<InboundMessage>,
) {
    if let Some(method) = msg.method {
        let params = msg.params.unwrap_or(serde_json::Value::Null);
        let inbound_msg = match msg.id {
            Some(id) => InboundMessage::Request { id, method, params },
            None => InboundMessage::Notification { method, params },
        };
        if inbound.send(inbound_msg).await.is_err() {
            tracing::debug!("inbound 수신자 dropped — 노티 폐기");
        }
        return;
    }

    if let Some(id) = msg.id {
        let mut map = pending.lock().await;
        if let Some(tx) = map.remove(&id) {
            let result = if let Some(err) = msg.error {
                Err(RpcError::Server {
                    code: err.code,
                    message: err.message,
                })
            } else {
                Ok(msg.result.unwrap_or(serde_json::Value::Null))
            };
            let _ = tx.send(result);
        }
    }
}
