//! Bounded helper communication; neither source APIs nor codecs run here.
use super::{
    super::platform,
    capture_admission::ParentCapture,
    capture_lifetime::{CaptureLifetime, ProcessOwner},
    child_reader::{self, Answer, Input},
    installation::Installation,
    nonce,
};
use crate::login::core::{
    model::{Operation, SessionBinding},
    wire::{self, HelperCall, HelperCommand, HelperInit, Reply},
};
use std::{
    io::{self, Read, Write},
    sync::{
        mpsc::{self, Receiver},
        Arc, Mutex,
    },
    time::Duration,
};
/// true means OS-confirmed process exit; EOF, timeout and kill request are false.
pub(crate) trait Lifetime: Send {
    fn stop(&mut self) -> bool;
    fn process_id(&self) -> u32 {
        0
    }
}
pub(crate) struct Spawned {
    pub input: Box<dyn Write + Send>,
    pub output: Box<dyn Read + Send>,
    pub life: Box<dyn Lifetime>,
}
impl Drop for Spawned {
    fn drop(&mut self) {
        let _ = self.life.stop();
    }
}
pub(crate) struct HelperLink {
    process: ProcessOwner,
    input: Input,
    answers: Receiver<io::Result<Answer>>,
    cookie: String,
    pid: u32,
}
impl HelperLink {
    pub(crate) fn process_id(&self) -> u32 {
        self.pid
    }
    pub(crate) fn start(
        install: &Installation,
        binding: &SessionBinding,
        probe: bool,
    ) -> io::Result<Self> {
        let cookie = nonce()?;
        let capture = if probe {
            None
        } else {
            Some(ParentCapture::new(&cookie)?)
        };
        let descriptor = capture.as_ref().map(|capture| capture.descriptor());
        let mut spawned = platform::spawn(install, binding, probe)?;
        let pid = spawned.life.process_id();
        let output = std::mem::replace(&mut spawned.output, Box::new(io::empty()));
        let input: Input = Arc::new(Mutex::new(std::mem::replace(
            &mut spawned.input,
            Box::new(io::sink()),
        )));
        let lifetime = CaptureLifetime::new(capture);
        let process = ProcessOwner::new(spawned, lifetime.clone());
        let (sender, answers) = mpsc::sync_channel(1);
        // Install the reader first so every failure has an explicit reader-end
        // fact and the helper can synchronously request its opening grant.
        child_reader::start(output, input.clone(), cookie.clone(), lifetime, sender)?;
        {
            let mut writer = input
                .lock()
                .map_err(|_| io::Error::other("login helper input poisoned"))?;
            wire::send(
                &mut *writer,
                &HelperInit {
                    binding: binding.clone(),
                    parent_id: std::process::id(),
                    cookie: cookie.clone(),
                    capture: descriptor,
                },
            )?;
        }
        Ok(Self {
            process,
            input,
            answers,
            cookie,
            pid,
        })
    }
    pub(crate) fn ready(&mut self) -> io::Result<Reply> {
        self.answer().map(|(reply, _)| reply)
    }
    pub(crate) fn ask(&mut self, operation: Operation) -> io::Result<Answer> {
        child_reader::send(
            &self.input,
            &HelperCall {
                cookie: self.cookie.clone(),
                command: HelperCommand::Operation(operation),
            },
        )?;
        self.answer()
    }
    pub(crate) fn observe(&mut self) -> io::Result<Reply> {
        child_reader::send(
            &self.input,
            &HelperCall {
                cookie: self.cookie.clone(),
                command: HelperCommand::Observe,
            },
        )?;
        self.answer().map(|(reply, _)| reply)
    }
    fn answer(&mut self) -> io::Result<Answer> {
        match self.answers.recv_timeout(Duration::from_secs(5)) {
            Ok(answer) => answer,
            Err(_) => {
                self.process.stop();
                Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "login helper did not answer within five seconds",
                ))
            }
        }
    }
}
impl Drop for HelperLink {
    fn drop(&mut self) {
        let _ = child_reader::send(
            &self.input,
            &HelperCall {
                cookie: self.cookie.clone(),
                command: HelperCommand::Operation(Operation::Stop),
            },
        );
        self.process.stop();
    }
}
