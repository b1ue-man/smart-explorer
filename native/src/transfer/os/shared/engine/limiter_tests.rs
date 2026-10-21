//! The engine side of "who limits": the time accounts of an operation, the
//! texts built from a verdict, a refused thread as overload, and a job that
//! tells what limited it.
use super::bottleneck::{describe, endpoint_name, NamesView};
use super::dispatch::refuse_next_starts;
use super::ops::Meter;
use super::stats::Stats;
use super::test_backend::{fwd, job, run, unique, write, Fake};
use super::Side;
use crate::transfer::flow::{acquire_pair, flow, flow_for};
use crate::transfer::limiter::Limiter;
use crate::transfer::{Endpoint, TransferMsg};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

const MS: u64 = 1_000_000;

fn sleep(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

#[test]
fn speed_task_meter_accounts_split_source_target_cpu() {
    let flow = flow(unique("meter"), Some(2));
    let cancel = AtomicBool::new(false);
    let permits = acquire_pair(&flow, None, 1, &cancel).expect("a permit");
    let stats = Stats::new();
    let meter = Meter::new(&permits, &stats);
    assert_eq!(meter.timed_source(|| 7), 7, "the result passes through");
    meter.timed_source(|| sleep(30));
    meter.timed_target(|| sleep(20));
    meter.timed_cpu(|| sleep(10));
    meter.add_wait(5 * MS);
    meter.add_source_ns(2 * MS);
    let accounts = meter.accounts();
    assert!(accounts.source_ns >= 32 * MS, "{accounts:?}");
    assert!(accounts.target_ns >= 20 * MS && accounts.target_ns < 30 * MS);
    assert!(accounts.cpu_ns >= 10 * MS && accounts.cpu_ns < 20 * MS);
    assert_eq!(accounts.wait_ns, 5 * MS);
    // the job's totals saw every charge as it was made
    assert_eq!(stats.totals().accounts, accounts);
    // and a range is counted while its guard lives
    let range = meter.segment();
    assert_eq!(
        stats
            .snapshot(crate::transfer::TransferKind::Download, 0)
            .segments,
        1
    );
    drop(range);
    permits.finish_with(crate::transfer::OpOutcome::Done, accounts);
    assert_eq!(flow_accounts(&flow), accounts);
}

fn flow_accounts(flow: &crate::transfer::Flow) -> crate::transfer::limiter::OpAccounts {
    flow.snapshot().accounts
}

fn names<'a>(
    source: &'a str,
    target: &'a str,
    remote: &'a str,
    pair: Option<(Option<f64>, Option<f64>)>,
) -> NamesView<'a> {
    NamesView {
        source_drive: source,
        target_drive: target,
        remote,
        same_volume: false,
        local_pair: pair,
    }
}

#[test]
fn speed_task_limiter_describe_names_drives_and_kinds_only() {
    let upload = names("sda1", "", "SFTP-Server", None);
    let (limiter, text) = describe(Limiter::Source, &upload, 8_000_000, 0);
    assert_eq!(
        (limiter, text.as_deref()),
        (Limiter::Source, Some("Begrenzt durch das Laufwerk (sda1)"))
    );
    let (_, text) = describe(Limiter::Remote, &upload, 8_000_000, 0);
    assert_eq!(
        text.as_deref(),
        Some("Begrenzt durch Leitung oder Server (SFTP-Server)")
    );
    let (_, text) = describe(Limiter::Link, &upload, 12_000_000, 12_500_000);
    let text = text.expect("a text");
    assert!(text.starts_with("Begrenzt durch die Leitung ("), "{text}");
    assert!(text.contains("MB/s von"), "{text}");
    // nothing to say without a verdict
    assert_eq!(
        describe(Limiter::None, &upload, 0, 0),
        (Limiter::None, None)
    );
    // a vault is named by its kind
    let vault = names("", "", "Tresor", None);
    let (_, text) = describe(Limiter::Remote, &vault, 0, 0);
    assert_eq!(
        text.as_deref(),
        Some("Begrenzt durch Leitung oder Server (Tresor)")
    );
}

#[test]
fn speed_task_limiter_describe_local_copy_names_the_longer_queue() {
    let both = names("sda1", "nvme0n1", "", Some((Some(4.0), Some(1.0))));
    let (limiter, text) = describe(Limiter::Target, &both, 0, 0);
    assert_eq!(
        limiter,
        Limiter::Source,
        "the source drive has the longer queue"
    );
    assert_eq!(text.as_deref(), Some("Begrenzt durch das Laufwerk (sda1)"));
    let unknown = names("sda1", "nvme0n1", "", Some((None, Some(1.0))));
    let (limiter, text) = describe(Limiter::Target, &unknown, 0, 0);
    assert_eq!(limiter, Limiter::Target);
    assert_eq!(
        text.as_deref(),
        Some("Begrenzt durch das Laufwerk (sda1 oder nvme0n1)")
    );
    let mut same = names("sda1", "sda1", "", Some((None, None)));
    same.same_volume = true;
    let (_, text) = describe(Limiter::Target, &same, 0, 0);
    assert_eq!(text.as_deref(), Some("Begrenzt durch das Laufwerk (sda1)"));
}

#[test]
fn speed_task_limiter_endpoint_names_are_kinds_not_addresses() {
    let (fake, _) = Fake::new("name").handle();
    assert_eq!(endpoint_name(Side::Local), "");
    // the double is a local scheme: no kind to name, and never its identity
    let name = endpoint_name(Side::Remote(&*fake));
    assert!(!name.contains(&fake.identity), "{name}");
}

fn download_fixture(
    files: usize,
    size: usize,
    tag: &str,
) -> (tempfile::TempDir, std::path::PathBuf, Fake) {
    let root = tempfile::tempdir().expect("temp dir");
    let remote = root.path().join("remote/Vault");
    for index in 0..files {
        write(&remote.join(format!("f{index:03}.bin")), &vec![1u8; size]);
    }
    let dest = root.path().join("dest");
    std::fs::create_dir(&dest).expect("dest");
    (root, dest, Fake::new(tag))
}

#[test]
fn speed_task_thread_refusal_is_overload() {
    let (root, dest, fake) = download_fixture(6, 64 * 1024, "refusal");
    let (fake, handle) = fake.handle();
    // the process refuses the first two workers: no panic, no file error
    refuse_next_starts(2);
    let remote = root.path().join("remote/Vault");
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &dest,
        &[&remote],
    ));
    assert!(finished.issues.is_empty(), "{:?}", finished.issues);
    assert_eq!(finished.progress.files_done, 6);
    assert_eq!(finished.progress.errors, 0);
    // the flow learned the bound: nothing ran when the threads were refused
    let snapshot = flow_for(&*fake, "/").snapshot();
    assert_eq!(snapshot.ceiling, 1, "{snapshot:?}");
    assert!(snapshot.limit <= 1);
    assert!(dest.join("Vault/f005.bin").is_file());
}

#[test]
fn speed_task_thread_refusal_ends_the_job_when_no_worker_can_start() {
    let (root, dest, fake) = download_fixture(3, 1024, "refusal-fatal");
    let (_fake, handle) = fake.handle();
    refuse_next_starts(10);
    let remote = root.path().join("remote/Vault");
    let finished = run(job(
        Endpoint::Remote(handle),
        Endpoint::Local,
        &dest,
        &[&remote],
    ));
    refuse_next_starts(0);
    assert!(
        finished.has_issue("Der Prozess kann keine weiteren Threads starten"),
        "{:?}",
        finished.issues
    );
    assert_eq!(finished.progress.files_done, 0);
}

#[test]
fn speed_task_progress_carries_limiter_fields() {
    // a source that answers slowly, two operations at a time: the source
    // side holds the time, and the progress says so after three windows
    let (root, dest, mut fake) = download_fixture(48, 1024 * 1024, "limiter");
    fake.read_delay = Duration::from_millis(60);
    fake.ceiling = Some(2);
    let (_fake, handle) = fake.handle();
    let remote = root.path().join("remote/Vault");
    let (tx, rx) = crossbeam_channel::unbounded();
    let cancel = AtomicBool::new(false);
    super::run_job(
        job(Endpoint::Remote(handle), Endpoint::Local, &dest, &[&remote]),
        &tx,
        &cancel,
    );
    let mut shown = Vec::new();
    let mut last = None;
    for message in rx.try_iter() {
        match message {
            TransferMsg::Progress(progress) => {
                if progress.limiter != Limiter::None {
                    shown.push((progress.limiter, progress.limiter_text.clone()));
                }
            }
            TransferMsg::Done { progress, .. } => last = Some(progress),
        }
    }
    let last = last.expect("the terminal message");
    assert_eq!(last.files_done, 48);
    // the final value keeps the last verdict that named a resource
    assert!(
        matches!(last.limiter, Limiter::Remote | Limiter::Link),
        "{:?} {:?}",
        last.limiter,
        last.limiter_text
    );
    let text = last.limiter_text.clone().expect("a text for the verdict");
    assert!(text.starts_with("Begrenzt durch"), "{text}");
    assert!(
        !text.contains(&fwd(root.path())) && !text.contains("Vault"),
        "no path in the text: {text}"
    );
    // the source's reads lead the resources; the waiting share grows on a
    // loaded runner (suite run 6: 47 % source, 53 % waiting)
    let [source, target, cpu, wait] = last.limiter_shares;
    assert!(
        source >= 40 && source > target && source > cpu,
        "{:?}",
        last.limiter_shares
    );
    assert!(u32::from(source) + u32::from(target) + u32::from(cpu) + u32::from(wait) == 100);
    // the live values never name a resource before three equal windows
    assert!(!shown.is_empty(), "no progress value carried a verdict");
    assert_eq!(last.segments, 0);
}
