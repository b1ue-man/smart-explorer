//! One explicitly requested firewall operation without blocking the GUI.
use std::sync::{mpsc, Mutex};

type Outcome = Result<(), String>;
static PENDING: Mutex<Option<mpsc::Receiver<Outcome>>> = Mutex::new(None);

pub fn start_lan_firewall_repair() -> Outcome {
    let mut pending = PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if pending.is_some() {
        return Err("Firewall-Freigabe laeuft bereits".into());
    }
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("lan-firewall-consent".into())
        .spawn(move || {
            let _ = sender
                .send(super::request_lan_firewall_repair().map_err(|error| error.to_string()));
        })
        .map_err(|error| format!("Firewall-Freigabe konnte nicht starten: {error}"))?;
    *pending = Some(receiver);
    Ok(())
}

pub fn poll_lan_firewall_repair() -> Option<Outcome> {
    let mut pending = PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match pending.as_ref()?.try_recv() {
        Ok(outcome) => {
            *pending = None;
            Some(outcome)
        }
        Err(mpsc::TryRecvError::Empty) => None,
        Err(mpsc::TryRecvError::Disconnected) => {
            *pending = None;
            Some(Err(
                "Firewall-Freigabe wurde abgebrochen; erneut versuchen".into()
            ))
        }
    }
}
