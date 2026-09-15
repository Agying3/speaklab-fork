//! 本地语音识别。
//!
//! 整个模块只在 `--features whisper` 时才真正编译进产物。没开这个
//! feature 时，下面的 `Transcript` 仍然存在，好让上层代码保持同一份
//! 签名，不用到处写 `#[cfg]`。
//!
//! 模型文件不随仓库分发（`ggml-*.bin` 动辄几百 MB）。获取方式：
//!
//! ```text
//! curl -L -o server/models/ggml-base.en.bin \
//!   https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin
//! ```
//!
//! 然后设置 `SPEAKLAB_ASR_MODEL=server/models/ggml-base.en.bin`。

/// 一次识别的结果。
///
/// 这个类型在两种 feature 下都存在，好让上层保持同一份签名。
#[derive(Debug, Clone)]
pub struct Transcript {
    pub text: String,
    pub language: String,
    pub duration_secs: f32,
}

#[cfg(feature = "whisper")]
mod imp {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, OnceLock};

    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    use super::Transcript;
    use crate::error::{AppError, AppResult};
    use crate::state::AppState;

    /// 模型加载一次要几百毫秒，之后复用。`WhisperContext` 内部是
    /// 线程安全的引用计数，但 `WhisperState` 不是，所以每次识别
    /// 新建一个 state，这也是 whisper.cpp 推荐的做法。
    static CTX: OnceLock<Mutex<Option<Arc<WhisperContext>>>> = OnceLock::new();

    fn context(path: &str) -> AppResult<Arc<WhisperContext>> {
        let cell = CTX.get_or_init(|| Mutex::new(None));
        let mut guard = cell
            .lock()
            .map_err(|_| AppError::Internal(anyhow::anyhow!("模型锁被污染")))?;

        if let Some(ctx) = guard.as_ref() {
            return Ok(Arc::clone(ctx));
        }

        if !PathBuf::from(path).exists() {
            return Err(AppError::NotImplemented(format!(
                "whisper 模型文件（找不到 {path}）"
            )));
        }

        let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map_err(|e| AppError::Internal(anyhow::anyhow!("加载模型失败：{e}")))?;

        let ctx = Arc::new(ctx);
        *guard = Some(Arc::clone(&ctx));
        Ok(ctx)
    }

    pub async fn transcribe(state: &AppState, audio: &[u8], language: &str) -> AppResult<Transcript> {
        let Some(model_path) = state.config().asr.model_path.clone() else {
            return Err(AppError::NotImplemented("whisper 模型（未设置 SPEAKLAB_ASR_MODEL）".into()));
        };

        let samples = decode_to_pcm(audio)?;
        let duration_secs = samples.len() as f32 / 16_000.0;

        // spawn_blocking 的闭包要求 'static，所以先把 &str 转成 String
        let lang = language.to_owned();
        let lang_for_task = lang.clone();

        // whisper 是同步的 CPU 活，扔到阻塞线程池，别占着 tokio 的 worker
        let text = tokio::task::spawn_blocking(move || -> AppResult<String> {
            let ctx = context(&model_path)?;
            let mut whisper_state = ctx
                .create_state()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("创建识别状态失败：{e}")))?;

            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_language(if lang_for_task == "auto" { None } else { Some(&lang_for_task) });
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            // 单线程推理，并发靠上面的信号量控制，比让每个请求
            // 各开一堆线程互相抢 CPU 要稳定
            params.set_n_threads(4);

            whisper_state
                .full(params, &samples)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("识别失败：{e}")))?;

            let n = whisper_state
                .full_n_segments()
                .map_err(|e| AppError::Internal(anyhow::anyhow!("读取分段失败：{e}")))?;

            let mut out = String::new();
            for i in 0..n {
                let seg = whisper_state
                    .full_get_segment_text(i)
                    .map_err(|e| AppError::Internal(anyhow::anyhow!("读取分段文本失败：{e}")))?;
                out.push_str(seg.trim());
                out.push(' ');
            }

            Ok(out.trim().to_owned())
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("识别任务崩溃：{e}")))??;

        Ok(Transcript {
            text,
            language: lang,
            duration_secs,
        })
    }

    /// 把上传的音频转成 whisper 要的 16kHz 单声道 f32。
    ///
    /// 现在只接受 WAV，因为解 mp3/opus 要再拖一个解码器进来。
    /// 前端的 MediaRecorder 默认出 webm/opus，所以要么前端改成
    /// 传 WAV，要么以后在这里补一个解码步骤。
    fn decode_to_pcm(bytes: &[u8]) -> AppResult<Vec<f32>> {
        let (sample_rate, channels, bits, data) = parse_wav(bytes)?;

        if bits != 16 {
            return Err(AppError::BadRequest(format!(
                "只支持 16 位 PCM WAV，收到 {bits} 位"
            )));
        }

        // 先按声道降到单声道
        let mono: Vec<f32> = data
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();

        let mono = if channels > 1 {
            mono.chunks(channels as usize)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                .collect()
        } else {
            mono
        };

        // 再重采样到 16k。线性插值对语音够用，且不用引入重采样库。
        if sample_rate == 16_000 {
            return Ok(mono);
        }
        if sample_rate == 0 {
            return Err(AppError::BadRequest("WAV 头里的采样率为 0".into()));
        }

        let ratio = sample_rate as f64 / 16_000.0;
        let out_len = (mono.len() as f64 / ratio).floor() as usize;
        let mut out = Vec::with_capacity(out_len);
        for i in 0..out_len {
            let pos = i as f64 * ratio;
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            let a = mono.get(idx).copied().unwrap_or(0.0);
            let b = mono.get(idx + 1).copied().unwrap_or(a);
            out.push(a + (b - a) * frac);
        }
        Ok(out)
    }

    /// 极简 WAV 解析：找 `fmt ` 和 `data` 两个块，返回需要的字段。
    fn parse_wav(bytes: &[u8]) -> AppResult<(u32, u16, u16, &[u8])> {
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err(AppError::BadRequest("不是合法的 WAV 文件".into()));
        }

        let mut pos = 12usize;
        let mut fmt: Option<(u32, u16, u16)> = None;
        let mut data: Option<&[u8]> = None;

        while pos + 8 <= bytes.len() {
            let id = &bytes[pos..pos + 4];
            let size = u32::from_le_bytes([
                bytes[pos + 4],
                bytes[pos + 5],
                bytes[pos + 6],
                bytes[pos + 7],
            ]) as usize;
            let body_start = pos + 8;
            let body_end = body_start.checked_add(size).unwrap_or(bytes.len()).min(bytes.len());

            match id {
                b"fmt " if size >= 16 => {
                    let body = &bytes[body_start..body_end];
                    let channels = u16::from_le_bytes([body[2], body[3]]);
                    let sample_rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                    let bits = u16::from_le_bytes([body[14], body[15]]);
                    fmt = Some((sample_rate, channels, bits));
                }
                b"data" => {
                    data = Some(&bytes[body_start..body_end]);
                }
                _ => {}
            }

            // 块长度按偶数对齐
            pos = body_start + size + (size & 1);
        }

        let (sample_rate, channels, bits) =
            fmt.ok_or_else(|| AppError::BadRequest("WAV 缺少 fmt 块".into()))?;
        let data = data.ok_or_else(|| AppError::BadRequest("WAV 缺少 data 块".into()))?;
        if channels == 0 {
            return Err(AppError::BadRequest("声道数为 0".into()));
        }

        Ok((sample_rate, channels, bits, data))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// 造一个最小可用的 WAV 头 + 数据。
        fn wav(sample_rate: u32, channels: u16, bits: u16, samples: &[i16]) -> Vec<u8> {
            let data_len = (samples.len() * 2) as u32;
            let mut v = Vec::new();
            v.extend_from_slice(b"RIFF");
            v.extend_from_slice(&(36 + data_len).to_le_bytes());
            v.extend_from_slice(b"WAVE");
            v.extend_from_slice(b"fmt ");
            v.extend_from_slice(&16u32.to_le_bytes());
            v.extend_from_slice(&1u16.to_le_bytes()); // PCM
            v.extend_from_slice(&channels.to_le_bytes());
            v.extend_from_slice(&sample_rate.to_le_bytes());
            v.extend_from_slice(&(sample_rate * channels as u32 * (bits / 8) as u32).to_le_bytes());
            v.extend_from_slice(&(channels * (bits / 8)).to_le_bytes());
            v.extend_from_slice(&bits.to_le_bytes());
            v.extend_from_slice(b"data");
            v.extend_from_slice(&data_len.to_le_bytes());
            for s in samples {
                v.extend_from_slice(&s.to_le_bytes());
            }
            v
        }

        #[test]
        fn parses_wav_header() {
            let bytes = wav(16_000, 1, 16, &[0, 100, -100]);
            let (sr, ch, bits, data) = parse_wav(&bytes).unwrap();
            assert_eq!(sr, 16_000);
            assert_eq!(ch, 1);
            assert_eq!(bits, 16);
            assert_eq!(data.len(), 6);
        }

        #[test]
        fn rejects_non_wav() {
            assert!(parse_wav(b"not a wav file at all").is_err());
        }

        #[test]
        fn resamples_to_16k() {
            // 32kHz 输入，1 秒 → 输出应约 16000 个采样
            let samples = vec![0i16; 32_000];
            let bytes = wav(32_000, 1, 16, &samples);
            let pcm = decode_to_pcm(&bytes).unwrap();
            assert!(
                (pcm.len() as i64 - 16_000).abs() <= 2,
                "重采样后长度 {} 不对",
                pcm.len()
            );
        }

        #[test]
        fn downmixes_stereo() {
            // 左右声道分别是 +0.5 和 -0.5，混完应该接近 0
            let samples = vec![16_384i16, -16_384, 16_384, -16_384];
            let bytes = wav(16_000, 2, 16, &samples);
            let pcm = decode_to_pcm(&bytes).unwrap();
            assert_eq!(pcm.len(), 2);
            assert!(pcm[0].abs() < 1e-4, "混音结果应接近 0，实际 {}", pcm[0]);
        }

        #[test]
        fn rejects_non_16bit() {
            let bytes = wav(16_000, 1, 8, &[0, 1]);
            assert!(decode_to_pcm(&bytes).is_err());
        }
    }
}

#[cfg(feature = "whisper")]
pub use imp::transcribe;
