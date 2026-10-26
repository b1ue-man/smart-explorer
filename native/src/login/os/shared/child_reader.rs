//! One inherited response reader; metadata precedes admitted pixel allocation.
use super::capture_lifetime::{CaptureLifetime, ReaderEnd};
use crate::{
    login::core::{
        capture_wire::CaptureCall,
        wire::{self, HelperCall, HelperCommand, Reply, CAPTURE, CONTROL, PIXELS},
    },
    screen::core::capture_bytes::AdmittedBytes,
};
use std::{
    io::{self, Read, Write},
    sync::{mpsc::SyncSender, Arc, Mutex},
};
pub(crate) type Input = Arc<Mutex<Box<dyn Write + Send>>>;
pub(crate) type Answer = (Reply, Option<AdmittedBytes>);

pub(crate) fn send(input: &Input, call: &HelperCall) -> io::Result<()> {
    let mut writer = input
        .lock()
        .map_err(|_| io::Error::other("login helper input poisoned"))?;
    wire::send(&mut *writer, call)
}
struct Reader {
    output: Box<dyn Read + Send>,
    lifetime: Arc<CaptureLifetime>,
    // Output handle is destroyed before its retirement fact is recorded.
    _end: ReaderEnd,
}
pub(crate) fn start(
    output: Box<dyn Read + Send>,
    input: Input,
    cookie: String,
    lifetime: Arc<CaptureLifetime>,
    sender: SyncSender<io::Result<Answer>>,
) -> io::Result<()> {
    let end = lifetime.reader();
    let mut reader = Reader {
        output,
        lifetime,
        _end: end,
    };
    std::thread::Builder::new()
        .name("login-helper-answer".into())
        .spawn(move || loop {
            let answer = next(&mut reader, &input, &cookie);
            match answer {
                Ok(None) => continue,
                Ok(Some(answer)) => {
                    if sender.send(Ok(answer)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(error));
                    break;
                }
            }
        })?;
    Ok(())
}
fn next(reader: &mut Reader, input: &Input, cookie: &str) -> io::Result<Option<Answer>> {
    let (tag, length) = wire::read_header(&mut reader.output)?;
    if tag != CONTROL && tag != CAPTURE {
        return Err(super::denied("unexpected login helper response tag"));
    }
    let bytes = wire::read_body(&mut reader.output, length)?;
    if tag == CAPTURE {
        let capture = reader
            .lifetime
            .capture()?
            .ok_or_else(|| super::denied("metadata probe attempted capture admission"))?;
        let call: CaptureCall = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        match call {
            CaptureCall::Prepare {
                cookie: actual,
                id,
                plan,
            } => {
                if actual != cookie {
                    return Err(super::denied("wrong inherited login capture capability"));
                }
                // An exact resource refusal is delivered to the capture
                // caller, which can release pressed inputs immediately. It
                // must not wait out the authority timeout after a refusal.
                let reply = capture.approve(id, plan.clone()).unwrap_or(
                    crate::login::core::capture_wire::GrantReply {
                        id,
                        approved: false,
                        plan,
                    },
                );
                send(
                    input,
                    &HelperCall {
                        cookie: cookie.into(),
                        command: HelperCommand::Capture(reply),
                    },
                )?;
            }
            CaptureCall::Retire { cookie: actual, id } => {
                if actual != cookie {
                    return Err(super::denied("wrong inherited login retirement capability"));
                }
                capture.retire(id)?;
            }
        }
        return Ok(None);
    }
    let reply: Reply = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let pixels = if reply.pixel_bytes == 0 {
        if reply.capture_id.is_some() || reply.state == "pixels" {
            return Err(super::denied("empty login capture result"));
        }
        None
    } else {
        let (tag, length) = wire::read_header(&mut reader.output)?;
        if tag != PIXELS {
            return Err(super::denied("login helper pixel tag required"));
        }
        let capture = reader
            .lifetime
            .capture()?
            .ok_or_else(|| super::denied("metadata probe returned pixels"))?;
        let mut pixels = capture.pixels(&reply, length)?;
        reader.output.read_exact(pixels.as_mut_slice())?;
        Some(pixels)
    };
    Ok(Some((reply, pixels)))
}
#[cfg(test)]
pub(crate) fn read_fixture(
    output: Box<dyn Read + Send>,
    input: Input,
    cookie: &str,
    lifetime: Arc<CaptureLifetime>,
) -> io::Result<Option<Answer>> {
    let end = lifetime.reader();
    next(
        &mut Reader {
            output,
            lifetime,
            _end: end,
        },
        &input,
        cookie,
    )
}
