use crossbeam_channel::unbounded;
use std::collections::HashMap;
use std::io;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use super::session::{worker, Answers, Session};
use super::system::random_fingerprint;
use super::transfer::accept_loop;
use super::types::{CmdTx, EventRx, ShareCmd, ShareEvent};

pub struct ShareService {
    pub events: EventRx,
    cmds: CmdTx,
    pub fingerprint: String,
    pub listen_port: u16,
}

impl ShareService {
    pub fn cmd(&self, c: ShareCmd) {
        let _ = self.cmds.send(c);
    }

    /// Start the background worker: bind a listener, spawn the accept loop, and
    /// process commands. `server` is the rendezvous host:port; `device` is our
    /// display name.
    pub fn start(server: String, device: String) -> io::Result<ShareService> {
        let fingerprint = random_fingerprint();
        let listener = TcpListener::bind("0.0.0.0:0")?;
        let listen_port = listener.local_addr()?.port();
        let (cmd_tx, cmd_rx) = unbounded::<ShareCmd>();
        let (ev_tx, ev_rx) = unbounded::<ShareEvent>();

        let session: Arc<Mutex<Session>> = Arc::new(Mutex::new(Session::default()));
        let answers: Answers = Arc::new(Mutex::new(HashMap::new()));

        {
            let session = session.clone();
            let answers = answers.clone();
            let ev = ev_tx.clone();
            std::thread::Builder::new()
                .name("share-accept".into())
                .spawn(move || accept_loop(listener, session, answers, ev))
                .ok();
        }

        {
            let ev = ev_tx.clone();
            let device = device.clone();
            let fp = fingerprint.clone();
            std::thread::Builder::new()
                .name("share-worker".into())
                .spawn(move || {
                    worker(
                        server,
                        device,
                        fp,
                        listen_port,
                        cmd_rx,
                        ev,
                        session,
                        answers,
                    )
                })
                .ok();
        }

        Ok(ShareService {
            events: ev_rx,
            cmds: cmd_tx,
            fingerprint,
            listen_port,
        })
    }
}
