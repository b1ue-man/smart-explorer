use std::io;
use std::time::{Duration, Instant};

const MAX_REQUEST_WORKERS: usize = 8;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct RequestWorkers {
    handles: Vec<std::thread::JoinHandle<io::Result<()>>>,
}

impl RequestWorkers {
    pub(super) fn has_capacity(&mut self) -> io::Result<bool> {
        let errors = self.reap();
        if errors.is_empty() {
            Ok(self.handles.len() < MAX_REQUEST_WORKERS)
        } else {
            Err(worker_errors(errors))
        }
    }

    pub(super) fn push(&mut self, handle: std::thread::JoinHandle<io::Result<()>>) {
        self.handles.push(handle);
    }

    pub(super) fn shutdown(&mut self) -> io::Result<()> {
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        let mut errors = Vec::new();
        while !self.handles.is_empty() {
            errors.extend(self.reap());
            if self.handles.is_empty() {
                break;
            }
            if Instant::now() >= deadline {
                errors.push(format!(
                    "{} daemon backend request worker(s) did not stop",
                    self.handles.len()
                ));
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(worker_errors(errors))
        }
    }

    fn reap(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut index = 0;
        while index < self.handles.len() {
            if self.handles[index].is_finished() {
                let handle = self.handles.swap_remove(index);
                match handle.join() {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => errors.push(error.to_string()),
                    Err(payload) => errors.push(format!(
                        "daemon backend request worker panicked: {}",
                        panic_message(&payload)
                    )),
                }
            } else {
                index += 1;
            }
        }
        errors
    }
}

fn worker_errors(errors: Vec<String>) -> io::Error {
    io::Error::other(errors.join("; "))
}

fn panic_message<'a>(payload: &'a (dyn std::any::Any + Send + 'static)) -> &'a str {
    payload
        .downcast_ref::<&'static str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic payload")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn shutdown_joins_completed_cleanup() {
        let completed = Arc::new(AtomicBool::new(false));
        let worker_completed = completed.clone();
        let mut workers = RequestWorkers::default();
        workers.push(std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            worker_completed.store(true, Ordering::Relaxed);
            Ok(())
        }));
        workers.shutdown().unwrap();
        assert!(completed.load(Ordering::Relaxed));
    }

    #[test]
    fn worker_errors_and_panics_are_returned() {
        let mut failed = RequestWorkers::default();
        failed.push(std::thread::spawn(|| {
            Err(io::Error::other("worker failed"))
        }));
        assert!(failed
            .shutdown()
            .unwrap_err()
            .to_string()
            .contains("worker failed"));

        let mut panicked = RequestWorkers::default();
        panicked.push(std::thread::spawn(|| -> io::Result<()> {
            panic!("worker panic")
        }));
        assert!(panicked
            .shutdown()
            .unwrap_err()
            .to_string()
            .contains("worker panic"));
    }
}
