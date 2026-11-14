//! E21 Linux grant channel: the kernel, not the request, names the account.
use super::*;
use crate::login::core::{
    self_grants::{SelfReply, SelfRequest},
    wire,
};

fn temporary_socket(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "se-login-selbst-{name}-{}.sock",
        std::process::id()
    ))
}

fn serve_on(path: &Path, gateway: u32, stop: &'static AtomicBool) {
    let _ = fs::remove_file(path);
    let listener = UnixListener::bind(path).expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let socket = path.to_path_buf();
    std::thread::spawn(move || serve_grants(listener, gateway, stop, &socket));
}

#[test]
fn login_selbst_task_linux_account_comes_from_the_kernel() {
    static STOP: AtomicBool = AtomicBool::new(false);
    let own = unsafe { libc::geteuid() };
    let path = temporary_socket("kernel");
    serve_on(&path, own.wrapping_add(1), &STOP);
    let mut stream = UnixStream::connect(&path).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    wire::send(&mut stream, &SelfRequest::Status).expect("send");
    let reply: SelfReply = wire::receive(&mut stream).expect("reply");
    // No installed policy here, so the broker refuses; every answer names
    // the account the kernel reported, nothing from the request.
    assert_eq!(reply.account, own.to_string());
    assert_ne!(reply.state, "ok");
    STOP.store(true, Ordering::SeqCst);
    let root = abilities(0);
    assert!(root.console && root.before_login && root.locked);
    assert!(!root.uac && !root.sas);
    assert!(
        SCRIPT.contains("self-install"),
        "embedded installer knows the self mode"
    );
    assert_eq!(
        crate::login::os::shared::self_install::app_image(Path::new("/opt/se/se")),
        Path::new("/opt/se").join(APP_FILE)
    );
}

#[test]
fn login_selbst_task_linux_gateway_account_is_refused() {
    static STOP: AtomicBool = AtomicBool::new(false);
    let own = unsafe { libc::geteuid() };
    let path = temporary_socket("gateway");
    // The caller is the gateway's own account: no answer at all.
    serve_on(&path, own, &STOP);
    let mut stream = UnixStream::connect(&path).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    let _ = wire::send(&mut stream, &SelfRequest::Status);
    assert!(wire::receive::<SelfReply>(&mut stream).is_err());
    STOP.store(true, Ordering::SeqCst);
}

#[test]
#[ignore = "Administratorrechte; Suite crates-task"]
fn login_selbst_task_linux_root_records_keep_installer_owner() {
    assert!(privileged(), "needs root and an installed login service");
    let ledger = crate::login::core::self_grants::Ledger::default();
    let bytes = serde_json::to_vec(&ledger).expect("ledger");
    write_self_record(Record::Ledger, &bytes).expect("write ledger");
    let meta = fs::metadata(record_path(Record::Ledger).expect("path")).expect("metadata");
    assert_eq!((meta.uid(), meta.mode() & 0o777), (0, 0o600));
    assert_eq!(
        read_self_record(Record::Ledger, 4096).expect("read"),
        Some(bytes)
    );
    assert!(write_self_record(Record::MachineKey, b"{}").is_err());
}
