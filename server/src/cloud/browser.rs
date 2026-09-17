//! 浏览器进程管理。
//!
//! 启动一个 Edge/Chrome，用 CDP 连上去，把它当作串流的渲染源。
//!
//! **必须是 `--headless=new`**。这里踩过坑，记一下：
//!
//! 最初的想法是「开一个有头窗口，挪到屏幕外（坐标 -32000）用户就看不见了」。
//! 实测**完全不出帧**：Chromium 对屏幕外的窗口不做合成，
//! `Page.startScreencast` 只会给一张首帧快照，之后永远静止。
//! 想靠 `--disable-features=CalculateNativeWinOcclusion` 之类的开关
//! 绕过也不行，仍然只有 1 帧。
//!
//! 窗口留在屏幕内倒是能出帧，但只有 13 帧/5 秒——被当成后台窗口节流了，
//! 而且用户看得见。
//!
//! 换到 `--headless=new` 就对了：完全不可见，且有完整的合成管线，
//! 实测 91 帧/5 秒，是屏幕内普通窗口的七倍。
//!
//! （旧版 `--headless` 也能出帧，但缺一些现代特性；`new` 是 Edge 和
//! Chrome 现在的默认无头实现，优先用它。）
//!
//! 顺带一提，无头还绕开了 `X-Frame-Options`：云游戏的官方页明确禁止
//! 被 iframe 嵌入，但这里不是 iframe，是一个完整的顶层浏览器窗口，
//! 那个头管不着。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::process::{Child, Command};

use crate::error::{AppError, AppResult};

/// 一个正在跑的浏览器实例。
pub struct Browser {
    /// `Option` 是为了让 `shutdown` 能收 `&self`：tokio 的 `Child::kill`
    /// 要 `&mut`，而调用方（`Session::shutdown`）手上只有共享引用。
    /// 关掉之后置 `None`，重复调用就是空操作——关会话这动作本身幂等。
    child: std::sync::Mutex<Option<Child>>,
    /// CDP 的浏览器级 WebSocket 地址。
    pub ws_url: String,
    /// 用户数据目录。每次启动用独立的目录，避免和用户自己的
    /// 浏览器配置打架（同一个目录被两个进程打开会直接失败）。
    profile_dir: PathBuf,
    port: u16,
}

impl Browser {
    /// 起一个新实例。
    ///
    /// `max_w`/`max_h` 只是初始窗口尺寸，后面 `Emulation.setDeviceMetricsOverride`
    /// 会按卡片实际尺寸再改一次。这里给对可以减少一次重排。
    /// `slot` 是这个 target 的名字（如 `genshin`）。它决定配置目录，
    /// 从而决定登录态是否跨会话保留。
    pub async fn launch(
        exe: &Path,
        port: u16,
        profile_root: &Path,
        slot: &str,
        max_w: u32,
        max_h: u32,
    ) -> AppResult<Self> {
        if !exe.exists() {
            return Err(AppError::NotImplemented(format!(
                "浏览器（找不到 {}）",
                exe.display()
            )));
        }

        // 配置目录按 target 固定，不按端口。这样重启服务、换端口之后
        // 还是同一个目录，登录态（cookie）留着，用户不用每次重新登录。
        //
        // 代价是账号凭证会落在磁盘上。下面 `shutdown` 里有一份说明。
        let profile_dir = profile_root.join(format!("profile-{slot}"));
        std::fs::create_dir_all(&profile_dir).map_err(|e| {
            AppError::Internal(anyhow::anyhow!("建不了浏览器配置目录：{e}"))
        })?;

        let mut cmd = Command::new(exe);
        cmd.arg(format!("--remote-debugging-port={port}"))
            .arg(format!("--user-data-dir={}", profile_dir.display()))
            // 新版无头。它有完整的合成器，screencast 能拿到帧；
            // 而「有头但挪到屏幕外」的窗口一帧都不出（见文件头注释）。
            .arg("--headless=new")
            // 无头下窗口尺寸仍然要定，页面按这个尺寸布局
            .arg(format!("--window-size={max_w},{max_h}"))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-features=Translate,MediaRouter")
            // 云游戏里音频要能自动播，否则用户一进来是静音的
            .arg("--autoplay-policy=no-user-gesture-required")
            // 无头下有些编码器路径不一样，显式打开软件回退，
            // 免得 H.264 流解不出来变成黑屏
            .arg("--enable-features=WebRTC-H264WithOpenH264FFmpeg")
            // 别让"恢复上次会话"弹窗挡住画面
            .arg("--hide-crash-restore-bubble")
            .arg("about:blank")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let child = cmd
            .spawn()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("启动浏览器失败：{e}")))?;

        // 等调试端口起来。浏览器从启动到监听端口通常 0.5~2 秒，
        // 这里最多等 15 秒，超时说明它没起来。
        let ws_url = wait_for_devtools(port).await?;

        Ok(Self {
            child: std::sync::Mutex::new(Some(child)),
            ws_url,
            profile_dir,
            port,
        })
    }

    /// 关掉浏览器进程，**保留**配置目录。
    ///
    /// 保留是有意的：配置目录里存着登录 cookie，留着下次启动就免登。
    /// 云游戏的登录要扫码/收短信，每次重来一遍太烦。
    ///
    /// 代价要说清楚：**账号凭证会留在磁盘上**（`profile_root` 下面）。
    /// 想清干净就删那个目录，或者调 `DELETE /api/v1/cloud/:target/profile`。
    /// 这是本机自部署的东西，凭证存在自己机器上可以接受；但如果哪天
    /// 要把它部署到公网，这里必须先改成不落盘。
    pub async fn shutdown(&self) {
        let child = self.child.lock().ok().and_then(|mut g| g.take());
        let Some(mut child) = child else {
            return; // 已经关过了
        };
        let _ = child.kill().await;
        // 等进程真正退出。不删目录，但得等它退出，否则下次启动
        // 会因为配置目录被占用而失败。
        let _ = child.wait().await;
    }

    /// 连配置目录一起删掉。用户在界面上点「退出登录」时用。
    pub async fn shutdown_and_forget(&self) {
        let child = self.child.lock().ok().and_then(|mut g| g.take());
        if let Some(mut child) = child {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        if let Err(e) = std::fs::remove_dir_all(&self.profile_dir) {
            tracing::warn!(dir = %self.profile_dir.display(), error = %e, "清理浏览器配置目录失败");
        }
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn profile_dir(&self) -> &Path {
        &self.profile_dir
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // kill_on_drop 负责杀进程。这里**不**删配置目录——那是登录态。
    }
}

/// 轮询 `http://127.0.0.1:port/json/version` 直到拿到 WebSocket 地址。
async fn wait_for_devtools(port: u16) -> AppResult<String> {
    let url = format!("http://127.0.0.1:{port}/json/version");
    let client = reqwest_get_client()?;

    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let mut last_err = String::from("未知");

    while std::time::Instant::now() < deadline {
        match fetch_version(&client, &url).await {
            Ok(v) => {
                if let Some(ws) = v.get("webSocketDebuggerUrl").and_then(Value::as_str) {
                    tracing::info!(port, "浏览器调试端口已就绪");
                    return Ok(ws.to_owned());
                }
                last_err = "响应里没有 webSocketDebuggerUrl".into();
            }
            Err(e) => last_err = e,
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    Err(AppError::Internal(anyhow::anyhow!(
        "浏览器在 15 秒内没有开放调试端口 {port}：{last_err}"
    )))
}

async fn fetch_version(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json::<Value>()
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp)
}

fn reqwest_get_client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("建 HTTP 客户端失败：{e}")))
}

/// 找系统里的 Edge 或 Chrome。
///
/// 返回第一个存在的。找不到不算致命——调用方会给出「需要装浏览器」
/// 的提示，而不是让整个服务起不来。
pub fn find_browser() -> Option<PathBuf> {
    let candidates = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        "/usr/bin/microsoft-edge",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ];
    candidates
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}
