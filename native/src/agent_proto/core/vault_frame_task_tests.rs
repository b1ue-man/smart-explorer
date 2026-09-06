//! M4 acceptance, selected only by the shared mount_vault_task entrypoint.
use std::io::{self, Cursor, ErrorKind, Read, Write};

use super::super::types::{Frame, SearchSpec, WireMeta, WireNode, CHUNK};
use super::{read_frame, write_frame, MAX_FRAME, MIN_WIRE_META_BYTES};

const REQUEST: u64 = 0x1020_3040_5060_7080;

fn join(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

fn body(tag: u8, fields: &[u8]) -> Vec<u8> {
    join(&[&REQUEST.to_le_bytes(), &[tag], fields])
}

fn transport(body: &[u8]) -> Vec<u8> {
    join(&[&u32::try_from(body.len()).unwrap().to_le_bytes(), body])
}

fn assert_error<T>(result: io::Result<T>, expected: ErrorKind) {
    // Never format an unexpectedly successful multi-megabyte payload.
    assert_eq!(result.err().expect("operation must fail").kind(), expected);
}

#[test]
fn mount_vault_task_all_frame_variants_keep_exact_protocol_bytes() {
    // Independent field literals: UTF-8 byte lengths, little-endian integers,
    // negative i64, both boolean values, and present/absent optional strings.
    let path = "é";
    let p = [2, 0, 0, 0, 0xc3, 0xa9];
    let number = 0x0102_0304_0506_0708;
    let n = [8, 7, 6, 5, 4, 3, 2, 1];
    let negative = [254, 255, 255, 255, 255, 255, 255, 255];
    let meta = WireMeta {
        name: path.into(), is_dir: true, is_symlink: false, size: number,
        mtime_ms: -2, content_md5: Some(path.into()),
    };
    let meta_bytes = join(&[&p, &[1, 0], &n, &negative, &[1], &p]);
    let tree = WireNode {
        name: path.into(), size: number, is_dir: true,
        children: vec![WireNode {
            name: path.into(), size: 0, is_dir: false, children: Vec::new(),
        }],
    };
    let tree_bytes = join(&[&p, &n, &[1], &[1, 0, 0, 0], &p, &[0; 13]]);
    let entry_bytes = join(&[&p, &[0], &n, &negative]);
    let cases = vec![
        (1, Frame::Hello { proto: 0x0102_0304 }, vec![4, 3, 2, 1]),
        (2, Frame::HelloOk { proto: 0x0102_0304, version: path.into() },
            join(&[&[4, 3, 2, 1], &p])),
        (3, Frame::ListDir(path.into()), p.to_vec()),
        (4, Frame::Dir(vec![meta.clone(), WireMeta::default()]),
            join(&[&[2, 0, 0, 0], &meta_bytes, &[0; 23]])),
        (5, Frame::Stat(path.into()), p.to_vec()),
        (6, Frame::Meta(meta), meta_bytes),
        (7, Frame::WalkTree(path.into()), p.to_vec()),
        (8, Frame::Tree(tree), tree_bytes),
        (9, Frame::Read { path: path.into(), offset: number, len: 0 },
            join(&[&p, &n, &[0; 8]])),
        (10, Frame::Write(path.into()), p.to_vec()),
        (11, Frame::Data(vec![0, 128, 255]), vec![3, 0, 0, 0, 0, 128, 255]),
        (12, Frame::Copy { src: path.into(), dst: "".into() }, join(&[&p, &[0; 4]])),
        (13, Frame::Rename { src: path.into(), dst: "".into() }, join(&[&p, &[0; 4]])),
        (14, Frame::Remove { path: path.into(), recursive: true }, join(&[&p, &[1]])),
        (15, Frame::Mkdir(path.into()), p.to_vec()),
        (16, Frame::GetTree(path.into()), p.to_vec()),
        (17, Frame::PutTree(path.into()), p.to_vec()),
        (18, Frame::TreeEntry {
            rel: path.into(), is_dir: false, size: number, mtime_ms: -2,
        }, entry_bytes.clone()),
        (19, Frame::Search { root: path.into(), spec: SearchSpec {
            query: path.into(), glob: true, min_size: number, max_size: 0,
            max_results: 1, want_dirs: false,
        } }, join(&[&p, &p, &[1], &n, &[0; 8], &[1, 0, 0, 0, 0, 0, 0, 0], &[0]])),
        (20, Frame::Match {
            rel: path.into(), is_dir: false, size: number, mtime_ms: -2,
        }, entry_bytes.clone()),
        (21, Frame::WalkHashed { root: path.into(), want_hash: true }, join(&[&p, &[1]])),
        (22, Frame::HashEntry {
            rel: path.into(), is_dir: false, size: number, mtime_ms: -2, md5: None,
        }, join(&[&entry_bytes, &[0]])),
        (23, Frame::Progress { done: number, total: 0 }, join(&[&n, &[0; 8]])),
        (24, Frame::Ok, vec![]),
        (25, Frame::End, vec![]),
        (26, Frame::Err(path.into()), p.to_vec()),
        (27, Frame::Cancel, vec![]),
        (28, Frame::TryExists(path.into()), p.to_vec()),
        (29, Frame::Exists(false), vec![0]),
        (30, Frame::RenameNoReplace { src: path.into(), dst: "".into() },
            join(&[&p, &[0; 4]])),
        (31, Frame::Promote { staged: path.into(), destination: "".into() },
            join(&[&p, &[0; 4]])),
        (32, Frame::PromoteNoReplace { staged: path.into(), destination: "".into() },
            join(&[&p, &[0; 4]])),
        (33, Frame::WriteNew(path.into()), p.to_vec()),
    ];
    assert_eq!(cases.len(), 33);
    for (index, (tag, frame, fields)) in cases.into_iter().enumerate() {
        assert_eq!(usize::from(tag), index + 1, "every opcode must be represented");
        let expected = body(tag, &fields);
        assert_eq!(frame.encode(REQUEST).unwrap(), expected, "opcode {tag}");
        assert_eq!(Frame::decode(&expected).unwrap(), (REQUEST, frame.clone()));
        let mut writer = ProbeWriter::default();
        write_frame(&mut writer, REQUEST, &frame).unwrap();
        assert_eq!(writer.bytes, transport(&expected), "framed opcode {tag}");
        assert_eq!((writer.calls, writer.flushes), (1, 1), "opcode {tag}");
        assert_eq!(
            read_frame(&mut Cursor::new(&writer.bytes)).unwrap(),
            Some((REQUEST, frame))
        );
    }
}

#[test]
fn mount_vault_task_directory_above_50000_real_entries_roundtrips() {
    let entries: Vec<_> = (0..50_001)
        .map(|index| WireMeta {
            name: format!("note-{index:05}-é.md"),
            is_dir: index % 17 == 0,
            is_symlink: index % 19 == 0,
            size: index as u64 * 31,
            mtime_ms: index as i64 - 25_000,
            content_md5: (index % 23 == 0).then(|| "0123456789abcdef0123456789abcdef".into()),
        })
        .collect();
    let frame = Frame::Dir(entries);
    let encoded = frame.encode(REQUEST).unwrap();
    assert_eq!(u32::from_le_bytes(encoded[9..13].try_into().unwrap()), 50_001);
    let (request, decoded) = Frame::decode(&encoded).unwrap();
    assert_eq!(request, REQUEST);
    assert!(decoded == frame, "the complete named directory must survive encoding");
}

#[test]
fn mount_vault_task_directory_minimum_record_guards_reject_malformed_frames() {
    assert_eq!(MIN_WIRE_META_BYTES, 23);
    let valid = body(4, &join(&[&[1, 0, 0, 0], &[0; 23]]));
    assert_eq!(
        Frame::Dir(vec![WireMeta::default()]).encode(REQUEST).unwrap(), valid
    );
    assert_eq!(
        Frame::decode(&valid).unwrap().1, Frame::Dir(vec![WireMeta::default()])
    );
    for end in 0..valid.len() {
        assert_error(Frame::decode(&valid[..end]), ErrorKind::InvalidData);
    }
    for count in [2_u32, 50_001, u32::MAX] {
        let mut malformed = valid.clone();
        malformed[9..13].copy_from_slice(&count.to_le_bytes());
        let error = Frame::decode(&malformed).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert_eq!(
            error.to_string(), "directory entry count exceeds the remaining frame bytes"
        );
        assert_error(Frame::decode(&malformed[..13]), ErrorKind::InvalidData);
    }
    let mut huge_name = valid.clone();
    huge_name[13..17].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_error(Frame::decode(&huge_name), ErrorKind::InvalidData);
    let mut invalid_utf8 = valid.clone();
    invalid_utf8[13] = 1;
    invalid_utf8.insert(17, 0xff);
    assert_error(Frame::decode(&invalid_utf8), ErrorKind::InvalidData);
    let mut truncated_md5 = valid.clone();
    truncated_md5[35] = 1;
    assert_error(Frame::decode(&truncated_md5), ErrorKind::InvalidData);
    truncated_md5.extend_from_slice(&u32::MAX.to_le_bytes());
    assert_error(Frame::decode(&truncated_md5), ErrorKind::InvalidData);
    for mut trailing in [valid, body(4, &[0; 4]), body(24, &[])] {
        trailing.push(0);
        assert_error(Frame::decode(&trailing), ErrorKind::InvalidData);
    }
    assert_error(Frame::decode(&body(255, &[])), ErrorKind::InvalidData);
    let invalid_data = body(11, &u32::try_from(CHUNK + 1).unwrap().to_le_bytes());
    assert_error(Frame::decode(&invalid_data), ErrorKind::InvalidData);
    assert_error(Frame::Data(vec![0; CHUNK + 1]).encode(REQUEST), ErrorKind::InvalidData);
}

#[test]
fn mount_vault_task_utf8_and_optional_md5_use_encoded_byte_lengths() {
    for (md5, expected_len) in [(None, 45), (Some(""), 49), (Some("哈"), 52)] {
        let frame = Frame::Dir(vec![WireMeta {
            name: "é🦀.md".into(), content_md5: md5.map(str::to_owned),
            ..WireMeta::default()
        }]);
        let encoded = frame.encode(REQUEST).unwrap();
        assert_eq!(encoded.len(), expected_len);
        assert_eq!(&encoded[13..17], &[9, 0, 0, 0]);
        assert_eq!(Frame::decode(&encoded).unwrap(), (REQUEST, frame));
    }
    for md5 in [Some(""), Some("哈")] {
        let frame = Frame::HashEntry {
            rel: "é".into(), is_dir: true, size: 0, mtime_ms: -1,
            md5: md5.map(str::to_owned),
        };
        let encoded = frame.encode(REQUEST).unwrap();
        assert_eq!(encoded.len(), 37 + md5.unwrap().len());
        assert_eq!(Frame::decode(&encoded).unwrap(), (REQUEST, frame));
    }
}

#[test]
fn mount_vault_task_exact_64_mib_body_and_one_byte_over() {
    assert_eq!(MAX_FRAME, 64 * 1024 * 1024);
    // Only the input and one encoded/decoded copy coexist. Do not clone the
    // giant frame, construct a transport copy, or print its contents on failure.
    let frame = Frame::Dir(vec![WireMeta {
        name: "x".repeat(MAX_FRAME - 13 - 23), ..WireMeta::default()
    }]);
    let encoded = frame.encode(REQUEST).unwrap();
    assert_eq!(encoded.len(), MAX_FRAME);
    drop(frame);
    let (request, mut decoded) = Frame::decode(&encoded).unwrap();
    assert_eq!(request, REQUEST);
    drop(encoded);
    let Frame::Dir(entries) = &mut decoded else {
        panic!("expected directory");
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name.len(), MAX_FRAME - 36);
    assert!(entries[0].name.bytes().all(|byte| byte == b'x'));
    entries[0].name.reserve_exact(1);
    entries[0].name.push('x');
    assert_error(decoded.encode(REQUEST), ErrorKind::InvalidData);
    let mut writer = ProbeWriter::default();
    assert_error(write_frame(&mut writer, REQUEST, &decoded), ErrorKind::InvalidData);
    assert_eq!((writer.calls, writer.flushes), (0, 0));
    drop(decoded);
    // The receive path must reject the oversized length before reading a body.
    let prefix = u32::try_from(MAX_FRAME + 1).unwrap().to_le_bytes();
    let mut reader = Cursor::new(prefix);
    assert_error(read_frame(&mut reader), ErrorKind::InvalidData);
    assert_eq!(reader.position(), 4);
}

#[derive(Default)]
struct ProbeWriter {
    bytes: Vec<u8>,
    calls: usize,
    flushes: usize,
    chunk: Option<usize>,
    interrupt_on: Option<usize>,
    write_error: Option<ErrorKind>,
    flush_error: Option<ErrorKind>,
}

impl Write for ProbeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.interrupt_on == Some(self.calls) {
            return Err(ErrorKind::Interrupted.into());
        }
        if let Some(kind) = self.write_error {
            return Err(kind.into());
        }
        let accepted = self.chunk.unwrap_or(bytes.len()).min(bytes.len());
        self.bytes.extend_from_slice(&bytes[..accepted]);
        Ok(accepted)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        self.flush_error.map_or(Ok(()), |kind| Err(kind.into()))
    }
}

#[test]
fn mount_vault_task_framed_writer_short_interrupt_and_error_semantics() {
    let frame = Frame::Data(vec![0, 128, 255]);
    let expected = transport(&body(11, &[3, 0, 0, 0, 0, 128, 255]));
    for chunk in [1, 3, expected.len()] {
        for interrupt_on in [None, Some(1), Some(2)] {
            let mut writer = ProbeWriter {
                chunk: Some(chunk), interrupt_on, ..Default::default()
            };
            write_frame(&mut writer, REQUEST, &frame).unwrap();
            assert_eq!(writer.bytes, expected);
            assert_eq!(writer.flushes, 1);
        }
    }
    let mut zero = ProbeWriter { chunk: Some(0), ..Default::default() };
    assert_error(write_frame(&mut zero, REQUEST, &frame), ErrorKind::WriteZero);
    assert_eq!((zero.calls, zero.flushes), (1, 0));
    let mut failed = ProbeWriter {
        write_error: Some(ErrorKind::BrokenPipe), ..Default::default()
    };
    assert_error(write_frame(&mut failed, REQUEST, &frame), ErrorKind::BrokenPipe);
    assert_eq!((failed.calls, failed.flushes), (1, 0));
    let mut flush = ProbeWriter {
        flush_error: Some(ErrorKind::PermissionDenied), ..Default::default()
    };
    assert_error(write_frame(&mut flush, REQUEST, &frame), ErrorKind::PermissionDenied);
    assert_eq!(flush.bytes, expected);
    assert_eq!((flush.calls, flush.flushes), (1, 1));
}

struct InterruptedReader<'a> {
    bytes: &'a [u8],
    calls: usize,
}

impl Read for InterruptedReader<'_> {
    fn read(&mut self, destination: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        // Before the header, midway through it, then inside the body.
        if [1, 4, 8].contains(&self.calls) {
            return Err(ErrorKind::Interrupted.into());
        }
        let length = self.bytes.len().min(destination.len()).min(1);
        destination[..length].copy_from_slice(&self.bytes[..length]);
        self.bytes = &self.bytes[length..];
        Ok(length)
    }
}

#[test]
fn mount_vault_task_framed_reader_interrupt_eof_and_truncation_semantics() {
    let expected = transport(&body(24, &[]));
    let mut interrupted = InterruptedReader { bytes: &expected, calls: 0 };
    assert_eq!(read_frame(&mut interrupted).unwrap(), Some((REQUEST, Frame::Ok)));
    assert_eq!(read_frame(&mut interrupted).unwrap(), None);
    assert_eq!(read_frame(&mut Cursor::new(&[] as &[u8])).unwrap(), None);
    for length in 1..4 {
        let error = read_frame(&mut Cursor::new(&expected[..length])).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "eof inside length");
    }
    for length in 4..expected.len() {
        assert_error(
            read_frame(&mut Cursor::new(&expected[..length])), ErrorKind::UnexpectedEof
        );
    }
    for length in [0_u32, 8] {
        let malformed = join(&[&length.to_le_bytes(), &[0; 8][..length as usize]]);
        assert_error(read_frame(&mut Cursor::new(malformed)), ErrorKind::InvalidData);
    }
}
