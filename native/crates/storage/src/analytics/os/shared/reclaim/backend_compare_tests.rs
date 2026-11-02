//! Content comparison of remote candidates without a hash: which files are
//! read, and that every group carries the SHA-256 of its whole content.
use super::super::types::HashAlgorithm;
use super::super::util::hex_lower;
use super::*;

#[test]
fn review_task_hashless_remote_reads_only_same_size_candidates() {
    let be = MockBackend::with_entries(vec![
        file("a.bin", 3, None),
        file("b.bin", 3, None),
        file("c.bin", 3, None),
        file("unique.bin", 4, None),
    ]);
    {
        let mut contents = be.contents.lock().unwrap();
        contents.insert("/a.bin".into(), b"abc".to_vec());
        contents.insert("/b.bin".into(), b"abc".to_vec());
        contents.insert("/c.bin".into(), b"xyz".to_vec());
        contents.insert("/unique.bin".into(), b"abcd".to_vec());
    }
    let p = ReclaimProgress::default();
    let report = find_backend_duplicates(be.clone(), "/", &p, 1);
    assert_eq!(report.groups.len(), 1);
    let paths: Vec<&str> = report.groups[0]
        .items
        .iter()
        .map(|item| item.path.as_str())
        .collect();
    assert_eq!(paths, ["/a.bin", "/b.bin"]);
    assert_eq!(report.summary.candidates, 4);
    assert_eq!(report.summary.compared, 3);
    // Files this small are read once (their ends are their content); a
    // file of a unique size is never read.
    let mut read = be.read_paths.lock().unwrap().clone();
    read.sort();
    assert_eq!(read, ["/a.bin", "/b.bin", "/c.bin"]);
}

#[test]
fn review_task_hashless_groups_carry_the_content_sha256() {
    // Two copies each: a file inside one sample, one whose head and tail
    // overlap, and one larger than both ends.
    let small = b"abc".to_vec();
    let overlapping: Vec<u8> = (0..100 * 1024u32).map(|i| (i % 251) as u8).collect();
    let large: Vec<u8> = (0..300 * 1024u32).map(|i| (i % 241) as u8).collect();
    let be = Arc::new(MockBackend {
        seekable: true,
        ..MockBackend::default()
    });
    let mut entries = Vec::new();
    {
        let mut contents = be.contents.lock().unwrap();
        for (name, content) in [
            ("s1.bin", &small),
            ("s2.bin", &small),
            ("o1.bin", &overlapping),
            ("o2.bin", &overlapping),
            ("l1.bin", &large),
            ("l2.bin", &large),
        ] {
            entries.push(file(name, content.len() as u64, None));
            contents.insert(format!("/{name}"), content.clone());
        }
    }
    be.entries.lock().unwrap().insert("/".to_string(), entries);
    let report = find_backend_duplicates(be.clone(), "/", &ReclaimProgress::default(), 1);
    assert!(
        report.summary.errors.is_empty(),
        "{:?}",
        report.summary.errors
    );
    assert_eq!(report.groups.len(), 3);
    let sha256 =
        |bytes: &[u8]| hex_lower(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref());
    for content in [&small, &overlapping, &large] {
        let group = report
            .groups
            .iter()
            .find(|group| group.size == content.len() as u64)
            .expect("a group of every size");
        assert_eq!(group.items.len(), 2);
        assert_eq!(group.hash.algorithm, HashAlgorithm::Sha256);
        assert_eq!(group.hash.hex, sha256(content));
    }
    // Ends that cover a file are its content: only the large copies are
    // read a third time, as a whole.
    let reads = be.read_paths.lock().unwrap().clone();
    let count = |prefix: &str| reads.iter().filter(|read| read.starts_with(prefix)).count();
    assert_eq!(
        (count("/s"), count("/o"), count("/l")),
        (2, 4, 6),
        "{reads:?}"
    );
}
