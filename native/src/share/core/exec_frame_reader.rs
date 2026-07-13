use std::io;

use tokio::io::AsyncRead;
use tokio::sync::mpsc;

use super::exec_protocol::{recv_client_frame, recv_server_frame, ClientFrame, ServerFrame};

const FRAME_CAPACITY: usize = 16;

/// Owns one uninterrupted frame decoder. Callers may cancel `next()` safely;
/// dropping the reader aborts the only task that still owns the stream half.
pub(super) struct FrameReader<T> {
    frames: mpsc::Receiver<io::Result<T>>,
    task: tokio::task::JoinHandle<()>,
}

impl<T> FrameReader<T> {
    pub(super) async fn next(&mut self) -> Option<io::Result<T>> {
        self.frames.recv().await
    }
}

impl<T> Drop for FrameReader<T> {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(super) fn client_frames<R>(mut reader: R) -> FrameReader<ClientFrame>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let (sender, frames) = mpsc::channel(FRAME_CAPACITY);
    let task = tokio::spawn(async move {
        loop {
            let frame = recv_client_frame(&mut reader).await;
            let terminal = frame.is_err();
            if sender.send(frame).await.is_err() || terminal {
                return;
            }
        }
    });
    FrameReader { frames, task }
}

pub(super) fn server_frames<R>(mut reader: R) -> FrameReader<ServerFrame>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let (sender, frames) = mpsc::channel(FRAME_CAPACITY);
    let task = tokio::spawn(async move {
        loop {
            let frame = recv_server_frame(&mut reader).await;
            let terminal = frame.is_err();
            if sender.send(frame).await.is_err() || terminal {
                return;
            }
        }
    });
    FrameReader { frames, task }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::exec_protocol::send_server_frame;
    use crate::share::exec_types::ExecId;
    use std::time::Duration;

    #[test]
    fn cancelling_one_next_call_does_not_cancel_frame_decoding() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let exec_id = ExecId::parse("ab".repeat(16)).unwrap();
            let (mut writer, reader) = tokio::io::duplex(512);
            let mut frames = server_frames(reader);

            assert!(
                tokio::time::timeout(Duration::from_millis(10), frames.next())
                    .await
                    .is_err()
            );

            let expected = ServerFrame::Started {
                exec_id: exec_id.clone(),
            };
            send_server_frame(&mut writer, &expected).await.unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), frames.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap(),
                expected
            );
        });
    }
}
