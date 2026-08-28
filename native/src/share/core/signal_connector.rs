use std::io;

use crossbeam_channel::{bounded, Receiver};

use super::identity::ShareIdentity;
use super::discovery_signal_types::DISCOVERY_EXCHANGE_CAPABILITY;
use super::signal_connection::{send_line, SignalConnection};
use super::signal_handshake::{await_hello_ok, SignalCapabilities};
use super::system::lan_ips;
use super::wire::{ClientMsg, TRACKED_DIRECT_CAPABILITY};

pub(super) struct NegotiatedSignal {
    pub(super) connection: SignalConnection,
    pub(super) capabilities: SignalCapabilities,
    pub(super) transport: String,
}

pub(super) fn spawn_connect(
    server: String,
    identity: ShareIdentity,
) -> io::Result<Receiver<io::Result<NegotiatedSignal>>> {
    let (send, receive) = bounded(1);
    std::thread::Builder::new()
        .name("share-signal-connect".into())
        .spawn(move || {
            let result = connect_and_negotiate(&server, &identity);
            let _ = send.send(result);
        })?;
    Ok(receive)
}

fn connect_and_negotiate(server: &str, identity: &ShareIdentity) -> io::Result<NegotiatedSignal> {
    let mut connection = SignalConnection::connect(server)?;
    let transport = connection.label().to_string();
    send_line(
        &mut connection,
        &ClientMsg::Hello {
            protocol_version: 3,
            device_id: identity.device_id.clone(),
            device_name: identity.device_name.clone(),
            listen_port: 0,
            lan: lan_ips(),
            public_key: identity.public_key.clone(),
            fingerprint: identity.fingerprint.clone(),
            capabilities: vec![
                TRACKED_DIRECT_CAPABILITY.to_string(),
                DISCOVERY_EXCHANGE_CAPABILITY.to_string(),
            ],
        },
    )?;
    let capabilities = await_hello_ok(&mut connection)?;
    Ok(NegotiatedSignal {
        connection,
        capabilities,
        transport,
    })
}
