use super::*;
use crate::share::wire::{FsHostFeatures, FsRequest, FsResponse};

fn full_query() -> ContentQuery {
    let mut query = ContentQuery::text("fn\\s+main");
    query.mode = TextMode::Regex;
    query.match_case = true;
    query.whole_word = true;
    query.dot_matches_newline = true;
    query.filters = "*.rs *.toml !\\target".into();
    query.folder_include = Some("^src(/|$)".into());
    query.folder_exclude = Some("^src/gen$".into());
    query.subfolders = false;
    query.hidden = true;
    query.context_lines = 2;
    query.max_hits = 50;
    query.files_only = true;
    query.max_line_bytes = 4096;
    query
}

#[test]
fn content_search_task_request_round_trips_as_a_read() {
    let request = FsRequest::ContentSearch(FsContentSearch {
        path: "/Docs".into(),
        query: FsContentQuery::from(&ContentQuery::text("needle")),
    });
    let encoded = serde_json::to_string(&request).unwrap();
    assert_eq!(
        encoded,
        r#"{"op":"content_search","path":"/Docs","query":{"pattern":"needle","mode":"normal","subfolders":true}}"#
    );
    let FsRequest::ContentSearch(decoded) = serde_json::from_str::<FsRequest>(&encoded).unwrap()
    else {
        panic!("content search expected: {encoded}");
    };
    assert_eq!(decoded.path, "/Docs");
    assert_eq!(
        decoded.query.to_query().unwrap(),
        ContentQuery::text("needle")
    );
    // A read: served on read-only exports, never batched, no transfer v1.
    assert!(!request.mutates_filesystem());
    assert!(!request.is_batch());
    assert!(!request.is_transfer_v1());

    // Every field survives the wire.
    let query = full_query();
    let wire = serde_json::to_string(&FsContentQuery::from(&query)).unwrap();
    let back: FsContentQuery = serde_json::from_str(&wire).unwrap();
    assert_eq!(back.to_query().unwrap(), query);

    // Absent fields read as the defaults of `ContentQuery::text`.
    let sparse: FsContentQuery = serde_json::from_str(r#"{"pattern":"x"}"#).unwrap();
    assert_eq!(sparse.to_query().unwrap(), ContentQuery::text("x"));

    // A mode of a later client is refused, never searched as something else.
    let later: FsContentQuery = serde_json::from_str(r#"{"pattern":"x","mode":"glob"}"#).unwrap();
    assert_eq!(later.mode, FsTextMode::Unknown);
    assert_eq!(
        later.to_query().unwrap_err().kind(),
        io::ErrorKind::Unsupported
    );
    let empty: FsContentQuery = serde_json::from_str(r#"{"pattern":""}"#).unwrap();
    assert_eq!(
        empty.to_query().unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );

    // The host cuts line text to what one portion holds.
    let mut long = ContentQuery::text("x");
    long.max_line_bytes = 1_000_000;
    let cut = FsContentQuery::from(&long).to_query().unwrap();
    assert_eq!(cut.max_line_bytes, CONTENT_LINE_BYTES_MAX);
}

#[test]
fn content_search_task_messages_round_trip() {
    let hits = ContentEvent::Hits {
        rel: "src/main.rs".into(),
        size: 120,
        mtime_ms: 1_700_000_000_000,
        lines: vec![
            LineHit {
                line: 3,
                offset: 40,
                text: "// before".into(),
                ranges: Vec::new(),
                truncated: false,
                context: true,
            },
            LineHit {
                line: 4,
                offset: 50,
                text: "fn main() {}".into(),
                ranges: vec![(0, 7)],
                truncated: true,
                context: false,
            },
        ],
        last: true,
    };
    let skips = [
        ContentEvent::Skipped {
            rel: "bin/tool".into(),
            reason: SkipReason::Binary,
        },
        ContentEvent::Skipped {
            rel: "locked.txt".into(),
            reason: SkipReason::Unreadable("Zugriff verweigert".into()),
        },
        ContentEvent::Skipped {
            rel: "secret".into(),
            reason: SkipReason::UnlistableFolder("Zugriff verweigert".into()),
        },
    ];
    let events: Vec<FsContentEvent> = std::iter::once(hits.clone())
        .chain(skips.iter().cloned())
        .map(|event| FsContentEvent::from_event(event).unwrap())
        .collect();
    let summary = FsContentSummary {
        files_searched: 9,
        files_matched: 1,
        hits: 1,
        bytes: 4096,
        skipped: 3,
        limited: true,
    };
    let messages = [
        FsContentMessage::Progress {
            files: 4,
            bytes: 900,
            hits: 1,
        },
        FsContentMessage::Events {
            events: events.clone(),
        },
        FsContentMessage::Done { summary },
    ];
    for message in messages {
        let encoded = serde_json::to_string(&FsResponse::ContentSearch {
            message: message.clone(),
        })
        .unwrap();
        let FsResponse::ContentSearch { message: decoded } =
            serde_json::from_str::<FsResponse>(&encoded).unwrap()
        else {
            panic!("content search response expected: {encoded}");
        };
        assert_eq!(decoded, message);
    }
    // Back to the search core's events, unchanged.
    let back: Vec<ContentEvent> = events.into_iter().map(Into::into).collect();
    let expected: Vec<ContentEvent> = std::iter::once(hits).chain(skips).collect();
    assert_eq!(back, expected);
    assert_eq!(
        ContentSummary::from(summary),
        ContentSummary {
            files_searched: 9,
            files_matched: 1,
            hits: 1,
            bytes: 4096,
            skipped: 3,
            limited: true,
        }
    );
    // Totals travel as the stream's own progress, not as events.
    let progress = ContentEvent::Progress {
        files: 1,
        bytes: 2,
        hits: 3,
    };
    assert_eq!(FsContentEvent::from_event(progress), None);
    // A skip reason of a later host is still a skip.
    let later: FsContentEvent =
        serde_json::from_str(r#"{"e":"skipped","rel":"a","reason":"too_large"}"#).unwrap();
    assert!(matches!(
        later,
        FsContentEvent::Skipped {
            reason: FsSkipKind::Unreadable,
            ..
        }
    ));
    // A range outside the text is refused by the client.
    let broken = FsLineHit {
        line: 1,
        offset: 0,
        text: "abc".into(),
        ranges: vec![(1, 9)],
        truncated: false,
        context: false,
    };
    assert!(!broken.well_formed());
}

#[test]
fn content_search_task_host_offers_content_search_and_old_hosts_do_not() {
    assert!(FsHostFeatures::host().content_search_v1);
    assert!(FsHostFeatures::host()
        .names()
        .contains(&"content_search_v1"));
    let old_host: FsHostFeatures = serde_json::from_str(r#"{"watch_v1":true}"#).unwrap();
    assert!(!old_host.content_search_v1);
    let encoded = serde_json::to_string(&FsHostFeatures::host()).unwrap();
    assert!(encoded.contains(r#""content_search_v1":true"#), "{encoded}");
}
