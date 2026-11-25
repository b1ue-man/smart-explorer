//! Where the host's pictures of one group go (the seam of the carriers,
//! `video_transport.rs`): a QUIC uni stream per group (`GopWriter`, the
//! picture streams of decision C3) or self-contained picture messages
//! (`video_message.rs`) on a message carrier: split into FEC-protected
//! packets on the direct UDP pair or as QUIC DATAGRAM frames (`SinkKind::
//! Packets`, „UDP“), or as one record each on the direct TCP link
//! (`SinkKind::Records`, „TCP“). Groups, ids, the link to the picture
//! before and the viewer's delivery (`video_delivery.rs`) are the same for
//! every message carrier; a group keeps its kind of carrier.
use std::io;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use iroh::endpoint::{Connection, SendDatagramError};

use super::super::capture_records::CapturedRecords;
use super::super::gop_stream::GopWriter;
use super::super::pacing::STALE_STREAM_US;
use super::super::wire::invalid;
use super::super::wire_binary::GopHeader;
use super::video_message::{encode, PictureMeta};
use super::video_packet::packetize;
use super::video_route::{quic_path, shard_len, DirectSender, PictureCarrier, RecordSender};
use super::VideoPath;

/// Cuts of one picture at most when the QUIC packet size shrinks under it.
const MAX_GENERATIONS: u8 = 3;

/// The pictures of one group.
pub enum PictureSink {
    /// One QUIC uni stream per group (boxed: its writer holds the pace).
    Stream(Box<GopWriter>),
    /// One message per picture on a message carrier.
    Messages(MessageGroup),
}

impl PictureSink {
    pub fn header(&self) -> &GopHeader {
        match self {
            Self::Stream(writer) => writer.header(),
            Self::Messages(group) => &group.header,
        }
    }

    /// Gives the group up: a stream is reset, the viewer drops what it has
    /// of it; sent messages are gone already.
    pub fn reset(self) {
        if let Self::Stream(writer) = self {
            writer.reset();
        }
    }

    /// Ends the group after its last picture.
    pub fn finish(self) {
        if let Self::Stream(writer) = self {
            writer.finish();
        }
    }

    pub fn is_messages(&self) -> bool {
        matches!(self, Self::Messages(_))
    }

    pub fn kind(&self) -> SinkKind {
        match self {
            Self::Stream(_) => SinkKind::Stream,
            Self::Messages(group) => group.carrier.kind(),
        }
    }
}

/// The kinds of carrier a group keeps from its first picture to its last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SinkKind {
    Stream,
    Packets,
    Records,
}

/// A carrier of picture messages.
#[derive(Clone)]
pub enum MessageCarrier {
    /// FEC packets sealed on the direct UDP pair.
    Direct(Arc<dyn DirectSender>),
    /// FEC packets as QUIC DATAGRAM frames of the session's connection.
    Quic(Connection),
    /// One record per message on the direct TCP link.
    Record(Arc<dyn RecordSender>),
}

impl MessageCarrier {
    pub fn kind(&self) -> SinkKind {
        match self {
            Self::Direct(_) | Self::Quic(_) => SinkKind::Packets,
            Self::Record(_) => SinkKind::Records,
        }
    }

    pub fn carrier(&self) -> PictureCarrier {
        match self {
            Self::Direct(_) => PictureCarrier::Udp(VideoPath::Direct),
            Self::Quic(connection) => PictureCarrier::Udp(quic_path(connection)),
            Self::Record(_) => PictureCarrier::Tcp,
        }
    }
}

/// What one picture took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageSent {
    /// Bytes of all its packets (or its record).
    pub bytes: u64,
    pub packets: usize,
    pub carrier: PictureCarrier,
}

/// The pictures of one group as messages.
pub struct MessageGroup {
    header: GopHeader,
    carrier: MessageCarrier,
    /// The picture sent before (0: none yet).
    last_frame: u64,
    /// The next picture is the first of a continuation (B-36).
    continues: bool,
}

/// Why a packet did not go.
enum PacketError {
    /// The QUIC packet size shrank below the cut: cut again.
    TooLarge,
    Failed(io::Error),
}

impl MessageGroup {
    pub fn new(header: GopHeader, carrier: MessageCarrier) -> Self {
        Self {
            header,
            carrier,
            last_frame: 0,
            continues: false,
        }
    }

    pub fn header(&self) -> &GopHeader {
        &self.header
    }

    /// The carrier of the next pictures (the direct pair came or went).
    pub fn set_carrier(&mut self, carrier: MessageCarrier) {
        self.carrier = carrier;
    }

    /// The group's first picture continues the chain (B-36).
    pub fn mark_continuation(&mut self) {
        self.continues = true;
    }

    /// Sends one picture: its message cut into packets with parity, all at
    /// once, none sent again, within the stale time or not at all; on the
    /// TCP link as one record, however long the line takes.
    pub async fn write_picture(
        &mut self,
        frame_id: u64,
        capture_ts_us: u64,
        keyframe: bool,
        records: CapturedRecords,
    ) -> io::Result<MessageSent> {
        let meta = PictureMeta {
            frame_id,
            capture_ts_us,
            keyframe,
            continuation: !keyframe && std::mem::take(&mut self.continues),
            prev_frame_id: self.last_frame,
        };
        // The next picture names this one also when this one fails: the
        // viewer then sees the gap.
        self.last_frame = frame_id;
        let header = self.header;
        let message = tokio::task::spawn_blocking(move || {
            let message = encode(&header, meta, records.as_records());
            drop(records);
            message
        })
        .await
        .map_err(io::Error::other)??;
        // „TCP“: the message whole as one record; a slower line delays it.
        if let MessageCarrier::Record(record) = &self.carrier {
            record.send(&message).await?;
            return Ok(MessageSent {
                bytes: message.len() as u64,
                packets: 1,
                carrier: PictureCarrier::Tcp,
            });
        }
        let message = Arc::new(message);
        let budget = Duration::from_micros(STALE_STREAM_US);
        tokio::time::timeout(budget, self.send_message(message, frame_id))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "picture packets stalled"))?
    }

    async fn send_message(&self, message: Arc<Vec<u8>>, frame_id: u64) -> io::Result<MessageSent> {
        let gop_id = self.header.gop_id;
        for generation in 0..MAX_GENERATIONS {
            let shard = shard_len(&self.carrier)
                .ok_or_else(|| invalid("the carrier's packets are too small"))?;
            let cut = message.clone();
            let packets = tokio::task::spawn_blocking(move || {
                packetize(&cut, shard, frame_id, gop_id, generation)
            })
            .await
            .map_err(io::Error::other)??;
            let mut sent = MessageSent {
                bytes: 0,
                packets: 0,
                carrier: self.carrier.carrier(),
            };
            let mut cut_again = false;
            for packet in packets {
                let bytes = packet.len() as u64;
                match self.send_packet(packet).await {
                    Ok(()) => {
                        sent.bytes += bytes;
                        sent.packets += 1;
                    }
                    Err(PacketError::TooLarge) => {
                        cut_again = true;
                        break;
                    }
                    Err(PacketError::Failed(error)) => return Err(error),
                }
            }
            if !cut_again {
                return Ok(sent);
            }
        }
        Err(invalid("the carrier's packet size kept shrinking"))
    }

    async fn send_packet(&self, packet: Vec<u8>) -> Result<(), PacketError> {
        match &self.carrier {
            MessageCarrier::Direct(direct) => {
                direct.send(&packet).await.map_err(PacketError::Failed)
            }
            MessageCarrier::Record(record) => {
                record.send(&packet).await.map_err(PacketError::Failed)
            }
            // Waits for room in the datagram buffer instead of dropping
            // older packets of the same picture.
            MessageCarrier::Quic(connection) => {
                match connection.send_datagram_wait(Bytes::from(packet)).await {
                    Ok(()) => Ok(()),
                    Err(SendDatagramError::TooLarge) => Err(PacketError::TooLarge),
                    Err(error) => Err(PacketError::Failed(io::Error::other(error))),
                }
            }
        }
    }
}
