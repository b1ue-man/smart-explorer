//! Transfer-engine task tests for the SFTP pipelining, the channel pool
//! decisions and the SSH window settings. No server: the READ source is a
//! script, so the ordering rules are checked exactly.
use super::channel_pool::{learned_limit, pick, Pick};
use super::pipeline::Pipeline;
use super::pipelined_read::{PipelinedRead, ReadSource, Reply};
use super::pool_writer::{check_length, SizedWriter};
use super::session::client_config;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Answers READs from `content`; `short` cuts the first answer at an offset.
struct Script {
    content: Vec<u8>,
    short: HashMap<u64, usize>,
    fail_at: Option<u64>,
    issued: Vec<(u64, u32)>,
    cancelled: usize,
    closed: usize,
}

impl Script {
    fn new(content: Vec<u8>) -> Self {
        Self {
            content,
            short: HashMap::new(),
            fail_at: None,
            issued: Vec::new(),
            cancelled: 0,
            closed: 0,
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
        if self.fail_at == Some(offset) {
            let error = io::Error::new(io::ErrorKind::ConnectionReset, "scripted failure");
            return (Err(error), Instant::now());
        }
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

    fn cancel(&mut self, _pending: (u64, u32)) {
        self.cancelled += 1;
    }

    fn close(&mut self) {
        self.closed += 1;
    }
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

fn reader(script: Script, start: u64, chunk: u32) -> PipelinedRead<Script> {
    PipelinedRead::new(script, start, chunk, Pipeline::new(u64::from(chunk), 64))
}

#[test]
fn transfer_engine_task_sftp_reader_short_tail_ends_without_gap_request() {
    let content = pattern(25);
    let mut read = reader(Script::new(content.clone()), 0, 10);
    let mut out = Vec::new();
    read.read_to_end(&mut out).unwrap();
    assert_eq!(out, content);
    let offsets: Vec<u64> = read
        .source()
        .issued
        .iter()
        .map(|(offset, _)| *offset)
        .collect();
    // The READ after the short tail (offset 30) reported the end, so the
    // missing range 25..30 was never asked for.
    assert!(!offsets.contains(&25), "issued: {offsets:?}");
    assert_eq!(&offsets[..3], &[0, 10, 20]);
    assert_eq!(read.source().closed, 1);
    // Delivered in order and nothing more after the end.
    assert_eq!(read.delivered(), 25);
    assert_eq!(read.read(&mut [0u8; 4]).unwrap(), 0);
}

#[test]
fn transfer_engine_task_sftp_reader_refills_a_gap_when_data_follows() {
    let content = pattern(47);
    let mut script = Script::new(content.clone());
    // A short answer in the middle of the file (a device or a growing file):
    // the data after it proves the range 3..10 must be read too.
    script.short.insert(0, 3);
    script.short.insert(20, 4);
    let mut read = reader(script, 0, 10);
    let mut out = Vec::new();
    read.read_to_end(&mut out).unwrap();
    assert_eq!(out, content);
    let issued = &read.source().issued;
    assert!(issued.contains(&(3, 7)), "issued: {issued:?}");
    assert!(issued.contains(&(24, 6)), "issued: {issued:?}");
}

#[test]
fn transfer_engine_task_sftp_reader_ends_exactly_at_a_chunk_boundary() {
    let content = pattern(40);
    let mut read = reader(Script::new(content.clone()), 0, 10);
    let mut out = Vec::new();
    read.read_to_end(&mut out).unwrap();
    assert_eq!(out, content);
    assert_eq!(read.source().closed, 1);
}

#[test]
fn transfer_engine_task_sftp_reader_resumes_at_an_offset() {
    let content = pattern(33);
    let mut read = reader(Script::new(content.clone()), 15, 10);
    let mut out = Vec::new();
    read.read_to_end(&mut out).unwrap();
    assert_eq!(out, content[15..]);
    assert_eq!(read.source().issued[0].0, 15);
}

#[test]
fn transfer_engine_task_sftp_reader_failure_stays_a_failure() {
    let mut script = Script::new(pattern(60));
    script.fail_at = Some(20);
    let mut read = reader(script, 0, 10);
    let mut out = Vec::new();
    let error = read.read_to_end(&mut out).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionReset);
    assert_eq!(out, pattern(20));
    // A later read must not look like the end of a shorter file.
    let again = read.read(&mut [0u8; 8]).unwrap_err();
    assert_eq!(again.kind(), io::ErrorKind::ConnectionReset);
    assert_eq!(read.source().closed, 1);
}

/// READs answered by tokio tasks, later ones sooner than earlier ones.
struct Delayed {
    rt: Arc<tokio::runtime::Runtime>,
    content: Arc<Vec<u8>>,
    issued: u64,
    arrivals: Arc<Mutex<Vec<u64>>>,
}

impl ReadSource for Delayed {
    type Pending = tokio::task::JoinHandle<(io::Result<Reply>, Instant)>;

    fn issue(&mut self, offset: u64, len: u32) -> Self::Pending {
        let content = self.content.clone();
        let arrivals = self.arrivals.clone();
        let delay = Duration::from_millis(40u64.saturating_sub(self.issued * 10));
        self.issued += 1;
        self.rt.spawn(async move {
            tokio::time::sleep(delay).await;
            arrivals.lock().unwrap().push(offset);
            let start = offset as usize;
            let reply = if start >= content.len() {
                Reply::Eof
            } else {
                let end = (start + len as usize).min(content.len());
                Reply::Data(content[start..end].to_vec())
            };
            (Ok(reply), Instant::now())
        })
    }

    fn wait(&mut self, pending: Self::Pending) -> (io::Result<Reply>, Instant) {
        match self.rt.block_on(pending) {
            Ok(answer) => answer,
            Err(error) => (Err(io::Error::other(error.to_string())), Instant::now()),
        }
    }

    fn cancel(&mut self, pending: Self::Pending) {
        pending.abort();
    }

    fn close(&mut self) {}
}

#[test]
fn transfer_engine_task_sftp_reader_orders_answers_that_arrive_out_of_order() {
    let rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_time()
            .build()
            .unwrap(),
    );
    let content = pattern(95);
    let arrivals = Arc::new(Mutex::new(Vec::new()));
    let source = Delayed {
        rt,
        content: Arc::new(content.clone()),
        issued: 0,
        arrivals: arrivals.clone(),
    };
    let mut read = PipelinedRead::new(source, 0, 10, Pipeline::new(10, 64));
    let mut out = Vec::new();
    read.read_to_end(&mut out).unwrap();
    assert_eq!(out, content);
    let arrivals = arrivals.lock().unwrap().clone();
    let first = arrivals.iter().position(|offset| *offset == 0).unwrap();
    let second = arrivals.iter().position(|offset| *offset == 10).unwrap();
    assert!(
        second < first,
        "the second READ answered first: {arrivals:?}"
    );
}

#[test]
fn transfer_engine_task_sftp_pipeline_grows_until_answers_queue() {
    let mut pipeline = Pipeline::new(4096, 8);
    assert_eq!(pipeline.depth(), 2);
    pipeline.answered(Duration::from_millis(10), true);
    assert_eq!(pipeline.depth(), 3);
    // A short tail says nothing about the link.
    pipeline.answered(Duration::from_millis(10), false);
    assert_eq!(pipeline.depth(), 3);
    // Within twice the fastest answer (plus the timer resolution): no queue.
    pipeline.answered(Duration::from_millis(21), true);
    assert_eq!(pipeline.depth(), 4);
    // Later than that: the pipe is full, the depth stays.
    pipeline.answered(Duration::from_millis(30), true);
    assert_eq!(pipeline.depth(), 4);
    for _ in 0..20 {
        pipeline.answered(Duration::from_millis(10), true);
    }
    assert_eq!(pipeline.depth(), 8, "never beyond the channel window");
}

#[test]
fn transfer_engine_task_sftp_pool_shares_channels_once_the_server_refuses() {
    // No channel yet: open one; an idle one is reused before opening more.
    assert_eq!(pick(&[], 0, None), Pick::Open);
    assert_eq!(pick(&[1, 0, 2], 0, None), Pick::Use(1));
    // Every channel busy and no limit known: grow.
    assert_eq!(pick(&[1, 1], 0, None), Pick::Open);
    // OpenSSH MaxSessions 10: the main channel and nine pool channels were
    // accepted, the tenth pool channel refused. Two stay free for the agent's
    // exec channel and posix-rename.
    let limit = learned_limit(9);
    assert_eq!(limit, 7);
    assert_eq!(pick(&[1, 3, 1, 1, 2, 1, 1], 0, Some(limit)), Pick::Use(0));
    // A channel being opened counts against the limit.
    assert_eq!(pick(&[1, 1, 1, 1, 1, 1], 1, Some(limit)), Pick::Use(0));
    // Servers allowing at most two sessions keep the transfer on the main one.
    assert_eq!(learned_limit(1), 0);
    assert_eq!(pick(&[], 0, Some(0)), Pick::MainSession);
}

#[test]
fn transfer_engine_task_sftp_client_window_covers_bandwidth_delay() {
    let config = client_config();
    assert_eq!(config.window_size, 16 * 1024 * 1024);
    assert_eq!(config.maximum_packet_size, 32 * 1024);
    // One full window of full-size packets per channel buffer.
    assert_eq!(
        config.channel_buffer_size,
        (config.window_size / config.maximum_packet_size) as usize
    );
    // Keepalive unchanged.
    assert_eq!(config.keepalive_interval, Some(Duration::from_secs(15)));
    assert_eq!(config.keepalive_max, 3);
}

#[test]
fn transfer_engine_task_sftp_sized_stage_needs_the_exact_length() {
    assert!(check_length(Some(5), 5).is_ok());
    assert!(check_length(None, 9).is_ok());
    assert_eq!(
        check_length(Some(5), 4).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );

    let mut short = SizedWriter::new(Vec::new(), 6);
    short.write_all(b"abc").unwrap();
    assert_eq!(
        short.flush().unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );

    let mut long = SizedWriter::new(Vec::new(), 2);
    assert_eq!(
        long.write_all(b"abc").unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );

    let mut exact = SizedWriter::new(Vec::new(), 3);
    exact.write_all(b"abc").unwrap();
    exact.flush().unwrap();
}
