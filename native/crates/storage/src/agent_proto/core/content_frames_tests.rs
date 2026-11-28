//! The content search frames (`content_search_v1`): every field survives the
//! wire, hits and skips are charged against the credit while progress and the
//! final totals are free, and the agent answers a `ContentSearch` with its
//! events and `ContentDone`.
use std::net::{TcpListener, TcpStream};

use crate::agent_proto::{
    credit_cost, read_frame, serve, server_version, write_frame, ContentEvent, ContentQuery,
    ContentSummary, Frame, LineHit, ServerFeatures, SkipReason, TextMode,
};

fn every_frame() -> Vec<Frame> {
    let query = ContentQuery {
        pattern: "a\\d+é".into(),
        mode: TextMode::Regex,
        match_case: true,
        whole_word: false,
        dot_matches_newline: true,
        filters: "*.rs !\\target !+\\.git".into(),
        folder_include: Some("^src".into()),
        folder_exclude: None,
        subfolders: false,
        hidden: true,
        context_lines: 3,
        max_hits: 0x0102_0304_0506,
        files_only: true,
        max_line_bytes: 77,
    };
    vec![
        Frame::ContentSearch {
            root: "/srv/data".into(),
            query: query.clone(),
        },
        Frame::ContentSearch {
            root: String::new(),
            query: ContentQuery::text("x"),
        },
        Frame::ContentEvent(ContentEvent::Hits {
            rel: "src/ä.rs".into(),
            size: 9,
            mtime_ms: -5,
            lines: vec![
                LineHit {
                    line: 1,
                    offset: 0,
                    text: "fn a1é()".into(),
                    ranges: vec![(3, 7), (0, 0)],
                    truncated: true,
                    context: false,
                },
                LineHit {
                    line: 2,
                    offset: 9,
                    text: String::new(),
                    ranges: Vec::new(),
                    truncated: false,
                    context: true,
                },
            ],
            last: true,
        }),
        Frame::ContentEvent(ContentEvent::Skipped {
            rel: "b.bin".into(),
            reason: SkipReason::Binary,
        }),
        Frame::ContentEvent(ContentEvent::Skipped {
            rel: "c".into(),
            reason: SkipReason::Unreadable("Zugriff verweigert".into()),
        }),
        Frame::ContentEvent(ContentEvent::Skipped {
            rel: "d".into(),
            reason: SkipReason::UnlistableFolder("os error 13".into()),
        }),
        Frame::ContentEvent(ContentEvent::Progress {
            files: 1,
            bytes: 2,
            hits: 3,
        }),
        Frame::ContentDone(ContentSummary {
            files_searched: 4,
            files_matched: 3,
            hits: 2,
            bytes: 1,
            skipped: 7,
            limited: true,
        }),
    ]
}

#[test]
fn content_search_task_frames_round_trip_and_charge_credit() {
    for frame in every_frame() {
        let body = frame.encode(42).expect("encodes");
        assert_eq!(body.len(), frame.wire_len().unwrap(), "{frame:?}");
        let (id, decoded) = Frame::decode(&body).expect("decodes");
        assert_eq!(id, 42);
        assert_eq!(decoded, frame);
        let mut wire = Vec::new();
        write_frame(&mut wire, 42, &frame).unwrap();
        let charged = matches!(
            frame,
            Frame::ContentEvent(ContentEvent::Hits { .. } | ContentEvent::Skipped { .. })
        );
        assert_eq!(credit_cost(&frame) > 0, charged, "{frame:?}");
        assert_eq!(
            crate::agent_proto::credit::encoded_cost(&wire),
            credit_cost(&frame),
            "{frame:?}"
        );
    }
    // A truncated body and unknown flag bits are refused.
    let body = every_frame()[0].encode(1).unwrap();
    assert!(Frame::decode(&body[..body.len() - 1]).is_err());
    let mut flags = body.clone();
    let flag_at = 8 + 1 + 4 + "/srv/data".len() + 4 + "a\\d+é".len() + 1;
    flags[flag_at] = 0x80;
    assert!(Frame::decode(&flags).is_err());

    let agent = ServerFeatures::parse(&server_version(true));
    assert!(agent.content_search);
    assert!(!ServerFeatures::parse("0.5.0+credit-v1").content_search);
}

#[test]
fn content_search_task_agent_answers_a_content_search() {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(dir.path().join("a.txt"), "one\nneedle two\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "nothing\n").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let read = socket.try_clone().unwrap();
        let _ = serve(read, socket);
    });
    let mut client = TcpStream::connect(address).unwrap();
    let root = dir.path().to_string_lossy().into_owned();
    let request = Frame::ContentSearch {
        root,
        query: ContentQuery::text("NEEDLE"),
    };
    write_frame(&mut client, 5, &request).unwrap();
    let mut hits = Vec::new();
    let summary = loop {
        let (id, frame) = read_frame(&mut client).unwrap().expect("a reply");
        assert_eq!(id, 5);
        match frame {
            Frame::ContentEvent(ContentEvent::Hits { rel, lines, .. }) => hits.push((rel, lines)),
            Frame::ContentEvent(_) => {}
            Frame::ContentDone(summary) => break summary,
            other => panic!("unexpected reply {other:?}"),
        }
    };
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, "a.txt");
    assert_eq!(hits[0].1[0].line, 2);
    assert_eq!(hits[0].1[0].ranges, [(0, 6)]);
    assert_eq!(summary.files_searched, 2);
    assert_eq!(summary.files_matched, 1);

    // An invalid pattern is answered with an error, not a broken connection.
    let mut invalid = ContentQuery::text("(");
    invalid.mode = TextMode::Regex;
    let request = Frame::ContentSearch {
        root: dir.path().to_string_lossy().into_owned(),
        query: invalid,
    };
    write_frame(&mut client, 6, &request).unwrap();
    let (id, frame) = read_frame(&mut client).unwrap().expect("a reply");
    assert_eq!(id, 6);
    assert!(matches!(frame, Frame::Err(text) if text.contains("Ungültiger")));
    drop(client);
    server.join().unwrap();
}
