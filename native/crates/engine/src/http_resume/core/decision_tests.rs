//! The decisions of a resumable download (user rule 2026-11-17): what an
//! answer means for the kept bytes, and that only attempts without a new
//! byte for the whole stall window end a download, never its total time.
use std::time::{Duration, Instant};

use super::{
    classify, if_range_validator, parse_content_range, range_header, refusal, Answer, Attempts,
    ContentRange, Kept, Next, Patience, Refusal, Reply,
};

fn range(first: u64, last: u64, complete: Option<u64>) -> Option<ContentRange> {
    Some(ContentRange {
        first,
        last,
        complete,
    })
}

#[test]
fn zeitlimit_task_content_range_is_read_strictly() {
    assert_eq!(
        parse_content_range("bytes 50-99/100"),
        range(50, 99, Some(100))
    );
    assert_eq!(parse_content_range("  Bytes 0-0/1 "), range(0, 0, Some(1)));
    assert_eq!(parse_content_range("bytes 7-9/*"), range(7, 9, None));
    for refused in [
        "bytes */100",
        "items 0-9/10",
        "bytes 9-7/10",
        "bytes 0-10/10",
        "bytes -5/10",
        "bytes 0-+5/10",
        "bytes 0-5",
        "bytes 0-5/",
        "",
    ] {
        assert_eq!(parse_content_range(refused), None, "{refused:?}");
    }
    assert_eq!(
        range_header(0),
        None,
        "the first attempt asks for everything"
    );
    assert_eq!(range_header(4096).as_deref(), Some("bytes=4096-"));
    assert_eq!(if_range_validator(Some("\"abc\"")), Some("\"abc\""));
    assert_eq!(if_range_validator(Some("W/\"abc\"")), None, "weak tag");
    assert_eq!(if_range_validator(Some("abc")), None);
    assert_eq!(if_range_validator(None), None);
}

fn kept(offset: u64, pinned: Option<u64>, known: Option<u64>) -> Kept {
    Kept {
        offset,
        pinned,
        known,
    }
}

fn reply(status: u16, content_range: Option<&str>, content_length: Option<u64>) -> Reply<'_> {
    Reply {
        status,
        content_range,
        content_length,
    }
}

#[test]
fn zeitlimit_task_an_answer_continues_restarts_or_is_refused() {
    // The first answer: the whole resource with its length.
    assert_eq!(
        classify(kept(0, None, None), reply(200, None, Some(100))),
        Answer::Restart { total: Some(100) }
    );
    // A server that ignores the range sends everything again.
    assert_eq!(
        classify(kept(50, None, Some(100)), reply(200, None, Some(100))),
        Answer::Restart { total: Some(100) }
    );
    // The rest from exactly the kept bytes continues them.
    assert_eq!(
        classify(
            kept(50, None, Some(100)),
            reply(206, Some("bytes 50-99/100"), Some(50))
        ),
        Answer::Continue { total: Some(100) }
    );
    assert_eq!(
        classify(
            kept(50, Some(100), Some(100)),
            reply(206, Some("bytes 50-79/*"), None)
        ),
        Answer::Continue { total: Some(100) },
        "a shorter range continues; the pinned length stays the total"
    );
    // Another start, a length that disagrees, a changed resource, no range.
    for (case, answer) in [
        ("other start", reply(206, Some("bytes 40-99/100"), None)),
        ("length", reply(206, Some("bytes 50-99/100"), Some(10))),
        ("changed", reply(206, Some("bytes 50-119/120"), None)),
        ("no range", reply(206, None, Some(50))),
        ("no content", reply(204, None, None)),
    ] {
        assert_eq!(
            classify(kept(50, None, Some(100)), answer),
            Answer::Unusable,
            "{case}"
        );
    }
    // A signed or pinned length no attempt can change.
    assert_eq!(
        classify(kept(0, Some(100), Some(100)), reply(200, None, Some(101))),
        Answer::WrongLength {
            announced: 101,
            pinned: 100
        }
    );
    assert_eq!(
        classify(
            kept(50, Some(100), Some(100)),
            reply(206, Some("bytes 50-98/99"), None)
        ),
        Answer::WrongLength {
            announced: 99,
            pinned: 100
        }
    );
    // Without a length the answer of a chunked body leaves it open.
    assert_eq!(
        classify(kept(0, None, None), reply(200, None, None)),
        Answer::Restart { total: None }
    );
    assert_eq!(refusal(416), Refusal::Restart);
    for status in [408, 425, 429, 500, 502, 503, 504] {
        assert_eq!(refusal(status), Refusal::Wait, "{status}");
    }
    for status in [400, 401, 403, 404, 410, 501] {
        assert_eq!(refusal(status), Refusal::Final, "{status}");
    }
}

#[test]
fn zeitlimit_task_only_attempts_without_a_new_byte_for_the_stall_window_end_a_download() {
    let patience = Patience::STANDARD;
    let start = Instant::now();
    let at = |seconds: u64| start + Duration::from_secs(seconds);
    let mut attempts = Attempts::new(start);

    // Hours of progress never end it: every attempt that brought a new byte
    // is followed by the next without a pause.
    for hour in 1..=10u64 {
        assert_eq!(
            attempts.ended(hour * 1_000_000, at(hour * 3600), &patience),
            Next::Retry(Duration::ZERO),
            "hour {hour}"
        );
    }
    let last_progress = 10 * 3600;
    // Attempts without a new byte pause 1, 2, 4, 8 s and then wait for the
    // stall window although five attempts failed.
    let mut now = last_progress;
    for (attempt, pause) in [1u64, 2, 4, 8].into_iter().enumerate() {
        now += 1;
        assert_eq!(
            attempts.ended(10_000_000, at(now), &patience),
            Next::Retry(Duration::from_secs(pause)),
            "attempt {attempt}"
        );
    }
    now += 1;
    assert_eq!(
        attempts.ended(10_000_000, at(now), &patience),
        Next::Retry(Duration::from_secs(16)),
        "five idle attempts within the stall window go on"
    );
    assert_eq!(attempts.idle(), 5);
    // A restarted answer that brings the same bytes again is no progress.
    assert_eq!(
        attempts.ended(9_000_000, at(last_progress + 100), &patience),
        Next::Retry(Duration::from_secs(30)),
        "the pause stays at its maximum"
    );
    assert_eq!(
        attempts.ended(10_000_000, at(last_progress + 120), &patience),
        Next::Stalled,
        "two minutes and five attempts without a new byte end it"
    );

    // One new byte starts the count again.
    let mut attempts = Attempts::new(start);
    for second in 1..=6u64 {
        assert!(matches!(
            attempts.ended(0, at(second), &patience),
            Next::Retry(_)
        ));
    }
    assert_eq!(
        attempts.ended(1, at(7), &patience),
        Next::Retry(Duration::ZERO)
    );
    assert_eq!(attempts.idle(), 0);
    assert_eq!(
        attempts.ended(1, at(500), &patience),
        Next::Retry(Duration::from_secs(1)),
        "the stall window alone does not end it either"
    );
}
