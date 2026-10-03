//! PIN rules of discovery pairing (FC2): the suggested PIN is six random
//! digits; shorter or trivially guessable PINs need the explicit "unsichere
//! PIN erlauben" opt-in. An offer lasts at most 30 minutes and ends after
//! the first pairing or after five failed attempts (`discovery_offer_guard`).

use super::core::random_bytes;

/// Characters a PIN needs without the explicit opt-in.
pub const DISCOVERY_MIN_PIN_CHARS: usize = 6;
/// Longest discoverability of one offer.
pub const DISCOVERY_MAX_OFFER_SECS: u64 = 30 * 60;
/// Attempts that never proved the PIN after which an offer ends.
pub const DISCOVERY_MAX_FAILED_PAIRINGS: u8 = 5;
const SUGGESTED_DIGITS: usize = 6;
/// Byte values below this map evenly onto ten digits (25 × 10).
const UNBIASED_BYTE_LIMIT: u8 = 250;
/// Frequent six-digit PINs no simple rule below catches.
const COMMON_PINS: [&str; 10] = [
    "112233", "123321", "159753", "159357", "147258", "258369", "789456", "520520", "102030",
    "654456",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoveryPinStrength {
    Acceptable,
    TooShort,
    Trivial,
}

impl DiscoveryPinStrength {
    pub fn is_acceptable(self) -> bool {
        self == Self::Acceptable
    }

    /// Why the PIN needs the opt-in; `None` when it does not.
    pub fn problem(self) -> Option<&'static str> {
        match self {
            Self::Acceptable => None,
            Self::TooShort => Some(
                "Die PIN ist zu kurz: mindestens 6 Zeichen, oder „Unsichere PIN erlauben“ wählen",
            ),
            Self::Trivial => Some(
                "Die PIN ist leicht zu erraten (Wiederholung, Zahlenfolge oder häufige PIN); \
                 eine andere wählen oder „Unsichere PIN erlauben“",
            ),
        }
    }
}

pub fn discovery_pin_strength(pin: &[u8]) -> DiscoveryPinStrength {
    let text = String::from_utf8_lossy(pin);
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < DISCOVERY_MIN_PIN_CHARS {
        DiscoveryPinStrength::TooShort
    } else if trivial(&text, &chars) {
        DiscoveryPinStrength::Trivial
    } else {
        DiscoveryPinStrength::Acceptable
    }
}

fn trivial(text: &str, chars: &[char]) -> bool {
    let repeats_with_period =
        |period: usize| (0..chars.len()).all(|index| chars[index] == chars[index % period]);
    if (1..=3).any(|period| chars.len() >= period * 2 && repeats_with_period(period)) {
        return true;
    }
    if !chars.iter().all(char::is_ascii_digit) {
        return false;
    }
    let digits: Vec<i16> = chars
        .iter()
        .filter_map(|digit| digit.to_digit(10))
        .map(|digit| digit as i16)
        .collect();
    let steps: Vec<i16> = digits.windows(2).map(|pair| pair[1] - pair[0]).collect();
    steps.iter().all(|step| *step == 1)
        || steps.iter().all(|step| *step == -1)
        || COMMON_PINS.contains(&text)
}

/// Six uniformly random digits that pass `discovery_pin_strength`.
pub fn suggest_discovery_pin() -> Result<String, String> {
    for _ in 0..32 {
        let mut pin = String::with_capacity(SUGGESTED_DIGITS);
        while pin.len() < SUGGESTED_DIGITS {
            for byte in random_bytes::<16>()? {
                if byte < UNBIASED_BYTE_LIMIT && pin.len() < SUGGESTED_DIGITS {
                    pin.push(char::from(b'0' + byte % 10));
                }
            }
        }
        if discovery_pin_strength(pin.as_bytes()).is_acceptable() {
            return Ok(pin);
        }
    }
    Err("Es konnte keine sichere Zufalls-PIN erzeugt werden".into())
}

#[cfg(test)]
mod review_task_tests {
    use super::{discovery_pin_strength, suggest_discovery_pin, DiscoveryPinStrength};

    #[test]
    fn review_task_short_and_trivial_pins_need_the_opt_in() {
        for (pin, strength) in [
            ("", DiscoveryPinStrength::TooShort),
            ("0", DiscoveryPinStrength::TooShort),
            ("1454", DiscoveryPinStrength::TooShort),
            ("12345", DiscoveryPinStrength::TooShort),
            ("000000", DiscoveryPinStrength::Trivial),
            ("123456", DiscoveryPinStrength::Trivial),
            ("987654", DiscoveryPinStrength::Trivial),
            ("121212", DiscoveryPinStrength::Trivial),
            ("123123", DiscoveryPinStrength::Trivial),
            ("abcabcabc", DiscoveryPinStrength::Trivial),
            ("159753", DiscoveryPinStrength::Trivial),
            ("481902", DiscoveryPinStrength::Acceptable),
            ("Haus-am-See", DiscoveryPinStrength::Acceptable),
            ("äöüßéè", DiscoveryPinStrength::Acceptable),
        ] {
            assert_eq!(discovery_pin_strength(pin.as_bytes()), strength, "{pin:?}");
        }
    }

    #[test]
    fn review_task_suggested_pins_are_six_acceptable_digits() {
        for _ in 0..64 {
            let pin = suggest_discovery_pin().expect("random PIN");
            assert_eq!(pin.len(), 6);
            assert!(pin.chars().all(|digit| digit.is_ascii_digit()));
            assert!(discovery_pin_strength(pin.as_bytes()).is_acceptable());
        }
    }
}
