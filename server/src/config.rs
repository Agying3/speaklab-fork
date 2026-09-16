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
