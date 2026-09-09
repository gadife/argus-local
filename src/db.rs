//! SQLite index for session records + shipping events.
use crate::models::{SessionRecord, ShippingEvent};
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
        // Local cache rebuilt each open — drop to pick up schema changes cheaply.
        self.conn.execute_batch(
            r#"
            DROP TABLE IF EXISTS sessions;
            DROP TABLE IF EXISTS shipping_events;
            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                tool TEXT NOT NULL,
                model TEXT NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                tokens_known INTEGER NOT NULL,
                tools_proposed INTEGER NOT NULL,
                tools_accepted INTEGER NOT NULL,
                source TEXT NOT NULL,
                cost_complete INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at);
            CREATE INDEX IF NOT EXISTS idx_sessions_tool ON sessions(tool);
            CREATE TABLE shipping_events (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                occurred_at TEXT NOT NULL,
                files_touched INTEGER NOT NULL,
                repo TEXT NOT NULL,
                title TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_shipping_occurred ON shipping_events(occurred_at);
            CREATE INDEX IF NOT EXISTS idx_shipping_kind ON shipping_events(kind);
            "#,
        )?;
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        self.conn.execute("DELETE FROM sessions", [])?;
        self.conn.execute("DELETE FROM shipping_events", [])?;
        Ok(())
    }

    pub fn upsert_many(&self, rows: &[SessionRecord]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                r#"
                INSERT INTO sessions (
                    id, tool, model, started_at, ended_at,
                    input_tokens, output_tokens, tokens_known,
                    tools_proposed, tools_accepted,
                    source, cost_complete
                ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
                ON CONFLICT(id) DO UPDATE SET
                    tool=excluded.tool,
                    model=excluded.model,
                    started_at=excluded.started_at,
                    ended_at=excluded.ended_at,
                    input_tokens=excluded.input_tokens,
                    output_tokens=excluded.output_tokens,
                    tokens_known=excluded.tokens_known,
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
                    if r.tokens_known { 1 } else { 0 },
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

    pub fn upsert_shipping(&self, rows: &[ShippingEvent]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                r#"
                INSERT INTO shipping_events (
                    id, kind, occurred_at, files_touched, repo, title
                ) VALUES (?1,?2,?3,?4,?5,?6)
                ON CONFLICT(id) DO UPDATE SET
                    kind=excluded.kind,
                    occurred_at=excluded.occurred_at,
                    files_touched=excluded.files_touched,
                    repo=excluded.repo,
                    title=excluded.title
                "#,
            )?;
            for r in rows {
                stmt.execute(params![
                    r.id,
                    r.kind,
                    r.occurred_at.to_rfc3339(),
                    r.files_touched,
                    r.repo,
                    r.title,
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
                   input_tokens, output_tokens, tokens_known,
                   tools_proposed, tools_accepted,
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
                tokens_known: row.get::<_, i64>(7)? != 0,
                tools_proposed: row.get(8)?,
                tools_accepted: row.get(9)?,
                source: row.get(10)?,
                cost_complete: row.get::<_, i64>(11)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn shipping_events(&self) -> Result<Vec<ShippingEvent>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, kind, occurred_at, files_touched, repo, title
            FROM shipping_events
            ORDER BY occurred_at DESC
            "#,
        )?;
        let iter = stmt.query_map([], |row| {
            let occurred: String = row.get(2)?;
            Ok(ShippingEvent {
                id: row.get(0)?,
                kind: row.get(1)?,
                occurred_at: chrono::DateTime::parse_from_rfc3339(&occurred)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now()),
                files_touched: row.get(3)?,
                repo: row.get(4)?,
                title: row.get(5)?,
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

    pub fn shipping_count(&self) -> Result<i64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM shipping_events", [], |r| r.get(0))?;
        Ok(n)
    }
}
