//! E50, M26 (L7 point 6): a second account of the same machine does not reach
//! the worker IPC of the first one. The test runs as the first account (the
//! runner) with a data folder of its own that every account may enter: it
//! creates its daemon token there exactly as the worker does and serves the
//! worker IPC with it. Then it starts itself again as the second account,
//! which the suite part `ignored` creates and names in
//! `SE_TASK_FOREIGN_ACCOUNT` (group „Second account on the worker IPC“,
//! Linux). That process must fail at every step: the operating system
//! refuses it the token file (only the modes the worker gave its app data
//! folder and its token stand in the way), a guessed token gets no proof
//! from the worker, and a request with a guessed token never reaches the
//! „Öffnen“ arm. The first account's own request with the token reaches the
//! arm, so the refusal is no unreachable worker.
use std::ffi::OsString;
use std::net::{SocketAddr, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::daemon::ipc_proof::{connect_with_token, ProofFailure};
use crate::daemon::ipc_protocol::tcp::TcpIpcRequest;
use crate::daemon::ipc_protocol::{read_response, write_request, IpcRequest, IpcResponse};

const TEST: &str =
    "daemon::tcp_ipc::dienste_oeffnen_task_tests::fremdkonto::dienste_oeffnen_task_foreign_account_is_refused";
/// Set by the suite group: the second account.
const ACCOUNT: &str = "SE_TASK_FOREIGN_ACCOUNT";
/// Set for the second process: the worker address of the first account.
const FOREIGN_ADDR: &str = "SE_TASK_FOREIGN_IPC";
/// Set for the second process: the token file of the first account.
const FOREIGN_TOKEN_FILE: &str = "SE_TASK_FOREIGN_TOKEN_FILE";
/// Names the data home of the app data folder (Linux).
const DATA_HOME: &str = "XDG_DATA_HOME";
/// The answer of the „Öffnen“ arm for a device without a feed.
const ARM_TEXT: &str = "Dienstliste";
const GUESS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn open_request(token: &str) -> IpcRequest {
    IpcRequest::Tcp {
        token: token.into(),
        req: TcpIpcRequest::OpenService {
            device_id: "dienste-fremdkonto-geraet".into(),
            public_key: "dienste-fremdkonto-schluessel".into(),
            port: 8080,
        },
    }
}

/// The answer to `request` on a connection that skipped the proof, or the
/// reason there was none.
fn answer(addr: &SocketAddr, request: &IpcRequest) -> Result<IpcResponse, String> {
    let mut stream = TcpStream::connect_timeout(addr, Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| error.to_string())?;
    write_request(&mut stream, request).map_err(|error| error.to_string())?;
    read_response(&mut stream).map_err(|error| error.to_string())
}

#[test]
#[ignore = "second account of the machine and isolated app data; Suite crates-task"]
fn dienste_oeffnen_task_foreign_account_is_refused() {
    match std::env::var(FOREIGN_ADDR) {
        Ok(addr) => foreign_side(addr.parse().expect("the worker address")),
        Err(_) => first_account_side(),
    }
}

/// The process of the second account.
fn foreign_side(addr: SocketAddr) {
    // The operating system refuses the token file of the first account. Every
    // folder above its app data folder is open to this account, so only the
    // modes the worker gave that folder and the token refuse it; a missing
    // file would be no proof.
    let token_file = PathBuf::from(
        std::env::var_os(FOREIGN_TOKEN_FILE).expect("the first account names its token file"),
    );
    match std::fs::read(&token_file) {
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {}
        Err(error) => panic!("the token file was not refused but not read: {error}"),
        Ok(_) => panic!("the second account read the first account's token file"),
    }
    // The worker's own reader does not hand it out either.
    let token = crate::daemon::ipc_storage::read_token();
    assert!(
        token.is_err(),
        "the second account read the first account's token"
    );
    // A guessed token gets no proof of the worker.
    match connect_with_token(&addr, Duration::from_secs(5), GUESS) {
        Err(ProofFailure::Unproven) => {}
        Err(ProofFailure::Unreachable(error)) => panic!("the worker is not reachable: {error}"),
        Ok(_) => panic!("the worker proved itself for a guessed token"),
    }
    // A request with a guessed token never reaches the arm.
    match answer(&addr, &open_request(GUESS)) {
        Ok(IpcResponse::Err { msg, .. }) => {
            assert!(
                !msg.contains(ARM_TEXT),
                "the arm answered the second account: {msg}"
            )
        }
        Ok(other) => panic!("the worker answered the second account: {other:?}"),
        Err(_closed) => {}
    }
}

/// The data home of the first account for this test, and the variable as it
/// was before. The suite's own isolated profile lies below a folder only the
/// runner may enter, which would hide the token whatever the worker did.
struct OpenDataHome {
    folder: tempfile::TempDir,
    saved: Option<OsString>,
}

impl OpenDataHome {
    fn new() -> Self {
        let folder = tempfile::Builder::new()
            .prefix("se-fremdkonto-daten-")
            .tempdir()
            .expect("a data home for the first account");
        std::fs::set_permissions(folder.path(), std::fs::Permissions::from_mode(0o755))
            .expect("every account may enter the data home");
        let saved = std::env::var_os(DATA_HOME);
        std::env::set_var(DATA_HOME, folder.path());
        Self { folder, saved }
    }
}

impl Drop for OpenDataHome {
    fn drop(&mut self) {
        match self.saved.take() {
            Some(value) => std::env::set_var(DATA_HOME, value),
            None => std::env::remove_var(DATA_HOME),
        }
    }
}

/// The process of the first account: worker, token, and the second process.
fn first_account_side() {
    let account = std::env::var(ACCOUNT).unwrap_or_else(|_| {
        panic!("{ACCOUNT} names no second account; the suite group provides it")
    });
    let data_home = OpenDataHome::new();
    let token = crate::daemon::ipc_storage::load_or_create_token().expect("the worker's token");
    // The worker's own protection: its app data folder and its token belong
    // to this account alone.
    let app_data = crate::support_dirs::app_data_dir();
    let token_file = app_data.join("sync").join("daemon.token");
    for (path, mode) in [(&app_data, 0o700), (&token_file, 0o600)] {
        let found = std::fs::metadata(path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(found, mode, "{}", path.display());
    }
    assert!(
        app_data.starts_with(data_home.folder.path()),
        "{app_data:?}"
    );
    let host = crate::daemon::ipc_host::ShareHost::new("6".repeat(32));
    let server = crate::daemon::ipc_listener::serve_for_test(host, token.clone())
        .expect("the worker IPC of the first account");
    let addr = server.addr();

    // The first account's own request reaches the „Öffnen“ arm.
    let mut proven = match connect_with_token(&addr, Duration::from_secs(5), &token) {
        Ok(stream) => stream,
        Err(_) => panic!("the worker refused its own account"),
    };
    write_request(&mut proven, &open_request(&token)).expect("request written");
    match read_response(&mut proven) {
        Ok(IpcResponse::Err { msg, .. }) => assert!(msg.contains(ARM_TEXT), "{msg}"),
        other => panic!("the arm did not answer the first account: {other:?}"),
    }

    let copy = tempfile::Builder::new()
        .prefix("se-fremdkonto-")
        .tempdir()
        .expect("a folder for the second process");
    let program = executable_copy(copy.path());
    let output = Command::new("sudo")
        .args(["-n", "-u", &account, "env"])
        .arg(format!("{FOREIGN_ADDR}={addr}"))
        .arg(format!("{FOREIGN_TOKEN_FILE}={}", token_file.display()))
        .arg(format!("{DATA_HOME}={}", data_home.folder.path().display()))
        .arg("HOME=/nonexistent")
        .arg(&program)
        .args(["--exact", TEST, "--ignored", "--test-threads=1"])
        .current_dir("/")
        .output()
        .expect("sudo starts the second process");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success(),
        "the second account's process: {text}"
    );
    assert!(
        text.contains("1 passed"),
        "the second process ran no test: {text}"
    );
}

/// This test binary in a folder the second account may read and run.
fn executable_copy(folder: &Path) -> PathBuf {
    let program = folder.join("se-engine-tests");
    std::fs::copy(std::env::current_exe().expect("this test binary"), &program)
        .expect("the test binary is copied");
    for (path, mode) in [(folder, 0o755), (program.as_path(), 0o755)] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .expect("the second account may run the copy");
    }
    program
}
