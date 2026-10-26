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
pub(crate) struct ProtectedFile {
    pub file: File,
}
pub(crate) struct ClientStream;
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
pub(crate) fn paths() -> io::Result<Paths> {
    unavailable()
}
pub(crate) fn open_protected(_: &Path, _: bool) -> io::Result<ProtectedFile> {
    unavailable()
}
pub(crate) fn validate_gateway_identity(_: &str) -> io::Result<()> {
    unavailable()
}
pub(crate) fn binding(_: &Installation) -> io::Result<SessionBinding> {
    unavailable()
}
pub(crate) fn prepare_helper(_: &SessionBinding, _: u32, _: bool) -> io::Result<SessionBinding> {
    unavailable()
}
pub(crate) fn helper_binding_matches(_: &SessionBinding) -> io::Result<bool> {
    unavailable()
}
pub(crate) fn finish_helper() {}
pub(crate) fn secure_attention(_: &SessionBinding) -> io::Result<()> {
    unavailable()
}
pub(crate) fn spawn(_: &Installation, _: &SessionBinding, _: bool) -> io::Result<Spawned> {
    unavailable()
}
pub(crate) fn connect() -> io::Result<ClientStream> {
    unavailable()
}
pub(crate) fn run_broker() -> io::Result<()> {
    unavailable()
}

pub(crate) fn helper_channel() -> io::Result<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
    unavailable()
}

pub(crate) fn run_probe(
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

pub(crate) fn gateway_state_directory() -> io::Result<std::path::PathBuf> {
    unavailable()
}

pub(crate) fn require_gateway(_: &crate::login::Policy) -> io::Result<()> {
    unavailable()
}
pub(crate) fn read_gateway_key() -> io::Result<Vec<u8>> {
    unavailable()
}
pub(crate) fn read_gateway_network() -> io::Result<Option<Vec<u8>>> {
    unavailable()
}
pub(crate) fn write_gateway_address(_: &[u8]) -> io::Result<()> {
    unavailable()
}
pub(crate) fn run_gateway_service() -> io::Result<()> {
    unavailable()
}
pub(crate) fn write_provision_bundle(_: &Path, _: &[(&str, &[u8])]) -> io::Result<()> {
    unavailable()
}
pub(crate) fn host_process_instance(_: u32) -> io::Result<String> {
    unavailable()
}
pub(crate) fn host_uptime_ms() -> io::Result<u64> {
    unavailable()
}
pub(crate) fn read_host_evidence() -> io::Result<Vec<u8>> {
    unavailable()
}
pub(crate) fn write_host_evidence(_: &[u8]) -> io::Result<()> {
    unavailable()
}
