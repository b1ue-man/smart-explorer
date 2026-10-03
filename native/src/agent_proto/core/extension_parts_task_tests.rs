use super::*;
use crate::agent_proto::{digest, omission, WireDuplicateItem};

#[test]
fn review_task_legacy_tree_budget_keeps_the_next_request_readable() {
    use crate::agent_proto::{
        read_frame, read_frame_with_tree_budget, write_frame, TreeDecodeBudget, WireNode,
        TREE_BUDGET_ERROR,
    };
    let frame = Frame::Tree(WireNode {
        name: "root".into(),
        size: 30,
        is_dir: true,
        children: (0..30)
            .map(|index| WireNode {
                name: index.to_string(),
                size: 1,
                is_dir: false,
                children: Vec::new(),
            })
            .collect(),
    });
    for memory in [64 * 1024 * 1024, 128] {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, 5, &frame).unwrap();
        write_frame(
            &mut bytes,
            9,
            &Frame::Meta(WireMeta {
                name: "later".into(),
                ..WireMeta::default()
            }),
        )
        .unwrap();
        let mut stream = std::io::Cursor::new(bytes);
        let reply = read_frame_with_tree_budget(&mut stream, |id| {
            assert_eq!(id, 5);
            Some(TreeDecodeBudget::new(2, memory))
        })
        .unwrap();
        assert!(matches!(reply, Some((5, Frame::Err(ref reason))) if reason == TREE_BUDGET_ERROR));
        assert!(
            matches!(read_frame(&mut stream).unwrap(), Some((9, Frame::Meta(ref meta))) if meta.name == "later")
        );
    }
}

#[test]
fn review_task_extension_parts_keep_special_files_and_every_omission() {
    let entries = (0..50_001).map(|index| WireMeta {
        name: format!("note-{index:05}-é"),
        special: index % 13 == 0,
        size: index,
        ..WireMeta::default()
    });
    let omitted = (0..50_001).map(|index| WireOmission {
        rel: format!("pipe-{index}"),
        reason: omission::SPECIAL,
        detail: "Pipe, Socket oder Gerät".into(),
    });
    let mut listed = 0usize;
    let mut protected = 0usize;
    let mut parts = 0;
    emit_listing_parts(entries, omitted, |part| {
        let encoded = part.encode(73)?;
        assert!(encoded.len() <= 1024 * 1024);
        let (id, decoded) = Frame::decode(&encoded)?;
        assert_eq!(id, 73);
        let Frame::DirPart { entries, omitted } = decoded else {
            panic!("listing part expected");
        };
        for entry in entries {
            assert_eq!(entry.name, format!("note-{listed:05}-é"));
            assert_eq!(entry.special, listed % 13 == 0);
            listed += 1;
        }
        for item in omitted {
            assert_eq!(item.rel, format!("pipe-{protected}"));
            assert_eq!(item.reason, omission::SPECIAL);
            protected += 1;
        }
        parts += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!((listed, protected), (50_001, 50_001));
    assert!(parts > 2);
}

#[test]
fn review_task_large_duplicate_group_is_one_group_after_transport() {
    let group = WireDuplicateGroup {
        algorithm: digest::SHA256,
        hex: "ab".repeat(32),
        evidence: 0,
        size: 11,
        reclaimable: 11 * 50_000,
        items: (0..50_001)
            .map(|index| WireDuplicateItem {
                path: format!("/root/copy-{index:05}\\literal"),
                name: format!("copy-{index}"),
                size: 11,
                ..WireDuplicateItem::default()
            })
            .collect(),
    };
    let mut groups = Vec::new();
    let mut parts = 0;
    emit_duplicate_parts(group, |part| {
        let encoded = part.encode(92)?;
        assert!(encoded.len() <= 1024 * 1024);
        let (_, Frame::DupGroup(part)) = Frame::decode(&encoded)? else {
            panic!("group part expected");
        };
        append_duplicate_part(&mut groups, part);
        parts += 1;
        Ok(())
    })
    .unwrap();
    assert!(parts > 1);
    assert_eq!(groups.len(), 1);
    let merged = &groups[0];
    assert_eq!(merged.items.len(), 50_001);
    assert_eq!(merged.reclaimable, 11 * 50_000);
    assert_eq!(
        merged.items.last().unwrap().path,
        "/root/copy-50000\\literal"
    );
}
