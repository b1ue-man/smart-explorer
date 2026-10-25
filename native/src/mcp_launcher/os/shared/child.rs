//! A verified process with the same captured streams on both desktop OSes.
use std::io::{Read, Write};
use std::process::ExitStatus;

pub struct GenerationChild {
    pub stdin: Option<Box<dyn Write + Send>>,
    pub stdout: Option<Box<dyn Read + Send>>,
    pub stderr: Option<Box<dyn Read + Send>>,
    pub(in crate::mcp_launcher) process: super::os::NativeChild,
    // Kept by the launcher until child exit; EOF signals unexpected parent death.
    pub(in crate::mcp_launcher) _lifecycle: Option<std::fs::File>,
}
impl GenerationChild {
    pub fn id(&self) -> u32 { self.process.id() }
    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> { self.process.try_wait() }
    pub fn wait(&mut self) -> std::io::Result<ExitStatus> { self.process.wait() }
    pub fn kill(&mut self) -> std::io::Result<()> { self.process.kill() }
    pub fn request_stop(&self) -> bool { super::os::request_stop(self.id()) }
}
