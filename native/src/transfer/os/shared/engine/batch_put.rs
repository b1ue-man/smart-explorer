//! Upload packets: the files of a packet stream one after another from disk
//! into `put_batch` (nothing is buffered first, K7). The chunk that completes
//! a file goes out only when the file ended there and did not change while
//! it was read; otherwise the packet stream fails at that file, so the peer
//! discards it and publishes nothing changed. What the stream delivered
//! decides which members of a failed packet may go again.
use super::super::engine_names::{base_name, parent_rel};
use super::super::flow::classify_error;
use super::super::flow_control::OpOutcome;
use super::super::memory::reserve_memory;
use super::batch::{Attempt, Failures};
use super::ops::{nanos, relabeled, At, Meter, OpError, Outcome};
use super::queue::FileWork;
use super::source::LocalSource;
use super::worker::{finished, run_file};
use super::Engine;
use crate::vfs::{Backend, BatchPut, BatchPutOutcome};
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub(super) fn upload(
    engine: &Engine<'_>,
    target: &dyn Backend,
    files: Vec<FileWork>,
    buffer: &mut [u8],
) {
    let mut opened = Vec::with_capacity(files.len());
    for file in files {
        match LocalSource::open(&file.source) {
            Ok(source) => opened.push((file, source)),
            // Alone again: that path asks for access or reports the reason.
            Err(_) => run_file(engine, file, buffer),
        }
    }
    if opened.len() < 2 {
        for (file, source) in opened {
            drop(source);
            run_file(engine, file, buffer);
        }
        return;
    }
    let bytes: u64 = opened.iter().map(|(_, source)| source.length()).sum();
    let memory_asked = Instant::now();
    let Some(reservation) = reserve_memory(bytes, engine.stop_flag()) else {
        return;
    };
    let memory_wait = nanos(memory_asked.elapsed());
    engine.stats.book_memory_wait(memory_wait);
    let permit_asked = Instant::now();
    let Some(permits) = engine.acquire() else {
        return;
    };
    let waited = memory_wait.saturating_add(nanos(permit_asked.elapsed()));
    let active = engine
        .stats
        .begin(&format!("{} kleine Dateien (Paket)", opened.len()));
    engine.queue.notify();
    let entries: Vec<BatchPut> = opened
        .iter()
        .map(|(file, source)| BatchPut {
            path: engine.folders.path_of(&file.rel),
            size: source.length(),
        })
        .collect();
    let meter = Meter::new(&permits, &engine.stats);
    meter.add_wait(waited);
    let started = Instant::now();
    let read_before = meter.accounts().source_ns;
    let (result, delivery) = {
        let mut data = PacketData::new(
            opened.iter_mut().map(|(_, source)| source).collect(),
            &meter,
            engine.stop_flag(),
        );
        let result = target.put_batch(&entries, &mut data);
        (result, data.delivery())
    };
    // The packet's reads are booked to the source as they happen; the rest
    // of the call is the wire and the peer: the target.
    let read = meter.accounts().source_ns.saturating_sub(read_before);
    meter.add_target_ns(nanos(started.elapsed()).saturating_sub(read));
    let accounts = meter.accounts();
    permits.finish_with(
        match &result {
            Ok(_) => OpOutcome::Done,
            Err(error) => classify_error(error),
        },
        accounts,
    );
    drop(active);
    drop(reservation);
    super::lock(&engine.sizer).record(bytes, started.elapsed().as_millis() as u64);
    let files: Vec<FileWork> = opened.into_iter().map(|(file, _)| file).collect();
    let outcomes = match result {
        Ok(outcomes) => outcomes,
        // Refused before anything was sent: each file goes alone.
        Err(error) if error.kind() == io::ErrorKind::Unsupported => {
            engine.stats.unmoved(delivery.served.iter().sum());
            for file in files {
                run_file(engine, file, buffer);
            }
            return;
        }
        Err(error) => return packet_lost(engine, files, &delivery, &error),
    };
    let mut failures = Failures::new(engine);
    let mut outcomes = outcomes.into_iter();
    for (index, file) in files.into_iter().enumerate() {
        match outcomes.next() {
            Some(BatchPutOutcome::Published(path)) => {
                if parent_rel(&file.rel).is_none() {
                    engine.folders.record_alias(&file.rel, base_name(&path));
                }
                finished(engine, Outcome::Done);
            }
            Some(BatchPutOutcome::Failed(error)) => {
                engine.stats.unmoved(delivery.served[index]);
                let (failure, mut attempt) = delivery.judge(index, OpError::target(error));
                // The peer says it published nothing for this file: it may
                // go again once, like a file whose bytes were refused.
                if attempt == Attempt::MaybePublished && crate::vfs::unpublished_of(&failure.error)
                {
                    attempt = Attempt::NotPublished;
                }
                failures.member(file, failure, attempt);
            }
            None => {
                engine.stats.unmoved(delivery.served[index]);
                let missing =
                    io::Error::other("Paket-Upload lieferte kein Ergebnis für diese Datei");
                let (failure, attempt) = delivery.judge(index, OpError::at(At::Unknown, missing));
                failures.member(file, failure, attempt);
            }
        }
    }
    failures.finish();
}

/// The packet's answer was lost (`Err`): what was not completely sent goes
/// again, everything else has an unknown result.
fn packet_lost(engine: &Engine<'_>, files: Vec<FileWork>, delivery: &Delivery, error: &io::Error) {
    engine.stats.unmoved(delivery.served.iter().sum());
    if engine.stopped() {
        return;
    }
    let mut failures = Failures::new(engine);
    for (index, file) in files.into_iter().enumerate() {
        let lost = OpError::at(At::Unknown, relabeled(error, "Paket-Upload"));
        let (failure, attempt) = delivery.judge(index, lost);
        failures.member(file, failure, attempt);
    }
    failures.finish();
}

/// What the packet stream handed to the peer.
struct Delivery {
    lengths: Vec<u64>,
    /// Bytes of each entry the stream delivered (and the progress counted).
    served: Vec<u64>,
    /// The entry whose read or final check failed; the stream ended there.
    failed: Option<(usize, io::ErrorKind, String)>,
}

impl Delivery {
    /// How a failed entry is treated. Its own read or check failure is its
    /// reason (discarded by the peer); an entry whose bytes did not all go
    /// out, or that came after the failed one, was never tried; any other
    /// entry may have been published.
    fn judge(&self, index: usize, failure: OpError) -> (OpError, Attempt) {
        match &self.failed {
            Some((at, kind, message)) if *at == index => (
                OpError::source(io::Error::new(*kind, message.clone())),
                Attempt::NotPublished,
            ),
            Some((at, ..)) if index > *at => (failure, Attempt::Untried),
            _ if self.served[index] < self.lengths[index] => (failure, Attempt::Untried),
            _ => (failure, Attempt::MaybePublished),
        }
    }
}

/// The files of a packet back to back, read straight from disk.
struct PacketData<'s, 'm> {
    sources: Vec<&'s mut LocalSource>,
    index: usize,
    sent: u64,
    served: Vec<u64>,
    failed: Option<(usize, io::ErrorKind, String)>,
    meter: &'s Meter<'m>,
    stop: &'s AtomicBool,
}

impl<'s, 'm> PacketData<'s, 'm> {
    fn new(sources: Vec<&'s mut LocalSource>, meter: &'s Meter<'m>, stop: &'s AtomicBool) -> Self {
        let served = vec![0; sources.len()];
        Self {
            sources,
            index: 0,
            sent: 0,
            served,
            failed: None,
            meter,
            stop,
        }
    }

    fn delivery(self) -> Delivery {
        Delivery {
            lengths: self.sources.iter().map(|source| source.length()).collect(),
            served: self.served,
            failed: self.failed,
        }
    }
}

impl Read for PacketData<'_, '_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.stop.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    super::super::cancel::CANCELED_ERROR,
                ));
            }
            if let Some((_, kind, message)) = &self.failed {
                return Err(io::Error::new(*kind, message.clone()));
            }
            let Some(source) = self.sources.get_mut(self.index) else {
                return Ok(0);
            };
            if self.sent >= source.length() {
                self.index += 1;
                self.sent = 0;
                continue;
            }
            if buffer.is_empty() {
                return Ok(0);
            }
            match self
                .meter
                .timed_source(|| source.read_entry(buffer, self.sent))
            {
                Ok(read) => {
                    self.sent += read as u64;
                    self.served[self.index] += read as u64;
                    self.meter.add(read as u64);
                    return Ok(read);
                }
                Err(failure) => {
                    let (kind, message) = (failure.error.kind(), failure.error.to_string());
                    self.failed = Some((self.index, kind, message.clone()));
                    return Err(io::Error::new(kind, message));
                }
            }
        }
    }
}

#[cfg(test)]
pub(super) mod speed_tests {
    use super::super::test_backend::{job, run, write, Fake};
    use crate::transfer::Endpoint;
    use crate::vfs::{
        unpublished_error, Backend, BackendHandle, BatchLimits, BatchPut, BatchPutOutcome, Scheme,
        VfsMeta, VfsResult,
    };
    use std::io::{self, Read, Write};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// Forwards the listed methods to the wrapped fake remote.
    macro_rules! forward {
        ($($name:ident($($arg:ident: $ty:ty),*) -> $ret:ty;)*) => {
            $(fn $name(&self, $($arg: $ty),*) -> $ret { self.0.$name($($arg),*) })*
        };
    }

    pub(in super::super) use forward;

    /// A packet peer whose first packet refuses its first member before
    /// publishing anything; the rest of the fake behaves as usual.
    struct Refusing(Arc<Fake>, AtomicBool);

    impl Backend for Refusing {
        forward! {
            scheme() -> Scheme; root_display() -> String; namespace_identity() -> String;
            list_dir(path: &str) -> VfsResult<Vec<VfsMeta>>; stat(path: &str) -> VfsResult<VfsMeta>;
            open_read(path: &str) -> VfsResult<Box<dyn Read + Send>>;
            open_write(path: &str) -> VfsResult<Box<dyn Write + Send>>;
            open_write_new(path: &str) -> VfsResult<Box<dyn Write + Send>>;
            open_write_fresh(path: &str, size: u64) -> VfsResult<Option<Box<dyn Write + Send>>>;
            promote_copy_stage(stage: &str, destination: &str) -> VfsResult<()>;
            rename(src: &str, dst: &str) -> VfsResult<()>;
            rename_no_replace(src: &str, dst: &str) -> VfsResult<()>;
            remove_file(path: &str) -> VfsResult<()>; remove_dir(path: &str) -> VfsResult<()>;
            mkdir_all(path: &str) -> VfsResult<()>; create_dir(path: &str) -> VfsResult<()>;
            create_dir_new(path: &str) -> VfsResult<()>;
            discard_copy_stage(stage: &str) -> VfsResult<()>;
            case_sensitive_paths(root: &str) -> bool; flow_key(path: &str) -> String;
            concurrent_read_write() -> bool; batch_limits(dir: &str) -> Option<BatchLimits>;
        }

        fn put_batch(
            &self,
            entries: &[BatchPut],
            data: &mut dyn Read,
        ) -> VfsResult<Vec<BatchPutOutcome>> {
            let Some((first, rest)) = entries.split_first() else {
                return Ok(Vec::new());
            };
            if self.1.swap(true, Ordering::SeqCst) {
                return self.0.put_batch(entries, data);
            }
            data.read_exact(&mut vec![0u8; first.size as usize])?;
            let refused = unpublished_error(io::ErrorKind::ConnectionReset, "fixture: refused");
            let mut outcomes = vec![BatchPutOutcome::Failed(refused)];
            outcomes.extend(self.0.put_batch(rest, data)?);
            Ok(outcomes)
        }
    }

    #[test]
    fn speed_task_batch_unpublished_failure_is_retried_once() {
        let root = tempfile::tempdir().expect("temp dir");
        let local = root.path().join("local/Vault");
        for index in 0..4 {
            write(
                &local.join(format!("f{index}.txt")),
                format!("file {index}").as_bytes(),
            );
        }
        let remote = root.path().join("remote");
        std::fs::create_dir(&remote).expect("remote");
        let mut fake = Fake::new("unpublished-retry");
        fake.batch = Some(BatchLimits {
            max_files: 64,
            max_bytes: 1024 * 1024,
        });
        let (fake, _) = fake.handle();
        let handle: BackendHandle = Arc::new(Refusing(fake.clone(), AtomicBool::new(false)));
        let finished = run(job(
            Endpoint::Local,
            Endpoint::Remote(handle),
            &remote,
            &[&local],
        ));
        assert!(finished.issues.is_empty(), "{:?}", finished.issues);
        assert_eq!(finished.progress.files_done, 4);
        for index in 0..4 {
            let published = remote.join(format!("Vault/f{index}.txt"));
            assert_eq!(
                std::fs::read(&published).expect("published"),
                format!("file {index}").as_bytes()
            );
        }
        // Exactly one single upload: the refused member, sent once more.
        assert_eq!(fake.counters.stages.load(Ordering::SeqCst), 1);
    }

    /// A packet peer far away: every packet takes a while on the wire.
    struct SlowWire(Arc<Fake>);

    impl Backend for SlowWire {
        forward! {
            scheme() -> Scheme; root_display() -> String; namespace_identity() -> String;
            list_dir(path: &str) -> VfsResult<Vec<VfsMeta>>; stat(path: &str) -> VfsResult<VfsMeta>;
            open_read(path: &str) -> VfsResult<Box<dyn Read + Send>>;
            open_write(path: &str) -> VfsResult<Box<dyn Write + Send>>;
            open_write_new(path: &str) -> VfsResult<Box<dyn Write + Send>>;
            open_write_fresh(path: &str, size: u64) -> VfsResult<Option<Box<dyn Write + Send>>>;
            promote_copy_stage(stage: &str, destination: &str) -> VfsResult<()>;
            rename(src: &str, dst: &str) -> VfsResult<()>;
            rename_no_replace(src: &str, dst: &str) -> VfsResult<()>;
            remove_file(path: &str) -> VfsResult<()>; remove_dir(path: &str) -> VfsResult<()>;
            mkdir_all(path: &str) -> VfsResult<()>; create_dir(path: &str) -> VfsResult<()>;
            create_dir_new(path: &str) -> VfsResult<()>;
            discard_copy_stage(stage: &str) -> VfsResult<()>;
            case_sensitive_paths(root: &str) -> bool; flow_key(path: &str) -> String;
            transfer_ceiling(path: &str) -> Option<usize>;
            concurrent_read_write() -> bool; batch_limits(dir: &str) -> Option<BatchLimits>;
        }

        fn put_batch(
            &self,
            entries: &[BatchPut],
            data: &mut dyn Read,
        ) -> VfsResult<Vec<BatchPutOutcome>> {
            std::thread::sleep(std::time::Duration::from_millis(300));
            self.0.put_batch(entries, data)
        }
    }

    #[test]
    fn speed_task_batch_jobs_book_time_accounts() {
        // 200 tiny files in packets of 8, each packet 300 ms on the wire: the
        // time goes to the target side, and after three windows the progress
        // names the connection, which only packets-with-accounts can do
        let root = tempfile::tempdir().expect("temp dir");
        let local = root.path().join("local/Vault");
        for index in 0..200 {
            write(&local.join(format!("f{index:03}.txt")), b"small file");
        }
        let remote = root.path().join("remote");
        std::fs::create_dir(&remote).expect("remote");
        let mut fake = Fake::new("packet-accounts");
        fake.ceiling = Some(1);
        fake.batch = Some(BatchLimits {
            max_files: 8,
            max_bytes: 1024 * 1024,
        });
        let (fake, _) = fake.handle();
        let handle: BackendHandle = Arc::new(SlowWire(fake.clone()));
        let finished = run(job(
            Endpoint::Local,
            Endpoint::Remote(handle),
            &remote,
            &[&local],
        ));
        assert!(finished.issues.is_empty(), "{:?}", finished.issues);
        assert_eq!(finished.progress.files_done, 200);
        assert!(fake.counters.put_batches.load(Ordering::SeqCst) >= 20);
        let progress = &finished.progress;
        assert!(
            progress.limiter.names_a_resource(),
            "{:?} {:?}",
            progress.limiter,
            progress.limiter_text
        );
        let [source, target, _, _] = progress.limiter_shares;
        assert!(target > source, "{:?}", progress.limiter_shares);
    }
}
