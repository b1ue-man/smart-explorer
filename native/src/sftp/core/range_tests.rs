//! Ranges over SFTP (E13, U48-3): the pipelined reader stops at the end of its
//! range and the pool gives distinct ranges distinct channels. The stage
//! several writers fill is in segmented_stage_tests.rs. No server: the READ
//! source is a script.
use super::channel_pool::{pick, Pick};
use super::pipelined_read::{PipelinedRead, ReadSource, Reply};
use crate::transfer::read_pipeline::Pipeline;
use std::collections::HashMap;
use std::io::{self, Read};
use std::time::Instant;

/// Answers READs from `content`; `short` cuts the first answer at an offset.
struct Script {
    content: Vec<u8>,
    short: HashMap<u64, usize>,
    issued: Vec<(u64, u32)>,
}

impl Script {
    fn new(len: usize) -> Self {
        Self {
            content: (0..len).map(|index| (index % 251) as u8).collect(),
            short: HashMap::new(),
            issued: Vec::new(),
        }
    }
}

impl ReadSource for Script {
    type Pending = (u64, u32);

    fn issue(&mut self, offset: u64, len: u32) -> (u64, u32) {
        self.issued.push((offset, len));
        (offset, len)
    }

    fn wait(&mut self, (offset, len): (u64, u32)) -> (io::Result<Reply>, Instant) {
        let start = offset as usize;
        if start >= self.content.len() {
            return (Ok(Reply::Eof), Instant::now());
        }
        let mut end = (start + len as usize).min(self.content.len());
        if let Some(cut) = self.short.remove(&offset) {
            end = end.min(start + cut);
        }
        (
            Ok(Reply::Data(self.content[start..end].to_vec())),
            Instant::now(),
        )
    }

    fn cancel(&mut self, _pending: (u64, u32)) {}

    fn close(&mut self) {}
}

fn range(script: Script, start: u64, end: u64, chunk: u32) -> PipelinedRead<Script> {
    PipelinedRead::new(script, start, chunk, Pipeline::new(u64::from(chunk), 64)).with_end(end)
}

fn drain(reader: &mut PipelinedRead<Script>) -> Vec<u8> {
    let mut out = Vec::new();
    // Small reads: the caller's buffer size must not matter.
    let mut buffer = [0u8; 37];
    loop {
        let read = reader.read(&mut buffer).unwrap();
        if read == 0 {
            return out;
        }
        out.extend_from_slice(&buffer[..read]);
    }
}

#[test]
fn speed_task_sftp_range_reader_issues_no_read_beyond_end() {
    let mut reader = range(Script::new(1000), 250, 520, 100);
    let content = reader.source().content.clone();
    let bytes = drain(&mut reader);
    assert_eq!(bytes, content[250..520], "exactly the range");
    let issued = &reader.source().issued;
    assert!(!issued.is_empty());
    assert!(
        issued
            .iter()
            .all(|(offset, len)| offset + u64::from(*len) <= 520),
        "no READ reaches past the end of the range: {issued:?}"
    );
    assert_eq!(issued.first().map(|read| read.0), Some(250));
}

#[test]
fn speed_task_sftp_range_reader_last_read_is_shortened_to_the_range() {
    let mut reader = range(Script::new(1000), 0, 250, 100);
    let bytes = drain(&mut reader);
    assert_eq!(bytes.len(), 250);
    assert_eq!(
        reader.source().issued,
        vec![(0, 100), (100, 100), (200, 50)],
        "the last READ is only as long as what is left"
    );
    // A range shorter than one chunk is one READ of its own length.
    let mut small = range(Script::new(1000), 900, 930, 100);
    assert_eq!(drain(&mut small).len(), 30);
    assert_eq!(small.source().issued, vec![(900, 30)]);
    // An empty range asks for nothing.
    let mut empty = range(Script::new(1000), 400, 400, 100);
    assert!(drain(&mut empty).is_empty());
    assert!(empty.source().issued.is_empty());
}

#[test]
fn speed_task_sftp_range_reader_short_answer_inside_range_refills() {
    // A short answer in the middle with data behind it: the missing bytes are
    // read first, nothing is skipped.
    let mut script = Script::new(1000);
    script.short.insert(100, 40);
    let mut reader = range(script, 0, 250, 100);
    let content = reader.source().content.clone();
    assert_eq!(drain(&mut reader), content[..250]);
    assert!(
        reader.source().issued.contains(&(140, 60)),
        "the gap is asked for: {:?}",
        reader.source().issued
    );
    // The short answer is the last READ of the range: nothing lies behind it
    // in flight, the gap is asked for itself.
    let mut script = Script::new(1000);
    script.short.insert(200, 20);
    let mut reader = range(script, 0, 250, 100);
    let content = reader.source().content.clone();
    assert_eq!(drain(&mut reader), content[..250]);
    assert!(reader.source().issued.contains(&(220, 30)));
    // The file ends inside the range: what exists is delivered, then the end.
    let mut reader = range(Script::new(130), 0, 1000, 100);
    let content = reader.source().content.clone();
    assert_eq!(drain(&mut reader), content);
}

#[test]
fn speed_task_sftp_range_reader_resumes_at_offset_inside_file() {
    let mut reader = range(Script::new(100_000), 40_000, 70_000, 4096);
    let content = reader.source().content.clone();
    assert_eq!(drain(&mut reader), content[40_000..70_000]);
    let issued = &reader.source().issued;
    assert_eq!(issued.first().map(|read| read.0), Some(40_000));
    assert_eq!(reader.delivered(), 30_000);
    // Ranges next to each other, read one after the other, make the file.
    let mut joined = Vec::new();
    for (start, end) in [(0u64, 33_000u64), (33_000, 66_000), (66_000, 100_000)] {
        let mut part = range(Script::new(100_000), start, end, 4096);
        joined.extend(drain(&mut part));
    }
    assert_eq!(joined, content);
}

#[test]
fn speed_task_sftp_range_pool_picks_distinct_channels_for_distinct_ranges() {
    // Each range takes a channel: while every channel is busy and the server
    // has not refused one, a new channel opens (own SSH window); at the
    // learned limit the least busy channel is shared.
    let limit = Some(3);
    let mut loads: Vec<usize> = Vec::new();
    let mut chosen = Vec::new();
    for _ in 0..5 {
        match pick(&loads, 0, limit) {
            Pick::Open => {
                loads.push(1);
                chosen.push(loads.len() - 1);
            }
            Pick::Use(index) => {
                loads[index] += 1;
                chosen.push(index);
            }
            Pick::MainSession => panic!("a pool with room never sends ranges to the main session"),
        }
    }
    assert_eq!(
        &chosen[..3],
        &[0, 1, 2],
        "distinct channels for distinct ranges"
    );
    assert_eq!(loads.len(), 3, "never more than the learned limit");
    assert_eq!(
        loads.iter().sum::<usize>(),
        5,
        "the rest shares the least busy"
    );
    // No limit learned yet: every busy pool grows.
    assert_eq!(pick(&[1, 1], 0, None), Pick::Open);
    // An idle channel is used before a new one opens.
    assert_eq!(pick(&[2, 0], 0, None), Pick::Use(1));
}
