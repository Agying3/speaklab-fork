//! 练习记录与语音识别接口。

use axum::extract::{Multipart, Query, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::domain::{PracticeKind, PracticeRecord, Stats};
use crate::error::{AppError, AppResult};
use crate::routes::bearer_token;
use crate::state::AppState;

// ---------------------------------------------------------------- 写入

#[derive(Deserialize)]
pub struct CreateRequest {
    /// 一次提交多条，方便前端把攒下来的记录一起推上来。
    pub records: Vec<PracticeRecord>,
}

#[derive(Serialize)]
pub struct CreateResponse {
    /// 实际新写入的条数。
    pub accepted: usize,
    /// 因为 id 已存在而跳过的条数。客户端重试会走到这里，
    /// 不是错误，所以单独报出来而不是混进 rejected。
    pub duplicates: usize,
    pub rejected: Vec<Rejection>,
}

#[derive(Serialize)]
pub struct Rejection {
    pub id: String,
    pub reason: String,
}

pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateRequest>,
) -> AppResult<Json<CreateResponse>> {
    state.authorize(bearer_token(&headers).as_deref())?;

    if req.records.is_empty() {
        return Err(AppError::BadRequest("records 不能为空".into()));
    }
    // 一次塞太多说明客户端逻辑有问题，直接挡掉比慢慢消化好
    const MAX_BATCH: usize = 500;
    if req.records.len() > MAX_BATCH {
        return Err(AppError::BadRequest(format!(
            "单次最多 {MAX_BATCH} 条，收到 {}",
            req.records.len()
        )));
    }

    // 先逐条校验。一条坏数据不该毁掉整批同步——客户端可以
    // 只重推被拒的那几条，所以这里分出「收下」和「退回」两堆。
    let mut valid = Vec::new();
    let mut rejected = Vec::new();

    for r in req.records {
        match r.validate() {
            Ok(()) => valid.push(r),
            Err(reason) => rejected.push(Rejection {
                id: r.id.clone(),
                reason,
            }),
        }
    }

    let submitted = valid.len();
    let stored = state.store().insert_many(valid).await?;
    // 写进去的比提交的少，说明有 id 重复。
    let duplicates = submitted - stored;

    tracing::info!(stored, duplicates, rejected = rejected.len(), "写入练习记录");

    Ok(Json(CreateResponse {
        accepted: stored,
        duplicates,
        rejected,
    }))
}

// ---------------------------------------------------------------- 读取

#[derive(Deserialize)]
pub struct ListQuery {
    /// 只取某个类型。
    pub kind: Option<String>,
    /// 最多返回多少条，默认 50。
    pub limit: Option<usize>,
}

#[derive(Serialize)]
pub struct ListResponse {
    pub records: Vec<PracticeRecord>,
    /// 本页条数。
    pub total: usize,
    /// 库里符合条件的总条数，用来判断还有没有下一页。
    pub grand_total: u64,
}

pub async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<ListQuery>,
) -> AppResult<Json<ListResponse>> {
    state.authorize(bearer_token(&headers).as_deref())?;

    let limit = q.limit.unwrap_or(50).min(500);

    // 类型写错时直接报错，而不是静默返回空列表——
    // 前端拼错了参数应该马上发现，而不是以为「今天没练过」。
    let kind = match q.kind.as_deref() {
        None => None,
        Some("shadow") => Some(PracticeKind::Shadow),
        Some("chat") => Some(PracticeKind::Chat),
        Some("phoneme") => Some(PracticeKind::Phoneme),
        Some(other) => {
            return Err(AppError::BadRequest(format!(
                "kind 只能是 shadow/chat/phoneme，收到 {other:?}"
            )))
        }
    };

    let records = state.store().list(kind, limit).await?;
    let grand_total = state.store().count().await?;

    Ok(Json(ListResponse {
        total: records.len(),
        grand_total,
        records,
    }))
}

pub async fn stats(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Stats>> {
    state.authorize(bearer_token(&headers).as_deref())?;
    Ok(Json(state.store().stats().await?))
}

/// 删除一条记录。
pub async fn delete(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> AppResult<axum::http::StatusCode> {
    state.authorize(bearer_token(&headers).as_deref())?;

    if state.store().delete(id.clone()).await? {
        tracing::info!(id, "删除记录");
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(AppError::NotFound(format!("记录 {id}")))
    }
}

// ---------------------------------------------------------------- 语音识别

#[derive(Serialize)]
pub struct TranscribeResponse {
    pub text: String,
    /// 识别用的语言。
    pub language: String,
    /// 音频时长，秒。
    pub duration_secs: f32,
    /// 耗时，毫秒。前端可以据此提示"本地识别较慢"。
    pub elapsed_ms: u64,
}

/// 接收音频并转写。
///
/// 用 multipart 而不是裸 body，是为了以后能一起传采样率、
/// 语言覆盖之类的元信息，而不用改请求形状。
pub async fn transcribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> AppResult<Json<TranscribeResponse>> {
    state.authorize(bearer_token(&headers).as_deref())?;

    // 先把音频收下来，再判断能力——这样即便没配模型，
    // 也能给出"收到了多少字节"这种有用的报错。
    let mut audio: Option<Vec<u8>> = None;
    let mut language: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart 解析失败：{e}")))?
    {
        match field.name() {
            Some("audio") => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("读取音频失败：{e}")))?;
                audio = Some(bytes.to_vec());
            }
            Some("language") => {
                language = field.text().await.ok();
            }
            _ => {} // 未知字段直接忽略，向前兼容
        }
    }

    let audio = audio.ok_or_else(|| AppError::BadRequest("缺少 audio 字段".into()))?;
    if audio.is_empty() {
        return Err(AppError::BadRequest("音频为空".into()));
    }
    tracing::info!(bytes = audio.len(), "收到待识别音频");

    if !state.config().asr.enabled() {
        return Err(AppError::NotImplemented(
            "本地语音识别（编译时加 --features whisper 并设置 SPEAKLAB_ASR_MODEL）".into(),
        ));
    }

    // 闸门限住同时在跑的推理数，超出的排队而不是一起挤 CPU
    let _permit = state
        .asr_gate()
        .acquire()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("并发闸门已关闭：{e}")))?;

    let lang = language.unwrap_or_else(|| state.config().asr.language.clone());
    let started = std::time::Instant::now();

    #[cfg(feature = "whisper")]
    let result = crate::asr::transcribe(&state, &audio, &lang).await;

    #[cfg(not(feature = "whisper"))]
    let result: AppResult<crate::asr::Transcript> = {
        // 走不到这里：上面 enabled() 已经挡掉了没开 feature 的情况。
        // 留着是为了让两种编译配置下的类型保持一致。
        let _ = &lang;
        Err(AppError::NotImplemented(
            "本地语音识别（编译时未打开 whisper feature）".into(),
        ))
    };

    let transcript = result?;

    Ok(Json(TranscribeResponse {
        text: transcript.text,
        language: transcript.language,
        duration_secs: transcript.duration_secs,
        elapsed_ms: started.elapsed().as_millis() as u64,
    }))
}
