//! SQLite index for session records.
use crate::models::SessionRecord;
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

pub struct Index {
    conn: Connection,
}

impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).context("open sqlite")?;
        let idx = Self { conn };
        idx.migrate()?;
        Ok(idx)
    }

    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let idx = Self { conn };
        idx.migrate()?;
        Ok(idx)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                tool TEXT NOT NULL,
                model TEXT NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                tools_proposed INTEGER NOT NULL,
                tools_accepted INTEGER NOT NULL,
                source TEXT NOT NULL,
                cost_complete INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at);
            CREATE INDEX IF NOT EXISTS idx_sessions_tool ON sessions(tool);
            "#,
        )?;
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        self.conn.execute("DELETE FROM sessions", [])?;
        Ok(())
    }

    pub fn upsert_many(&self, rows: &[SessionRecord]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                r#"
                INSERT INTO sessions (
                    id, tool, model, started_at, ended_at,
                    input_tokens, output_tokens, tools_proposed, tools_accepted,
                    source, cost_complete
                ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                ON CONFLICT(id) DO UPDATE SET
                    tool=excluded.tool,
                    model=excluded.model,
                    started_at=excluded.started_at,
                    ended_at=excluded.ended_at,
                    input_tokens=excluded.input_tokens,
                    output_tokens=excluded.output_tokens,
                    tools_proposed=excluded.tools_proposed,
                    tools_accepted=excluded.tools_accepted,
                    source=excluded.source,
                    cost_complete=excluded.cost_complete
                "#,
            )?;
            for r in rows {
                stmt.execute(params![
                    r.id,
                    r.tool,
                    r.model,
                    r.started_at.to_rfc3339(),
                    r.ended_at.to_rfc3339(),
                    r.input_tokens,
                    r.output_tokens,
                    r.tools_proposed,
                    r.tools_accepted,
                    r.source,
                    if r.cost_complete { 1 } else { 0 },
                ])?;
            }
        }
        tx.commit()?;
        Ok(rows.len())
    }

    pub fn sessions_since(&self, since: &str) -> Result<Vec<SessionRecord>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, tool, model, started_at, ended_at,
                   input_tokens, output_tokens, tools_proposed, tools_accepted,
                   source, cost_complete
            FROM sessions
            WHERE started_at >= ?1
            ORDER BY started_at DESC
            "#,
        )?;
        let iter = stmt.query_map(params![since], |row| {
            let started: String = row.get(3)?;
            let ended: String = row.get(4)?;
            Ok(SessionRecord {
                id: row.get(0)?,
                tool: row.get(1)?,
                model: row.get(2)?,
                started_at: chrono::DateTime::parse_from_rfc3339(&started)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                ended_at: chrono::DateTime::parse_from_rfc3339(&ended)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                input_tokens: row.get(5)?,
                output_tokens: row.get(6)?,
                tools_proposed: row.get(7)?,
                tools_accepted: row.get(8)?,
                source: row.get(9)?,
                cost_complete: row.get::<_, i64>(10)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn count(&self) -> Result<i64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))?;
        Ok(n)
    }
}
