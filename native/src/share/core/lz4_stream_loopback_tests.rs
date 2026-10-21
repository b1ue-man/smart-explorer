//! LZ4 content on the Share host over a real loopback connection: a client
//! that negotiated blocks is unpacked, one that did not is refused, and a bad
//! block never reaches a file.
use std::fs;
use std::io;

use super::stage_ranges_tests::{call, ctrl, on_disk, open_stage, write_request};
use super::CopyPastePeerFixture;
use crate::agent_proto::{pack_lz4, pack_raw, unpack};
use crate::share::framing::{
    decode_resp, recv_resp_wire, send_ctrl, send_tagged, TAG_DATA, TAG_DATA_LZ4,
};
use crate::share::io_deadline;
use crate::share::wire::{FsRequest, FsResponse, FsStageWrite};

fn text(len: usize) -> Vec<u8> {
    b"lines of a log file, again and again, until the block is full. "
        .iter()
        .copied()
        .cycle()
        .take(len)
        .collect()
}

/// A `WriteNew` whose data frames are `frames` (tag and bytes), then the
/// closing `WriteDone`; the answer to the open and the answer to the end.
fn upload(
    fixture: &CopyPastePeerFixture,
    path: &str,
    lz4: bool,
    frames: &[(u8, Vec<u8>)],
) -> io::Result<(FsResponse, io::Result<FsResponse>)> {
    let peer = &fixture.peer;
    let endpoint = peer.current_endpoint()?;
    let mut opened = peer.node.open_stream(&endpoint, &peer.identity)?;
    let open = ctrl(FsRequest::WriteNew {
        path: path.to_string(),
        lz4,
    });
    let ready = peer.node.block_on(io_deadline::run("test open", async {
        send_ctrl(&mut opened.send, &open).await?;
        recv_resp_wire(&mut opened.recv).await
    }))?;
    let ended = peer.node.block_on(io_deadline::run("test data", async {
        for (tag, bytes) in frames {
            send_tagged(&mut opened.send, *tag, bytes).await?;
        }
        send_ctrl(&mut opened.send, &ctrl(FsRequest::WriteDone)).await?;
        recv_resp_wire(&mut opened.recv).await
    }));
    Ok((ready, ended.and_then(decode_resp)))
}

#[test]
fn speed_task_lz4_share_host_unpacks_blocks_of_a_negotiated_upload() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let first = text(crate::share::fs::CHUNK);
    let second = text(1000);
    let packed = pack_lz4(&first).expect("text shrinks");
    assert!(packed.len() < first.len() / 5);
    let frames = [
        (TAG_DATA_LZ4, packed),
        (TAG_DATA, b"plain in between".to_vec()),
        (TAG_DATA_LZ4, pack_lz4(&second).expect("shrinks")),
        (TAG_DATA_LZ4, pack_raw(b"a raw block")),
    ];
    let (ready, ended) = upload(&fixture, "/A/log.txt", true, &frames)?;
    assert!(matches!(ready, FsResponse::Ready), "{ready:?}");
    assert!(matches!(ended?, FsResponse::Ok));
    let expected = [
        first,
        b"plain in between".to_vec(),
        second,
        b"a raw block".to_vec(),
    ]
    .concat();
    assert_eq!(fs::read(fixture.root_a.join("log.txt"))?, expected);
    Ok(())
}

#[test]
fn speed_task_lz4_share_host_refuses_unannounced_blocks() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let block = pack_lz4(&text(5000)).expect("shrinks");
    let (ready, ended) = upload(&fixture, "/A/plain.txt", false, &[(TAG_DATA_LZ4, block)])?;
    assert!(matches!(ready, FsResponse::Ready), "{ready:?}");
    assert!(ended.is_err(), "a block without the request's flag");
    assert_eq!(fs::metadata(fixture.root_a.join("plain.txt"))?.len(), 0);
    Ok(())
}

#[test]
fn speed_task_lz4_share_host_refuses_bad_blocks() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let good = pack_lz4(&text(50_000)).expect("shrinks");
    let mut too_long = good.clone();
    too_long[1..5].copy_from_slice(&(262_145u32).to_le_bytes());
    let mut wrong = good.clone();
    wrong[1..5].copy_from_slice(&(49_000u32).to_le_bytes());
    let cut = good[..good.len() / 2].to_vec();
    for (name, block) in [("a.txt", too_long), ("b.txt", wrong), ("c.txt", cut)] {
        let path = format!("/A/{name}");
        let (_, ended) = upload(&fixture, &path, true, &[(TAG_DATA_LZ4, block)])?;
        assert!(ended.is_err(), "{name}");
        assert_eq!(fs::metadata(fixture.root_a.join(name))?.len(), 0, "{name}");
    }
    assert_eq!(unpack(&good).expect("the good block").len(), 50_000);
    Ok(())
}

#[test]
fn speed_task_lz4_share_range_upload_takes_blocks() -> io::Result<()> {
    let fixture = CopyPastePeerFixture::for_transfer_engine_task()?;
    let body = text(300_000);
    let stage = open_stage(&fixture, "/A/ranged.txt", body.len() as u64)?;
    let peer = &fixture.peer;
    let endpoint = peer.current_endpoint()?;
    let mut opened = peer.node.open_stream(&endpoint, &peer.identity)?;
    let request = ctrl(FsRequest::StageWriteAt(FsStageWrite {
        stage: stage.clone(),
        offset: 0,
        len: body.len() as u64,
        lz4: true,
    }));
    let blocks = [
        pack_lz4(&body[..200_000]).expect("shrinks"),
        pack_lz4(&body[200_000..]).expect("shrinks"),
    ];
    let reply = peer.node.block_on(io_deadline::run("test range", async {
        send_ctrl(&mut opened.send, &request).await?;
        let ready = recv_resp_wire(&mut opened.recv).await?;
        for block in &blocks {
            send_tagged(&mut opened.send, TAG_DATA_LZ4, block).await?;
        }
        opened.send.finish().map_err(io::Error::other)?;
        Ok::<_, io::Error>((ready, recv_resp_wire(&mut opened.recv).await?))
    }))?;
    assert!(matches!(reply.0, FsResponse::Ready), "{:?}", reply.0);
    assert!(matches!(reply.1, FsResponse::Ok), "{:?}", reply.1);
    assert_eq!(fs::read(on_disk(&fixture, &stage))?, body);

    // The same blocks without the flag are refused and leave the range free.
    let other = open_stage(&fixture, "/A/refused.txt", 1000)?;
    let block = pack_lz4(&text(1000)).expect("shrinks");
    let peer_request = ctrl(write_request(&other, 0, 1000));
    let mut refused = peer.node.open_stream(&endpoint, &peer.identity)?;
    let outcome = peer
        .node
        .block_on(io_deadline::run("test range", async {
            send_ctrl(&mut refused.send, &peer_request).await?;
            let _ready = recv_resp_wire(&mut refused.recv).await?;
            send_tagged(&mut refused.send, TAG_DATA_LZ4, &block).await?;
            refused.send.finish().map_err(io::Error::other)?;
            recv_resp_wire(&mut refused.recv).await
        }))
        .and_then(decode_resp);
    assert!(outcome.is_err());
    assert!(matches!(
        call(&fixture, write_request(&other, 0, 1000))?,
        FsResponse::Ready
    ));
    Ok(())
}
