//! Android never bypasses MediaProjection or a locked platform session.
use crate::login::{
    core::model::SessionBinding,
    os::shared::{
        child::Spawned,
        installation::{Installation, Paths},
    },
};
use std::{
    fs::File,
    io::{self, Read, Write},
    path::Path,
};
fn unavailable<T>() -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "privileged login capture/input is unavailable on this platform; Android requires active MediaProjection"))
}
pub struct ProtectedFile {
    pub file: File,
}
pub struct ClientStream;
impl Read for ClientStream {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        unavailable()
    }
}
impl Write for ClientStream {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        unavailable()
    }
    fn flush(&mut self) -> io::Result<()> {
        unavailable()
    }
}
pub fn paths() -> io::Result<Paths> {
    unavailable()
}
pub fn open_protected(_: &Path, _: bool) -> io::Result<ProtectedFile> {
    unavailable()
}
pub fn validate_gateway_identity(_: &str) -> io::Result<()> {
    unavailable()
}
pub fn binding(_: &Installation) -> io::Result<SessionBinding> {
    unavailable()
}
pub fn prepare_helper(_: &SessionBinding, _: u32, _: bool) -> io::Result<SessionBinding> {
    unavailable()
}
pub fn helper_binding_matches(_: &SessionBinding) -> io::Result<bool> {
    unavailable()
}
pub fn finish_helper() {}
pub fn secure_attention(_: &SessionBinding) -> io::Result<()> {
    unavailable()
}
pub fn spawn(_: &Installation, _: &SessionBinding, _: bool) -> io::Result<Spawned> {
    unavailable()
}
pub fn connect() -> io::Result<ClientStream> {
    unavailable()
}
pub fn run_broker() -> io::Result<()> {
    unavailable()
}

pub fn helper_channel() -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    unavailable()
}

pub fn run_probe(
    _: Box<dyn std::io::Read + Send>,
    _: Box<dyn std::io::Write + Send>,
    _: crate::login::core::wire::HelperInit,
    _: crate::login::core::model::SessionBinding,
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "session probe process is only used by Windows",
    ))
}

pub fn gateway_state_directory() -> io::Result<std::path::PathBuf> {
    unavailable()
}

pub fn require_gateway(_: &crate::login::Policy) -> io::Result<()> {
    unavailable()
}
pub fn read_gateway_key() -> io::Result<Vec<u8>> {
    unavailable()
}
pub fn read_gateway_network() -> io::Result<Option<Vec<u8>>> {
    unavailable()
}
pub fn write_gateway_address(_: &[u8]) -> io::Result<()> {
    unavailable()
}
pub fn run_gateway_service() -> io::Result<()> {
    unavailable()
}
pub fn write_provision_bundle(_: &Path, _: &[(&str, &[u8])]) -> io::Result<()> {
    unavailable()
}
pub fn host_process_instance(_: u32) -> io::Result<String> {
    unavailable()
}
pub fn host_uptime_ms() -> io::Result<u64> {
    unavailable()
}
pub fn read_host_evidence() -> io::Result<Vec<u8>> {
    unavailable()
}
pub fn write_host_evidence(_: &[u8]) -> io::Result<()> {
    unavailable()
}

// Login self-setup (E21): no privileged login host on this platform.
use crate::login::os::shared::{
    self_channel::Record,
    self_install::{Elevation, ScriptCall},
};
pub const APP_FILE: &str = "smart_explorer";
pub const SCRIPT_NAME: &str = "install-login-service";
pub const SCRIPT: &str = "";
pub const STAGE_IMAGE: &str = "smart-explorer-login-image";
pub fn privileged() -> bool {
    false
}
pub fn elevation() -> Result<Elevation, String> {
    Err("unsupported".into())
}
pub fn run_elevated(_: &Path, _: &[String]) -> Result<i32, String> {
    Err("unsupported".into())
}
pub fn stage_root() -> io::Result<std::path::PathBuf> {
    unavailable()
}
pub fn open_image(_: &Path) -> io::Result<File> {
    unavailable()
}
pub fn create_stage_file(_: &Path) -> io::Result<File> {
    unavailable()
}
pub fn run_install_script(_: &Path, _: &ScriptCall<'_>) -> io::Result<i32> {
    unavailable()
}
pub fn read_self_record(_: Record, _: usize) -> io::Result<Option<Vec<u8>>> {
    unavailable()
}
pub fn write_self_record(_: Record, _: &[u8]) -> io::Result<()> {
    unavailable()
}
pub fn connect_grants() -> io::Result<ClientStream> {
    unavailable()
}
