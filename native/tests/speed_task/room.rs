//! Three parallel downloads from a Share room, twenty times (U44-7 (a), M-5
//! row "EAGAIN", M-6 "Raum"): 60 runs without `WouldBlock`/`EAGAIN`. Before
//! every repetition the case records the thread count of the process, the
//! `pids.current` of its cgroup and, for the device processes the stage names
//! in `SE_SPEED_ROOM_PIDS`, their threads; at the first failure it stops and
//! reports these values and the `Max processes` line of `/proc/<pid>/limits`.
use crate::data::{random_file, Scratch, MIB};
use crate::remote::{connect_room, Remote};
use crate::run::run_job;
use crate::support::{
    check_profile, fast_dir, finish, guarded, job, slash, var, Case, ReportLine, CASE_LIMIT,
};
use smart_explorer::transfer::Endpoint;
use std::panic::{catch_unwind, AssertUnwindSafe};

const PARALLEL: usize = 3;
const REPETITIONS: usize = 20;
const FILE_BYTES: u64 = 32 * MIB;

fn proc_line(pid: &str, file: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/{file}")).ok()?;
    text.lines()
        .find(|line| line.starts_with(key))
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// `pids.current` of the cgroup of this process (cgroup v2).
fn pids_current() -> Option<String> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = cgroup.lines().find_map(|line| line.strip_prefix("0::"))?;
    std::fs::read_to_string(format!("/sys/fs/cgroup{path}/pids.current"))
        .ok()
        .map(|text| text.trim().to_string())
}

/// Threads, `pids.current` and, per named device process, its threads and
/// process limit.
fn counts() -> String {
    let own = proc_line("self", "status", "Threads:").unwrap_or_else(|| "Threads: ?".into());
    let mut text = format!(
        "{own}; pids.current {}",
        pids_current().unwrap_or_else(|| "?".into())
    );
    for pid in var("SE_SPEED_ROOM_PIDS").unwrap_or_default().split(',') {
        let pid = pid.trim();
        if pid.is_empty() {
            continue;
        }
        text.push_str(&format!(
            "; pid {pid}: {} / {}",
            proc_line(pid, "status", "Threads:").unwrap_or_else(|| "Threads: ?".into()),
            proc_line(pid, "limits", "Max processes").unwrap_or_else(|| "Max processes: ?".into())
        ));
    }
    text
}

fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|text| text.to_string()))
        .unwrap_or_else(|| "panic without text".into())
}

/// One repetition: three downloads at the same time; the first error text.
fn repetition(
    remote: &Remote,
    names: &[String],
    scratch: &Scratch,
    round: usize,
) -> Option<String> {
    let results: Vec<Result<(), String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                let target = scratch.sub(&format!("r{round}-{index}"));
                let file = name.clone();
                scope.spawn(move || {
                    catch_unwind(AssertUnwindSafe(|| {
                        run_job(
                            job(
                                Endpoint::Remote(remote.backend.clone()),
                                Endpoint::Local,
                                vec![file],
                                slash(&target),
                                "speed_task_share_room_parallel_downloads_no_eagain",
                            ),
                            CASE_LIMIT,
                        )
                    }))
                    .map(|_| ())
                    .map_err(panic_text)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err("thread panicked".into()))
            })
            .collect()
    });
    results.into_iter().find_map(|result| result.err())
}

pub fn run_room() -> Result<(), String> {
    let case = Case::new(
        "speed_task_share_room_parallel_downloads_no_eagain",
        "share",
        "down",
        "raw",
    );
    check_profile(&case)?;
    let remote = connect_room()?;
    let scratch = Scratch::new(&fast_dir()?, case.name);
    let seeded = remote.guarded_folder("room");
    let mut names = Vec::new();
    for index in 0..PARALLEL {
        let local = scratch.path().join(format!("room-{index}.bin"));
        random_file(&local, FILE_BYTES);
        remote.seed_file(&seeded.path, &local, &format!("room-{index}.bin"))?;
        names.push(remote.join(&seeded.path, &format!("room-{index}.bin")));
        let _ = std::fs::remove_file(&local);
    }
    let mut line = ReportLine::new(&case, "Läufe");
    line.size_bytes = FILE_BYTES * PARALLEL as u64;
    line.requirement = "60 Läufe ohne WouldBlock/EAGAIN".into();
    line.capacity = (PARALLEL * REPETITIONS) as f64;
    let mut records = Vec::new();
    for round in 0..REPETITIONS {
        let before = counts();
        records.push(format!("{round}: {before}"));
        if let Some(error) = repetition(&remote, &names, &scratch, round) {
            line.se_rate = (round * PARALLEL) as f64;
            line.share = line.se_rate / line.capacity;
            line.notes.push(format!(
                "Abbruch in Wiederholung {round}: {error}; Zähler davor: {before}; \
                 Zähler danach: {}",
                counts()
            ));
            line.notes.push(records.join(" | "));
            let message = format!(
                "{}: Fehler nach {} Läufen: {error} (Zähler davor: {before}; danach: {})",
                case.name,
                round * PARALLEL,
                counts()
            );
            finish(&line, Some(&message));
            return Ok(());
        }
    }
    line.se_rate = (PARALLEL * REPETITIONS) as f64;
    line.required = 1.0;
    line.share = 1.0;
    line.notes.push(records.join(" | "));
    finish(&line, None);
    Ok(())
}

#[test]
#[ignore = "needs the stage's Share room"]
fn speed_task_share_room_parallel_downloads_no_eagain() {
    guarded(
        "speed_task_share_room_parallel_downloads_no_eagain",
        run_room,
    );
}
