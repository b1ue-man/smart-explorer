//! The cleartext names „Beide behalten“ gives (vault plan C6.4, U26-6):
//! `<Name> (Konflikt <JJJJ-MM-TT HHMM> <Bezeichnung>).<Erweiterung>` in the
//! folder of the original, with `-2`, `-3`, … before the extension while
//! the name is taken. Pure: the caller formats the time (`stamp`) and tells
//! which names exist.

/// The label of a side as a part of a name: `/`, `\` and control
/// characters become `_` (a name has no path separators).
fn clean_label(label: &str) -> String {
    label
        .chars()
        .map(|character| {
            if character == '/' || character == '\\' || character.is_control() {
                '_'
            } else {
                character
            }
        })
        .collect()
}

/// `name` split before its extension; names without a dot, with only a
/// leading dot (`.profile`) or with a trailing dot have none.
fn split_extension(name: &str) -> (&str, Option<&str>) {
    match name.rsplit_once('.') {
        Some((base, extension)) if !base.is_empty() && !extension.is_empty() => {
            (base, Some(extension))
        }
        _ => (name, None),
    }
}

fn first_free(
    base: &str,
    extension: Option<&str>,
    middle: &str,
    taken: &dyn Fn(&str) -> bool,
) -> String {
    let build = |suffix: &str| match extension {
        Some(extension) => format!("{base} ({middle}){suffix}.{extension}"),
        None => format!("{base} ({middle}){suffix}"),
    };
    let first = build("");
    if !taken(&first) {
        return first;
    }
    (2u64..)
        .map(|number| build(&format!("-{number}")))
        .find(|candidate| !taken(candidate))
        .unwrap_or_else(|| build(&format!("-{}", u64::MAX)))
}

/// The conflict name of the file `name`: `stamp` is `JJJJ-MM-TT HHMM`,
/// `side_label` the label of the side whose version loses the original
/// name, `taken` tells whether a name of the folder is in use.
pub(crate) fn conflict_name(
    name: &str,
    stamp: &str,
    side_label: &str,
    taken: &dyn Fn(&str) -> bool,
) -> String {
    let (base, extension) = split_extension(name);
    let middle = format!("Konflikt {stamp} {}", clean_label(side_label));
    first_free(base, extension, &middle, taken)
}

/// The conflict name of the folder `name` (a folder has no extension).
pub(crate) fn folder_conflict_name(
    name: &str,
    stamp: &str,
    side_label: &str,
    taken: &dyn Fn(&str) -> bool,
) -> String {
    let middle = format!("Konflikt {stamp} {}", clean_label(side_label));
    first_free(name, None, &middle, taken)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAMP: &str = "2026-10-19 1405";

    #[test]
    fn vault_task_conflict_name_keeps_extension_and_counts_up() {
        let free = |_: &str| false;
        assert_eq!(
            conflict_name("Bericht.txt", STAMP, "Laptop", &free),
            "Bericht (Konflikt 2026-10-19 1405 Laptop).txt"
        );
        let once = |name: &str| name == "Bericht (Konflikt 2026-10-19 1405 Laptop).txt";
        assert_eq!(
            conflict_name("Bericht.txt", STAMP, "Laptop", &once),
            "Bericht (Konflikt 2026-10-19 1405 Laptop)-2.txt"
        );
        let twice = |name: &str| {
            name == "Bericht (Konflikt 2026-10-19 1405 Laptop).txt"
                || name == "Bericht (Konflikt 2026-10-19 1405 Laptop)-2.txt"
        };
        assert_eq!(
            conflict_name("Bericht.txt", STAMP, "Laptop", &twice),
            "Bericht (Konflikt 2026-10-19 1405 Laptop)-3.txt"
        );
    }

    #[test]
    fn vault_task_conflict_name_without_extension_and_with_separators() {
        let free = |_: &str| false;
        assert_eq!(
            conflict_name("Makefile", STAMP, "A/B\\C", &free),
            "Makefile (Konflikt 2026-10-19 1405 A_B_C)"
        );
        assert_eq!(
            conflict_name(".profile", STAMP, "A", &free),
            ".profile (Konflikt 2026-10-19 1405 A)"
        );
        assert_eq!(
            conflict_name("archiv.tar.gz", STAMP, "A", &free),
            "archiv.tar (Konflikt 2026-10-19 1405 A).gz"
        );
        assert_eq!(
            folder_conflict_name("Fotos.2026", STAMP, "B", &free),
            "Fotos.2026 (Konflikt 2026-10-19 1405 B)"
        );
    }
}
