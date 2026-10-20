//! Monitor keys of the X11 and GNOME sessions (U34-3, U34-14): the name of a
//! connector or output as a file name (`layout/<key>.json`). Everything but
//! ASCII letters, digits, `-`, `.` and `_` becomes `_`, a key never starts
//! with a dot and holds at most 64 bytes; a name that occurs twice gets `-2`,
//! `-3`, … in the order of the list.

/// Longest monitor key (`core::layout`).
const MAX_KEY: usize = 64;

/// The key of one name; an empty name is `monitor`.
pub(crate) fn clean_key(name: &str) -> String {
    let mut key: String = name
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '.' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if key.is_empty() {
        key.push_str("monitor");
    }
    if key.starts_with('.') {
        key.replace_range(0..1, "_");
    }
    key.truncate(MAX_KEY);
    key
}

/// The keys of `names`, in the same order, each one only once.
pub(crate) fn unique_keys(names: &[String]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for name in names {
        let base = clean_key(name);
        let key = (1u32..)
            .map(|number| {
                if number == 1 {
                    return base.clone();
                }
                let suffix = format!("-{number}");
                let mut key = base.clone();
                key.truncate(MAX_KEY - suffix.len());
                key.push_str(&suffix);
                key
            })
            .find(|candidate| !keys.contains(candidate))
            .unwrap_or_else(|| base.clone());
        keys.push(key);
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_file_names_and_unique() {
        assert_eq!(clean_key("DP-1"), "DP-1");
        assert_eq!(clean_key("HDMI A/2"), "HDMI_A_2");
        assert_eq!(clean_key(".hidden"), "_hidden");
        assert_eq!(clean_key("  "), "monitor");
        assert_eq!(clean_key(&"x".repeat(100)).len(), 64);
        let names: Vec<String> = ["DP-1", "DP-1", "DP-1", "eDP-1"]
            .iter()
            .map(|name| name.to_string())
            .collect();
        assert_eq!(unique_keys(&names), ["DP-1", "DP-1-2", "DP-1-3", "eDP-1"]);
        let long = vec!["y".repeat(70), "y".repeat(70)];
        let keys = unique_keys(&long);
        assert!(keys.iter().all(|key| key.len() <= 64), "{keys:?}");
        assert_ne!(keys[0], keys[1]);
    }
}
