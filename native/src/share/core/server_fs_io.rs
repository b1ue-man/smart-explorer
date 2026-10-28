//! Admitted Share file streams keep their slot and I/O ownership together.
use std::io;

use iroh::endpoint::{RecvStream, SendStream};

use super::StreamContext;
use crate::share::framing::reply_err;
use crate::share::fs_access::FsAccess;
use crate::share::mount_lease::MountLeaseAuthorization;
use crate::share::server_transfer::{ReadSource, WriteMode};

pub(super) async fn read(
    mut send: SendStream,
    context: &StreamContext,
    source: ReadSource,
    access: FsAccess,
) -> io::Result<()> {
    let admitted = context.admit(&send).await;
    let slot = match admitted {
        Ok(slot) => slot,
        Err(error) => return reply_err(&mut send, error).await,
    };
    let stream = (context.stall(), Some(context.lz4.clone()));
    crate::share::server_transfer::read_file(send, source, access, slot, stream).await
}

pub(super) async fn write(
    mut send: SendStream,
    recv: RecvStream,
    context: &StreamContext,
    (path, mode, lz4, evidence): (String, WriteMode, bool, Option<Option<String>>),
    access: FsAccess,
    authorization: Option<MountLeaseAuthorization>,
) -> io::Result<()> {
    let admitted = context.admit(&send).await;
    let slot = match admitted {
        Ok(slot) => slot,
        Err(error) => return reply_err(&mut send, error).await,
    };
    let target = crate::share::server_transfer::WriteTarget {
        path,
        mode,
        authorization,
        lz4,
        evidence,
    };
    crate::share::server_transfer::write_file(send, recv, target, access, slot, context.stall())
        .await
}
