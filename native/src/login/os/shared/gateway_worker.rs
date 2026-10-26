//! Serial local broker bridge; command and image owners end with the worker.
use super::super::{client::GatewayClient, denied};
use crate::login::core::{
    model::{Challenge, SignedRequest},
    wire::Reply,
};
use std::{io, sync::Arc};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

struct Work {
    request: SignedRequest,
    answer: oneshot::Sender<
        io::Result<(
            Reply,
            Option<crate::screen::core::capture_image::CapturedImage>,
        )>,
    >,
}
pub(super) struct BrokerWorker {
    commands: Option<mpsc::Sender<Work>>,
    ready: Option<
        oneshot::Receiver<
            io::Result<(
                Challenge,
                Arc<crate::screen::core::image_admission::ImageAdmission>,
            )>,
        >,
    >,
    worker: Option<JoinHandle<()>>,
    images: Option<Arc<crate::screen::core::image_admission::ImageAdmission>>,
}
impl BrokerWorker {
    pub(super) fn start(peer: String, user: String) -> Self {
        let (commands, mut input) = mpsc::channel::<Work>(1);
        let (ready, initial) = oneshot::channel();
        let worker = tokio::task::spawn_blocking(move || {
            let mut client = match GatewayClient::connect(&peer, &user) {
                Ok(client) => client,
                Err(error) => {
                    let _ = ready.send(Err(error));
                    return;
                }
            };
            if ready
                .send(Ok((client.challenge().clone(), client.images())))
                .is_err()
            {
                return;
            }
            while let Some(work) = input.blocking_recv() {
                // Abandoned queued work must not become a new privileged IPC
                // request. An operation already handed to the broker is in flight.
                if input.is_closed() || work.answer.is_closed() {
                    break;
                }
                let answer = client.send(work.request);
                let end = answer.is_err();
                if work.answer.send(answer).is_err() || end {
                    break;
                }
            }
            // Dropping the only IPC client ends capture/input at the independent broker.
        });
        Self {
            commands: Some(commands),
            ready: Some(initial),
            worker: Some(worker),
            images: None,
        }
    }
    pub(super) async fn ready(&mut self) -> io::Result<Challenge> {
        let (challenge, images) = self
            .ready
            .take()
            .ok_or_else(|| denied("broker startup challenge was already consumed"))?
            .await
            .map_err(io::Error::other)??;
        self.images = Some(images);
        Ok(challenge)
    }
    pub(super) async fn ask(
        &mut self,
        request: SignedRequest,
    ) -> io::Result<(
        Reply,
        Option<crate::screen::core::capture_image::CapturedImage>,
    )> {
        let (answer, received) = oneshot::channel();
        self.commands
            .as_ref()
            .ok_or_else(|| denied("local broker bridge is closed"))?
            .send(Work { request, answer })
            .await
            .map_err(|_| io::Error::other("local broker bridge stopped"))?;
        received.await.map_err(io::Error::other)?
    }
    pub(super) fn images(
        &self,
    ) -> io::Result<Arc<crate::screen::core::image_admission::ImageAdmission>> {
        self.images
            .clone()
            .ok_or_else(|| denied("broker image admission is unavailable"))
    }
    pub(super) async fn finish(mut self) {
        self.commands.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.await;
        }
    }
}
impl Drop for BrokerWorker {
    fn drop(&mut self) {
        self.commands.take();
    }
}
