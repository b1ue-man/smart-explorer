use super::prelude::*;
use super::*;

impl App {
    /// Begin a line-merge for one conflict: read both versions off-thread, diff.
    pub(in crate::app) fn start_merge(&mut self, rel: String) {
        if self.conflict_resolution.is_some() || self.conflict_bulk.is_some() {
            self.error_msg =
                Some("Eine laufende Konfliktauflösung muss zuerst beendet werden.".into());
            return;
        }
        let ctx = match &self.bisync_ctx {
            Some(c) => c,
            None => return,
        };
        let (a, ra, b, rb) = (
            ctx.a.clone(),
            ctx.root_a.clone(),
            ctx.b.clone(),
            ctx.root_b.clone(),
        );
        let rel_t = rel.clone();
        let (tx, rx) = unbounded();
        let spawned = std::thread::Builder::new()
            .name("merge-load".into())
            .spawn(move || {
                let res = (|| -> Result<(String, Vec<crate::linemerge::Row>), String> {
                    let ta = read_text(&*a, &ep_join(&ra, &rel_t))?;
                    let tb = read_text(&*b, &ep_join(&rb, &rel_t))?;
                    let rows = crate::linemerge::rows(&ta, &tb).map_err(|e| e.to_string())?;
                    Ok((rel_t.clone(), rows))
                })();
                let _ = tx.send(res);
            });
        if let Err(error) = spawned {
            self.error_msg = Some(format!(
                "Zusammenführen konnte nicht gestartet werden: {error}"
            ));
            self.merge_load_rx = None;
            self.merge = None;
            return;
        }
        self.merge_load_rx = Some(rx);
        self.merge = Some(MergeUi {
            rel,
            rows: Vec::new(),
        });
    }

    pub(in crate::app) fn drain_merge(&mut self) {
        match poll_merge_result(&self.merge_load_rx) {
            MergePoll::Ready(result) => {
                self.merge_load_rx = None;
                match result {
                    Ok((rel, rows)) => self.merge = Some(MergeUi { rel, rows }),
                    Err(error) => {
                        self.error_msg = Some(format!("Zusammenführen: {error}"));
                        self.merge = None;
                    }
                }
            }
            MergePoll::Disconnected => {
                self.merge_load_rx = None;
                self.merge = None;
                self.error_msg = Some("Zusammenführen wurde unerwartet abgebrochen".into());
            }
            MergePoll::Pending => {}
        }
        match poll_merge_result(&self.merge_apply_rx) {
            MergePoll::Ready(result) => {
                self.merge_apply_rx = None;
                match result {
                    Ok((rel, sa, sb)) => {
                        let Some(ctx) = self.bisync_ctx.as_mut() else {
                            self.error_msg = Some(
                                "Zusammenführung wurde geschrieben, aber der Synchronisationskontext fehlt; bitte erneut vergleichen."
                                    .into(),
                            );
                            return;
                        };
                        ctx.baseline.insert(rel.clone(), (Some(sa), Some(sb)));
                        self.conflict_baseline_dirty = true;
                        if let Some(index) = self
                            .bisync_conflicts
                            .iter()
                            .position(|conflict| conflict.rel == rel)
                        {
                            self.bisync_conflicts.swap_remove(index);
                        }
                        let persisted =
                            !self.bisync_conflicts.is_empty() || self.finish_bisync_conflicts();
                        if persisted {
                            self.notice = Some((
                                format!("✓ „{rel}“ zusammengeführt"),
                                std::time::Instant::now(),
                            ));
                        }
                    }
                    Err(error) => self.error_msg = Some(format!("Zusammenführen: {error}")),
                }
            }
            MergePoll::Disconnected => {
                self.merge_apply_rx = None;
                self.error_msg = Some("Zusammenführen wurde unerwartet abgebrochen".into());
            }
            MergePoll::Pending => {}
        }
    }

    /// Write the merged text to both sides off-thread, then resolve the conflict.
    pub(in crate::app) fn start_merge_apply(&mut self, rel: String, merged: String) {
        let Some((a, ra, b, rb)) = self.merge_endpoints() else {
            return;
        };
        let (tx, rx) = unbounded();
        let spawned = std::thread::Builder::new()
            .name("merge-apply".into())
            .spawn(move || {
                let result =
                    (|| -> Result<(String, crate::bisync::Sig, crate::bisync::Sig), String> {
                        let pa = ep_join(&ra, &rel);
                        let pb = ep_join(&rb, &rel);
                        write_bytes(&*a, &pa, merged.as_bytes())?;
                        write_bytes(&*b, &pb, merged.as_bytes())?;
                        Ok((rel.clone(), sig_from(&*a, &pa)?, sig_from(&*b, &pb)?))
                    })();
                let _ = tx.send(result);
            });
        self.set_merge_apply_worker(spawned, rx);
    }

    /// Keep both versions as separate files on both sides.
    pub(in crate::app) fn start_merge_keep_both(
        &mut self,
        rel: String,
        a_full: String,
        b_full: String,
    ) {
        let Some((a, ra, b, rb)) = self.merge_endpoints() else {
            return;
        };
        let (tx, rx) = unbounded();
        let spawned = std::thread::Builder::new()
            .name("merge-keepboth".into())
            .spawn(move || {
                let result =
                    (|| -> Result<(String, crate::bisync::Sig, crate::bisync::Sig), String> {
                        let crel = conflict_rel_name(&rel);
                        let pa = ep_join(&ra, &rel);
                        let pb = ep_join(&rb, &rel);
                        write_bytes(&*a, &pa, a_full.as_bytes())?;
                        write_bytes(&*b, &pb, a_full.as_bytes())?;
                        write_bytes(&*a, &ep_join(&ra, &crel), b_full.as_bytes())?;
                        write_bytes(&*b, &ep_join(&rb, &crel), b_full.as_bytes())?;
                        Ok((rel.clone(), sig_from(&*a, &pa)?, sig_from(&*b, &pb)?))
                    })();
                let _ = tx.send(result);
            });
        self.set_merge_apply_worker(spawned, rx);
    }

    fn merge_endpoints(
        &mut self,
    ) -> Option<(
        crate::vfs::BackendHandle,
        String,
        crate::vfs::BackendHandle,
        String,
    )> {
        let Some(ctx) = &self.bisync_ctx else {
            self.error_msg = Some("Zusammenführen: Synchronisationskontext fehlt".into());
            return None;
        };
        Some((
            ctx.a.clone(),
            ctx.root_a.clone(),
            ctx.b.clone(),
            ctx.root_b.clone(),
        ))
    }

    fn set_merge_apply_worker<T>(
        &mut self,
        spawned: std::io::Result<std::thread::JoinHandle<T>>,
        rx: Receiver<Result<(String, crate::bisync::Sig, crate::bisync::Sig), String>>,
    ) {
        match spawned {
            Ok(_) => self.merge_apply_rx = Some(rx),
            Err(error) => {
                self.merge_apply_rx = None;
                self.error_msg = Some(format!(
                    "Zusammenführen konnte nicht gestartet werden: {error}"
                ));
            }
        }
    }
}

enum MergePoll<T> {
    Pending,
    Ready(T),
    Disconnected,
}

fn poll_merge_result<T>(rx: &Option<Receiver<T>>) -> MergePoll<T> {
    let Some(rx) = rx else {
        return MergePoll::Pending;
    };
    match rx.try_recv() {
        Ok(value) => MergePoll::Ready(value),
        Err(crossbeam_channel::TryRecvError::Empty) => MergePoll::Pending,
        Err(crossbeam_channel::TryRecvError::Disconnected) => MergePoll::Disconnected,
    }
}
