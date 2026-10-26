//! Execute only the pinned ELF; drop to the detected greeter, clear groups/env.
use std::{fs::File, io::{self, Read, Seek, SeekFrom, Write}, os::{fd::{AsRawFd, FromRawFd}, unix::process::CommandExt}, process::{Child, Command, Stdio}, time::{Duration, Instant}};
use crate::login::{core::model::{SessionBinding, HELPER_SWITCH, PROBE_SWITCH}, os::shared::{child::{Lifetime, Spawned}, denied, installation::{self, Installation}}};
struct Life(Child);
impl Drop for Life { fn drop(&mut self) { self.stop(); } }
impl Lifetime for Life {
    fn process_id(&self) -> u32 { self.0.id() }
    fn stop(&mut self) {
        if self.0.try_wait().ok().flatten().is_some() { return; }
        unsafe { libc::kill(self.0.id() as i32, libc::SIGTERM); }
        let end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < end { if self.0.try_wait().ok().flatten().is_some() { return; } std::thread::sleep(Duration::from_millis(20)); }
        let _ = self.0.kill(); let _ = self.0.wait();
    }
}
pub(crate) fn spawn(install: &Installation, binding: &SessionBinding, probe: bool) -> io::Result<Spawned> {
    if unsafe { libc::geteuid() } != 0 { return Err(denied("only root broker may launch a greeter helper")); }
    let facts = super::session::inspect()?;
    if facts.binding != *binding || facts.binding.desktop != crate::login::core::model::DesktopKind::X11Greeter { return Err(denied("no current eligible X11 greeter")); }
    let mut image = install.image.file.try_clone()?; let mut magic = [0u8; 4]; image.seek(SeekFrom::Start(0))?; image.read_exact(&mut magic)?;
    if magic != *b"\x7fELF" { return Err(denied("login helper must be the protected native ELF, not a script")); }
    installation::verify_hash(&image, &install.manifest.sha256)?;
    let fd = unsafe { libc::memfd_create(c"se-login-xauthority".as_ptr(), libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    let mut authority = unsafe { File::from_raw_fd(fd) };
    if unsafe { libc::fchmod(fd, 0o444) } != 0 { return Err(io::Error::last_os_error()); }
    authority.write_all(&facts.authority)?; authority.seek(SeekFrom::Start(0))?;
    if unsafe { libc::fcntl(fd, libc::F_ADD_SEALS, libc::F_SEAL_WRITE | libc::F_SEAL_GROW | libc::F_SEAL_SHRINK | libc::F_SEAL_SEAL) } < 0 { return Err(io::Error::last_os_error()); }
    let image_fd = image.as_raw_fd(); let parent = std::process::id(); let uid = facts.uid; let gid = facts.gid;
    let mut command = Command::new(format!("/proc/self/fd/{image_fd}"));
    command.arg(if probe { PROBE_SWITCH } else { HELPER_SWITCH }).env_clear()
        .env("DISPLAY", &facts.display).env("XAUTHORITY", format!("/proc/self/fd/{fd}"))
        .env("XDG_SESSION_TYPE", "x11").env("XDG_SESSION_CLASS", "greeter")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    unsafe { command.pre_exec(move || {
        if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 || libc::getppid() != parent as i32
            || libc::setgroups(0, std::ptr::null()) != 0 || libc::setgid(gid) != 0 || libc::setuid(uid) != 0
            || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 || libc::fcntl(fd, libc::F_SETFD, 0) < 0 {
            return Err(io::Error::last_os_error());
        }
        // setuid clears PDEATHSIG on Linux; re-arm after the identity change.
        if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 || libc::getppid() != parent as i32 { return Err(io::Error::last_os_error()); }
        Ok(())
    }); }
    let mut child = command.spawn()?;
    let input = child.stdin.take().ok_or_else(|| io::Error::other("greeter helper has no input pipe"))?;
    let output = child.stdout.take().ok_or_else(|| io::Error::other("greeter helper has no output pipe"))?;
    Ok(Spawned { input: Box::new(input), output: Box::new(output), life: Box::new(Life(child)) })
}
