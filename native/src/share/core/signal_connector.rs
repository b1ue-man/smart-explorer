use std::io;

use crossbeam_channel::{bounded, Receiver};

use super::discovery_signal_types::DISCOVERY_EXCHANGE_CAPABILITY;
use super::identity::ShareIdentity;
use super::signal_connection::{send_line, SignalConnection};
use super::signal_handshake::{await_hello_ok, SignalCapabilities, KEY_LOGIN_CAPABILITY};
use super::wire::{ClientMsg, IDLE_KEEPALIVE_CAPABILITY, TRACKED_DIRECT_CAPABILITY};

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
            // The server never used the local addresses; they only leaked the
            // address plan (S13). The field stays for older servers.
            lan: Vec::new(),
            public_key: identity.public_key.clone(),
            fingerprint: identity.fingerprint.clone(),
            capabilities: vec![
                TRACKED_DIRECT_CAPABILITY.to_string(),
                DISCOVERY_EXCHANGE_CAPABILITY.to_string(),
                IDLE_KEEPALIVE_CAPABILITY.to_string(),
                KEY_LOGIN_CAPABILITY.to_string(),
            ],
        },
    )?;
    let capabilities = await_hello_ok(&mut connection, identity)?;
    if capabilities.key_login {
        connection.confirm_key_login();
    }
    Ok(NegotiatedSignal {
        connection,
        capabilities,
        transport,
    })
}
