//! 云游戏串流。
//!
//! 后端起一个看不见的浏览器加载云游戏页面，把画面按帧推给前端卡片，
//! 前端把鼠标/键盘/触摸事件发回来注入进去。登录也在同一路会话里
//! 完成——卡片上显示的就是那个浏览器本身，所以扫码、输密码都在
//! 卡片里进行。
//!
//! 为什么不能用 iframe：云游戏的官方页带 `X-Frame-Options: DENY`，
//! 实测浏览器会直接 `ERR_BLOCKED_BY_RESPONSE`，连渲染都不渲染。
//! 但那个头只约束 iframe —— 这里是独立的顶层浏览器窗口，它管不着。

pub mod browser;
pub mod cdp;
pub mod session;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::cloud::browser::Browser;
use crate::cloud::cdp::Cdp;
use crate::cloud::session::Session;
use crate::error::{AppError, AppResult};

/// 一个可以串流的站点。
///
/// 架构做成按名字查表，是为了以后加云崩铁、云异环时只改这里，
/// 不用动会话逻辑。
#[derive(Debug, Clone)]
pub struct Target {
    pub name: &'static str,
    pub title: &'static str,
    pub url: &'static str,
    /// 伪装成移动端。云游戏的触摸版在小窗口里更好操作，
    /// 桌面版布局在 480x320 里挤成一团。
    pub mobile_user_agent: &'static str,
}

pub const TARGETS: &[Target] = &[
    Target {
        name: "genshin",
        title: "云·原神",
        url: "https://ys.mihoyo.com/cloud/",
        mobile_user_agent: "Mozilla/5.0 (Linux; Android 12.0; Pixel 5) AppleWebKit/537.36 \
                            (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36",
    },
    // 下面两个还没验证过，先把入口占上。加的时候只需要确认
    // url 和移动版地址对不对，会话逻辑一行都不用改。
    Target {
        name: "starrail",
        title: "云·星穹铁道",
        url: "https://sr.mihoyo.com/cloud/",
        mobile_user_agent: "Mozilla/5.0 (Linux; Android 12.0; Pixel 5) AppleWebKit/537.36 \
                            (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36",
    },
    Target {
        name: "yihuan",
        title: "云·异环",
        url: "https://yh.wanmei.com/cloud/",
        mobile_user_agent: "Mozilla/5.0 (Linux; Android 12.0; Pixel 5) AppleWebKit/537.36 \
                            (KHTML, like Gecko) Chrome/120.0.0.0 Mobile Safari/537.36",
    },
];

pub fn find_target(name: &str) -> Option<&'static Target> {
    TARGETS.iter().find(|t| t.name == name)
}

/// 串流服务的全局状态。
///
/// 每个 target 同时只允许一路会话。云游戏一个实例就吃一个 CPU 核，
/// 而且官方对同账号多端登录有限制，允许多路只会互相踢下线。
pub struct CloudService {
    /// 浏览器可执行文件路径。找不到时为 None，相关接口返回 501。
    browser_exe: Option<PathBuf>,
    profile_root: PathBuf,
    /// target 名 → 正在跑的会话
    sessions: Mutex<HashMap<String, Arc<Session>>>,
    /// 分配调试端口用，从 base 递增
    next_port: std::sync::atomic::AtomicU16,
    port_base: u16,
    /// 默认画质
    pub default_quality: u8,
    pub default_max_width: u32,
    pub default_max_height: u32,
    /// 会话休眠多久之后彻底关掉（秒）。
    ///
    /// **这道超时才是真正释放内存的地方。** 休眠本身只是停发帧，
    /// 浏览器那约 700 MB 还占着（实测休眠 725 MB vs 醒着 764 MB）。
    ///
    /// 分界线就是「人还在不在」：切后台查个攻略是几十秒的事，切走去
    /// 吃饭就是几十分钟。前者保活，后者该把内存还回来。
    pub dormant_timeout_secs: u64,
}

impl CloudService {
    pub fn new(
        browser_exe: Option<PathBuf>,
        profile_root: PathBuf,
        port_base: u16,
        quality: u8,
        max_width: u32,
        max_height: u32,
        dormant_timeout_secs: u64,
    ) -> Self {
        Self {
            browser_exe,
            profile_root,
            sessions: Mutex::new(HashMap::new()),
            next_port: std::sync::atomic::AtomicU16::new(0),
            port_base,
            default_quality: quality,
            default_max_width: max_width,
            default_max_height: max_height,
            dormant_timeout_secs,
        }
    }

    pub fn available(&self) -> bool {
        self.browser_exe.is_some()
    }

    /// 某个 target 的会话是不是正跑着。
    ///
    /// 用 `try_lock`：这个只在列接口里用，拿不到锁说明别的请求正在
    /// 建/关会话，那就先报「没在跑」，没必要为了一行状态去排队等。
    pub fn is_running(&self, name: &str) -> bool {
        match self.sessions.try_lock() {
            Ok(m) => m.contains_key(name),
            Err(_) => false,
        }
    }

    /// 拿到（必要时新建）某个 target 的会话。
    ///
    /// 已有的会话直接复用——用户刷新页面不该让云游戏重开一次，
    /// 那意味着重新登录。
    pub async fn session(&self, target: &'static Target) -> AppResult<Arc<Session>> {
        let mut sessions = self.sessions.lock().await;

        if let Some(existing) = sessions.get(target.name) {
            if existing.is_alive().await {
                return Ok(Arc::clone(existing));
            }
            tracing::info!(target = target.name, "旧会话已失效，重建");
            sessions.remove(target.name);
        }

        let exe = self.browser_exe.as_ref().ok_or_else(|| {
            AppError::NotImplemented("云游戏串流（没找到 Edge 或 Chrome）".into())
        })?;

        // 端口从 base 往上发，找一个没被占的
        let port = self.alloc_port();
        let browser = Browser::launch(
            exe,
            port,
            &self.profile_root,
            target.name,
            self.default_max_width,
            self.default_max_height,
        )
        .await?;
        let cdp = Cdp::connect(&browser.ws_url).await?;

        let session = Session::start(
            target,
            browser,
            cdp,
            self.default_quality,
            self.default_max_width,
            self.default_max_height,
        )
        .await?;

        let session = Arc::new(session);
        sessions.insert(target.name.to_owned(), Arc::clone(&session));
        Ok(session)
    }

    fn alloc_port(&self) -> u16 {
        // 不用 fetch_add 的返回值直接当端口，因为可能被别的程序占了。
        // 这里只做轮转，真正的冲突由启动失败来暴露。
        let n = self
            .next_port
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.port_base + (n % 20)
    }

    /// 关掉某路会话。
    ///
    /// 从表里摘掉之后**无论如何都要 shutdown**，不能只处理
    /// `Arc::into_inner` 成功的情况：调用方（WebSocket 处理函数、
    /// 输入注入任务）手上通常还攥着 Arc 克隆，`into_inner` 会返回
    /// Err，那样浏览器进程就永远关不掉了。`shutdown` 内部是幂等的，
    /// 重复调没关系。
    pub async fn close(&self, name: &str) -> bool {
        let mut sessions = self.sessions.lock().await;
        let Some(s) = sessions.remove(name) else {
            return false;
        };
        s.shutdown().await;
        true
    }

    /// 关掉会话并清掉登录态。用户点「退出登录」时用。
    pub async fn forget(&self, name: &str) -> AppResult<()> {
        let mut sessions = self.sessions.lock().await;
        if let Some(s) = sessions.remove(name) {
            // 这里不能 return，反正后面还要删目录；
            // 而且就算 Arc 还被别处持有，也必须先把浏览器停掉，
            // 否则它握着 profile 目录不放，删目录会失败。
            s.shutdown_and_forget().await;
            return Ok(());
        }
        // 会话本来就没在跑，直接删目录
        let dir = self.profile_root.join(format!("profile-{name}"));
        if dir.exists() {
            std::fs::remove_dir_all(&dir)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("删配置目录失败：{e}")))?;
        }
        Ok(())
    }

    /// 关掉全部。服务退出时调用，避免留下孤儿浏览器进程。
    pub async fn close_all(&self) {
        let mut sessions = self.sessions.lock().await;
        for (_, s) in sessions.drain() {
            s.shutdown().await;
        }
    }

    /// 起一个后台任务，定期把「休眠太久没人回来」的会话关掉。
    ///
    /// 这是唯一真正释放内存的地方：休眠只停帧流，浏览器还在，
    /// 实测休眠时仍占约 725 MB。用户切后台查攻略是几十秒的事，
    /// 切走去吃饭就是几十分钟——这条线就是区分两者的。
    pub fn spawn_reaper(self: &Arc<Self>) {
        if self.dormant_timeout_secs == 0 {
            tracing::info!("休眠超时设为 0，会话不会被自动回收");
            return;
        }
        let me = Arc::clone(self);
        let timeout = std::time::Duration::from_secs(self.dormant_timeout_secs);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
            loop {
                tick.tick().await;

                // 先收集要关的，再逐个关——不能拿着 sessions 的锁去
                // await（close 也要拿同一把锁，会死锁）。
                let victims: Vec<(String, Arc<Session>)> = {
                    let guard = me.sessions.lock().await;
                    guard
                        .iter()
                        .filter(|(_, s)| s.is_dormant() && s.idle_for() >= timeout)
                        .map(|(k, s)| (k.clone(), Arc::clone(s)))
                        .collect()
                };

                for (name, s) in victims {
                    // 再确认一次还没醒：收集和关闭之间用户可能刚好
                    // 切回来了，那就别关了。
                    if !s.is_dormant() || s.idle_for() < timeout {
                        continue;
                    }
                    tracing::info!(
                        target = name,
                        idle_secs = s.idle_for().as_secs(),
                        "休眠超时，关掉会话"
                    );
                    me.close(&name).await;
                }
            }
        });
    }
}
