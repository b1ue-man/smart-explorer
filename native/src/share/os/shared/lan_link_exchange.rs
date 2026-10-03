//! Short mutually challenged status rounds over an already pinned TLS channel.
use std::io;
use std::sync::Arc;

use futures_util::StreamExt;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use tokio::io::AsyncWriteExt;
use tokio::time::{timeout_at, Instant};

use super::core::{eio, random_bytes};
use super::lan_link_facts::{self, LanPeerPin};
use super::lan_link_transport::{
    self, LanLinkTransport, SelectedIpPath, REFRESH_INTERVAL, ROUND_DEADLINE, SESSION_DEADLINE,
};
use super::lan_link_wire::{self, LinkFrame, MAX_LINK_FRAME};
use super::node::ShareIrohNode;

pub(super) async fn run(
    node: Arc<ShareIrohNode>,
    transport: Arc<LanLinkTransport>,
    connection: Connection,
    expected: Option<LanPeerPin>,
) -> io::Result<()> {
    let mut paths = std::pin::pin!(connection.path_events());
    let work = async {
        if let Some(pin) = expected {
            client(&node, &transport, &connection, pin).await
        } else {
            server(&node, &transport, &connection).await
        }
    };
    let monitor = async {
        while paths.next().await.is_some() {
            transport.path_changed(&connection);
            // Keep the frame actor and its deadline polled during event bursts.
            tokio::task::yield_now().await;
        }
        Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "LAN-Link geschlossen",
        ))
    };
    tokio::time::timeout(SESSION_DEADLINE, async {
        tokio::select! {
            biased;
            result = monitor => result,
            result = work => result,
        }
    })
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "LAN-Link-Sitzung abgelaufen"))?
}

fn checked_path(
    transport: &LanLinkTransport,
    connection: &Connection,
) -> io::Result<SelectedIpPath> {
    let path = lan_link_transport::selected_ip_path(connection)
        .ok_or_else(|| eio("LAN-Link hat keinen selektierten IP-Pfad"))?;
    let host = transport.host()?;
    lan_link_facts::private_interface(path.local, path.remote, &host.interfaces)
        .ok_or_else(|| eio("LAN-Link hat kein eindeutiges privates Interface"))?;
    Ok(path)
}

async fn client(
    node: &ShareIrohNode,
    transport: &LanLinkTransport,
    connection: &Connection,
    expected: LanPeerPin,
) -> io::Result<()> {
    let mut deadline = Instant::now() + ROUND_DEADLINE;
    for _ in 0..24 {
        let result = timeout_at(deadline, async {
            node.require_sharing_active()?;
            let (path, revision) = snapshot(deadline, || {
                Ok((
                    checked_path(transport, connection)?,
                    transport.path_revision(connection)?,
                ))
            })
            .await?;
            let nonce = random_bytes::<32>().map_err(eio)?;
            let own = snapshot(deadline, || lan_link_transport::local_identity(node)).await?;
            let (mut send, mut recv) = connection.open_bi().await.map_err(eio)?;
            send_frame(
                &mut send,
                &LinkFrame::Challenge {
                    nonce,
                    identity: own,
                },
            )
            .await?;
            let (remote_nonce, identity, uplink) = match recv_frame(&mut recv).await? {
                LinkFrame::Answer {
                    echo,
                    nonce: remote_nonce,
                    identity,
                    uplink,
                } => {
                    lan_link_wire::check_echo(&nonce, &echo)?;
                    (remote_nonce, identity, uplink)
                }
                _ => return Err(eio("LAN-Link erwartet Challenge-Antwort")),
            };
            let pin = snapshot(deadline, || {
                lan_link_transport::current_pin(node, connection, &identity)
            })
            .await?;
            if pin != expected {
                return Err(eio("LAN-Link-Kontaktpin wurde geaendert"));
            }
            let own_uplink = snapshot(deadline, || Ok(transport.host()?.own_uplink)).await?;
            send_frame(
                &mut send,
                &LinkFrame::Confirm {
                    echo: remote_nonce,
                    uplink: own_uplink,
                },
            )
            .await?;
            send.finish().map_err(eio)?;
            snapshot(deadline, || {
                transport.confirm(
                    node,
                    connection,
                    pin.clone(),
                    path.clone(),
                    revision,
                    nonce,
                    uplink,
                )
            })
            .await
        })
        .await
        .map_err(|_| challenge_timeout())?;
        if discarded_revision(&result) {
            continue; // Fresh stream/nonces, same absolute deadline and attempt budget.
        }
        result?;
        tokio::time::sleep(REFRESH_INTERVAL).await;
        deadline = Instant::now() + ROUND_DEADLINE;
    }
    Ok(())
}

async fn server(
    node: &ShareIrohNode,
    transport: &LanLinkTransport,
    connection: &Connection,
) -> io::Result<()> {
    let mut deadline = Instant::now() + ROUND_DEADLINE;
    for _ in 0..24 {
        node.require_sharing_active()?;
        let result = timeout_at(deadline, async {
            check_deadline(deadline)?;
            let (mut send, mut recv) = connection.accept_bi().await.map_err(eio)?;
            let (echo, identity) = match recv_frame(&mut recv).await? {
                LinkFrame::Challenge { nonce, identity } => (nonce, identity),
                _ => return Err(eio("LAN-Link erwartet Challenge")),
            };
            let pin = snapshot(deadline, || {
                lan_link_transport::current_pin(node, connection, &identity)
            })
            .await?;
            let (path, revision) = snapshot(deadline, || {
                Ok((
                    checked_path(transport, connection)?,
                    transport.path_revision(connection)?,
                ))
            })
            .await?;
            let nonce = random_bytes::<32>().map_err(eio)?;
            let own = snapshot(deadline, || lan_link_transport::local_identity(node)).await?;
            let uplink = snapshot(deadline, || Ok(transport.host()?.own_uplink)).await?;
            send_frame(
                &mut send,
                &LinkFrame::Answer {
                    echo,
                    nonce,
                    identity: own,
                    uplink,
                },
            )
            .await?;
            send.finish().map_err(eio)?;
            let peer_uplink = match recv_frame(&mut recv).await? {
                LinkFrame::Confirm { echo, uplink } => {
                    lan_link_wire::check_echo(&nonce, &echo)?;
                    uplink
                }
                _ => return Err(eio("LAN-Link erwartet Bestaetigung")),
            };
            snapshot(deadline, || {
                transport.confirm(
                    node,
                    connection,
                    pin.clone(),
                    path.clone(),
                    revision,
                    nonce,
                    peer_uplink,
                )
            })
            .await
        })
        .await
        .map_err(|_| challenge_timeout())?;
        if discarded_revision(&result) {
            continue;
        }
        result?;
        deadline = Instant::now() + ROUND_DEADLINE;
    }
    Ok(())
}

fn discarded_revision(result: &io::Result<()>) -> bool {
    result.as_ref().is_err_and(|error| {
        error
            .get_ref()
            .is_some_and(|cause| cause.is::<lan_link_transport::PathRevisionChanged>())
    })
}

fn challenge_timeout() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "LAN-Link-Challenge abgelaufen")
}

fn check_deadline(deadline: Instant) -> io::Result<()> {
    if Instant::now() >= deadline {
        Err(challenge_timeout())
    } else {
        Ok(())
    }
}

async fn snapshot<T>(deadline: Instant, mut read: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    loop {
        check_deadline(deadline)?;
        match read() {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                tokio::task::yield_now().await;
            }
            result => return result,
        }
    }
}

async fn send_frame(send: &mut SendStream, frame: &LinkFrame) -> io::Result<()> {
    let bytes = lan_link_wire::encode(frame)?;
    send.write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .map_err(eio)?;
    send.write_all(&bytes).await.map_err(eio)?;
    send.flush().await.map_err(eio)
}

async fn recv_frame(recv: &mut RecvStream) -> io::Result<LinkFrame> {
    let mut length = [0u8; 4];
    recv.read_exact(&mut length)
        .await
        .map_err(super::framing::read_exact_error)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_LINK_FRAME {
        return Err(eio("LAN-Link-Frame zu gross"));
    }
    let mut bytes = vec![0u8; length];
    recv.read_exact(&mut bytes)
        .await
        .map_err(super::framing::read_exact_error)?;
    lan_link_wire::decode(&bytes)
}
