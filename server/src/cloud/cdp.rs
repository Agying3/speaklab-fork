//! 极简 CDP（Chrome DevTools Protocol）客户端。
//!
//! 为什么不用 Playwright：我们要的是「启动浏览器 → 持续拿帧 → 回传输入」
//! 这条链路，Playwright 的进程外通信和它自带的帧缓冲反而碍事。
//! 直接说 CDP 更短、更可控，也少一个几百 MB 的依赖。
//!
//! 分工：
//!   `browser.rs`  启动 Edge、拿到 WebSocket 调试地址
//!   `cdp.rs`      （本文件）WebSocket 上的命令/事件收发
//!   `session.rs`  一路串流会话：起 screencast、转发帧、注入输入

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;

use crate::error::{AppError, AppResult};

/// 一条 CDP 连接。
///
/// 内部是一个「发送任务 + 接收任务」的结构：
/// 发出去的每个命令带一个自增 id，回包按 id 唤醒对应的等待者；
/// 事件（没有 id 的帧）走 broadcast 给所有订阅者。
#[derive(Clone)]
pub struct Cdp {
    inner: Arc<Inner>,
}

struct Inner {
    tx: mpsc::UnboundedSender<Message>,
    /// 命令 id → 等待回包的通道
    pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    next_id: AtomicU64,
    /// 事件订阅。用 broadcast 是因为 screencast 的帧要边收边发，
    /// 不能等消费者处理完才继续。
    events: tokio::sync::broadcast::Sender<Event>,
}

/// 一条 CDP 事件。`method` 是 `Page.screencastFrame` 这类名字。
#[derive(Debug, Clone)]
pub struct Event {
    pub method: String,
    pub params: Value,
    /// sessionId。直连目标时为空。
    pub session_id: Option<String>,
}

impl Cdp {
    /// 连上一个 WebSocket 调试地址（`ws://127.0.0.1:port/devtools/...`）。
    pub async fn connect(url: &str) -> AppResult<Self> {
        let (stream, _) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("连不上浏览器调试端口 {url}：{e}")))?;

        let (mut sink, mut source) = stream.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        // 容量给大一些：screencast 全速跑时每秒几十帧，接收端
        // （帧转发循环）还要做 base64 处理和 ack，偶尔落后几十帧
        // 很正常。容量太小会频繁 Lagged，虽然现在能正确跳过，
        // 但每次跳过都意味着丢帧。
        let (events, _) = tokio::sync::broadcast::channel(1024);

        // 写方向
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if sink.send(msg).await.is_err() {
                    break;
                }
            }
        });

        let inner = Arc::new(Inner {
            tx,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            events: events.clone(),
        });

        // 读方向：分派回包和事件
        let read_inner = Arc::clone(&inner);
        tokio::spawn(async move {
            while let Some(Ok(msg)) = source.next().await {
                let text = match msg {
                    Message::Text(t) => t,
                    Message::Binary(b) => match String::from_utf8(b) {
                        Ok(s) => s,
                        Err(_) => continue,
                    },
                    Message::Close(_) => break,
                    _ => continue,
                };

                let Ok(v) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };

                if let Some(id) = v.get("id").and_then(Value::as_u64) {
                    // 命令回包
                    if let Some(waiter) = read_inner.pending.lock().await.remove(&id) {
                        let _ = waiter.send(v);
                    }
                } else if let Some(method) = v.get("method").and_then(Value::as_str) {
                    // 事件。没人订阅时 send 会失败，忽略即可——
                    // 大多数事件我们本来就不关心。
                    let _ = read_inner.events.send(Event {
                        method: method.to_owned(),
                        params: v.get("params").cloned().unwrap_or(Value::Null),
                        session_id: v.get("sessionId").and_then(Value::as_str).map(str::to_owned),
                    });
                }
            }

            // 连接断了，唤醒所有还在等的命令，避免它们永久挂起
            let mut pending = read_inner.pending.lock().await;
            for (_, waiter) in pending.drain() {
                let _ = waiter.send(Value::Null);
            }
        });

        Ok(Self { inner })
    }

    /// 发一条命令并等回包。
    pub async fn call(&self, method: &str, params: Value) -> AppResult<Value> {
        self.call_on(method, params, None).await
    }

    /// 发一条命令到指定 session。
    pub async fn call_on(
        &self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> AppResult<Value> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().await.insert(id, tx);

        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(sid) = session_id {
            msg["sessionId"] = json!(sid);
        }

        self.inner
            .tx
            .send(Message::Text(msg.to_string()))
            .map_err(|_| AppError::Internal(anyhow::anyhow!("CDP 连接已关闭")))?;

        // 浏览器卡死时不能让请求永久挂着
        let reply = tokio::time::timeout(std::time::Duration::from_secs(20), rx)
            .await
            .map_err(|_| AppError::Internal(anyhow::anyhow!("CDP 命令 {method} 超时")))?
            .map_err(|_| AppError::Internal(anyhow::anyhow!("CDP 连接在等待 {method} 时断开")))?;

        if reply.is_null() {
            return Err(AppError::Internal(anyhow::anyhow!("CDP 连接在等待 {method} 时断开")));
        }

        // CDP 的错误回包长这样：{"id":1,"error":{"code":-32601,"message":"..."}}
        if let Some(err) = reply.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("未知错误");
            // 这里是排查会话启动问题最有效的落点：每条失败的命令
            // 都会留下方法名和浏览器给的原因。
            tracing::debug!(method, error = msg, "CDP 命令被拒绝");
            // 用 Upstream 而不是 Internal：这是浏览器拒绝了我们，
            // 不是服务自己的 bug，消息可以原样给前端看，方便现场排查。
            return Err(AppError::Upstream(format!("CDP {method} 被浏览器拒绝：{msg}")));
        }

        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }

    /// 订阅事件。返回的接收端只收到订阅之后的事件。
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }
}
