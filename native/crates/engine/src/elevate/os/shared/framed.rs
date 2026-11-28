//! The caller's side of the elevated helper: after its ready frame, its
//! frames become the child's events and the child's input becomes frames.
//! A cancel sends the cancel frame and also ends the launcher while it still
//! belongs to the caller (pkexec before it started the helper); the helper
//! also cancels when its input ends without an end-of-input frame, i.e. when
//! the caller is gone.
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use super::child::{ElevatedChild, InputPort};
use super::types::{ElevatedEvent, ElevatedExit, Elevation};
use super::wire::{self, FromHelper, ToHelper};

/// A started launcher whose helper sent its ready frame.
pub(super) struct Launched {
    pub(super) reader: Box<dyn Read + Send>,
    pub(super) writer: Box<dyn Write + Send>,
    /// Ends the launcher process where the caller may (best effort).
    pub(super) abort: Box<dyn Fn() + Send + Sync>,
    /// Waits for the launcher process after the helper's last frame.
    pub(super) reap: Box<dyn FnOnce() + Send>,
}

/// Reads the helper's first frame: `true` for the ready frame; `false` at
/// the end of the stream or for anything else (a launcher's own text).
pub(super) fn await_ready(reader: &mut dyn Read) -> bool {
    matches!(wire::read_from_helper(reader), Ok(Some(FromHelper::Ready)))
}

struct FramedInput {
    writer: Mutex<Box<dyn Write + Send>>,
    abort: Box<dyn Fn() + Send + Sync>,
    cancelled: Arc<AtomicBool>,
}

impl FramedInput {
    fn send(&self, frame: &ToHelper) -> io::Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| io::Error::other("helper input lock poisoned"))?;
        wire::send_to_helper(&mut **writer, frame)
    }
}

impl InputPort for FramedInput {
    fn stdin(&self, bytes: &[u8]) -> io::Result<()> {
        self.send(&ToHelper::Stdin(bytes.to_vec()))
    }

    fn eof(&self) -> io::Result<()> {
        self.send(&ToHelper::Eof)
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let _ = self.send(&ToHelper::Cancel);
        (self.abort)();
    }
}

pub(super) fn start(launched: Launched, via: Elevation) -> ElevatedChild {
    let Launched {
        mut reader,
        writer,
        abort,
        reap,
    } = launched;
    let cancelled = Arc::new(AtomicBool::new(false));
    let input = Arc::new(FramedInput {
        writer: Mutex::new(writer),
        abort,
        cancelled: cancelled.clone(),
    });
    let (sender, events) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("se-elevate-frames".into())
        .spawn(move || {
            let exit = relay(&mut *reader, &sender);
            let exit = ElevatedExit {
                cancelled: exit.cancelled || cancelled.load(Ordering::SeqCst),
                ..exit
            };
            let _ = sender.send(ElevatedEvent::Exited(exit));
            reap();
        });
    if let Err(error) = spawned {
        input.cancel();
        let (sender, failed) = mpsc::channel();
        let _ = sender.send(ElevatedEvent::Exited(ElevatedExit {
            error: Some(format!("cannot read the elevated helper: {error}")),
            ..ElevatedExit::default()
        }));
        return ElevatedChild::new(failed, input, via);
    }
    ElevatedChild::new(events, input, via)
}

/// Passes output on until the helper's exit frame; the exit it reported, or
/// why there is none.
fn relay(reader: &mut dyn Read, sender: &mpsc::Sender<ElevatedEvent>) -> ElevatedExit {
    loop {
        match wire::read_from_helper(reader) {
            Ok(Some(FromHelper::Stdout(bytes))) => {
                let _ = sender.send(ElevatedEvent::Stdout(bytes));
            }
            Ok(Some(FromHelper::Stderr(bytes))) => {
                let _ = sender.send(ElevatedEvent::Stderr(bytes));
            }
            Ok(Some(FromHelper::Ready)) => {}
            Ok(Some(FromHelper::Exit(exit))) => return exit,
            Ok(Some(FromHelper::Failed(text))) => {
                return ElevatedExit {
                    error: Some(text),
                    ..ElevatedExit::default()
                }
            }
            Ok(None) => {
                return ElevatedExit {
                    error: Some("the elevated helper ended without a result".into()),
                    ..ElevatedExit::default()
                }
            }
            Err(error) => {
                return ElevatedExit {
                    error: Some(format!("the elevated helper's channel broke: {error}")),
                    ..ElevatedExit::default()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_elevate_task_framed_child_relays_output_and_exit() {
        let mut stream = Vec::new();
        for frame in [
            FromHelper::Ready,
            FromHelper::Stdout(b"out".to_vec()),
            FromHelper::Stderr(b"err".to_vec()),
            FromHelper::Exit(ElevatedExit {
                code: Some(4),
                ..ElevatedExit::default()
            }),
        ] {
            wire::send_from_helper(&mut stream, &frame).unwrap();
        }
        let mut reader = std::io::Cursor::new(stream);
        assert!(await_ready(&mut reader));
        let written = Arc::new(Mutex::new(Vec::new()));
        struct Sink(Arc<Mutex<Vec<u8>>>);
        impl Write for Sink {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut child = start(
            Launched {
                reader: Box::new(reader),
                writer: Box::new(Sink(written.clone())),
                abort: Box::new(|| {}),
                reap: Box::new(|| {}),
            },
            Elevation::Polkit,
        );
        child.input().stdin(b"in").unwrap();
        child.input().eof().unwrap();
        assert_eq!(
            child.next_event(),
            Some(ElevatedEvent::Stdout(b"out".to_vec()))
        );
        assert_eq!(
            child.next_event(),
            Some(ElevatedEvent::Stderr(b"err".to_vec()))
        );
        assert_eq!(
            child.next_event(),
            Some(ElevatedEvent::Exited(ElevatedExit {
                code: Some(4),
                ..ElevatedExit::default()
            }))
        );
        assert_eq!(child.next_event(), None);
        assert_eq!(child.via(), Elevation::Polkit);
        let sent = written.lock().unwrap().clone();
        let mut sent = sent.as_slice();
        assert_eq!(
            wire::read_to_helper(&mut sent).unwrap(),
            Some(ToHelper::Stdin(b"in".to_vec()))
        );
        assert_eq!(
            wire::read_to_helper(&mut sent).unwrap(),
            Some(ToHelper::Eof)
        );
        // A launcher that printed text instead of starting the helper.
        let mut text: &[u8] = b"Error executing command";
        assert!(!await_ready(&mut text));
        let mut empty: &[u8] = b"";
        assert!(!await_ready(&mut empty));
    }
}
