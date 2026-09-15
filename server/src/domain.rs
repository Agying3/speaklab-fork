//! 领域模型。
//!
//! 这里只放纯数据结构和与外部无关的规则，不依赖 axum、不依赖文件系统，
//! 因此可以直接单测。前端 `index.html` 里的 `History` 模块已经有一套
//! 本地记录结构，字段名保持一致，同步时不用做映射。

use serde::{Deserialize, Serialize};

/// 一次练习记录。
///
/// 对应前端 `History` 里存进 localStorage 的条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PracticeRecord {
    /// 客户端生成的 id，服务端不重新分配，避免同步时产生重复。
    pub id: String,
    /// 记录类型：跟读 / 对话 / 音标。
    pub kind: PracticeKind,
    /// 目标文本。对话类记录留空。
    #[serde(default)]
    pub target: String,
    /// 识别到的文本。
    #[serde(default)]
    pub transcript: String,
    /// 总分，0..=1。
    pub total: f32,
    /// 三个维度，和前端评分引擎一致。
    #[serde(default)]
    pub accuracy: f32,
    #[serde(default)]
    pub fluency: f32,
    #[serde(default)]
    pub completeness: f32,
    /// 场景或音标名。
    #[serde(default)]
    pub label: String,
    /// 客户端发生时间，RFC 3339。
    pub at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PracticeKind {
    Shadow,
    Chat,
    Phoneme,
}

impl PracticeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shadow => "shadow",
            Self::Chat => "chat",
            Self::Phoneme => "phoneme",
        }
    }
}

impl PracticeRecord {
    /// 基本校验。分数越界、id 为空这类问题在写入前就该挡掉，
    /// 否则脏数据会一直留在库里，后面很难分辨是谁写坏的。
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("id 不能为空".into());
        }
        if self.id.len() > 128 {
            return Err("id 过长".into());
        }
        for (name, v) in [
            ("total", self.total),
            ("accuracy", self.accuracy),
            ("fluency", self.fluency),
            ("completeness", self.completeness),
        ] {
            if !v.is_finite() {
                return Err(format!("{name} 不是有限数"));
            }
            if !(0.0..=1.0).contains(&v) {
                return Err(format!("{name} 应在 0..=1 之间，实际 {v}"));
            }
        }
        if self.at.trim().is_empty() {
            return Err("at 不能为空".into());
        }
        Ok(())
    }
}

/// 汇总统计。前端统计页现在是自己算的，接上后端后可以直接取。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub total_sessions: u64,
    /// 练习过的不同天数，用于算连续天数。
    pub active_days: u64,
    pub average_total: f32,
    /// 最近一次练习时间，RFC 3339。
    pub last_practiced_at: Option<String>,
}

impl Stats {
    /// 从记录列表汇总。假设输入不做去重，调用方保证。
    pub fn from_records(records: &[PracticeRecord]) -> Self {
        if records.is_empty() {
            return Self::default();
        }

        let mut days = std::collections::BTreeSet::new();
        let mut sum = 0f64;
        let mut last: Option<&str> = None;

        for r in records {
            // 只取日期部分，时间戳里的时分秒对"练习了几天"没有意义
            if let Some(day) = r.at.split('T').next() {
                days.insert(day.to_owned());
            }
            sum += r.total as f64;
            last = match last {
                Some(prev) if prev >= r.at.as_str() => Some(prev),
                _ => Some(r.at.as_str()),
            };
        }

        Self {
            total_sessions: records.len() as u64,
            active_days: days.len() as u64,
            average_total: (sum / records.len() as f64) as f32,
            last_practiced_at: last.map(str::to_owned),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, total: f32, at: &str) -> PracticeRecord {
        PracticeRecord {
            id: id.into(),
            kind: PracticeKind::Shadow,
            target: "hello".into(),
            transcript: "hello".into(),
            total,
            accuracy: total,
            fluency: total,
            completeness: total,
            label: "Daily".into(),
            at: at.into(),
        }
    }

    #[test]
    fn validates_score_range() {
        let mut r = rec("a", 1.5, "2026-01-01T00:00:00Z");
        assert!(r.validate().is_err(), "总分超过 1 应该被拒");

        // 把四个分数一起改回合法区间
        r.total = 0.5;
        r.accuracy = 0.5;
        r.fluency = 0.5;
        r.completeness = 0.5;
        assert!(r.validate().is_ok());

        // 边界值是合法的
        r.total = 0.0;
        assert!(r.validate().is_ok());
        r.total = 1.0;
        assert!(r.validate().is_ok());

        // 只有某一个维度越界也要被拒
        r.accuracy = -0.1;
        assert!(r.validate().is_err(), "单个维度越界也应被拒");
    }

    #[test]
    fn rejects_blank_id() {
        let r = rec("  ", 0.5, "2026-01-01T00:00:00Z");
        assert!(r.validate().is_err());
    }

    #[test]
    fn rejects_nan() {
        let r = rec("a", f32::NAN, "2026-01-01T00:00:00Z");
        assert!(r.validate().is_err());
    }

    #[test]
    fn stats_counts_distinct_days() {
        let records = vec![
            rec("a", 0.8, "2026-01-01T09:00:00Z"),
            rec("b", 0.6, "2026-01-01T21:00:00Z"),
            rec("c", 1.0, "2026-01-03T10:00:00Z"),
        ];
        let s = Stats::from_records(&records);
        assert_eq!(s.total_sessions, 3);
        assert_eq!(s.active_days, 2, "同一天的两条只算一天");
        assert!((s.average_total - 0.8).abs() < 1e-6);
        assert_eq!(s.last_practiced_at.as_deref(), Some("2026-01-03T10:00:00Z"));
    }

    #[test]
    fn stats_of_empty_is_default() {
        assert_eq!(Stats::from_records(&[]), Stats::default());
    }
}
