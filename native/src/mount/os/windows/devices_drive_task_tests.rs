//! Windows end-to-end signal of the device tree (DRV-M6, stage windows-core:
//! drive-cli; fixture `dokany`, one test thread, `--include-ignored`): the
//! production Dokany host serves the device tree as a drive letter; the
//! devices are local folders behind the fixture link of the core tests.
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use super::super::super::shared::host_session::devices::{run_with, DeviceFeed};
use super::super::super::shared::host_session::{
    HostControl, HostExit, HostInputs, UpdateStopCheck,
};
use super::super::callback_reporter::CallbackReporter;
use super::DokanyHost;
use crate::daemon::{MountHostConfig, UpdateStopBaseline, UPDATE_STOP_POLL};
use crate::mount::device_tree::support::{direct, Env};
use crate::mount::{
    devices_label, ChildLink, DeviceRow, DriveLetter, DriveSelection, MountConfig, MountId,
    MountMode, MountRecovery, MountSource, MountStatus,
};
use crate::vfs::{BackendHandle, LocalBackend};

const LIMIT: Duration = Duration::from_secs(120);

type Report = (MountStatus, Option<MountRecovery>);

struct Control {
    reports: mpsc::Sender<Report>,
    stop: Arc<AtomicBool>,
}

impl HostControl for Control {
    fn report(&self, status: MountStatus, recovery: Option<MountRecovery>) -> Result<(), String> {
        let _ = self.reports.send((status, recovery));
        Ok(())
    }

    fn wait_for_stop(&self, timeout: Duration) -> bool {
        std::thread::sleep(timeout);
        self.stop.load(Ordering::Acquire)
    }
}

/// The device list of the fixture; it never changes after the first look.
struct Feed(Vec<DeviceRow>);

impl DeviceFeed for Feed {
    fn watch(&self, since: u64) -> Result<(u64, Vec<DeviceRow>), String> {
        if since != 0 {
            std::thread::sleep(Duration::from_millis(200));
        }
        Ok((1, self.0.clone()))
    }
}

/// One production Dokany host over the device tree; ends on drop.
struct TreeHost {
    drive: DriveLetter,
    stop: Arc<AtomicBool>,
    done: mpsc::Receiver<Result<HostExit, String>>,
    _statuses: mpsc::Receiver<MountStatus>,
    ended: bool,
}

impl TreeHost {
    fn start(env: &Env) -> io::Result<Self> {
        let mut config = MountHostConfig::from(&MountConfig::new(
            MountId::parse(crate::mount::DEVICES_MOUNT_ID)?,
            MountSource::Devices,
            DriveSelection::Automatic,
            MountMode::ReadWrite,
            devices_label(),
        )?);
        config.device_tree = true;
        let placeholder: BackendHandle = Arc::new(LocalBackend::new("/"));
        let inputs = HostInputs {
            config,
            backend: placeholder,
            cache_root: env.spool.clone(),
        };
        let link: Arc<dyn ChildLink> = env.link.clone();
        let feed = Arc::new(Feed(env.rows.clone()));
        let (report_send, reports) = mpsc::channel();
        let (status_send, statuses) = mpsc::channel();
        let (done_send, done) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("devices-dokany-e2e".into())
            .spawn(move || {
                let baseline =
                    UpdateStopBaseline::with_source(Arc::new(|| None::<std::time::SystemTime>));
                let check = UpdateStopCheck::new(baseline, UPDATE_STOP_POLL);
                let control = Control {
                    reports: report_send,
                    stop: thread_stop,
                };
                let mut platform = DokanyHost::new(CallbackReporter::Capture(status_send));
                let _ = done_send.send(run_with(
                    inputs,
                    &control,
                    move |_lease| link,
                    feed,
                    &mut platform,
                    &check,
                ));
            })?;
        let deadline = Instant::now() + LIMIT;
        let drive = loop {
            match reports.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok((MountStatus::Mounted { drive }, _)) => break drive,
                Ok((MountStatus::Failed { detail }, _))
                | Ok((MountStatus::RuntimeUnavailable { detail }, _)) => {
                    return Err(io::Error::other(format!("host did not mount: {detail}")))
                }
                Ok(_) => {}
                Err(error) => return Err(io::Error::other(format!("no Mounted: {error}"))),
            }
        };
        // The volume answers once the `mounted` callback confirmed it.
        loop {
            match statuses.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(MountStatus::Mounted { drive: mounted }) if mounted == drive => break,
                Ok(_) => {}
                Err(error) => {
                    return Err(io::Error::other(format!("no mounted callback: {error}")))
                }
            }
        }
        Ok(Self {
            drive,
            stop,
            done,
            _statuses: statuses,
            ended: false,
        })
    }

    fn path(&self, name: &str) -> PathBuf {
        PathBuf::from(format!("{}:\\{name}", self.drive.get()))
    }

    fn finish(mut self) -> Result<HostExit, String> {
        self.stop.store(true, Ordering::Release);
        self.ended = true;
        self.done
            .recv_timeout(LIMIT)
            .map_err(|error| format!("the host did not end: {error}"))?
    }
}

impl Drop for TreeHost {
    fn drop(&mut self) {
        if !self.ended {
            self.stop.store(true, Ordering::Release);
            let _ = self.done.recv_timeout(LIMIT);
        }
    }
}

fn listed(path: PathBuf) -> io::Result<Vec<String>> {
    let mut names = fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<io::Result<Vec<_>>>()?;
    names.sort();
    Ok(names)
}

#[test]
#[ignore = "requires the pinned Dokany runtime and its installed driver (windows-core: drive-cli)"]
fn drive_task_devices_windows_drive_lists_peer() -> io::Result<()> {
    let env = Env::new(&[direct("a", "Laptop", &["Docs"])]);
    fs::write(env.share_file("direct~a", "Docs/readme.txt"), b"hallo")?;
    let host = TreeHost::start(&env)?;
    // The device is a folder of the drive; its shared folder lists the file.
    assert_eq!(listed(host.path(""))?, ["Laptop"]);
    assert_eq!(listed(host.path("Laptop"))?, ["Docs"]);
    assert_eq!(listed(host.path("Laptop\\Docs"))?, ["readme.txt"]);
    assert_eq!(fs::read(host.path("Laptop\\Docs\\readme.txt"))?, b"hallo");
    // The folders above the shared folders take no writes.
    assert!(fs::create_dir(host.path("Neu")).is_err());
    assert!(fs::create_dir(host.path("Laptop\\Neu")).is_err());
    // A file in the shared folder reaches the device.
    fs::write(host.path("Laptop\\Docs\\neu.txt"), b"neu")?;
    assert_eq!(host.finish(), Ok(HostExit::Stopped));
    assert_eq!(
        fs::read(env.share_file("direct~a", "Docs/neu.txt"))?,
        b"neu"
    );
    Ok(())
}
