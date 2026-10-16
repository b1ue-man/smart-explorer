use super::*;

fn key(byte: u8) -> DataKey {
    DataKey::from_bytes([byte; KEY_BYTES])
}

#[test]
fn luecken_task_sealed_record_round_trips_and_hides_the_secret() {
    let digest = account_digest("sftp://alice@host:22/");
    let record = seal(&digest, b"hunter2-geheim", &key(1)).unwrap();
    assert!(!record
        .windows(b"hunter2-geheim".len())
        .any(|window| window == b"hunter2-geheim"));
    assert_eq!(
        record_kind(&record).unwrap(),
        RecordKind::Sealed(*key(1).id())
    );
    let opened = open_sealed(&digest, &record, &key(1)).unwrap();
    assert_eq!(opened.as_slice(), b"hunter2-geheim");
}

#[test]
fn luecken_task_sealed_record_rejects_other_key_account_and_tampering() {
    let digest = account_digest("first");
    let record = seal(&digest, b"secret", &key(1)).unwrap();
    assert!(open_sealed(&digest, &record, &key(2))
        .unwrap_err()
        .contains("another data key"));
    assert!(open_sealed(&account_digest("second"), &record, &key(1))
        .unwrap_err()
        .contains("another account"));
    let mut tampered = record.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert!(open_sealed(&digest, &tampered, &key(1))
        .unwrap_err()
        .contains("authentication"));
    let mut forged = record.clone();
    let length = SEALED_AAD_BYTES + NONCE_BYTES;
    forged[length..length + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(open_sealed(&digest, &forged, &key(1))
        .unwrap_err()
        .contains("exceeds"));
    assert!(record_kind(&record[..10])
        .unwrap_err()
        .contains("truncated"));
    let mut version = record;
    version[SEALED_MAGIC.len()] = 3;
    assert!(record_kind(&version).unwrap_err().contains("version"));
}

#[test]
fn luecken_task_legacy_record_still_reads_for_porting() {
    let record = encode_legacy("account", "s3cr3t-ä");
    assert_eq!(record_kind(&record).unwrap(), RecordKind::Legacy);
    let opened = open_legacy(&account_digest("account"), &record).unwrap();
    assert_eq!(opened.as_slice(), "s3cr3t-ä".as_bytes());
    assert!(open_legacy(&account_digest("other"), &record)
        .unwrap_err()
        .contains("another account"));
    let mut corrupt = record;
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    assert!(open_legacy(&account_digest("account"), &corrupt)
        .unwrap_err()
        .contains("checksum"));
}

#[test]
fn luecken_task_wrapped_data_key_needs_the_same_key_file() {
    let kek = [9u8; KEY_BYTES];
    let data = DataKey::generate().unwrap();
    let wrapped = wrap_key(&kek, &data).unwrap();
    assert_eq!(wrapped.len(), WRAPPED_KEY_BYTES);
    assert_eq!(unwrap_key(&kek, &wrapped).unwrap().id(), data.id());
    assert!(unwrap_key(&[8u8; KEY_BYTES], &wrapped).is_err());
}

#[test]
fn luecken_task_record_names_map_back_to_their_digest() {
    let digest = account_digest("x");
    let name = record_name_for_digest(&digest);
    assert_eq!(digest_from_record_name(&name), Some(digest));
    assert_eq!(digest_from_record_name("store.lock"), None);
    assert_eq!(digest_from_record_name(".stage-1-ab.secret"), None);
    assert_eq!(
        digest_from_record_name(&name.to_ascii_uppercase().replace(".SECRET", ".secret")),
        None
    );
}

#[test]
fn luecken_task_descriptor_round_trips_and_rejects_unknown_formats() {
    let descriptor = KeyringDescriptor {
        format: DESCRIPTOR_FORMAT,
        protector: "app-key-file".into(),
        key_id: key(1).id_hex(),
        wrapped: "AAAA".into(),
    };
    let parsed = KeyringDescriptor::parse(&descriptor.encode().unwrap()).unwrap();
    assert_eq!(parsed, descriptor);
    let mut future = descriptor;
    future.format = 3;
    assert!(KeyringDescriptor::parse(&future.encode().unwrap()).is_err());
    assert_eq!(hex_decode(&hex(&[0, 255, 16])), Some(vec![0, 255, 16]));
    assert_eq!(hex_decode("zz"), None);
}
