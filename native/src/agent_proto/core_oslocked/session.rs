use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use super::{write_frame, Frame};

/// A shared, mutex-guarded frame sink.
pub(crate) type Sink = Arc<Mutex<Box<dyn Write + Send>>>;

pub(crate) fn emit(sink: &Sink, id: u64, frame: &Frame) -> io::Result<()> {
    let mut w = sink
        .lock()
        .map_err(|_| io::Error::new(io::ErrorKind::Other, "sink poisoned"))?;
    write_frame(&mut *w, id, frame)
}
