//! Bounded stdin and an independent, coalesced cancellation signal.
use super::ExecClientInput;
use iroh::endpoint::Connection;
use tokio::sync::{mpsc, watch};

#[derive(Clone)]
pub(crate) struct ExecInputSender {
    data: mpsc::Sender<ExecClientInput>,
    cancel: watch::Sender<bool>,
}

impl ExecInputSender {
    pub(super) fn new(data: mpsc::Sender<ExecClientInput>, cancel: watch::Sender<bool>) -> Self {
        Self { data, cancel }
    }

    pub(crate) async fn send(
        &self,
        input: ExecClientInput,
    ) -> Result<(), mpsc::error::SendError<ExecClientInput>> {
        if matches!(input, ExecClientInput::Cancel) {
            return self
                .cancel
                .send(true)
                .map_err(|_| mpsc::error::SendError(input));
        }
        let mut cancel = self.cancel.subscribe();
        tokio::select! {
            biased;
            _ = cancelled(&mut cancel) => Err(mpsc::error::SendError(input)),
            permit = self.data.reserve() => match permit {
                Ok(permit) => { permit.send(input); Ok(()) }
                Err(_) => Err(mpsc::error::SendError(input)),
            },
        }
    }

    pub(crate) fn cancel(&self) {
        let _ = self.cancel.send(true);
    }
}

pub(super) async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            return;
        }
    }
}

pub(super) async fn forward(
    connection: &Connection,
    mut input: mpsc::Receiver<ExecClientInput>,
    mut cancel: watch::Receiver<bool>,
    output: mpsc::Sender<ExecClientInput>,
) {
    loop {
        let command = tokio::select! {
            biased;
            _ = cancelled(&mut cancel) => break,
            command = input.recv() => match command { Some(command) => command, None => return },
            _ = connection.closed() => return,
        };
        tokio::select! {
            biased;
            _ = cancelled(&mut cancel) => break,
            sent = output.send(command) => if sent.is_err() { return; },
            _ = connection.closed() => return,
        }
    }
    // The parallel cancellation watcher closes a paused connection even when
    // this bounded queue is full. An admitted route keeps the existing CANCEL
    // exchange and its terminal acknowledgement.
    tokio::select! {
        _ = output.send(ExecClientInput::Cancel) => {},
        _ = connection.closed() => {},
    }
}
