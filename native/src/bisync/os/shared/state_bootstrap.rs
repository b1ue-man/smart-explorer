//! Cache bootstrap is one transaction: a cursor is never trusted before
//! all corresponding rows exist. Missing sides need no tombstone rows.
use super::state_store::{upsert_item_tx, write_pair, SyncStateStore};
use super::state_types::{ItemRecord, PairRecord, Side};
use super::state_validation::StateBudget;
use super::types::Baseline;
use rusqlite::params;
use std::collections::BTreeMap;

type Ids = BTreeMap<String, (Option<String>, Option<String>)>;

impl SyncStateStore {
    pub fn replace_from_baseline(
        &mut self,
        pair: &str,
        baseline: &Baseline,
        ids_a: &Ids,
        ids_b: &Ids,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        replace(&tx, pair, baseline, ids_a, ids_b)?;
        tx.commit()
    }

    pub(super) fn bootstrap(
        &mut self,
        record: &PairRecord,
        baseline: &Baseline,
        ids_a: &Ids,
        ids_b: &Ids,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        replace(&tx, &record.pair, baseline, ids_a, ids_b)?;
        write_pair(&tx, record)?;
        tx.commit()
    }

    pub(super) fn forget_pair_family(&mut self, pair: &str) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        for table in ["items", "pairs"] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE pair = ?1 OR pair LIKE ?2"),
                params![pair, format!("{pair}:%")],
            )?;
        }
        tx.commit()
    }

    pub(super) fn forget_owner(&mut self, token: &str) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        for table in ["items", "pairs"] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE instr(pair, ?1) > 0"),
                [format!(":{token}:")],
            )?;
        }
        tx.commit()
    }
}

fn replace(
    tx: &rusqlite::Transaction<'_>,
    pair: &str,
    baseline: &Baseline,
    ids_a: &Ids,
    ids_b: &Ids,
) -> rusqlite::Result<()> {
    let mut budget = StateBudget::for_pair();
    tx.execute("DELETE FROM items WHERE pair = ?1", [pair])?;
    for (rel, (a, b)) in baseline {
        for (side, sig, ids) in [(Side::A, *a, ids_a.get(rel)), (Side::B, *b, ids_b.get(rel))] {
            if sig.is_none() {
                continue;
            }
            let item = ItemRecord {
                side,
                rel: rel.clone(),
                id: ids.and_then(|ids| ids.0.clone()),
                parent_id: ids.and_then(|ids| ids.1.clone()),
                name: rel.rsplit('/').next().map(str::to_string),
                sig,
                is_dir: false,
                deleted: false,
            };
            budget.record_item(&item)?;
            upsert_item_tx(tx, pair, &item)?;
        }
    }
    Ok(())
}
