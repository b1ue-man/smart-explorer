//! Reading and writing the hellos (moved out of `wire_hello.rs`).
use std::io;

use tokio::io::{AsyncRead, AsyncWrite};

use super::super::wire::{
    from_json, invalid, read_frame, recv_json, send_json, APPROVAL_PENDING, CLIENT_HELLO,
    HELLO_ERROR, HELLO_OK, MAX_CONTROL_BYTES, SERVER_HELLO,
};
use super::{HelloReply, ScreenClientHello, ScreenHelloOk, ScreenServerHello};

pub async fn send_server_hello<W: AsyncWrite + Unpin>(
    writer: &mut W,
    hello: &ScreenServerHello,
) -> io::Result<()> {
    send_json(writer, SERVER_HELLO, hello).await
}

pub async fn recv_server_hello<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<ScreenServerHello> {
    recv_json(reader, SERVER_HELLO).await
}

pub async fn send_client_hello<W: AsyncWrite + Unpin>(
    writer: &mut W,
    hello: &ScreenClientHello,
) -> io::Result<()> {
    hello.validate()?;
    send_json(writer, CLIENT_HELLO, hello).await
}

// The host reads the hello with `recv_json` (`share/core/node_screen.rs`), so
// that an unknown version gets `screen_unsupported` before the field checks;
// the wire tests use this one.
#[allow(dead_code)]
pub async fn recv_client_hello<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<ScreenClientHello> {
    let hello: ScreenClientHello = recv_json(reader, CLIENT_HELLO).await?;
    hello.validate()?;
    Ok(hello)
}

pub async fn send_hello_reply<W: AsyncWrite + Unpin>(
    writer: &mut W,
    reply: &HelloReply,
) -> io::Result<()> {
    match reply {
        HelloReply::Accepted(accepted) => {
            accepted.validate()?;
            send_json(writer, HELLO_OK, accepted).await
        }
        HelloReply::Refused(refused) => send_json(writer, HELLO_ERROR, refused).await,
        HelloReply::Pending(pending) => send_json(writer, APPROVAL_PENDING, pending).await,
    }
}

pub async fn recv_hello_reply<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<HelloReply> {
    let (tag, payload) = read_frame(reader, MAX_CONTROL_BYTES).await?;
    match tag {
        HELLO_OK => {
            let accepted: ScreenHelloOk = from_json(&payload)?;
            accepted.validate()?;
            Ok(HelloReply::Accepted(accepted))
        }
        HELLO_ERROR => Ok(HelloReply::Refused(from_json(&payload)?)),
        APPROVAL_PENDING => Ok(HelloReply::Pending(from_json(&payload)?)),
        _ => Err(invalid("unexpected screen handshake frame")),
    }
}
