//! 跨请求共享的状态。
//!
//! 现在只有配置和一个并发闸门。以后要加数据库连接池、
//! 缓存之类的，都挂在这里，由 `main` 建好传下去。

use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    config: Config,
    /// 语音识别并发闸门。whisper 是 CPU 密集的，
    /// 放任并发只会让所有请求一起变慢。
    asr_gate: Semaphore,
    started_at: std::time::Instant,
    store: crate::store::Store,
}

impl AppState {
    pub async fn new(config: Config, store: crate::store::Store) -> anyhow::Result<Self> {
        let permits = config.asr.max_concurrency.max(1);
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                asr_gate: Semaphore::new(permits),
                started_at: std::time::Instant::now(),
                store,
            }),
        })
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    pub fn store(&self) -> &crate::store::Store {
        &self.inner.store
    }

    pub fn asr_gate(&self) -> &Semaphore {
        &self.inner.asr_gate
    }

    /// 进程已运行秒数，健康检查用。
    pub fn uptime_secs(&self) -> u64 {
        self.inner.started_at.elapsed().as_secs()
    }

    /// 校验调用方是否带了对的令牌。
    ///
    /// 没配 `SPEAKLAB_TOKEN` 时一律放行——此时服务只监听本地，
    /// 加个假的门禁只会让人以为它是安全的。启动日志会明确说明。
    pub fn authorize(&self, provided: Option<&str>) -> Result<(), crate::error::AppError> {
        let Some(expected) = self.inner.config.api_token.as_deref() else {
            return Ok(());
        };

        // 常量时间比较，避免用响应时间逐字节试出令牌。
        let ok = match provided {
            Some(got) => constant_time_eq(got.as_bytes(), expected.as_bytes()),
            None => false,
        };

        if ok {
            Ok(())
        } else {
            Err(crate::error::AppError::Unauthorized)
        }
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_normal_eq() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }
}
