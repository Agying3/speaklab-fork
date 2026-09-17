//! 运行期配置。全部来自环境变量，带合理默认值，目标是
//! `cargo run` 不配任何东西就能起来。

use std::env;

use axum::http::HeaderValue;

use crate::DEFAULT_UPSTREAM_TIMEOUT;

#[derive(Debug, Clone)]
pub struct Config {
    /// 监听地址，如 `127.0.0.1:8787`。
    pub bind: String,
    /// 开发模式：放开 CORS，日志更啰嗦。
    pub dev_mode: bool,
    /// 生产模式下允许的来源。
    pub allowed_origins: Vec<HeaderValue>,
    /// 单个请求体上限。录音上传按 20 秒 opus 估，留足余量。
    pub max_body_bytes: usize,
    /// 语音识别相关配置。
    pub asr: AsrConfig,
    /// 上游 LLM 配置，未设置时相关接口返回 503。
    pub llm: Option<LlmConfig>,
    /// 会话密钥。未设置时服务拒绝写入型接口，避免"看起来能用其实没保护"。
    pub api_token: Option<String>,
    /// SQLite 数据库文件路径。
    pub db_path: std::path::PathBuf,
    /// 云游戏串流相关配置。
    pub cloud: CloudConfig,
}

#[derive(Debug, Clone)]
pub struct CloudConfig {
    /// 浏览器可执行文件。None 表示没找到，云游戏接口返回 501。
    pub browser_exe: Option<std::path::PathBuf>,
    /// 浏览器临时配置目录的父目录。
    pub profile_root: std::path::PathBuf,
    /// 调试端口起始值。每个会话往后一个。
    pub port_base: u16,
    /// JPEG 质量。云游戏 60 左右肉眼无差，再高只是白费带宽。
    pub quality: u8,
    /// 画面宽高。这是卡片里那块地方的实际像素尺寸。
    pub max_width: u32,
    pub max_height: u32,
    /// 会话休眠多久之后彻底关掉（秒）。0 表示不自动回收。
    ///
    /// 前端切到后台时会话进入休眠：只停帧流，浏览器留着、画面和
    /// 登录态都在，切回来不用重进游戏。**但内存并不因此释放**
    /// （实测休眠约 725 MB，醒着约 764 MB），真正还回内存的是这道超时。
    ///
    /// 默认 90 秒。**这是唯一真正省内存的旋钮**（见下面实现处的说明）。
    pub dormant_timeout_secs: u64,
}

impl CloudConfig {
    /// 给启动日志用。
    pub fn describe(&self) -> String {
        match &self.browser_exe {
            Some(p) => format!(
                "已启用（{}，{}x{}）",
                p.file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
                self.max_width,
                self.max_height
            ),
            None => "未启用（没找到 Edge 或 Chrome）".into(),
        }
    }

    pub fn enabled(&self) -> bool {
        self.browser_exe.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct AsrConfig {
    /// whisper 模型文件路径。
    pub model_path: Option<String>,
    /// 语言，`auto` 表示自动检测。
    pub language: String,
    /// 并发推理上限。whisper 吃 CPU，超过核数只会互相拖慢。
    pub max_concurrency: usize,
}

impl AsrConfig {
    /// 给启动日志用的一句话描述。
    pub fn describe(&self) -> String {
        if !cfg!(feature = "whisper") {
            return "未启用（编译时未打开 whisper feature）".into();
        }
        match &self.model_path {
            Some(p) => format!("whisper 本地推理，模型 {p}"),
            None => "未启用（未设置 SPEAKLAB_ASR_MODEL）".into(),
        }
    }

    pub fn enabled(&self) -> bool {
        cfg!(feature = "whisper") && self.model_path.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let dev_mode = env_bool("SPEAKLAB_DEV", true);

        let mut allowed_origins = Vec::new();
        if let Ok(raw) = env::var("SPEAKLAB_ALLOWED_ORIGINS") {
            for part in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                allowed_origins.push(part.parse::<HeaderValue>().map_err(|e| {
                    anyhow::anyhow!("SPEAKLAB_ALLOWED_ORIGINS 里的 {part:?} 不是合法的 Origin：{e}")
                })?);
            }
        }

        let llm = match env::var("SPEAKLAB_LLM_API_KEY") {
            Ok(api_key) if !api_key.trim().is_empty() => Some(LlmConfig {
                base_url: env::var("SPEAKLAB_LLM_BASE_URL")
                    .unwrap_or_else(|_| "https://api.deepseek.com".into()),
                api_key,
                model: env::var("SPEAKLAB_LLM_MODEL").unwrap_or_else(|_| "deepseek-chat".into()),
            }),
            _ => None,
        };

        let api_token = env::var("SPEAKLAB_TOKEN")
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());

        Ok(Self {
            bind: env::var("SPEAKLAB_BIND").unwrap_or_else(|_| "127.0.0.1:8787".into()),
            dev_mode,
            allowed_origins,
            max_body_bytes: env_usize("SPEAKLAB_MAX_BODY_BYTES", 8 * 1024 * 1024),
            asr: AsrConfig {
                model_path: env::var("SPEAKLAB_ASR_MODEL")
                    .ok()
                    .filter(|s| !s.trim().is_empty()),
                language: env::var("SPEAKLAB_ASR_LANG").unwrap_or_else(|_| "en".into()),
                max_concurrency: env_usize(
                    "SPEAKLAB_ASR_CONCURRENCY",
                    std::thread::available_parallelism()
                        .map(|n| (n.get() / 2).max(1))
                        .unwrap_or(2),
                ),
            },
            llm,
            api_token,
            db_path: env::var("SPEAKLAB_DB")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("speaklab.db")),
            cloud: CloudConfig {
                // 允许用 SPEAKLAB_BROWSER 指定，否则自动找
                browser_exe: env::var("SPEAKLAB_BROWSER")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
                    .map(std::path::PathBuf::from)
                    .or_else(crate::cloud::browser::find_browser),
                profile_root: env::var("SPEAKLAB_CLOUD_PROFILE")
                    .ok()
                    .filter(|s| !s.trim().is_empty())
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::env::temp_dir().join("speaklab-cloud")),
                port_base: env_usize("SPEAKLAB_CLOUD_PORT", 9222).clamp(1024, 65000) as u16,
                quality: env_usize("SPEAKLAB_CLOUD_QUALITY", 60).clamp(10, 100) as u8,
                // 默认 480x320。**这个是量出来的，不是拍的。**
                //
                // 云游戏的移动版页面用固定像素布局，不跟视口宽度走：
                // 实测 720x480 的手机号输入框还是 217px 宽，跟 480x320
                // 时一模一样。所以提高分辨率**不会让表单变大**——
                // 画面是等比缩放进卡片的，渲染得越宽，缩得越小：
                //
                //   渲染 480 宽 -> 卡片里缩放 1.27x -> LABEL 显示 405px
                //   渲染 720 宽 -> 卡片里缩放 0.84x -> LABEL 显示 270px
                //   渲染 960 宽 -> 卡片里缩放 0.63x -> LABEL 显示 203px
                //
                // 「同意用户协议」那个复选框本身就 0x0，全靠外层
                // 320x32 的 LABEL 承接触摸。480 时它显示成 405px 宽，
                // 手机上（卡片约 360px）也有 240px，手指点得中；
                // 720 就只剩 160px 了，容易点偏。
                //
                // 所以取 480x320：触控优先。想更清楚就把卡片加宽
                // （卡片越宽，同一个画面缩放越大），不要动这个值。
                max_width: env_usize("SPEAKLAB_CLOUD_WIDTH", 480).clamp(160, 1920) as u32,
                max_height: env_usize("SPEAKLAB_CLOUD_HEIGHT", 320).clamp(120, 1080) as u32,
                // 上限给到 1 天。0 是有意义的取值（不自动回收），
                // 所以不能直接 clamp 到 [1, ..]。
                // 默认 90 秒。这是**唯一真正省内存的旋钮**：
                // 休眠只是停发帧，浏览器那 700 多 MB 照占着
                // （实测连卸载页面也只回落 63 MB，因为那些内存
                // 几乎全是浏览器骨架——空的 Edge 本身就占 833 MB）。
                // 只有杀掉进程才真的还回来。
                //
                // 90 秒够切走查个攻略再切回来（那期间画面还在，
                // 不用重进游戏），走去吃饭就会被回收。
                // 想更激进就调小，0 表示永不回收。
                dormant_timeout_secs: env_usize("SPEAKLAB_CLOUD_SLEEP_TIMEOUT", 90)
                    .min(86_400) as u64,
            },
        })
    }

    /// 上游请求超时。单独抽出来是为了测试时能调小。
    pub fn upstream_timeout(&self) -> std::time::Duration {
        DEFAULT_UPSTREAM_TIMEOUT
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    match env::var(key) {
        Ok(v) => matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => default,
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asr_disabled_without_model() {
        let cfg = AsrConfig {
            model_path: None,
            language: "en".into(),
            max_concurrency: 1,
        };
        assert!(!cfg.enabled());
    }

    #[test]
    fn asr_enabled_requires_both_feature_and_model() {
        let cfg = AsrConfig {
            model_path: Some("/tmp/model.bin".into()),
            language: "en".into(),
            max_concurrency: 1,
        };
        // 没开 feature 时即便配了模型也不算启用
        assert_eq!(cfg.enabled(), cfg!(feature = "whisper"));
    }

    #[test]
    fn env_usize_falls_back_on_garbage() {
        std::env::set_var("SPEAKLAB_TEST_GARBAGE", "abc");
        assert_eq!(env_usize("SPEAKLAB_TEST_GARBAGE", 7), 7);
        std::env::remove_var("SPEAKLAB_TEST_GARBAGE");
    }
}
