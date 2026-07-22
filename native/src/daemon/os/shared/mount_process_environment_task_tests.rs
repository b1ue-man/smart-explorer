use std::{collections::BTreeMap, ffi::OsString, net::SocketAddr, path::Path, process::Command};

use crate::mount::MountId;

use super::mount_process_environment::{
    self, MOUNT_CACHE_DIR_ENV, MOUNT_IPC_ADDR_ENV, MOUNT_TOKEN_ENV,
};

#[test]
fn remote_drive_task_mount_host_environment_is_isolated_and_winsock_ready() {
    let system_windows_directory = OsString::from(r"C:\Windows");
    let ipc_addr: SocketAddr = "127.0.0.1:43123".parse().unwrap();
    let cache_root = Path::new(r"C:\Users\Explorer\mount-cache");
    let mount_id = MountId::parse("environment-contract").unwrap();
    let mut command = Command::new("ignored-current-executable");
    command
        .env("PATH", r"C:\poisoned")
        .env("Path", r"C:\also-poisoned")
        .env("TEMP", r"C:\untrusted-temp")
        .env("AWS_SECRET_ACCESS_KEY", "must-not-leak");

    mount_process_environment::configure(
        &mut command,
        &mount_id,
        &system_windows_directory,
        "one-use-token",
        ipc_addr,
        cache_root,
    );

    let arguments: Vec<_> = command.get_args().map(OsString::from).collect();
    assert_eq!(
        arguments,
        [
            OsString::from("--mount-host"),
            OsString::from("environment-contract"),
        ]
    );

    let environment: BTreeMap<OsString, OsString> = command
        .get_envs()
        .map(|(key, value)| (key.to_owned(), value.unwrap().to_owned()))
        .collect();
    assert_eq!(
        environment,
        BTreeMap::from([
            (
                OsString::from("SystemRoot"),
                system_windows_directory.clone()
            ),
            (OsString::from("WINDIR"), system_windows_directory),
            (
                OsString::from(MOUNT_TOKEN_ENV),
                OsString::from("one-use-token")
            ),
            (
                OsString::from(MOUNT_IPC_ADDR_ENV),
                OsString::from(ipc_addr.to_string()),
            ),
            (
                OsString::from(MOUNT_CACHE_DIR_ENV),
                cache_root.as_os_str().to_owned(),
            ),
        ])
    );
}
