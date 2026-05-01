use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{oneshot, Mutex};

use crate::spawn::CodexBinary;

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

type Pending = Arc<Mutex<std::collections::HashMap<i64, oneshot::Sender<Result<serde_json::Value, RpcError>>>>>;

pub struct RpcClient {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: AtomicI64,
    pending: Pending,
    // M0 범위에선 server→client 노티/리퀘스트 소비자가 없다.
    // M1에서 mpsc 채널로 외부에 노출 예정.
}

impl RpcClient {
    pub async fn spawn_app_server(bin: &CodexBinary) -> Result<Self, RpcError> {
        let mut child = Command::new(&bin.path)
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(RpcError::Spawn)?;

        let stdin = child.stdin.take().ok_or(RpcError::NoStdio)?;
        let stdout = child.stdout.take().ok_or(RpcError::NoStdio)?;

        let pending: Pending = Arc::new(Mutex::new(std::collections::HashMap::new()));
        spawn_reader_loop(stdout, pending.clone());

        Ok(Self {
            child,
            stdin: Arc::new(Mutex::new(stdin)),
            next_id: AtomicI64::new(0),
            pending,
        })
    }

    pub async fn request<P: Serialize, R: DeserializeOwned>(
        &mut self,
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

    pub async fn notify<P: Serialize>(&mut self, method: &str, params: P) -> Result<(), RpcError> {
        let body = serde_json::to_vec(&RpcNotification {
            jsonrpc: "2.0",
            method,
            params,
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

fn spawn_reader_loop(stdout: ChildStdout, pending: Pending) {
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
                        Ok(msg) => dispatch(msg, &pending).await,
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
    });
}

async fn dispatch(msg: RpcResponse, pending: &Pending) {
    if let Some(method) = msg.method {
        // M1에서 채널로 외부에 노출. 그 전엔 디버그 로그로만 흘림.
        tracing::debug!(
            "inbound {} (id={:?}): {}",
            method,
            msg.id,
            msg.params.unwrap_or(serde_json::Value::Null)
        );
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
