//! The background service's answer to `ContentSearch` (`content_search_v1`):
//! the content search of the backend it serves (`vfs::content_search`: a
//! Share peer's own search where the peer offers it, else reading the files
//! through the backend). The events go out as `ContentEvent` frames under the
//! request's credit, then `ContentDone`; a client that stops reading stops
//! the search.
use std::io;
use std::sync::atomic::AtomicBool;

use crate::agent_proto::{ContentQuery, Frame};
use crate::vfs::BackendHandle;

use super::{emit, Sink};

/// Events between the search and the connection: enough to keep the
/// searching threads busy while one frame waits for credit.
const EVENT_QUEUE: usize = 64;

pub(super) fn content_search(
    sink: &Sink,
    id: u64,
    backend: &BackendHandle,
    (root, query): (&str, &ContentQuery),
    cancel: &AtomicBool,
) -> io::Result<()> {
    let (tx, rx) = crossbeam_channel::bounded(EVENT_QUEUE);
    let (sent, searched) = std::thread::scope(|scope| {
        let search =
            scope.spawn(move || crate::vfs::content_search(&**backend, root, query, tx, cancel));
        let mut sent = Ok(());
        while let Ok(event) = rx.recv() {
            if let Err(error) = emit(sink, id, &Frame::ContentEvent(event)) {
                sent = Err(error);
                break;
            }
        }
        // A failed frame ends the search: its next event finds no receiver.
        drop(rx);
        let searched = search
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("content search worker panicked")));
        (sent, searched)
    });
    sent?;
    emit(sink, id, &Frame::ContentDone(searched?))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use crate::agent_proto::{ContentEvent, ContentQuery};
    use crate::vfs::Backend;

    /// The service announces `content_search_v1` and answers through the
    /// backend it serves; a client of the service gets the hits.
    #[test]
    fn content_search_task_worker_answers_through_its_backend() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("a.txt"), "x\nneedle\n").unwrap();
        std::fs::write(dir.path().join("b.log"), "needle\n").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let served: crate::vfs::BackendHandle = Arc::new(crate::vfs::LocalBackend::new("/"));
        let server = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            let read = socket.try_clone().unwrap();
            let _ = super::super::serve_backend(read, socket, served);
        });
        let client = TcpStream::connect(address).unwrap();
        let shutdown = client.try_clone().unwrap();
        let read: Box<dyn Read + Send> = Box::new(client.try_clone().unwrap());
        let write: Box<dyn Write + Send> = Box::new(client);
        let inner: crate::vfs::BackendHandle = Arc::new(crate::vfs::LocalBackend::new("/"));
        let backend = crate::agent::AgentBackend::from_streams(read, write, inner).unwrap();
        assert!(backend.features().service);
        assert!(backend.supports_content_search());

        let mut query = ContentQuery::text("needle");
        query.filters = "*.txt".into();
        let root = dir.path().to_string_lossy().replace('\\', "/");
        let (tx, rx) = crossbeam_channel::unbounded();
        let summary = backend
            .content_search(&root, &query, tx, &AtomicBool::new(false))
            .unwrap()
            .expect("the service searches");
        let files: Vec<String> = rx
            .try_iter()
            .filter_map(|event| match event {
                ContentEvent::Hits { rel, .. } => Some(rel),
                _ => None,
            })
            .collect();
        assert_eq!(files, ["a.txt"]);
        assert_eq!(summary.files_searched, 1);
        assert_eq!(summary.hits, 1);

        let _ = shutdown.shutdown(Shutdown::Both);
        drop(backend);
        server.join().unwrap();
    }
}
