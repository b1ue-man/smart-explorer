use super::*;

const KEY_ONE: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIAEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEB";
const KEY_TWO: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIAICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgIC";

fn key(base64: &str) -> russh::keys::PublicKey {
    russh::keys::parse_public_key_base64(base64).unwrap()
}

#[test]
fn luecken_task_unknown_keys_are_never_stored_silently() {
    let directory = tempfile::tempdir().unwrap();
    let verdict = verify_in(directory.path(), &[], "host", 22, &key(KEY_ONE)).unwrap();
    assert_eq!(verdict, HostKeyVerdict::Unknown);
    assert!(!directory.path().join(STORE_NAME).exists());
}

#[test]
fn luecken_task_pins_trust_their_key_and_reject_a_changed_one() {
    let directory = tempfile::tempdir().unwrap();
    let first = fingerprint(&key(KEY_ONE));
    pin_in(directory.path(), "host", 2222, &first, false).unwrap();
    pin_in(directory.path(), "host", 2222, &first, false).unwrap();
    assert_eq!(
        verify_in(directory.path(), &[], "host", 2222, &key(KEY_ONE)).unwrap(),
        HostKeyVerdict::Trusted
    );
    assert_eq!(
        verify_in(directory.path(), &[], "host", 2222, &key(KEY_TWO)).unwrap(),
        HostKeyVerdict::Changed {
            source: STORE_NAME.into()
        }
    );
    let other = fingerprint(&key(KEY_TWO));
    assert_eq!(
        pin_in(directory.path(), "host", 2222, &other, false)
            .unwrap_err()
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    pin_in(directory.path(), "host", 2222, &other, true).unwrap();
    assert_eq!(
        verify_in(directory.path(), &[], "host", 2222, &key(KEY_TWO)).unwrap(),
        HostKeyVerdict::Trusted
    );
    assert!(forget_in(directory.path(), "host", 2222).unwrap());
    assert!(!forget_in(directory.path(), "host", 2222).unwrap());
    assert!(pin_in(directory.path(), "host", 22, "MD5:aa", false).is_err());
}

#[test]
fn luecken_task_openssh_known_hosts_vouch_for_and_against_keys() {
    let directory = tempfile::tempdir().unwrap();
    let openssh = directory.path().join("known_hosts");
    std::fs::write(
        &openssh,
        format!(
            "# OpenSSH\n[nas]:2222 ssh-ed25519 {KEY_ONE}\nbox,10.0.0.9 ssh-ed25519 {KEY_ONE}\n\
             @revoked old ssh-ed25519 {KEY_TWO}\n@cert-authority *.corp ssh-ed25519 {KEY_TWO}\n"
        ),
    )
    .unwrap();
    let files = vec![openssh];
    let check = |host: &str, port: u16, base64: &str| {
        verify_in(directory.path(), &files, host, port, &key(base64)).unwrap()
    };
    assert_eq!(check("nas", 2222, KEY_ONE), HostKeyVerdict::Trusted);
    assert_eq!(check("box", 22, KEY_ONE), HostKeyVerdict::Trusted);
    assert!(matches!(
        check("box", 22, KEY_TWO),
        HostKeyVerdict::Changed { .. }
    ));
    assert!(matches!(
        check("old", 22, KEY_TWO),
        HostKeyVerdict::Revoked { .. }
    ));
    assert_eq!(check("nas", 22, KEY_ONE), HostKeyVerdict::Unknown);
    assert_eq!(check("srv.corp", 22, KEY_TWO), HostKeyVerdict::Unknown);
    // A Smart Explorer pin takes precedence over OpenSSH's files.
    pin_in(
        directory.path(),
        "box",
        22,
        &fingerprint(&key(KEY_TWO)),
        false,
    )
    .unwrap();
    assert_eq!(check("box", 22, KEY_TWO), HostKeyVerdict::Trusted);
}

#[test]
fn malformed_conflicting_and_oversized_files_fail_closed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(STORE_NAME);
    std::fs::write(&path, "broken\n").unwrap();
    assert!(verify_in(directory.path(), &[], "host", 22, &key(KEY_ONE)).is_err());
    std::fs::write(&path, "host:22 one\nhost:22 two\n").unwrap();
    assert!(verify_in(directory.path(), &[], "other", 22, &key(KEY_ONE)).is_err());
    std::fs::File::create(&path)
        .unwrap()
        .set_len(MAX_KNOWN_HOSTS_BYTES + 1)
        .unwrap();
    assert!(verify_in(directory.path(), &[], "host", 22, &key(KEY_ONE)).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_store_fails_closed_without_touching_target() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target");
    let path = directory.path().join(STORE_NAME);
    std::fs::write(&target, "unchanged").unwrap();
    symlink(&target, &path).unwrap();
    let fingerprint = fingerprint(&key(KEY_ONE));
    assert!(pin_in(directory.path(), "host", 22, &fingerprint, false).is_err());
    assert_eq!(std::fs::read_to_string(target).unwrap(), "unchanged");
}

fn openssh_file(directory: &Path, name: &str, text: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn luecken_task_revoked_openssh_entry_refuses_a_pinned_key() {
    let directory = tempfile::tempdir().unwrap();
    pin_in(
        directory.path(),
        "box",
        22,
        &fingerprint(&key(KEY_ONE)),
        false,
    )
    .unwrap();
    let before = verify_in(directory.path(), &[], "box", 22, &key(KEY_ONE)).unwrap();
    assert_eq!(before, HostKeyVerdict::Trusted);

    let revoking = openssh_file(
        directory.path(),
        "known_hosts",
        &format!("@revoked box ssh-ed25519 {KEY_ONE}\n"),
    );
    let files = vec![revoking.clone()];
    let after = verify_in(directory.path(), &files, "box", 22, &key(KEY_ONE)).unwrap();
    assert_eq!(
        after,
        HostKeyVerdict::Revoked {
            source: revoking.display().to_string()
        }
    );
    // The pin file itself is untouched; only the refusal changes.
    assert!(directory.path().join(STORE_NAME).exists());
}

#[test]
fn luecken_task_revoked_wildcard_and_other_hosts_and_other_keys() {
    let directory = tempfile::tempdir().unwrap();
    for (host, port) in [("box", 22), ("nas", 2222)] {
        pin_in(
            directory.path(),
            host,
            port,
            &fingerprint(&key(KEY_ONE)),
            false,
        )
        .unwrap();
    }
    let revoking = openssh_file(
        directory.path(),
        "known_hosts",
        &format!("@revoked * ssh-ed25519 {KEY_ONE}\n"),
    );
    let files = vec![revoking];
    for (host, port) in [("box", 22), ("nas", 2222)] {
        assert!(matches!(
            verify_in(directory.path(), &files, host, port, &key(KEY_ONE)).unwrap(),
            HostKeyVerdict::Revoked { .. }
        ));
    }
    // A revoked entry for another host name does not touch this pin ...
    let other_host = openssh_file(
        directory.path(),
        "other",
        &format!("@revoked elsewhere ssh-ed25519 {KEY_ONE}\n"),
    );
    assert_eq!(
        verify_in(directory.path(), &[other_host], "box", 22, &key(KEY_ONE)).unwrap(),
        HostKeyVerdict::Trusted
    );
    // ... and neither does a revoked entry for a different key.
    let other_key = openssh_file(
        directory.path(),
        "other-key",
        &format!("@revoked * ssh-ed25519 {KEY_TWO}\n"),
    );
    assert_eq!(
        verify_in(directory.path(), &[other_key], "box", 22, &key(KEY_ONE)).unwrap(),
        HostKeyVerdict::Trusted
    );
}

#[test]
fn luecken_task_revoked_beats_a_changed_pin_and_trust_in_an_earlier_file() {
    let directory = tempfile::tempdir().unwrap();
    pin_in(
        directory.path(),
        "box",
        22,
        &fingerprint(&key(KEY_TWO)),
        false,
    )
    .unwrap();
    let trusting = openssh_file(
        directory.path(),
        "user",
        &format!("box ssh-ed25519 {KEY_ONE}\n"),
    );
    let revoking = openssh_file(
        directory.path(),
        "system",
        &format!("@revoked box ssh-ed25519 {KEY_ONE}\n"),
    );
    // KEY_ONE differs from the pin (changed) and is vouched for by the first
    // file, but the second file revokes it: refused, never merely "changed".
    let verdict = verify_in(
        directory.path(),
        &[trusting, revoking.clone()],
        "box",
        22,
        &key(KEY_ONE),
    )
    .unwrap();
    assert_eq!(
        verdict,
        HostKeyVerdict::Revoked {
            source: revoking.display().to_string()
        }
    );
}

#[test]
fn luecken_task_a_key_listed_in_any_openssh_file_outvotes_a_stale_one() {
    let directory = tempfile::tempdir().unwrap();
    let stale = openssh_file(
        directory.path(),
        "user",
        &format!("box ssh-ed25519 {KEY_TWO}\n"),
    );
    let current = openssh_file(
        directory.path(),
        "system",
        &format!("box ssh-ed25519 {KEY_ONE}\n"),
    );
    for files in [
        vec![stale.clone(), current.clone()],
        vec![current.clone(), stale.clone()],
    ] {
        assert_eq!(
            verify_in(directory.path(), &files, "box", 22, &key(KEY_ONE)).unwrap(),
            HostKeyVerdict::Trusted
        );
    }
    // Without the current file the stale entry is a changed key.
    assert!(matches!(
        verify_in(directory.path(), &[stale], "box", 22, &key(KEY_ONE)).unwrap(),
        HostKeyVerdict::Changed { .. }
    ));
}

#[test]
fn luecken_task_pins_in_the_0_5_180_format_are_read_and_written_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let one = fingerprint(&key(KEY_ONE));
    let two = fingerprint(&key(KEY_TWO));
    // Exactly what 0.5.180 wrote: `host:port SHA256:<fingerprint>` per line.
    let path = directory.path().join(STORE_NAME);
    std::fs::write(&path, format!("nas.example:22 {one}\nbox:2222 {two}\n\n")).unwrap();
    assert_eq!(
        verify_in(directory.path(), &[], "nas.example", 22, &key(KEY_ONE)).unwrap(),
        HostKeyVerdict::Trusted
    );
    assert_eq!(
        verify_in(directory.path(), &[], "box", 2222, &key(KEY_TWO)).unwrap(),
        HostKeyVerdict::Trusted
    );
    assert_eq!(
        verify_in(directory.path(), &[], "box", 2222, &key(KEY_ONE)).unwrap(),
        HostKeyVerdict::Changed {
            source: STORE_NAME.into()
        }
    );
    assert!(has_pin_in(directory.path(), "nas.example", 22));
    assert!(!has_pin_in(directory.path(), "nas.example", 2222));
    // A new pin is appended in the same line format; old lines stay.
    pin_in(directory.path(), "new.example", 22, &one, false).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!("nas.example:22 {one}\nbox:2222 {two}\n\nnew.example:22 {one}\n")
    );
}

#[test]
fn luecken_task_replacing_a_pin_names_the_replace_option() {
    let directory = tempfile::tempdir().unwrap();
    pin_in(
        directory.path(),
        "box",
        22,
        &fingerprint(&key(KEY_ONE)),
        false,
    )
    .unwrap();
    let error = pin_in(
        directory.path(),
        "box",
        22,
        &fingerprint(&key(KEY_TWO)),
        false,
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert!(error
        .to_string()
        .contains("se connections trust-host-key box:22 --replace"));
}

#[test]
fn luecken_task_openssh_files_that_are_not_regular_files_cannot_vouch() {
    let directory = tempfile::tempdir().unwrap();
    // A directory in place of the file (a FIFO would block `open` for good).
    let not_a_file = directory.path().join("known_hosts");
    std::fs::create_dir(&not_a_file).unwrap();
    assert_eq!(
        verify_in(directory.path(), &[not_a_file], "box", 22, &key(KEY_ONE)).unwrap(),
        HostKeyVerdict::Unknown
    );
}

#[cfg(unix)]
#[test]
fn luecken_task_openssh_files_are_read_through_links_like_ssh_does() {
    let directory = tempfile::tempdir().unwrap();
    let real = openssh_file(
        directory.path(),
        "dotfiles-known-hosts",
        &format!("box ssh-ed25519 {KEY_ONE}\n"),
    );
    let link = directory.path().join("known_hosts");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(
        verify_in(directory.path(), &[link], "box", 22, &key(KEY_ONE)).unwrap(),
        HostKeyVerdict::Trusted
    );
}

#[test]
fn luecken_task_openssh_files_are_read_only_up_to_the_size_limit() {
    let directory = tempfile::tempdir().unwrap();
    // 1 MiB of comment lines, then an entry that would vouch for the key.
    let mut text = "#".repeat(1023);
    text.push('\n');
    let mut padded = text.repeat(1024);
    padded.push_str(&format!("box ssh-ed25519 {KEY_ONE}\n"));
    let path = openssh_file(directory.path(), "known_hosts", &padded);
    assert_eq!(
        verify_in(directory.path(), &[path], "box", 22, &key(KEY_ONE)).unwrap(),
        HostKeyVerdict::Unknown
    );
}
