//! codex(OpenAI Responses API 풀 형식) ↔ vLLM(단순 형식) HTTP 변환 프록시.
//!
//! 사용:
//!     cargo run -p stcode-vllm-proxy
//!     # env: STCODE_VLLM_UPSTREAM=http://100.105.145.6:8000 (default)
//!     #      STCODE_VLLM_PROXY_PORT=8001 (default)
//!
//! Codex 측에서:
//!     codex -c model_providers.local-vllm.base_url=http://localhost:8001/v1 ...
//!
//! 변환 규칙 (`POST /v1/responses` 본문의 `input` 배열만):
//!  - 각 message의 `content`가 `[{"type": "input_text", "text": "..."}, ...]` 형태면
//!    text를 concat해 `content: "..."` 단순 string으로
//!  - message 자체의 `type` 필드 제거 (vLLM이 인식 못함)
//!  - role/기타 필드는 그대로 보존 (developer는 vLLM chat template에서 system 매핑)
//!
//! 그 외 path는 raw forward.

use std::env;
use std::net::SocketAddr;

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::Response,
    routing::{any, post},
    Json, Router,
};
use reqwest::Client;
use serde_json::Value;

#[derive(Clone)]
struct AppState {
    upstream: String,
    client: Client,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,stcode_vllm_proxy=debug".into()),
        )
        .init();

    let upstream = env::var("STCODE_VLLM_UPSTREAM")
        .unwrap_or_else(|_| "http://100.105.145.6:8000".into());
    let port: u16 = env::var("STCODE_VLLM_PROXY_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8001);

    let state = AppState {
        upstream: upstream.trim_end_matches('/').to_string(),
        client: Client::builder()
            .pool_idle_timeout(std::time::Duration::from_secs(60))
            .build()?,
    };

    let app = Router::new()
        .route("/v1/responses", post(handle_responses))
        .fallback(any(forward_raw))
        .with_state(state.clone());

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("listening on {addr} → upstream {}", state.upstream);
    axum::serve(listener, app).await?;
    Ok(())
}

/// `/v1/responses` 요청을 변환 후 upstream에 forward, 응답을 streaming으로 반환.
async fn handle_responses(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(mut body): Json<Value>,
) -> Result<Response, (StatusCode, String)> {
    let before = body.get("input").map(|v| v.to_string().chars().count()).unwrap_or(0);
    transform_input(&mut body);
    let after = body.get("input").map(|v| v.to_string().chars().count()).unwrap_or(0);
    tracing::debug!("input transform: {before} → {after} chars");

    let url = format!("{}/v1/responses", state.upstream);
    let mut req = state.client.post(&url);
    for (k, v) in headers.iter() {
        // hop-by-hop / 자동 헤더 제외
        if !is_hop_header(k.as_str()) {
            req = req.header(k, v);
        }
    }
    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream error: {e}")))?;

    forward_response(resp).await
}

/// Catch-all forward — body/headers 그대로.
async fn forward_raw(
    State(state): State<AppState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, (StatusCode, String)> {
    let path_q = uri.path_and_query().map(|p| p.as_str()).unwrap_or("");
    let url = format!("{}{}", state.upstream, path_q);
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("read body: {e}")))?;

    let mut req = state.client.request(method, &url).body(bytes);
    for (k, v) in headers.iter() {
        if !is_hop_header(k.as_str()) {
            req = req.header(k, v);
        }
    }
    let resp = req
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("upstream error: {e}")))?;

    forward_response(resp).await
}

async fn forward_response(resp: reqwest::Response) -> Result<Response, (StatusCode, String)> {
    let status = StatusCode::from_u16(resp.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let mut response_headers = resp.headers().clone();
    response_headers.remove("content-length");
    response_headers.remove("transfer-encoding");

    let stream = resp.bytes_stream();
    let body = Body::from_stream(stream);

    let mut builder = Response::builder().status(status);
    for (k, v) in response_headers.iter() {
        builder = builder.header(k, v);
    }
    builder
        .body(body)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("build response: {e}")))
}

fn is_hop_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host"
            | "content-length"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// `input: [{type:"message", role, content:[{type:"input_text", text}, ...]}]` →
/// `input: [{role, content:"..."}]`. message 외 타입(reasoning/function_call 등)은
/// vLLM이 모르므로 drop.
fn transform_input(body: &mut Value) {
    let Some(input) = body.get_mut("input") else { return };
    let Some(arr) = input.as_array_mut() else { return };

    // 통과/제거 통계
    let mut kept = Vec::new();
    let mut dropped: Vec<String> = Vec::new();

    for msg in arr.drain(..) {
        let Value::Object(mut obj) = msg else {
            dropped.push("non-object".into());
            continue;
        };
        let mtype = obj
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !mtype.is_empty() && mtype != "message" {
            dropped.push(mtype);
            continue;
        }
        obj.remove("type");
        // role 없으면 vLLM이 거절 — drop
        if !obj.contains_key("role") {
            dropped.push("no-role".into());
            continue;
        }
        // content 변환
        if let Some(content) = obj.get_mut("content") {
            if let Some(content_arr) = content.as_array() {
                let mut text = String::new();
                for piece in content_arr {
                    if let Some(t) = piece.get("text").and_then(|v| v.as_str()) {
                        text.push_str(t);
                    }
                    // input_image 등 비-텍스트는 일단 무시 (v2)
                }
                *content = Value::String(text);
            }
        }
        kept.push(Value::Object(obj));
    }
    *arr = kept;
    if !dropped.is_empty() {
        tracing::debug!("dropped input items: {:?}", dropped);
    }
    tracing::debug!("kept {} message items", arr.len());
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn flattens_content_array() {
        let mut body = json!({
            "model": "x",
            "input": [
                {"type": "message", "role": "developer",
                 "content": [{"type": "input_text", "text": "hi"}, {"type": "input_text", "text": " there"}]},
                {"type": "message", "role": "user",
                 "content": [{"type": "input_text", "text": "yo"}]}
            ]
        });
        transform_input(&mut body);
        assert_eq!(
            body["input"],
            json!([
                {"role": "developer", "content": "hi there"},
                {"role": "user", "content": "yo"}
            ])
        );
    }

    #[test]
    fn leaves_string_content_alone() {
        let mut body = json!({"input": [{"role": "user", "content": "ok"}]});
        transform_input(&mut body);
        assert_eq!(body["input"], json!([{"role": "user", "content": "ok"}]));
    }
}
