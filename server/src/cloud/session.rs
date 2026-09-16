//! 一路串流会话。
//!
//! 生命周期：
//!
//! ```text
//! Session::start
//!   ├─ Target.createTarget      开一个标签页
//!   ├─ Emulation.setUserAgent   伪装成安卓，骗出触摸版界面
//!   ├─ Page.navigate            打开云游戏
//!   └─ Page.startScreencast     开始吐帧
//!
//! 之后：
//!   浏览器 ──Page.screencastFrame──→ 订阅者（WebSocket 转发给前端）
//!   前端  ──Input.* ───────────────→ cdp.call（注入到浏览器）
//! ```
//!
//! 帧必须逐帧 ack，否则 Chrome 只发第一帧就不发了——这是 CDP 的
//! 流控设计，忘了 ack 会表现为「画面卡住不动」，很难查。

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use serde::Serialize;
use serde_json::json;
use tokio::sync::broadcast;

use crate::cloud::browser::Browser;
use crate::cloud::cdp::Cdp;
use crate::cloud::Target;
use crate::error::{AppError, AppResult};

/// 一帧画面。前端拿到后直接塞进 Image/canvas 显示。
#[derive(Debug, Clone, Serialize)]
pub struct Frame {
    /// base64 的 JPEG。CDP 直接给的就是这个格式，后端不做二次编码。
    pub data: String,
    /// CDP 给的时间戳（秒，单调时钟）。前端可以用它算实际的帧间隔。
    pub ts: f64,
    pub width: u32,
    pub height: u32,
}

/// 会话里发生的事。WebSocket 那一层把它序列化后发给前端。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outgoing {
    /// 会话就绪，附上初始信息。
    Ready {
        target: String,
        title: String,
        width: u32,
        height: u32,
    },
    /// 一帧画面。
    Frame(Frame),
    /// 页面标题变了。可以用它判断是否已经进到游戏里。
    Title { title: String },
    /// 出了错。前端可以显示提示。
    Error { message: String },
}

/// 等页面进入可渲染状态。
///
/// 判据是 `document.readyState` 到了 `interactive`。更早的状态下页面
/// 还没有活动的渲染表面，`Page.startScreencast` 会被浏览器拒绝。
///
/// 最多等 8 秒。超时不算失败——有的页面会卡在加载中（比如一直等一个
/// 慢请求），但主文档已经渲染出来了，照样能开帧流。所以超时就放行，
/// 真开不了 screencast 的话后面那一步会报错。
async fn wait_for_page_ready(cdp: &Cdp, session: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);

    while std::time::Instant::now() < deadline {
        let probe = cdp
            .call_on(
                "Runtime.evaluate",
                json!({ "expression": "document.readyState", "returnByValue": true }),
                Some(session),
            )
            .await;

        match probe {
            Ok(v) => {
                let state = v
                    .get("result")
                    .and_then(|r| r.get("value"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                if state == "interactive" || state == "complete" {
                    tracing::debug!(state, "页面已就绪");
                    return;
                }
            }
            // 求值失败通常意味着执行上下文还没建好，接着等
            Err(_) => {}
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    tracing::warn!("等页面就绪超时，仍然尝试开帧流");
}

pub struct Session {
    target: &'static Target,
    cdp: Cdp,
    /// 当前正在串流的标签页 sessionId。
    ///
    /// **这是会变的**。云游戏点「开始游戏」后会开新标签页（用户协议、
    /// 登录页、游戏本体），画面在那个新标签页里。如果死盯着最初那个，
    /// 用户点到一半画面就黑了。所以用 RwLock 包起来，由
    /// `follow_new_tabs` 那个循环在检测到新页面时换掉。
    page_session: std::sync::RwLock<String>,
    /// 整个浏览器窗口的目标 id。
    target_id: String,
    browser: Option<Browser>,
    /// 帧和事件的广播源。
    ///
    /// 容量给 8：前端消费得慢时宁可丢旧帧也不要堆积。
    /// 云游戏里「跳到最新一帧」永远比「按顺序补完旧的」正确。
    ///
    /// 注意广播**没有订阅者时 send 会失败**，这是 tokio 的行为。
    /// 帧的转发循环因此不能因为「没人看」就退出——否则用户刷新
    /// 页面之后就再也收不到帧了。见下面转发循环里的处理。
    tx: broadcast::Sender<Outgoing>,
    alive: Arc<AtomicBool>,
    frames_sent: Arc<AtomicU64>,
    /// 当前画面尺寸。输入坐标要按它换算，而它会被 `Resize` 改，
    /// 所以必须是内部可变——用原子量而不是 Cell，因为 `input`
    /// 有可能被并发调用（拖动时 start/move/end 会连发）。
    width: AtomicU32,
    height: AtomicU32,
}

impl Session {
    /// 当前页面 sessionId。
    fn sid(&self) -> String {
        self.page_session
            .read()
            .map(|g| g.clone())
            .unwrap_or_default()
    }
    pub async fn start(
        target: &'static Target,
        browser: Browser,
        cdp: Cdp,
        quality: u8,
        max_width: u32,
        max_height: u32,
    ) -> AppResult<Self> {
        // 建窗口。
        //
        // 这里直接给真实 URL，不能用 `about:blank` 再 `Page.navigate`：
        // 空白页在 CDP 里不算 "active page"，后面 `Page.startScreencast`
        // 会被拒（"Not attached to an active page"）。实测只有带真实
        // URL 建出来的目标才能开帧流。
        //
        // UA 和视口紧接着在 attach 之后设。页面这时还在加载，改 UA 是
        // 来得及的——先设 UA 再导航本来是为了这个，现在顺序反了但效果
        // 一样，因为导航请求发出前 UA 已经改好了。
        //
        // `newWindow: true` 是必须的：云游戏页面会申请全屏和指针锁定，
        // 在标签页里会被拒；独立窗口也才有自己的合成表面。
        // 无头模式下这个窗口本来就不会显示给用户。
        let created = cdp
            .call(
                "Target.createTarget",
                json!({
                    "url": target.url,
                    "newWindow": true,
                    "width": max_width,
                    "height": max_height,
                }),
            )
            .await?;
        let target_id = created
            .get("targetId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("createTarget 没返回 targetId")))?
            .to_owned();

        // 挂到目标上，之后的页面级命令都走这个 session
        let attached = cdp
            .call(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
            )
            .await?;
        let page_session = attached
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Internal(anyhow::anyhow!("attachToTarget 没返回 sessionId")))?
            .to_owned();

        // 各域要显式开，否则命令会报 "not enabled"。
        // 注意 Emulation 没有 enable 命令——它是一组独立指令，
        // 直接调就行。把它列进来会得到 was not found 而整个会话起不来。
        for domain in ["Page", "Runtime", "Network"] {
            cdp.call_on(&format!("{domain}.enable"), json!({}), Some(&page_session))
                .await?;
        }

        // 伪装成移动端。云游戏的触摸版布局在 480x320 里才用得起来，
        // 桌面版会挤成一团。
        cdp.call_on(
            "Emulation.setUserAgentOverride",
            json!({ "userAgent": target.mobile_user_agent }),
            Some(&page_session),
        )
        .await?;

        // 触摸支持。这是"能在小窗口里操作"的关键——
        // 移动版页面用触摸事件，鼠标事件它不认。
        cdp.call_on(
            "Emulation.setTouchEmulationEnabled",
            json!({ "enabled": true, "maxTouchPoints": 5 }),
            Some(&page_session),
        )
        .await?;

        // 视口固定成卡片尺寸。云游戏页面会按这个尺寸布局。
        cdp.call_on(
            "Emulation.setDeviceMetricsOverride",
            json!({
                "width": max_width,
                "height": max_height,
                "deviceScaleFactor": 1.0,
                "mobile": true,
            }),
            Some(&page_session),
        )
        .await?;

        let (tx, _) = broadcast::channel::<Outgoing>(8);
        let alive = Arc::new(AtomicBool::new(true));
        let frames_sent = Arc::new(AtomicU64::new(0));

        let session = Self {
            target,
            cdp: cdp.clone(),
            page_session: std::sync::RwLock::new(page_session.clone()),
            target_id: target_id.clone(),
            browser: Some(browser),
            tx: tx.clone(),
            alive: Arc::clone(&alive),
            frames_sent: Arc::clone(&frames_sent),
            width: AtomicU32::new(max_width),
            height: AtomicU32::new(max_height),
        };

        // 等页面真的变成 "active page" 再开帧流。
        //
        // 这里不能靠 sleep 猜时间。`createTarget` 给的是真实 URL，页面要
        // 经历一次网络请求；在它完成之前调 `Page.startScreencast` 会被
        // 拒："Not attached to an active page"。实测固定等 600ms 有时够、
        // 有时不够（网络慢的时候失败），压测里就复现了。
        //
        // 所以改成轮询 `document.readyState`：只要它到了 interactive
        // 以上，页面就有活动的渲染表面了。
        wait_for_page_ready(&cdp, &page_session).await;

        // 跟着新标签页走。
        //
        // **云游戏一定会开新标签页**：点「开始游戏」弹出用户协议、然后
        // 是登录页，这些都在新标签里。实测点一下之后浏览器里多出
        // `https://user.mihoyo.com/#/agreement`，而我们的 screencast
        // 还盯着最早那个首页——用户看到的就是点完没反应。
        //
        // 所以开 `Target.setDiscoverTargets`，每发现一个新的 page 目标
        // 就切过去：上一路停帧流，新的一路设 UA/视口/触摸并开帧流。
        let cdp_tabs = cdp.clone();
        let alive_tabs = Arc::clone(&alive);
        let initial_sid = page_session.clone();
        let browser_target_id = target_id.clone();
        // 这个 Arc 是给下面的自愈循环读「当前该盯哪个页面」用的
        let current_sid = Arc::new(std::sync::RwLock::new(page_session.clone()));
        let current_sid_writer = Arc::clone(&current_sid);

        cdp.call("Target.setDiscoverTargets", json!({ "discover": true }))
            .await?;

        let ua = target.mobile_user_agent;
        tokio::spawn(async move {
            let mut events = cdp_tabs.subscribe();
            loop {
                let ev = match events.recv().await {
                    Ok(ev) => ev,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                };
                if !alive_tabs.load(Ordering::Relaxed) {
                    break;
                }
                if ev.method != "Target.targetCreated" && ev.method != "Target.targetInfoChanged" {
                    continue;
                }

                let info = match ev.params.get("targetInfo") {
                    Some(i) => i,
                    None => continue,
                };
                // 只看标签页。其它类型（iframe、worker、浏览器本身）跳过。
                if info.get("type").and_then(|v| v.as_str()) != Some("page") {
                    continue;
                }
                let url = info.get("url").and_then(|v| v.as_str()).unwrap_or("");
                // about:blank 和 devtools 不算我们要跟的页面
                if url.is_empty() || url.starts_with("about:") || url.starts_with("devtools:")
                    || url.starts_with("edge:")
                {
                    continue;
                }
                let tid = match info.get("targetId").and_then(|v| v.as_str()) {
                    Some(t) => t.to_owned(),
                    None => continue,
                };
                // 已经跟在它后面了，不用重复处理
                {
                    let cur = current_sid_writer.read().map(|g| g.clone()).unwrap_or_default();
                    if cur == tid || cur == initial_sid && tid == browser_target_id {
                        continue;
                    }
                }

                // 挂到新标签页上
                let attached = cdp_tabs
                    .call(
                        "Target.attachToTarget",
                        json!({ "targetId": tid, "flatten": true }),
                    )
                    .await;
                let new_sid = match attached {
                    Ok(a) => match a.get("sessionId").and_then(|v| v.as_str()) {
                        Some(s) => s.to_owned(),
                        None => continue,
                    },
                    Err(e) => {
                        tracing::debug!(error = %e, url, "挂载新标签页失败");
                        continue;
                    }
                };

                tracing::info!(url, "跟随到新标签页");

                // 新页面要重新设一遍这些，它们是按 session 生效的
                for domain in ["Page", "Runtime"] {
                    let _ = cdp_tabs
                        .call_on(&format!("{domain}.enable"), json!({}), Some(&new_sid))
                        .await;
                }
                let _ = cdp_tabs
                    .call_on(
                        "Emulation.setUserAgentOverride",
                        json!({ "userAgent": ua }),
                        Some(&new_sid),
                    )
                    .await;

                // 记下来。自愈循环下一轮就会盯上这个新页面。
                if let Ok(mut w) = current_sid_writer.write() {
                    *w = new_sid.clone();
                }
                // 顺手把旧那路帧流停掉，免得两边抢
                let _ = cdp_tabs
                    .call_on("Page.stopScreencast", json!({}), Some(&initial_sid))
                    .await;
            }
        });

        // 帧流的自愈循环。
        //
        // **`Page.startScreencast` 不是一劳永逸的**：页面导航（云游戏加载时
        // 会从首页跳到登录页、再跳到游戏页）会让它失效，之后一帧都不出。
        // 这是压测里抓到的核心 bug——冷启动那条连接有 197 帧，断开重连
        // 之后全是 0 帧，注入输入也救不回来。
        //
        // 所以这里做成一个循环：开帧流 → 等它失效 → 重开。
        // 靠"多久没收到帧"来判断失效，而不是靠猜导航时机。
        //
        // 每轮都从 `current_sid` 重新读目标页面，这样跟随标签页之后
        // 帧流会自动开到新页面上。
        let cdp_sc = cdp.clone();
        let alive_sc = Arc::clone(&alive);
        let counter_sc = Arc::clone(&frames_sent);
        let sid_sc = Arc::clone(&current_sid);
        tokio::spawn(async move {
            let q = quality;
            let (mw, mh) = (max_width, max_height);

            let start = move |cdp: Cdp, sid: String| async move {
                cdp.call_on(
                    "Page.startScreencast",
                    json!({
                        "format": "jpeg",
                        "quality": q,
                        "maxWidth": mw,
                        "maxHeight": mh,
                        "everyNthFrame": 1,
                    }),
                    Some(&sid),
                )
                .await
            };

            let read_sid = |r: &Arc<std::sync::RwLock<String>>| {
                r.read().map(|g| g.clone()).unwrap_or_default()
            };

            if let Err(e) = start(cdp_sc.clone(), read_sid(&sid_sc)).await {
                tracing::warn!(error = %e, "首次开帧流失败");
            }

            // 每 2 秒检查一次：连续两轮没有新帧就重开。
            //
            // "一段时间没帧"不能直接等同于坏了——真正静止的页面确实
            // 可能几秒没有新帧（screencast 只在重绘时出帧）。但如果
            // 一直没帧又一直不重开，导航造成的失效就会变成永久黑屏。
            // 重开一次在页面静止时是空操作，代价可以接受。
            let mut last_seen = counter_sc.load(Ordering::Relaxed);
            let mut idle_ticks = 0u32;
            let mut last_sid = read_sid(&sid_sc);

            while alive_sc.load(Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if !alive_sc.load(Ordering::Relaxed) {
                    break;
                }

                // 目标页面可能已经换了（跟随新标签页），换了就立刻重开
                let cur_sid = read_sid(&sid_sc);
                let switched = cur_sid != last_sid;

                let now = counter_sc.load(Ordering::Relaxed);
                let producing = now != last_seen;
                if producing {
                    last_seen = now;
                }

                if switched {
                    tracing::debug!("目标页面已切换，把帧流挪过去");
                    last_sid = cur_sid.clone();
                    idle_ticks = 0;
                    // 停掉旧页面的，开新的
                    let _ = cdp_sc
                        .call_on("Page.stopScreencast", json!({}), Some(&cur_sid))
                        .await;
                    if let Err(e) = start(cdp_sc.clone(), cur_sid.clone()).await {
                        tracing::debug!(error = %e, "在新页面开帧流失败");
                    }
                    continue;
                }

                if producing {
                    idle_ticks = 0;
                    continue;
                }

                idle_ticks += 1;
                if idle_ticks < 2 {
                    continue;
                }

                tracing::debug!("帧流静止超过 4 秒，重开 screencast");
                // 先停再开：直接重开可能返回 "Screencast is already active"，
                // 而那个状态下其实是不出帧的。
                let _ = cdp_sc
                    .call_on("Page.stopScreencast", json!({}), Some(&cur_sid))
                    .await;
                match start(cdp_sc.clone(), cur_sid).await {
                    Ok(_) => tracing::debug!("帧流已重开"),
                    Err(e) => tracing::debug!(error = %e, "重开帧流失败，下次再试"),
                }
                idle_ticks = 0;
            }
        });

        // 帧的转发循环
        //
        // 不再按 sessionId 过滤：云游戏会开新标签页，帧可能来自任何一个
        // 我们跟随过的页面。所以这里只转发某一路 screencast 的帧——
        // 同一时刻只有一路 screencast 在跑（跟随新标签页时会先停旧的），
        // 因此不需要额外区分来源。
        let mut events = cdp.subscribe();
        let tx_frames = tx.clone();
        let cdp_frames = cdp.clone();
        let alive_frames = Arc::clone(&alive);
        let counter = Arc::clone(&frames_sent);

        tokio::spawn(async move {
            loop {
                // 不能用 `while let Ok(ev) = recv().await`：那样一遇到
                // Lagged（消费慢于生产）就退出循环，之后这个会话永远
                // 不再转发帧。Lagged 要显式跳过并继续。
                let ev = match events.recv().await {
                    Ok(ev) => ev,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!(skipped = n, "CDP 事件积压，跳过");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };

                if !alive_frames.load(Ordering::Relaxed) {
                    break;
                }

                match ev.method.as_str() {
                    "Page.screencastFrame" => {
                        let data = ev
                            .params
                            .get("data")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_owned();
                        let meta = ev.params.get("metadata").cloned().unwrap_or(json!({}));
                        let frame_session = ev
                            .params
                            .get("sessionId")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);

                        // 必须先 ack，否则浏览器只发这一帧就不发了。
                        // 表现为「画面冻住」，是这套机制最容易踩的坑。
                        //
                        // ack 要回到**发这一帧的那个 session**，不能写死
                        // 成最初那个页面——跟随新标签页之后帧是从新页面
                        // 来的，ack 回错地方等于没 ack。
                        let cdp_ack = cdp_frames.clone();
                        let sid_ack = ev.session_id.clone();
                        tokio::spawn(async move {
                            let _ = cdp_ack
                                .call_on(
                                    "Page.screencastFrameAck",
                                    json!({ "sessionId": frame_session }),
                                    sid_ack.as_deref(),
                                )
                                .await;
                        });

                        if data.is_empty() {
                            continue;
                        }

                        counter.fetch_add(1, Ordering::Relaxed);
                        // 没人看时 send 会失败，正常，忽略
                        let _ = tx_frames.send(Outgoing::Frame(Frame {
                            data,
                            ts: meta
                                .get("timestamp")
                                .and_then(|v| v.as_f64())
                                .unwrap_or(0.0),
                            width: meta
                                .get("deviceWidth")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as u32,
                            height: meta
                                .get("deviceHeight")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as u32,
                        }));
                    }
                    "Page.frameNavigated" => {
                        // 页面跳转时 CDP 会自动继续发帧，不用重开 screencast
                        tracing::debug!(target = target.name, "页面跳转");
                    }
                    _ => {}
                }
            }
            tracing::debug!(target = target.name, "帧转发循环结束");
        });

        // 标题变化的循环，单独一条，因为它不需要跟着帧一起处理。
        // 同样不按 session 过滤——我们关心所有跟随过的页面的跳转。
        let mut events_title = cdp.subscribe();
        let tx_title = tx.clone();
        tokio::spawn(async move {
            loop {
                // 同样要处理 Lagged，理由见上面帧循环的注释
                let ev = match events_title.recv().await {
                    Ok(ev) => ev,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if ev.method == "Page.frameNavigated" {
                    // 只看主框架，iframe 的跳转没意义
                    let is_main = ev
                        .params
                        .get("frame")
                        .and_then(|f| f.get("parentId"))
                        .map(|p| p.is_null())
                        .unwrap_or(true);
                    if !is_main {
                        continue;
                    }
                    if let Some(url) = ev
                        .params
                        .get("frame")
                        .and_then(|f| f.get("url"))
                        .and_then(|v| v.as_str())
                    {
                        let _ = tx_title.send(Outgoing::Title {
                            title: url.to_owned(),
                        });
                    }
                }
            }
        });

        let _ = tx.send(Outgoing::Ready {
            target: target.name.to_owned(),
            title: target.title.to_owned(),
            width: max_width,
            height: max_height,
        });

        tracing::info!(target = target.name, url = target.url, "云游戏会话已启动");
        Ok(session)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Outgoing> {
        self.tx.subscribe()
    }

    pub fn size(&self) -> (u32, u32) {
        (
            self.width.load(Ordering::Relaxed),
            self.height.load(Ordering::Relaxed),
        )
    }

    pub fn frames_sent(&self) -> u64 {
        self.frames_sent.load(Ordering::Relaxed)
    }

    pub async fn is_alive(&self) -> bool {
        if !self.alive.load(Ordering::Relaxed) {
            return false;
        }
        // 真正探一次，别只信标志位——浏览器可能已经崩了
        let sid = self.sid();
        self.cdp
            .call_on("Runtime.evaluate", json!({ "expression": "1" }), Some(&sid))
            .await
            .is_ok()
    }

    /// 注入输入事件。
    ///
    /// 坐标用 0..1 的归一化值，由调用方换算好——因为前端卡片尺寸
    /// 和后端浏览器视口尺寸不一定一样，在前端做归一化最省事，
    /// 后端只管乘回去。
    pub async fn input(&self, event: InputEvent) -> AppResult<()> {
        // 读一次就够，中途 Resize 也不用重读——一个事件对应一个坐标。
        let (w, h) = self.size();
        let (w, h) = (w as f64, h as f64);
        // 目标页面可能是后来跟过来的那个（云游戏会开新标签页），
        // 所以每个事件都重新取一次 sessionId。
        let sid = self.sid();

        match event {
            InputEvent::Touch {
                kind,
                x,
                y,
                id,
            } => {
                let px = (x.clamp(0.0, 1.0) * w).round();
                let py = (y.clamp(0.0, 1.0) * h).round();
                let ty = match kind.as_str() {
                    "start" => "touchStart",
                    "move" => "touchMove",
                    "end" => "touchEnd",
                    "cancel" => "touchCancel",
                    _ => return Err(AppError::BadRequest(format!("未知的触摸类型 {kind}"))),
                };

                // touchEnd/cancel 的 touchPoints 必须是空的，
                // 否则页面会认为手指还按着。
                let points = if ty == "touchEnd" || ty == "touchCancel" {
                    json!([])
                } else {
                    json!([{ "x": px, "y": py, "id": id }])
                };

                self.cdp
                    .call_on(
                        "Input.dispatchTouchEvent",
                        json!({ "type": ty, "touchPoints": points }),
                        Some(&sid),
                    )
                    .await?;
            }
            InputEvent::Mouse {
                kind,
                x,
                y,
                button,
            } => {
                let px = (x.clamp(0.0, 1.0) * w).round();
                let py = (y.clamp(0.0, 1.0) * h).round();
                let ty = match kind.as_str() {
                    "down" => "mousePressed",
                    "up" => "mouseReleased",
                    "move" => "mouseMoved",
                    _ => return Err(AppError::BadRequest(format!("未知的鼠标类型 {kind}"))),
                };
                let btn = button.as_deref().unwrap_or("left");
                let mut params = json!({
                    "type": ty, "x": px, "y": py,
                    "button": btn, "clickCount": if ty == "mouseMoved" { 0 } else { 1 },
                });
                // 只有按下/抬起需要 buttons 位掩码，move 时带上会让
                // 页面以为是拖拽。
                if ty != "mouseMoved" {
                    params["buttons"] = json!(if btn == "left" { 1 } else { 0 });
                }
                self.cdp
                    .call_on("Input.dispatchMouseEvent", params, Some(&sid))
                    .await?;
            }
            InputEvent::Key {
                kind,
                key,
                code,
                vk,
            } => {
                let ty = match kind.as_str() {
                    "down" => "keyDown",
                    "up" => "keyUp",
                    _ => return Err(AppError::BadRequest(format!("未知的按键类型 {kind}"))),
                };
                self.cdp
                    .call_on(
                        "Input.dispatchKeyEvent",
                        json!({
                            "type": ty,
                            "key": key,
                            "code": code,
                            "windowsVirtualKeyCode": vk,
                            "nativeVirtualKeyCode": vk,
                        }),
                        Some(&sid),
                    )
                    .await?;
            }
            InputEvent::Text { text } => {
                // 扫码/输密码时要能打字。用 insertText 而不是逐个
                // dispatchKeyEvent，这样输入法、密码框都能正常工作。
                self.cdp
                    .call_on("Input.insertText", json!({ "text": text }), Some(&sid))
                    .await?;
            }
            InputEvent::Scroll { x, y, dx, dy } => {
                let px = (x.clamp(0.0, 1.0) * w).round();
                let py = (y.clamp(0.0, 1.0) * h).round();
                self.cdp
                    .call_on(
                        "Input.dispatchMouseEvent",
                        json!({
                            "type": "mouseWheel", "x": px, "y": py,
                            "deltaX": dx, "deltaY": dy,
                        }),
                        Some(&sid),
                    )
                    .await?;
            }
            InputEvent::Resize { width, height } => {
                // 前端卡片尺寸变了，改视口。
                //
                // 帧流这边不用手动重开：改了视口尺寸页面会重绘，
                // 自愈循环看到有新帧就不会动；万一 screencast 因此
                // 失效，最多 4 秒后自愈循环会把它重开（用的是
                // 创建时的尺寸上限，对 480x320 这种量级够用）。
                let nw = width.clamp(160, 1920);
                let nh = height.clamp(120, 1080);
                self.width.store(nw, Ordering::Relaxed);
                self.height.store(nh, Ordering::Relaxed);
                self.cdp
                    .call_on(
                        "Emulation.setDeviceMetricsOverride",
                        json!({
                            "width": nw, "height": nh,
                            "deviceScaleFactor": 1.0, "mobile": true,
                        }),
                        Some(&sid),
                    )
                    .await?;
            }
        }
        Ok(())
    }

    /// 让页面重新加载。用户在卡片上点「重来」时用。
    pub async fn reload(&self) -> AppResult<()> {
        let sid = self.sid();
        self.cdp
            .call_on("Page.reload", json!({}), Some(&sid))
            .await?;
        Ok(())
    }

    /// 关掉会话：停帧流、关标签页、杀浏览器。**保留登录态。**
    pub async fn shutdown(self) {
        self.shutdown_inner(false).await;
    }

    /// 同上，但连登录态一起清掉。
    pub async fn shutdown_and_forget(self) {
        self.shutdown_inner(true).await;
    }

    async fn shutdown_inner(self, forget_login: bool) {
        self.alive.store(false, Ordering::Relaxed);
        let sid = self.sid();
        let _ = self
            .cdp
            .call_on("Page.stopScreencast", json!({}), Some(&sid))
            .await;
        let _ = self
            .cdp
            .call("Target.closeTarget", json!({ "targetId": self.target_id }))
            .await;

        if let Some(browser) = self.browser {
            if forget_login {
                browser.shutdown_and_forget().await;
            } else {
                browser.shutdown().await;
            }
        }
        tracing::info!(target = self.target.name, forget_login, "云游戏会话已关闭");
    }
}

/// 前端发来的输入事件。
///
/// `x`/`y` 是 0..1 的归一化坐标。前端按自己的显示尺寸算好再发，
/// 后端不用关心前端的实际像素尺寸。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputEvent {
    Touch {
        kind: String,
        x: f64,
        y: f64,
        #[serde(default)]
        id: i64,
    },
    Mouse {
        kind: String,
        x: f64,
        y: f64,
        #[serde(default)]
        button: Option<String>,
    },
    Key {
        kind: String,
        key: String,
        #[serde(default)]
        code: String,
        #[serde(default)]
        vk: i64,
    },
    Text {
        text: String,
    },
    Scroll {
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
    },
    Resize {
        width: u32,
        height: u32,
    },
}
