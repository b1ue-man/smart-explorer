use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

pub use super::state_types::{ItemRecord, PairRecord, Side};
use super::state_validation::{
    baseline_from_items, parse_bool, parse_side, parse_sig, validate_cursor, validate_item,
    validate_pair, validate_pair_load_bytes, validate_relative_path, StateBudget,
};
use super::types::{Baseline, Sig};

pub struct SyncStateStore {
    conn: Connection,
}

impl SyncStateStore {
    pub fn open_default() -> rusqlite::Result<Self> {
        Self::open_at(default_db_path())
    }

    pub fn open_at(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path)?;
        let store = SyncStateStore { conn };
        store.init()?;
        Ok(store)
    }

    fn init(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER NOT NULL
            );
            INSERT INTO schema_version(version)
                SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM schema_version);
            CREATE TABLE IF NOT EXISTS pairs (
                pair TEXT PRIMARY KEY,
                root_a TEXT NOT NULL,
                root_b TEXT NOT NULL,
                mode TEXT NOT NULL,
                source_side TEXT NOT NULL,
                source_cursor TEXT,
                root_a_id TEXT,
                root_b_id TEXT,
                bootstrapped INTEGER NOT NULL,
                target_managed INTEGER NOT NULL,
                updated_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS items (
                pair TEXT NOT NULL,
                side TEXT NOT NULL,
                rel TEXT NOT NULL,
                id TEXT,
                parent_id TEXT,
                name TEXT,
                size TEXT,
                mtime_ms INTEGER,
                hash TEXT,
                is_dir INTEGER NOT NULL,
                deleted INTEGER NOT NULL,
                updated_ms INTEGER NOT NULL,
                PRIMARY KEY(pair, side, rel)
            );
            CREATE INDEX IF NOT EXISTS idx_items_pair_side_id
                ON items(pair, side, id);
            ",
        )
    }

    pub fn load_pair(&self, pair: &str) -> rusqlite::Result<Option<PairRecord>> {
        let text_bytes = self
            .conn
            .query_row(
                "SELECT length(CAST(pair AS BLOB))
                      + length(CAST(root_a AS BLOB))
                      + length(CAST(root_b AS BLOB))
                      + length(CAST(mode AS BLOB))
                      + length(CAST(source_side AS BLOB))
                      + COALESCE(length(CAST(source_cursor AS BLOB)), 0)
                      + COALESCE(length(CAST(root_a_id AS BLOB)), 0)
                      + COALESCE(length(CAST(root_b_id AS BLOB)), 0)
                 FROM pairs WHERE pair = ?1",
                [pair],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(text_bytes) = text_bytes {
            validate_pair_load_bytes(text_bytes)?;
        }
        self.conn
            .query_row(
                "SELECT pair, root_a, root_b, mode, source_side, source_cursor,
                        root_a_id, root_b_id, bootstrapped, target_managed
                 FROM pairs WHERE pair = ?1",
                [pair],
                |r| {
                    let record = PairRecord {
                        pair: r.get(0)?,
                        root_a: r.get(1)?,
                        root_b: r.get(2)?,
                        mode: r.get(3)?,
                        source_side: parse_side(4, &r.get::<_, String>(4)?)?,
                        source_cursor: r.get(5)?,
                        root_a_id: r.get(6)?,
                        root_b_id: r.get(7)?,
                        bootstrapped: parse_bool(8, r.get(8)?)?,
                        target_managed: parse_bool(9, r.get(9)?)?,
                    };
                    validate_pair(&record)?;
                    Ok(record)
                },
            )
            .optional()
    }

    pub fn save_pair(&self, rec: &PairRecord) -> rusqlite::Result<()> {
        validate_pair(rec)?;
        self.conn.execute(
            "INSERT INTO pairs(pair, root_a, root_b, mode, source_side, source_cursor,
                 root_a_id, root_b_id, bootstrapped, target_managed, updated_ms)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(pair) DO UPDATE SET
                 root_a = excluded.root_a,
                 root_b = excluded.root_b,
                 mode = excluded.mode,
                 source_side = excluded.source_side,
                 source_cursor = excluded.source_cursor,
                 root_a_id = excluded.root_a_id,
                 root_b_id = excluded.root_b_id,
                 bootstrapped = excluded.bootstrapped,
                 target_managed = excluded.target_managed,
                 updated_ms = excluded.updated_ms",
            params![
                rec.pair,
                rec.root_a,
                rec.root_b,
                rec.mode,
                rec.source_side.as_str(),
                rec.source_cursor,
                rec.root_a_id,
                rec.root_b_id,
                rec.bootstrapped as i64,
                rec.target_managed as i64,
                now_ms(),
            ],
        )?;
        Ok(())
    }

    pub fn update_cursor(&self, pair: &str, cursor: Option<&str>) -> rusqlite::Result<()> {
        validate_cursor(cursor)?;
        self.conn.execute(
            "UPDATE pairs SET source_cursor = ?2, updated_ms = ?3 WHERE pair = ?1",
            params![pair, cursor, now_ms()],
        )?;
        Ok(())
    }

    pub fn save_items(&mut self, pair: &str, items: &[ItemRecord]) -> rusqlite::Result<()> {
        let mut budget = StateBudget::for_pair();
        for item in items {
            budget.record_item(item)?;
        }
        let tx = self.conn.transaction()?;
        for item in items {
            upsert_item_tx(&tx, pair, item)?;
        }
        tx.commit()
    }

    pub fn save_items_and_cursor(
        &mut self,
        pair: &str,
        items: &[ItemRecord],
        cursor: Option<&str>,
    ) -> rusqlite::Result<()> {
        validate_cursor(cursor)?;
        let mut budget = StateBudget::for_pair();
        for item in items {
            budget.record_item(item)?;
        }
        let tx = self.conn.transaction()?;
        for item in items {
            upsert_item_tx(&tx, pair, item)?;
        }
        let changed = tx.execute(
            "UPDATE pairs SET source_cursor = ?2, updated_ms = ?3 WHERE pair = ?1",
            params![pair, cursor, now_ms()],
        )?;
        if changed != 1 {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        }
        tx.commit()
    }

    pub fn replace_from_baseline(
        &mut self,
        pair: &str,
        baseline: &Baseline,
        ids_a: &BTreeMap<String, (Option<String>, Option<String>)>,
        ids_b: &BTreeMap<String, (Option<String>, Option<String>)>,
    ) -> rusqlite::Result<()> {
        let mut budget = StateBudget::for_pair();
        for (rel, (a, b)) in baseline {
            for (side, sig, ids) in [(Side::A, *a, ids_a.get(rel)), (Side::B, *b, ids_b.get(rel))] {
                budget.record_item(&ItemRecord {
                    side,
                    rel: rel.clone(),
                    id: ids.and_then(|value| value.0.clone()),
                    parent_id: ids.and_then(|value| value.1.clone()),
                    name: rel.rsplit('/').next().map(str::to_owned),
                    sig,
                    is_dir: false,
                    deleted: sig.is_none(),
                })?;
            }
        }
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM items WHERE pair = ?1", [pair])?;
        for (rel, (a, b)) in baseline {
            let name = rel.rsplit('/').next().map(|s| s.to_string());
            upsert_item_tx(
                &tx,
                pair,
                &ItemRecord {
                    side: Side::A,
                    rel: rel.clone(),
                    id: ids_a.get(rel).and_then(|v| v.0.clone()),
                    parent_id: ids_a.get(rel).and_then(|v| v.1.clone()),
                    name: name.clone(),
                    sig: *a,
                    is_dir: false,
                    deleted: a.is_none(),
                },
            )?;
            upsert_item_tx(
                &tx,
                pair,
                &ItemRecord {
                    side: Side::B,
                    rel: rel.clone(),
                    id: ids_b.get(rel).and_then(|v| v.0.clone()),
                    parent_id: ids_b.get(rel).and_then(|v| v.1.clone()),
                    name,
                    sig: *b,
                    is_dir: false,
                    deleted: b.is_none(),
                },
            )?;
        }
        tx.commit()
    }

    pub fn load_side(
        &self,
        pair: &str,
        side: Side,
    ) -> rusqlite::Result<BTreeMap<String, ItemRecord>> {
        let mut budget = StateBudget::new();
        self.load_side_with_budget(pair, side, &mut budget)
    }

    pub fn load_pair_items(
        &self,
        pair: &str,
    ) -> rusqlite::Result<(BTreeMap<String, ItemRecord>, BTreeMap<String, ItemRecord>)> {
        let mut budget = StateBudget::for_pair();
        let a = self.load_side_with_budget(pair, Side::A, &mut budget)?;
        let b = self.load_side_with_budget(pair, Side::B, &mut budget)?;
        Ok((a, b))
    }

    fn load_side_with_budget(
        &self,
        pair: &str,
        side: Side,
        budget: &mut StateBudget,
    ) -> rusqlite::Result<BTreeMap<String, ItemRecord>> {
        let (nodes, text_bytes) = self.conn.query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(
                        length(CAST(rel AS BLOB))
                      + COALESCE(length(CAST(id AS BLOB)), 0)
                      + COALESCE(length(CAST(parent_id AS BLOB)), 0)
                      + COALESCE(length(CAST(name AS BLOB)), 0)
                    ), 0)
             FROM items
             WHERE pair = ?1 AND (side = ?2 OR side NOT IN ('A', 'B'))",
            params![pair, side.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        budget.ensure_fits(nodes, text_bytes)?;
        let mut stmt = self.conn.prepare(
            "SELECT side, rel, id, parent_id, name, size, mtime_ms, hash, is_dir, deleted
             FROM items
             WHERE pair = ?1 AND (side = ?2 OR side NOT IN ('A', 'B'))",
        )?;
        let mut rows = stmt.query(params![pair, side.as_str()])?;
        let mut out = BTreeMap::new();
        while let Some(row) = rows.next()? {
            let item = ItemRecord {
                side: parse_side(0, &row.get::<_, String>(0)?)?,
                rel: row.get(1)?,
                id: row.get(2)?,
                parent_id: row.get(3)?,
                name: row.get(4)?,
                sig: parse_sig(5, row.get(5)?, row.get(6)?, row.get(7)?)?,
                is_dir: parse_bool(8, row.get(8)?)?,
                deleted: parse_bool(9, row.get(9)?)?,
            };
            budget.record_item(&item)?;
            if item.side != side || out.insert(item.rel.clone(), item).is_some() {
                return Err(rusqlite::Error::InvalidQuery);
            }
        }
        Ok(out)
    }

    pub fn load_baseline(&self, pair: &str) -> rusqlite::Result<Baseline> {
        let (a, b) = self.load_pair_items(pair)?;
        Ok(baseline_from_items(&a, &b))
    }

    pub fn rel_for_id(&self, pair: &str, side: Side, id: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT rel FROM items
                 WHERE pair = ?1 AND side = ?2 AND id = ?3 AND deleted = 0
                 LIMIT 1",
                params![pair, side.as_str(), id],
                |row| {
                    let rel: String = row.get(0)?;
                    validate_relative_path(0, &rel)?;
                    Ok(rel)
                },
            )
            .optional()
    }
}

fn upsert_item_tx(
    tx: &rusqlite::Transaction<'_>,
    pair: &str,
    item: &ItemRecord,
) -> rusqlite::Result<()> {
    validate_item(item)?;
    let (size, mtime, hash) = match item.sig {
        Some(sig) => (
            Some(sig.size.to_string()),
            Some(sig.mtime_ms),
            Some(sig.hash.to_string()),
        ),
        None => (None, None, None),
    };
    tx.execute(
        "INSERT INTO items(pair, side, rel, id, parent_id, name, size, mtime_ms, hash,
             is_dir, deleted, updated_ms)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
         ON CONFLICT(pair, side, rel) DO UPDATE SET
             id = excluded.id,
             parent_id = excluded.parent_id,
             name = excluded.name,
             size = excluded.size,
             mtime_ms = excluded.mtime_ms,
             hash = excluded.hash,
             is_dir = excluded.is_dir,
             deleted = excluded.deleted,
             updated_ms = excluded.updated_ms",
        params![
            pair,
            item.side.as_str(),
            item.rel,
            item.id,
            item.parent_id,
            item.name,
            size,
            mtime,
            hash,
            item.is_dir as i64,
            item.deleted as i64,
            now_ms(),
        ],
    )?;
    Ok(())
}

fn default_db_path() -> PathBuf {
    crate::support_dirs::sync_data_dir().join("sync_state.sqlite")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
#[path = "tests/state_store.rs"]
mod tests;
