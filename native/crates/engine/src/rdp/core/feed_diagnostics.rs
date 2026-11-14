//! Bounds of the RDP diagnostic lines that the device feed carries (E54,
//! S16): at most 16 lines of at most 300 characters each. The sending host
//! applies them (`bounded_feed_diagnostics`) and so does every reader of a
//! summary (`deserialize_bounded`), so a peer that sends more cannot inflate
//! the feed of the devices it reaches.

use serde::de::{Deserializer, IgnoredAny, SeqAccess, Visitor};

/// Most diagnostic lines of the device feed's RDP part.
const MAX_FEED_DIAGNOSTIC_LINES: usize = 16;
/// Most characters of one diagnostic line of the device feed.
const MAX_FEED_DIAGNOSTIC_CHARS: usize = 300;

/// The first lines of a host probe in feed form. Each line keeps its first
/// 300 characters (Unicode scalar values, so no UTF-8 sequence is split);
/// order and wording stay as the probe reported them.
pub(super) fn bounded_feed_diagnostics(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .take(MAX_FEED_DIAGNOSTIC_LINES)
        .map(|line| bounded_line(line))
        .collect()
}

/// The reading side of the same bounds: a summary from a peer's feed or from
/// the worker keeps the first 16 lines, each cut to 300 characters; further
/// lines are skipped without being kept.
pub(super) fn deserialize_bounded<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_seq(BoundedLines)
}

fn bounded_line(line: &str) -> String {
    line.chars().take(MAX_FEED_DIAGNOSTIC_CHARS).collect()
}

struct BoundedLines;

impl<'de> Visitor<'de> for BoundedLines {
    type Value = Vec<String>;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a list of diagnostic lines")
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut lines = Vec::new();
        while lines.len() < MAX_FEED_DIAGNOSTIC_LINES {
            match seq.next_element::<String>()? {
                Some(line) => lines.push(bounded_line(&line)),
                None => return Ok(lines),
            }
        }
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(lines)
    }
}
