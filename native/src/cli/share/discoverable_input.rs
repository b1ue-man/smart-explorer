//! Input rules of `se share discoverable`: PIN source, display name, room and
//! offer selectors. Kept free of I/O except `read_pin` so they stay testable.
use std::io::BufRead;

use zeroize::Zeroize;

use crate::share::{OwnDiscoveryOffer, RoomProfile};

const PIN_PROMPT: &str = "PIN (input hidden): ";
const MISSING_PIN: &str =
    "no PIN given: pass --pin PIN, pipe it with --pin-stdin, or run in a terminal to type it";

#[derive(Debug, PartialEq, Eq)]
pub(super) enum PinSource {
    Argument(String),
    Stdin,
    Prompt,
}

pub(super) fn pin_source(
    pin: Option<String>,
    pin_stdin: bool,
    stdin_is_terminal: bool,
) -> Result<PinSource, String> {
    match (pin, pin_stdin) {
        (Some(_), true) => Err("--pin and --pin-stdin exclude each other".to_string()),
        (Some(pin), false) => Ok(PinSource::Argument(pin)),
        (None, true) => Ok(PinSource::Stdin),
        (None, false) if stdin_is_terminal => Ok(PinSource::Prompt),
        (None, false) => Err(MISSING_PIN.to_string()),
    }
}

/// The exact PIN bytes the other device has to enter. Only the line break
/// that ends the stdin line or the typed line is removed.
pub(super) fn read_pin(source: PinSource) -> Result<String, String> {
    let pin = match source {
        PinSource::Argument(pin) => pin,
        PinSource::Stdin => read_pin_line(std::io::stdin().lock())?,
        PinSource::Prompt => crate::cli::os::read_hidden_line(PIN_PROMPT)
            .map_err(|error| format!("{error}; pass the PIN with --pin-stdin instead"))?,
    };
    if pin.len() > crate::share::DISCOVERY_PIN_MAX_BYTES {
        let (length, limit) = (pin.len(), crate::share::DISCOVERY_PIN_MAX_BYTES);
        return Err(format!(
            "the PIN is {length} bytes long; at most {limit} bytes are allowed"
        ));
    }
    Ok(pin)
}

/// Exactly one line: the PIN may be followed by more input, which stays
/// unread. Input that ends before any byte is not an (empty) PIN.
pub(super) fn read_pin_line(reader: impl BufRead) -> Result<String, String> {
    // The longest PIN plus a CR LF line end.
    let limit = crate::share::DISCOVERY_PIN_MAX_BYTES as u64 + 2;
    let mut line = String::new();
    let read = reader
        .take(limit)
        .read_line(&mut line)
        .map_err(|error| format!("read the PIN from stdin: {error}"))?;
    if read == 0 {
        return Err("stdin ended before a PIN line".to_string());
    }
    if read as u64 == limit && !line.ends_with('\n') {
        line.zeroize();
        return Err("the PIN line on stdin is too long".to_string());
    }
    while line.ends_with(['\r', '\n']) {
        line.pop();
    }
    Ok(line)
}

/// Same notice as the desktop UI: allowed, but anyone can guess it.
pub(super) fn trivial_pin(pin: &str) -> bool {
    pin.is_empty() || pin == "0"
}

pub(super) fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("the discoverable name must not be empty".to_string());
    }
    if name.len() > crate::share::MAX_DISCOVERY_ALIAS_BYTES {
        return Err(format!(
            "the discoverable name is {} bytes long; at most {} bytes are allowed",
            name.len(),
            crate::share::MAX_DISCOVERY_ALIAS_BYTES
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("the discoverable name must not contain control characters".to_string());
    }
    Ok(name.to_string())
}

pub(super) fn duration_secs(minutes: u64) -> Result<u64, String> {
    minutes
        .checked_mul(60)
        .ok_or_else(|| format!("{minutes} minutes cannot be represented in seconds"))
}

/// A room by profile id, relation id, exact name, or unique name ignoring case.
pub(super) fn resolve_room<'a>(
    rooms: &'a [RoomProfile],
    selector: &str,
) -> Result<&'a RoomProfile, String> {
    if let Some(room) = rooms
        .iter()
        .find(|room| room.id == selector || room.room_id == selector)
    {
        return Ok(room);
    }
    let exact: Vec<_> = rooms.iter().filter(|room| room.name == selector).collect();
    let matches = if exact.is_empty() {
        let wanted = selector.to_lowercase();
        rooms
            .iter()
            .filter(|room| room.name.to_lowercase() == wanted)
            .collect()
    } else {
        exact
    };
    match matches.as_slice() {
        [room] => Ok(*room),
        [] => Err(format!(
            "no room matches {selector:?}; rooms: {}",
            room_list(rooms)
        )),
        several => Err(format!(
            "{selector:?} names {} rooms; use the room id: {}",
            several.len(),
            several
                .iter()
                .map(|room| room.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn room_list(rooms: &[RoomProfile]) -> String {
    if rooms.is_empty() {
        return "none (create one with `se share room create`)".to_string();
    }
    rooms
        .iter()
        .map(|room| format!("{} ({})", room.name, room.id))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Offers to stop: all, the one named by id or unique id prefix, or the only
/// running offer when no selector is given.
pub(super) fn select_offers(
    offers: &[OwnDiscoveryOffer],
    selector: Option<&str>,
    all: bool,
) -> Result<Vec<OwnDiscoveryOffer>, String> {
    if all {
        return Ok(offers.to_vec());
    }
    let Some(selector) = selector else {
        return match offers {
            [] | [_] => Ok(offers.to_vec()),
            several => Err(format!(
                "{} offers are discoverable; name one or use --all: {}",
                several.len(),
                offer_ids(several)
            )),
        };
    };
    if let Some(offer) = offers.iter().find(|offer| offer.offer_id == selector) {
        return Ok(vec![offer.clone()]);
    }
    let matches: Vec<_> = offers
        .iter()
        .filter(|offer| offer.offer_id.starts_with(selector))
        .cloned()
        .collect();
    match matches.len() {
        1 => Ok(matches),
        0 => Err(format!(
            "no discoverable offer matches {selector:?}; running: {}",
            offer_ids(offers)
        )),
        _ => Err(format!(
            "{selector:?} matches several offers: {}",
            offer_ids(&matches)
        )),
    }
}

fn offer_ids(offers: &[OwnDiscoveryOffer]) -> String {
    if offers.is_empty() {
        return "none".to_string();
    }
    offers
        .iter()
        .map(|offer| offer.offer_id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::{
        duration_secs, pin_source, read_pin_line, resolve_room, select_offers, trivial_pin,
        validate_name, PinSource,
    };
    use crate::share::{DiscoveryPublishTarget, OwnDiscoveryOffer, RoomProfile};

    #[test]
    fn cli_task_discoverable_pin_stdin_reads_one_line() {
        assert_eq!(read_pin_line(&b"1454\nrest\n"[..]).unwrap(), "1454");
        assert_eq!(read_pin_line(&b"1454\r\n"[..]).unwrap(), "1454");
        assert_eq!(read_pin_line(&b"1454"[..]).unwrap(), "1454");
        assert_eq!(read_pin_line(&b" 1 4 \n"[..]).unwrap(), " 1 4 ");
        // An empty line is the empty PIN; no input at all is no PIN.
        assert_eq!(read_pin_line(&b"\n"[..]).unwrap(), "");
        assert!(read_pin_line(&b""[..]).unwrap_err().contains("ended"));
        let limit = crate::share::DISCOVERY_PIN_MAX_BYTES;
        let mut exact = vec![b'7'; limit];
        exact.extend_from_slice(b"\r\n");
        assert_eq!(read_pin_line(&exact[..]).unwrap().len(), limit);
        let long = vec![b'7'; limit + 3];
        assert!(read_pin_line(&long[..]).unwrap_err().contains("too long"));
    }

    fn room(id: &str, room_id: &str, name: &str) -> RoomProfile {
        RoomProfile {
            id: id.into(),
            name: name.into(),
            room_id: room_id.into(),
            auto_join: true,
            last_seen: None,
            status: Default::default(),
            members: Vec::new(),
            exports: Default::default(),
        }
    }

    fn offer(offer_id: &str) -> OwnDiscoveryOffer {
        OwnDiscoveryOffer {
            offer_id: offer_id.into(),
            target: DiscoveryPublishTarget::Direct,
            display_alias: "Laptop".into(),
            discoverable_until: 1_900_000_000,
            published: true,
        }
    }

    #[test]
    fn cli_task_discoverable_pin_source_rules() {
        assert_eq!(
            pin_source(Some("1454".into()), false, false),
            Ok(PinSource::Argument("1454".into()))
        );
        // An empty PIN is a value, not a missing one.
        assert_eq!(
            pin_source(Some(String::new()), false, true),
            Ok(PinSource::Argument(String::new()))
        );
        assert_eq!(pin_source(None, true, true), Ok(PinSource::Stdin));
        assert_eq!(pin_source(None, false, true), Ok(PinSource::Prompt));
        assert!(pin_source(None, false, false)
            .unwrap_err()
            .contains("--pin-stdin"));
        assert!(pin_source(Some("1".into()), true, true).is_err());
        assert!(trivial_pin(""));
        assert!(trivial_pin("0"));
        assert!(!trivial_pin("1454"));
    }

    #[test]
    fn cli_task_discoverable_name_and_duration_rules() {
        assert_eq!(validate_name("  Laptop  ").unwrap(), "Laptop");
        assert!(validate_name("   ").is_err());
        assert!(validate_name("a\tb").is_err());
        assert!(validate_name(&"x".repeat(257)).is_err());
        assert_eq!(validate_name(&"x".repeat(256)).unwrap().len(), 256);
        assert_eq!(duration_secs(5), Ok(300));
        assert!(duration_secs(u64::MAX).is_err());
    }

    #[test]
    fn cli_task_discoverable_room_selector_matches_id_relation_or_unique_name() {
        let rooms = vec![
            room("p1", "r1", "Team"),
            room("p2", "r2", "team"),
            room("p3", "r3", "Family"),
        ];
        assert_eq!(resolve_room(&rooms, "p3").unwrap().id, "p3");
        assert_eq!(resolve_room(&rooms, "r2").unwrap().id, "p2");
        assert_eq!(resolve_room(&rooms, "Team").unwrap().id, "p1");
        assert_eq!(resolve_room(&rooms, "family").unwrap().id, "p3");
        assert!(resolve_room(&rooms, "TEAM")
            .unwrap_err()
            .contains("use the room id"));
        assert!(resolve_room(&rooms, "Work")
            .unwrap_err()
            .contains("Family (p3)"));
        assert!(resolve_room(&[], "Team").unwrap_err().contains("create"));
    }

    #[test]
    fn cli_task_discoverable_stop_selection_by_id_prefix_single_and_all() {
        let offers = vec![offer("abc123"), offer("abd456")];
        let ids = |selected: Vec<OwnDiscoveryOffer>| {
            selected
                .into_iter()
                .map(|offer| offer.offer_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(select_offers(&offers, Some("abd"), false).unwrap()),
            ["abd456"]
        );
        assert_eq!(
            ids(select_offers(&offers, Some("abc123"), false).unwrap()),
            ["abc123"]
        );
        assert!(select_offers(&offers, Some("ab"), false)
            .unwrap_err()
            .contains("several"));
        assert!(select_offers(&offers, Some("zz"), false)
            .unwrap_err()
            .contains("abc123, abd456"));
        assert!(select_offers(&offers, None, false)
            .unwrap_err()
            .contains("--all"));
        assert_eq!(select_offers(&offers, None, true).unwrap().len(), 2);
        assert_eq!(
            ids(select_offers(&offers[..1], None, false).unwrap()),
            ["abc123"]
        );
        assert!(select_offers(&[], None, false).unwrap().is_empty());
    }
}
