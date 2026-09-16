//! 云游戏串流接口。
//!
//! `GET /api/v1/cloud/:target/ws` 升级成 WebSocket，之后这一条连接
//! 同时承载两个方向：
//!
//! ```text
//! 后端 → 前端   JSON 文本：{"type":"frame","data":"<base64 jpeg>"} 等
//! 前端 → 后端   JSON 文本：{"type":"touch","kind":"start","x":0.5,"y":0.5}
//! ```
//!
//! 帧走文本而不是二进制，是因为要跟输入事件复用同一条连接。
//! base64 会多 33% 体积，但在局域网这个量级下（约 1 Mbps）
//! 换来「一条连接搞定双向」是划算的。

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;

use crate::cloud::session::{InputEvent, Outgoing};
use crate::cloud::{find_target, TARGETS};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// 列出可用的云游戏。
#[derive(Serialize)]
pub struct TargetInfo {
    pub name: &'static str,
    pub title: &'static str,
    /// 该站点是否已经在跑会话。
    pub running: bool,
}

#[derive(Serialize)]
pub struct CloudList {
    /// 云游戏功能是否可用（找到了浏览器才算）。
    pub available: bool,
    /// 当前画面尺寸，前端可以据此设卡片比例。
    pub width: u32,
    pub height: u32,
    pub targets: Vec<TargetInfo>,
}

pub async fn list(State(state): State<AppState>) -> Json<CloudList> {
    let cfg = state.config();
    Json(CloudList {
        available: state.cloud().available(),
        width: cfg.cloud.max_width,
        height: cfg.cloud.max_height,
        targets: TARGETS
            .iter()
            .map(|t| TargetInfo {
                name: t.name,
                title: t.title,
                running: false, // 列表接口不查状态，避免为了显示去启动浏览器
            })
            .collect(),
    })
}

/// 主动关掉某路会话。用户点「退出游戏」时调。**保留登录态。**
pub async fn stop(State(state): State<AppState>, Path(target): Path<String>) -> AppResult<Json<serde_json::Value>> {
    let t = find_target(&target)
        .ok_or_else(|| AppError::NotFound(format!("云游戏 {target}")))?;
    let closed = state.cloud().close(t.name).await;
    Ok(Json(serde_json::json!({ "closed": closed })))
}

/// 退出登录：关掉会话并清掉配置目录里的 cookie。
///
/// 跟 `stop` 分开是有意的。`stop` 只是关掉这次会话，下次进来还是
/// 登录状态（因为配置目录按 target 固定保留）；这个接口是把登录
/// 态真的抹掉，下次得重新扫码。
pub async fn forget(State(state): State<AppState>, Path(target): Path<String>) -> AppResult<Json<serde_json::Value>> {
    let t = find_target(&target)
        .ok_or_else(|| AppError::NotFound(format!("云游戏 {target}")))?;
    state.cloud().forget(t.name).await?;
    Ok(Json(serde_json::json!({ "forgotten": true })))
}

/// WebSocket 入口。
pub async fn ws(
    State(state): State<AppState>,
    Path(target): Path<String>,
    upgrade: WebSocketUpgrade,
) -> AppResult<impl IntoResponse> {
    let t = find_target(&target)
        .ok_or_else(|| AppError::NotFound(format!("云游戏 {target}")))?;

    if !state.cloud().available() {
        return Err(AppError::NotImplemented(
            "云游戏串流（没找到 Edge 或 Chrome，或用 SPEAKLAB_BROWSER 指定路径）".into(),
        ));
    }

    Ok(upgrade.on_upgrade(move |socket| handle(socket, state, t)))
}

async fn handle(socket: WebSocket, state: AppState, target: &'static crate::cloud::Target) {
    let (mut sink, mut stream) = socket.split();

    // 起会话。这一步要启动浏览器、等调试端口、开帧流，
    // 通常 1~3 秒。失败就把原因发回去，不要让前端干等。
    let session = match state.cloud().session(target).await {
        Ok(s) => s,
        Err(e) => {
            // 这条日志很重要：客户端只会收到「服务内部错误」，
            // 真正的原因只在这里。会话启动涉及启动浏览器、连 CDP、
            // 发十来个命令，没日志基本没法查。
            tracing::error!(target = target.name, error = %e, "云游戏会话启动失败");
            let msg = serde_json::json!({
                "type": "error",
                "message": e.to_string(),
            });
            let _ = sink.send(Message::Text(msg.to_string())).await;
            return;
        }
    };

    let (w, h) = session.size();

    // 先告诉前端会话信息，它据此设置画布比例
    let ready = serde_json::json!({
        "type": "ready",
        "target": target.name,
        "title": target.title,
        "width": w,
        "height": h,
    });
    if sink.send(Message::Text(ready.to_string())).await.is_err() {
        return;
    }

    // 两条方向各跑一个任务，任一条结束就整体结束。
    //
    // 发送方向：订阅广播，把帧和事件写出去。
    // 广播容量只有 8，前端消费慢时旧帧会被丢掉——这是有意的，
    // 云游戏里「跳到最新一帧」永远比「补完积压的旧帧」正确。
    let mut frames = session.subscribe();
    let mut send_task = tokio::spawn(async move {
        loop {
            match frames.recv().await {
                Ok(out) => {
                    let text = match serde_json::to_string(&out) {
                        Ok(t) => t,
                        Err(_) => continue,
                    };
                    if sink.send(Message::Text(text)).await.is_err() {
                        break;
                    }
                }
                // 落后于广播（消费慢），跳过丢失的帧继续
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::debug!(skipped = n, "帧积压，跳过旧帧");
                    continue;
                }
                Err(_) => break,
            }
        }
    });

    // 接收方向：解析前端发来的输入事件，注入浏览器。
    let session_for_input = std::sync::Arc::clone(&session);
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = stream.next().await {
            let text = match msg {
                Message::Text(t) => t,
                Message::Close(_) => break,
                Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => continue,
            };

            match serde_json::from_str::<InputEvent>(&text) {
                Ok(ev) => {
                    if let Err(e) = session_for_input.input(ev).await {
                        // 输入注入失败不该断掉整条会话——
                        // 比如快速拖动时中间某个坐标越界，报错但继续。
                        tracing::debug!(error = %e, "输入注入失败");
                    }
                }
                Err(e) => {
                    tracing::debug!(error = %e, raw = %text.chars().take(120).collect::<String>(), "无法解析的输入事件");
                }
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }

    tracing::debug!(target = target.name, "WebSocket 会话结束");
}

/// 给「未处理的错误」留个统一的出口，避免 Outgoing::Error 没人用。
#[allow(dead_code)]
fn error_frame(message: impl Into<String>) -> String {
    serde_json::to_string(&Outgoing::Error {
        message: message.into(),
    })
    .unwrap_or_else(|_| r#"{"type":"error","message":"序列化失败"}"#.to_owned())
}
