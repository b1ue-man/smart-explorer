//! Bounded helper communication; neither source APIs nor codecs run here.
use std::{io::{self, Read, Write}, sync::mpsc::{self, Receiver}, time::Duration};
use crate::login::core::{model::{Operation, SessionBinding}, wire::{self, HelperCall, HelperCommand, HelperInit, Reply, CONTROL, PIXELS}};
use super::{installation::Installation, nonce, super::platform};
pub(crate) trait Lifetime: Send { fn stop(&mut self); fn process_id(&self) -> u32 { 0 } }
pub(crate) struct Spawned {
    pub input: Box<dyn Write + Send>, pub output: Box<dyn Read + Send>, pub life: Box<dyn Lifetime>,
}
impl Drop for Spawned { fn drop(&mut self) { self.life.stop(); } }
pub(crate) struct HelperLink { process: Spawned, answers: Receiver<io::Result<(Reply, Vec<u8>)>>, cookie: String }
impl HelperLink {
    pub(crate) fn process_id(&self) -> u32 { self.process.life.process_id() }
    pub(crate) fn start(install: &Installation, binding: &SessionBinding, probe: bool) -> io::Result<Self> {
        let mut process = platform::spawn(install, binding, probe)?;
        let cookie = nonce()?;
        wire::send(&mut process.input, &HelperInit { binding: binding.clone(), parent_id: std::process::id(), cookie: cookie.clone() })?;
        // Move the real reader out; the replacement never performs I/O.
        let mut output = std::mem::replace(&mut process.output, Box::new(io::empty()));
        let (sender, answers) = mpsc::sync_channel(1);
        std::thread::Builder::new().name("login-helper-answer".into()).spawn(move || loop {
            let answer = (|| { let bytes = wire::read_frame(&mut output, CONTROL)?;
                let reply: Reply = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
                let pixels = if reply.pixel_bytes == 0 { Vec::new() } else { wire::read_frame(&mut output, PIXELS)? };
                if pixels.len() != reply.pixel_bytes { return Err(io::Error::other("login helper pixel length mismatch")); }
                Ok((reply, pixels)) })();
            let end = answer.is_err(); if sender.send(answer).is_err() || end { break; }
        })?;
        Ok(Self { process, answers, cookie })
    }
    pub(crate) fn ready(&mut self) -> io::Result<Reply> { self.answer().map(|(reply, _)| reply) }
    pub(crate) fn ask(&mut self, operation: Operation) -> io::Result<(Reply, Vec<u8>)> {
        wire::send(&mut self.process.input, &HelperCall { cookie: self.cookie.clone(), command: HelperCommand::Operation(operation) })?;
        self.answer()
    }
    pub(crate) fn observe(&mut self) -> io::Result<Reply> {
        wire::send(&mut self.process.input, &HelperCall { cookie: self.cookie.clone(), command: HelperCommand::Observe })?;
        self.answer().map(|(reply, _)| reply)
    }
    fn answer(&mut self) -> io::Result<(Reply, Vec<u8>)> {
        match self.answers.recv_timeout(Duration::from_secs(5)) {
            Ok(answer) => answer,
            Err(_) => { self.process.life.stop(); Err(io::Error::new(io::ErrorKind::TimedOut, "login helper did not answer within five seconds")) }
        }
    }
}
impl Drop for HelperLink {
    fn drop(&mut self) {
        let _ = wire::send(&mut self.process.input, &HelperCall { cookie: self.cookie.clone(), command: HelperCommand::Operation(Operation::Stop) });
        self.process.life.stop();
    }
}
