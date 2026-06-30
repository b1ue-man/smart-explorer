use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use super::types::{Baseline, Sig};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

impl Side {
    pub fn as_str(self) -> &'static str {
        match self {
            Side::A => "A",
            Side::B => "B",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PairRecord {
    pub pair: String,
    pub root_a: String,
    pub root_b: String,
    pub mode: String,
    pub source_side: Side,
    pub source_cursor: Option<String>,
    pub root_a_id: Option<String>,
    pub root_b_id: Option<String>,
    pub bootstrapped: bool,
    pub target_managed: bool,
}

#[derive(Clone, Debug)]
pub struct ItemRecord {
    pub side: Side,
    pub rel: String,
    pub id: Option<String>,
    pub parent_id: Option<String>,
    pub name: Option<String>,
    pub sig: Option<Sig>,
    pub is_dir: bool,
    pub deleted: bool,
}

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
        self.conn
            .query_row(
                "SELECT pair, root_a, root_b, mode, source_side, source_cursor,
                        root_a_id, root_b_id, bootstrapped, target_managed
                 FROM pairs WHERE pair = ?1",
                [pair],
                |r| {
                    Ok(PairRecord {
                        pair: r.get(0)?,
                        root_a: r.get(1)?,
                        root_b: r.get(2)?,
                        mode: r.get(3)?,
                        source_side: parse_side(&r.get::<_, String>(4)?),
                        source_cursor: r.get(5)?,
                        root_a_id: r.get(6)?,
                        root_b_id: r.get(7)?,
                        bootstrapped: r.get::<_, i64>(8)? != 0,
                        target_managed: r.get::<_, i64>(9)? != 0,
                    })
                },
            )
            .optional()
    }

    pub fn save_pair(&self, rec: &PairRecord) -> rusqlite::Result<()> {
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
        self.conn.execute(
            "UPDATE pairs SET source_cursor = ?2, updated_ms = ?3 WHERE pair = ?1",
            params![pair, cursor, now_ms()],
        )?;
        Ok(())
    }

    pub fn save_items(&mut self, pair: &str, items: &[ItemRecord]) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        for item in items {
            upsert_item_tx(&tx, pair, item)?;
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
        let mut stmt = self.conn.prepare(
            "SELECT rel, id, parent_id, name, size, mtime_ms, hash, is_dir, deleted
             FROM items WHERE pair = ?1 AND side = ?2",
        )?;
        let mut rows = stmt.query(params![pair, side.as_str()])?;
        let mut out = BTreeMap::new();
        while let Some(row) = rows.next()? {
            let rel: String = row.get(0)?;
            out.insert(
                rel.clone(),
                ItemRecord {
                    side,
                    rel,
                    id: row.get(1)?,
                    parent_id: row.get(2)?,
                    name: row.get(3)?,
                    sig: parse_sig(row.get(4)?, row.get(5)?, row.get(6)?),
                    is_dir: row.get::<_, i64>(7)? != 0,
                    deleted: row.get::<_, i64>(8)? != 0,
                },
            );
        }
        Ok(out)
    }

    pub fn load_baseline(&self, pair: &str) -> rusqlite::Result<Baseline> {
        let a = self.load_side(pair, Side::A)?;
        let b = self.load_side(pair, Side::B)?;
        let mut rels: BTreeSet<String> = a.keys().cloned().collect();
        rels.extend(b.keys().cloned());
        let mut out = Baseline::new();
        for rel in rels {
            let sa = a
                .get(&rel)
                .and_then(|i| (!i.deleted).then_some(i.sig).flatten());
            let sb = b
                .get(&rel)
                .and_then(|i| (!i.deleted).then_some(i.sig).flatten());
            if sa.is_some() || sb.is_some() {
                out.insert(rel, (sa, sb));
            }
        }
        Ok(out)
    }

    pub fn rel_for_id(&self, pair: &str, side: Side, id: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT rel FROM items
                 WHERE pair = ?1 AND side = ?2 AND id = ?3 AND deleted = 0
                 LIMIT 1",
                params![pair, side.as_str(), id],
                |r| r.get(0),
            )
            .optional()
    }
}

fn upsert_item_tx(
    tx: &rusqlite::Transaction<'_>,
    pair: &str,
    item: &ItemRecord,
) -> rusqlite::Result<()> {
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

fn parse_side(s: &str) -> Side {
    if s == "B" {
        Side::B
    } else {
        Side::A
    }
}

fn parse_sig(size: Option<String>, mtime: Option<i64>, hash: Option<String>) -> Option<Sig> {
    Some(Sig {
        size: size?.parse().ok()?,
        mtime_ms: mtime?,
        hash: hash.and_then(|h| h.parse().ok()).unwrap_or(0),
    })
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
mod tests {
    use super::*;

    fn temp_db() -> PathBuf {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!(
            "se_sync_state_{}_{}.sqlite",
            std::process::id(),
            nanos
        ));
        p
    }

    #[test]
    fn pair_and_items_roundtrip() {
        let path = temp_db();
        let mut store = SyncStateStore::open_at(&path).unwrap();
        let pair = PairRecord {
            pair: "p".into(),
            root_a: "a".into(),
            root_b: "b".into(),
            mode: "mirror".into(),
            source_side: Side::A,
            source_cursor: Some("c1".into()),
            root_a_id: None,
            root_b_id: Some("root".into()),
            bootstrapped: true,
            target_managed: true,
        };
        store.save_pair(&pair).unwrap();
        store
            .save_items(
                "p",
                &[ItemRecord {
                    side: Side::A,
                    rel: "f.txt".into(),
                    id: Some("id1".into()),
                    parent_id: None,
                    name: Some("f.txt".into()),
                    sig: Some(Sig {
                        size: 3,
                        mtime_ms: 9,
                        hash: 7,
                    }),
                    is_dir: false,
                    deleted: false,
                }],
            )
            .unwrap();
        assert_eq!(
            store
                .load_pair("p")
                .unwrap()
                .unwrap()
                .source_cursor
                .as_deref(),
            Some("c1")
        );
        assert_eq!(
            store.rel_for_id("p", Side::A, "id1").unwrap().as_deref(),
            Some("f.txt")
        );
        assert_eq!(
            store.load_side("p", Side::A).unwrap()["f.txt"]
                .sig
                .unwrap()
                .hash,
            7
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn rollback_keeps_previous_item_on_failed_transaction() {
        let path = temp_db();
        let mut store = SyncStateStore::open_at(&path).unwrap();
        store
            .save_items(
                "p",
                &[ItemRecord {
                    side: Side::A,
                    rel: "f.txt".into(),
                    id: None,
                    parent_id: None,
                    name: None,
                    sig: Some(Sig {
                        size: 1,
                        mtime_ms: 1,
                        hash: 1,
                    }),
                    is_dir: false,
                    deleted: false,
                }],
            )
            .unwrap();
        let tx = store.conn.transaction().unwrap();
        upsert_item_tx(
            &tx,
            "p",
            &ItemRecord {
                side: Side::A,
                rel: "f.txt".into(),
                id: None,
                parent_id: None,
                name: None,
                sig: Some(Sig {
                    size: 2,
                    mtime_ms: 2,
                    hash: 2,
                }),
                is_dir: false,
                deleted: false,
            },
        )
        .unwrap();
        drop(tx);
        assert_eq!(
            store.load_side("p", Side::A).unwrap()["f.txt"]
                .sig
                .unwrap()
                .size,
            1
        );
        let _ = std::fs::remove_file(path);
    }
}
