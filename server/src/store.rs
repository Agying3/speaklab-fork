//! 持久化。
//!
//! 对外只暴露 `Store` 这一个类型，上层不关心底下是 SQLite 还是别的。
//! 选 SQLite 是因为这个服务基本都自部署：一个文件就是全部状态，
//! 备份等于拷文件，不需要另起一个数据库进程。
//!
//! 用 `rusqlite` 的 bundled 特性，把 SQLite 源码一起编进来，
//! 产物不依赖系统里的 sqlite3.dll。

use std::path::Path;

use rusqlite::{params, Connection};
#[cfg(test)]
use rusqlite::OptionalExtension;
use tokio_rusqlite::Connection as AsyncConnection;

use crate::domain::{PracticeKind, PracticeRecord, Stats};
use crate::error::{AppError, AppResult};

/// 练习记录仓库。
///
/// 内部是单条连接 + 一把异步锁。SQLite 的写入本来就是串行的，
/// 连接池在这个规模下只会增加复杂度。真到了写不动的时候，
/// 换 Postgres 也是替换这个类型，上层不用动。
#[derive(Clone)]
pub struct Store {
    conn: AsyncConnection,
}

impl Store {
    /// 打开（或新建）数据库并把表建好。
    pub async fn open(path: &Path) -> AppResult<Self> {
        // 父目录可能不存在，先建出来，否则 SQLite 会报 unable to open
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir).map_err(|e| {
                    AppError::Internal(anyhow::anyhow!(
                        "建不了数据目录 {}：{e}",
                        dir.display()
                    ))
                })?;
            }
        }

        let conn = AsyncConnection::open(path).await.map_err(|e| {
            AppError::Internal(anyhow::anyhow!("打开数据库 {} 失败：{e}", path.display()))
        })?;

        conn.call(|c| {
            init_schema(c)?;
            Ok(())
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("初始化表结构失败：{e}")))?;

        Ok(Self { conn })
    }

    /// 内存库，测试用。
    #[cfg(test)]
    pub async fn open_memory() -> AppResult<Self> {
        let conn = AsyncConnection::open_in_memory()
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("建内存库失败：{e}")))?;
        conn.call(|c| {
            init_schema(c)?;
            Ok(())
        })
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("初始化表结构失败：{e}")))?;
        Ok(Self { conn })
    }

    /// 写入一批记录。
    ///
    /// 用 `INSERT OR IGNORE`：同一条记录可能因为客户端重试而推两次，
    /// 主键冲突时跳过而不是报错，让重试变成幂等操作。
    /// 返回真正新写入的条数（已存在的会被忽略掉）。
    pub async fn insert_many(&self, records: Vec<PracticeRecord>) -> AppResult<usize> {
        self.conn
            .call(move |c| {
                let tx = c.transaction()?;
                let mut written = 0usize;
                {
                    let mut stmt = tx.prepare(
                        "INSERT OR IGNORE INTO practices
                         (id, kind, target, transcript, total, accuracy, fluency,
                          completeness, label, at)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    )?;
                    for r in &records {
                        written += stmt.execute(params![
                            r.id,
                            r.kind.as_str(),
                            r.target,
                            r.transcript,
                            r.total,
                            r.accuracy,
                            r.fluency,
                            r.completeness,
                            r.label,
                            r.at,
                        ])?;
                    }
                }
                tx.commit()?;
                Ok(written)
            })
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("写入记录失败：{e}")))
    }

    /// 按时间倒序列出记录。
    pub async fn list(&self, kind: Option<PracticeKind>, limit: usize) -> AppResult<Vec<PracticeRecord>> {
        let kind = kind.map(|k| k.as_str().to_owned());
        self.conn
            .call(move |c| {
                // 两种查询分开写，而不是在 SQL 里拼 OR，
                // 这样能各自用上索引。
                let (sql, args): (&str, Vec<Box<dyn rusqlite::ToSql>>) = match &kind {
                    Some(k) => (
                        "SELECT id,kind,target,transcript,total,accuracy,fluency,
                                completeness,label,at
                         FROM practices WHERE kind = ?1 ORDER BY at DESC LIMIT ?2",
                        vec![Box::new(k.clone()), Box::new(limit as i64)],
                    ),
                    None => (
                        "SELECT id,kind,target,transcript,total,accuracy,fluency,
                                completeness,label,at
                         FROM practices ORDER BY at DESC LIMIT ?1",
                        vec![Box::new(limit as i64)],
                    ),
                };
                let mut stmt = c.prepare(sql)?;
                let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
                let rows = stmt.query_map(refs.as_slice(), row_to_record)?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("读取记录失败：{e}")))
    }

    /// 记录总数，不限类型。
    pub async fn count(&self) -> AppResult<u64> {
        self.conn
            .call(|c| {
                let n: i64 = c.query_row("SELECT COUNT(*) FROM practices", [], |r| r.get(0))?;
                Ok(n as u64)
            })
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("统计条数失败：{e}")))
    }

    /// 汇总统计。直接在 SQL 里算，不把全部记录拉到内存。
    pub async fn stats(&self) -> AppResult<Stats> {
        self.conn
            .call(|c| {
                let (total, days, avg, last): (i64, i64, Option<f64>, Option<String>) = c
                    .query_row(
                        "SELECT COUNT(*),
                                COUNT(DISTINCT substr(at, 1, 10)),
                                AVG(total),
                                MAX(at)
                         FROM practices",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )?;

                Ok(Stats {
                    total_sessions: total as u64,
                    active_days: days as u64,
                    average_total: avg.unwrap_or(0.0) as f32,
                    last_practiced_at: last,
                })
            })
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("统计失败：{e}")))
    }

    /// 删除一条。返回是否真的删掉了。
    pub async fn delete(&self, id: String) -> AppResult<bool> {
        self.conn
            .call(move |c| {
                let n = c.execute("DELETE FROM practices WHERE id = ?1", params![id])?;
                Ok(n > 0)
            })
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("删除记录失败：{e}")))
    }

    /// 存在性检查，测试用。
    #[cfg(test)]
    pub async fn exists(&self, id: &str) -> AppResult<bool> {
        let id = id.to_owned();
        self.conn
            .call(move |c| {
                let found: Option<i64> = c
                    .query_row(
                        "SELECT 1 FROM practices WHERE id = ?1",
                        params![id],
                        |r| r.get(0),
                    )
                    .optional()?;
                Ok(found.is_some())
            })
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("查询失败：{e}")))
    }
}

/// 建表。用 `IF NOT EXISTS`，每次启动都跑一遍是安全的。
fn init_schema(c: &mut Connection) -> rusqlite::Result<()> {
    c.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;

         CREATE TABLE IF NOT EXISTS practices (
             id           TEXT PRIMARY KEY,
             kind         TEXT NOT NULL,
             target       TEXT NOT NULL DEFAULT '',
             transcript   TEXT NOT NULL DEFAULT '',
             total        REAL NOT NULL,
             accuracy     REAL NOT NULL DEFAULT 0,
             fluency      REAL NOT NULL DEFAULT 0,
             completeness REAL NOT NULL DEFAULT 0,
             label        TEXT NOT NULL DEFAULT '',
             at           TEXT NOT NULL,
             created_at   TEXT NOT NULL DEFAULT (datetime('now'))
         );

         CREATE INDEX IF NOT EXISTS idx_practices_at   ON practices(at DESC);
         CREATE INDEX IF NOT EXISTS idx_practices_kind ON practices(kind, at DESC);",
    )
}

/// 把一行读成 `PracticeRecord`。
fn row_to_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<PracticeRecord> {
    let kind: String = r.get(1)?;
    // 库里存的是字符串。遇到未知值退回 Shadow 而不是整体报错——
    // 旧版本写进去的类型不该让新版本读不出数据。
    let kind = match kind.as_str() {
        "chat" => PracticeKind::Chat,
        "phoneme" => PracticeKind::Phoneme,
        _ => PracticeKind::Shadow,
    };

    Ok(PracticeRecord {
        id: r.get(0)?,
        kind,
        target: r.get(2)?,
        transcript: r.get(3)?,
        total: r.get(4)?,
        accuracy: r.get(5)?,
        fluency: r.get(6)?,
        completeness: r.get(7)?,
        label: r.get(8)?,
        at: r.get(9)?,
    })
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

    #[tokio::test]
    async fn insert_then_read_back() {
        let s = Store::open_memory().await.unwrap();
        let n = s.insert_many(vec![rec("a", 0.8, "2026-01-01T09:00:00Z")]).await.unwrap();
        assert_eq!(n, 1);

        let rows = s.list(None, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "a");
        assert!((rows[0].total - 0.8).abs() < 1e-6);
    }

    #[tokio::test]
    async fn duplicate_id_is_ignored_not_an_error() {
        let s = Store::open_memory().await.unwrap();
        s.insert_many(vec![rec("a", 0.5, "2026-01-01T09:00:00Z")]).await.unwrap();
        // 客户端重试，同一条再推一次
        let n = s.insert_many(vec![rec("a", 0.5, "2026-01-01T09:00:00Z")]).await.unwrap();

        assert_eq!(n, 0, "重复的 id 不该被写入");
        assert_eq!(s.count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn list_is_newest_first() {
        let s = Store::open_memory().await.unwrap();
        s.insert_many(vec![
            rec("old", 0.1, "2026-01-01T00:00:00Z"),
            rec("new", 0.9, "2026-03-01T00:00:00Z"),
            rec("mid", 0.5, "2026-02-01T00:00:00Z"),
        ])
        .await
        .unwrap();

        let ids: Vec<_> = s.list(None, 10).await.unwrap().into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["new", "mid", "old"]);
    }

    #[tokio::test]
    async fn list_respects_limit() {
        let s = Store::open_memory().await.unwrap();
        s.insert_many(vec![
            rec("a", 0.1, "2026-01-01T00:00:00Z"),
            rec("b", 0.2, "2026-01-02T00:00:00Z"),
            rec("c", 0.3, "2026-01-03T00:00:00Z"),
        ])
        .await
        .unwrap();
        assert_eq!(s.list(None, 2).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn list_filters_by_kind() {
        let s = Store::open_memory().await.unwrap();
        let mut chat = rec("c1", 0.5, "2026-01-01T00:00:00Z");
        chat.kind = PracticeKind::Chat;
        s.insert_many(vec![rec("s1", 0.5, "2026-01-01T00:00:00Z"), chat]).await.unwrap();

        let shadows = s.list(Some(PracticeKind::Shadow), 10).await.unwrap();
        assert_eq!(shadows.len(), 1);
        assert_eq!(shadows[0].id, "s1");

        let chats = s.list(Some(PracticeKind::Chat), 10).await.unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].id, "c1");
    }

    #[tokio::test]
    async fn stats_roll_up_correctly() {
        let s = Store::open_memory().await.unwrap();
        s.insert_many(vec![
            rec("a", 0.8, "2026-01-01T09:00:00Z"),
            rec("b", 0.6, "2026-01-01T21:00:00Z"),
            rec("c", 1.0, "2026-01-03T10:00:00Z"),
        ])
        .await
        .unwrap();

        let st = s.stats().await.unwrap();
        assert_eq!(st.total_sessions, 3);
        assert_eq!(st.active_days, 2, "同一天的两条只算一天");
        assert!((st.average_total - 0.8).abs() < 1e-6, "平均 {}", st.average_total);
        assert_eq!(st.last_practiced_at.as_deref(), Some("2026-01-03T10:00:00Z"));
    }

    #[tokio::test]
    async fn stats_of_empty_db() {
        let s = Store::open_memory().await.unwrap();
        let st = s.stats().await.unwrap();
        assert_eq!(st.total_sessions, 0);
        assert_eq!(st.active_days, 0);
        assert!(st.last_practiced_at.is_none());
    }

    #[tokio::test]
    async fn delete_removes_row() {
        let s = Store::open_memory().await.unwrap();
        s.insert_many(vec![rec("a", 0.5, "2026-01-01T00:00:00Z")]).await.unwrap();
        assert!(s.delete("a".into()).await.unwrap());
        assert!(!s.exists("a").await.unwrap());
        assert!(!s.delete("a".into()).await.unwrap(), "删不存在的应返回 false");
    }

    #[tokio::test]
    async fn unknown_kind_falls_back_instead_of_erroring() {
        let s = Store::open_memory().await.unwrap();
        // 模拟旧版本写进去的、当前代码不认识的值
        s.conn
            .call(|c| {
                c.execute(
                    "INSERT INTO practices (id,kind,total,at) VALUES ('x','legacy_kind',0.5,'2026-01-01T00:00:00Z')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let rows = s.list(None, 10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, PracticeKind::Shadow, "未知类型应退回默认值");
    }
}
