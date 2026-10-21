//! Linux drives in the daemon (stage `linux-root: drive-fuse`, fixture
//! `home` for a fresh HOME): SMB stays refused as a drive source (D3,
//! preservation of S2), and the Linux FUSE part of
//! `drive_task_auto_remount_after_daemon_restart_fuse` (DRV-M3): a worker of the
//! built `se` (`SMART_EXPLORER_DRIVE_SE_BIN`) mounts a saved SFTP drive with
//! `--auto-remount`; after that worker stopped, the start of the login
//! entry (`se --sync-daemon --drive-restore`) mounts it again at the same
//! folder. Both tests are `#[ignore]` (`SMART_EXPLORER_FUSE_TASK=1`).
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::fuse_task_backends::{sftp, variable};
use super::fuse_task_harness::{
    eventually, mounted, program, remove_remote_tree, require_task_stage, tool,
};
use crate::creds::{AuthKind, Protocol, SavedConnection};
use crate::mount::{
    BackendRoot, DriveSelection, MountConfig, MountId, MountMode, MountSource, MountStatus,
};

const SE_BIN: &str = "SMART_EXPLORER_DRIVE_SE_BIN";
const LABEL: &str = "fuse-e2e";
const CONTENT: &[u8] = b"mounted again after the restart";
const START_LIMIT: Duration = Duration::from_secs(120);

/// Saved connections, secrets and drives of these tests live in the fresh
/// HOME of the fixture `home`, never in the runner user's own.
fn require_isolated_home() {
    for name in ["HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME"] {
        assert!(
            std::env::var_os(name).is_some(),
            "{name} fehlt: diese Prüfung braucht die Fixture home der Stufe linux-root: drive-fuse"
        );
    }
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse with the home fixture (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_smb_mount_still_refused() -> io::Result<()> {
    require_task_stage();
    require_isolated_home();
    let connection = SavedConnection {
        protocol: Protocol::Smb,
        host: "nas.example.test".into(),
        port: Protocol::Smb.default_port(),
        user: "user".into(),
        auth: AuthKind::Password,
        root: "/share".into(),
        label: "nas".into(),
        use_agent: false,
    };
    crate::creds::save_connection(&connection)?;
    let config = MountConfig::new(
        MountId::new_random()?,
        MountSource::SavedRemote {
            account: connection.account(),
            root: BackendRoot::parse("/share")?,
        },
        DriveSelection::Automatic,
        MountMode::ReadOnly,
        "nas",
    )?;
    let id = config.id.clone();
    // The start path of the daemon, which now continues on Linux.
    let host = super::super::ipc_host::ShareHost::new("0".repeat(32));
    let snapshot = host.mounts.start(config, &host).map_err(io::Error::other)?;
    assert_eq!(
        snapshot.status,
        MountStatus::Failed {
            detail: "SMB lässt sich nicht als Laufwerk einbinden".into()
        }
    );
    // Nothing was mounted, and nothing was registered for a later start.
    if let Some(folder) = snapshot.config.location_directory() {
        assert!(!mounted(folder));
    }
    assert!(!super::super::mount_registry::registered_for_completion()
        .iter()
        .any(|registered| registered.id == id));
    Ok(())
}

/// Stops the worker of this HOME and waits until no worker holds the
/// single-instance lock.
fn stop_workers() -> io::Result<()> {
    super::super::state::request_stop()?;
    match super::super::platform::acquire_daemon_instance_guard(START_LIMIT) {
        Some(guard) => {
            drop(guard);
            Ok(())
        }
        None => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!(
                "the background worker did not stop\n{}\nworker log:\n{}",
                worker_state(),
                crate::daemon::read_log_tail(40)
            ),
        )),
    }
}

/// Command-line parts that name the worker, its guardian, a drive host or the
/// FUSE helper.
const WORKER_MARKERS: [&str; 4] = [
    "--sync-daemon",
    "--sync-guardian",
    "--mount-host",
    "fusermount",
];
/// Threads listed per process in the failure message.
const THREAD_LIMIT: usize = 64;
/// How long a debugger may take to print the stacks of a stuck process.
const DEBUGGER_LIMIT: Duration = Duration::from_secs(30);

/// One process line of `ps -eo pid,ppid,...`.
struct PsRow<'a> {
    pid: u32,
    parent: u32,
    line: &'a str,
}

fn ps_row(line: &str) -> Option<PsRow<'_>> {
    let mut fields = line.split_whitespace();
    let pid = fields.next()?.parse::<u32>().ok()?;
    let parent = fields.next()?.parse::<u32>().ok()?;
    Some(PsRow { pid, parent, line })
}

/// What the failure message of a worker that does not stop shows: the
/// processes of the worker, its guardian, its drive hosts and the FUSE helper
/// with their children (a host nobody reaped is `<defunct>`), every thread of
/// them with its name and kernel wait, the user-space stacks when a debugger is
/// installed, and the se mounts of this machine.
fn worker_state() -> String {
    let table = Command::new("ps")
        .args(["-eo", "pid,ppid,etimes,stat,wchan:20,args"])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_else(|error| format!("ps: {error}"));
    let rows: Vec<PsRow> = table.lines().filter_map(ps_row).collect();
    let named: Vec<u32> = rows
        .iter()
        .filter(|row| {
            WORKER_MARKERS
                .iter()
                .any(|marker| row.line.contains(marker))
        })
        .map(|row| row.pid)
        .collect();
    let shown: Vec<&PsRow> = rows
        .iter()
        .filter(|row| named.contains(&row.pid) || named.contains(&row.parent))
        .collect();
    let mut report = String::from("processes:\n");
    report.push_str(table.lines().next().unwrap_or_default());
    report.push('\n');
    for row in &shown {
        report.push_str(row.line);
        report.push('\n');
    }
    report.push_str("threads (pid/tid state name: kernel wait; syscall; kernel stack):\n");
    for row in &shown {
        report.push_str(&thread_table(row.pid));
    }
    let mut stacked = false;
    for row in &shown {
        if row.line.contains("--sync-daemon") || row.line.contains("--mount-host") {
            if let Some(stacks) = user_stacks(row.pid) {
                report.push_str(&format!("user-space stacks of {}:\n{stacks}\n", row.pid));
                stacked = true;
            }
        }
    }
    if !stacked {
        report.push_str("user-space stacks: none (no gdb, eu-stack or pstack ran)\n");
    }
    report.push_str("se mounts (mountinfo):\n");
    report.push_str(&se_mounts());
    report
}

/// Id, state, name, kernel wait and syscall of every thread of `pid`, and the
/// kernel stack where this test may read it (root, or `sudo -n`).
fn thread_table(pid: u32) -> String {
    let tasks = PathBuf::from(format!("/proc/{pid}/task"));
    let mut ids: Vec<u32> = match fs::read_dir(&tasks) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
            .collect(),
        Err(error) => return format!("  {pid}: no thread list: {error}\n"),
    };
    ids.sort_unstable();
    let mut table = String::new();
    for id in ids.iter().take(THREAD_LIMIT) {
        let thread = tasks.join(id.to_string());
        let stat = proc_text(&thread.join("stat"));
        table.push_str(&format!(
            "  {pid}/{id} {} {}: wchan {}; syscall {}\n",
            thread_state(&stat),
            proc_text(&thread.join("comm")),
            proc_text(&thread.join("wchan")),
            proc_text(&thread.join("syscall")),
        ));
        if let Some(stack) = kernel_stack(&thread.join("stack")) {
            for line in stack.lines() {
                table.push_str(&format!("      {line}\n"));
            }
        }
    }
    if ids.len() > THREAD_LIMIT {
        table.push_str(&format!(
            "  {pid}: {} more threads not listed\n",
            ids.len() - THREAD_LIMIT
        ));
    }
    table
}

/// The text of a `/proc` file without its line break, or why it is unreadable.
fn proc_text(path: &Path) -> String {
    fs::read_to_string(path)
        .map(|text| text.trim().to_string())
        .unwrap_or_else(|error| format!("({error})"))
}

/// The state letter of a `/proc/<pid>/task/<tid>/stat` line: the field after
/// the command, which is written in parentheses.
fn thread_state(stat: &str) -> &str {
    stat.rsplit_once(')')
        .and_then(|(_, rest)| rest.split_whitespace().next())
        .unwrap_or("?")
}

/// The kernel stack of a thread. Reading it needs root; a test run by the
/// runner user asks `sudo -n`, which never prompts.
fn kernel_stack(path: &Path) -> Option<String> {
    match fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            let output = Command::new("sudo")
                .arg("-n")
                .arg("cat")
                .arg(path)
                .output()
                .ok()?;
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
        }
        Err(_) => None,
    }
}

fn is_root() -> bool {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

/// The user-space stacks of every thread of `pid`, when `gdb`, `eu-stack` or
/// `pstack` is installed; attached through `sudo -n` unless this test is root.
fn user_stacks(pid: u32) -> Option<String> {
    let pid = pid.to_string();
    for (name, arguments) in [
        (
            "gdb",
            vec![
                "-p",
                pid.as_str(),
                "-batch",
                "-ex",
                "set pagination off",
                "-ex",
                "thread apply all bt 30",
            ],
        ),
        ("eu-stack", vec!["-p", pid.as_str()]),
        ("pstack", vec![pid.as_str()]),
    ] {
        let Some(program) = tool(name) else {
            continue;
        };
        let mut command = if is_root() {
            Command::new(program.as_path())
        } else {
            let mut sudo = Command::new("sudo");
            sudo.arg("-n").arg(program.as_path());
            sudo
        };
        command.args(arguments);
        if let Some(text) = run_limited(&mut command, DEBUGGER_LIMIT) {
            return Some(format!("{name}:\n{text}"));
        }
    }
    None
}

/// The output (stdout and stderr) of `command` when it ends within `limit`;
/// otherwise it is killed and there is none. The output goes to an unnamed
/// file: a debugger prints more than a pipe holds while nobody reads it.
fn run_limited(command: &mut Command, limit: Duration) -> Option<String> {
    let mut output = tempfile::tempfile().ok()?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(output.try_clone().ok()?)
        .stderr(output.try_clone().ok()?)
        .spawn()
        .ok()?;
    let deadline = Instant::now() + limit;
    let ended = loop {
        match child.try_wait() {
            Ok(Some(_)) => break true,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            _ => break false,
        }
    };
    if !ended {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    let mut bytes = Vec::new();
    output.seek(SeekFrom::Start(0)).ok()?;
    output.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The se mounts `mountinfo` lists, to tell whether the host unmounted.
fn se_mounts() -> String {
    match fs::read_to_string("/proc/self/mountinfo") {
        Ok(text) => {
            let lines: Vec<&str> = text
                .lines()
                .filter(|line| line.contains("smartexplorer"))
                .collect();
            if lines.is_empty() {
                "(none)\n".to_string()
            } else {
                format!("{}\n", lines.join("\n"))
            }
        }
        Err(error) => format!("mountinfo: {error}\n"),
    }
}

/// `se drive list` (text), for the failure messages.
fn listing(se: &Path) -> String {
    program(Command::new(se).args(["drive", "list"]))
        .map(|(code, output)| format!("exit {code}: {output}"))
        .unwrap_or_else(|error| error.to_string())
}

fn end_child(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
#[ignore = "FUSE task stage linux-root: drive-fuse with se, sftp and home (SMART_EXPLORER_FUSE_TASK=1)"]
fn drive_task_auto_remount_after_daemon_restart_fuse() -> io::Result<()> {
    require_task_stage();
    require_isolated_home();
    let se = std::env::var_os(SE_BIN)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{SE_BIN} fehlt")))?;
    // The server folder with one file, and the saved connection `se` uses
    // (host key pinned by `sftp`, password in the sealed store).
    let (raw, _drive, server) = sftp(MountMode::ReadWrite)?;
    let mut writer = raw.open_write(&format!("{server}/restart.txt"))?;
    io::Write::write_all(&mut writer, CONTENT)?;
    io::Write::flush(&mut writer)?;
    drop(writer);
    let connection = SavedConnection {
        protocol: Protocol::Sftp,
        host: variable("SE_TASK_SFTP_HOST")?,
        port: variable("SE_TASK_SFTP_PORT")?
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "SE_TASK_SFTP_PORT"))?,
        user: variable("SE_TASK_SFTP_USER")?,
        auth: AuthKind::Password,
        root: server.clone(),
        label: LABEL.into(),
        use_agent: false,
    };
    let password = variable("SE_TASK_SFTP_PASSWORD")?;
    crate::creds::save_connection_with_secret(&connection, Some(&password))
        .map_err(io::Error::other)?;
    let base = tempfile::tempdir()?;
    let folder = base.path().join("drive");
    let outcome = remount_round_trip(&se, &folder);
    // Whatever happened: no worker, mount or server folder stays behind.
    let stopped = stop_workers();
    if mounted(&folder) {
        let _ = crate::mount::unmount_drive_folder(&folder, true);
    }
    let _ = remove_remote_tree(&raw, &server);
    outcome.and(stopped)
}

fn remount_round_trip(se: &Path, folder: &Path) -> io::Result<()> {
    // First worker, started by `se drive mount`. Read-only, as the daemon
    // mounts an SFTP drive: that backend guarantees no safe replace of an
    // existing file, so a read-write SFTP drive is refused by design
    // (`mount_manager_capabilities.rs`); the round trip only reads.
    let (code, output) = program(
        Command::new(se)
            .args(["drive", "mount"])
            .arg(format!("@{LABEL}:/"))
            .arg("--mount-point")
            .arg(folder)
            .args(["--trust-remote-root", "--auto-remount"]),
    )?;
    assert_eq!(code, 0, "se drive mount: {output}");
    assert!(
        eventually(START_LIMIT, || mounted(folder)),
        "first mount: {}",
        listing(se)
    );
    assert_eq!(fs::read(folder.join("restart.txt"))?, CONTENT);
    // The worker stops; its host ends like at an update stop and unmounts,
    // the drive stays registered for the restore.
    stop_workers()?;
    assert!(
        !mounted(folder),
        "the stopped worker left the drive mounted"
    );
    // The start of the login entry.
    let mut restore = Command::new(se)
        .args(["--sync-daemon", "--drive-restore"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let remounted = eventually(START_LIMIT, || mounted(folder));
    let shown = listing(se);
    let content = fs::read(folder.join("restart.txt"));
    let stopped = stop_workers();
    end_child(&mut restore);
    assert!(remounted, "no mount after the restart: {shown}");
    assert!(
        shown.contains(&format!("mounted:{}", folder.display())),
        "{shown}"
    );
    assert_eq!(content?, CONTENT);
    stopped
}
