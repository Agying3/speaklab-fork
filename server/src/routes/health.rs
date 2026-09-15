//! 健康检查与能力探测。
//!
//! 前端启动时打一次 `/api/v1/meta`，就知道后端在不在、支持哪些能力，
//! 据此决定是走本地兜底还是走后端。这样同一份前端既能脱离后端跑，
//! 也能在有后端时自动升级。

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
pub struct Health {
    pub ok: bool,
    pub version: &'static str,
    /// 进程已运行秒数。
    pub uptime_secs: u64,
}

pub async fn health(State(state): State<AppState>) -> Json<Health> {
    Json(Health {
        ok: true,
        version: env!("CARGO_PKG_VERSION"),
        uptime_secs: state.uptime_secs(),
    })
}

/// 能力清单。前端据此决定按钮显不显示，而不是先试着调用再处理 501。
#[derive(Serialize)]
pub struct Meta {
    pub name: &'static str,
    pub version: &'static str,
    pub api_version: u32,
    pub capabilities: Capabilities,
}

#[derive(Serialize)]
pub struct Capabilities {
    /// 本地语音识别是否可用。
    pub asr_local: bool,
    /// 服务端是否配了 LLM 密钥。
    pub llm_proxy: bool,
    /// 是否要求令牌。
    pub auth_required: bool,
    /// 识别支持的语言列表。
    pub asr_languages: Vec<&'static str>,
}

pub async fn meta(State(state): State<AppState>) -> Json<Meta> {
    let cfg = state.config();

    let mut languages = vec!["en", "zh", "ja", "ko", "auto"];
    // 配了具体语言就把 auto 去掉，避免给出误导性的选项
    if cfg.asr.language != "auto" {
        languages.retain(|l| *l != "auto");
    }

    Json(Meta {
        name: "speaklab-server",
        version: env!("CARGO_PKG_VERSION"),
        api_version: 1,
        capabilities: Capabilities {
            asr_local: cfg.asr.enabled(),
            llm_proxy: cfg.llm.is_some(),
            auth_required: cfg.api_token.is_some(),
            asr_languages: languages,
        },
    })
}
