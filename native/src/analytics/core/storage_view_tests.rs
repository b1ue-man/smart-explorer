use super::*;

fn dir(name: &str, children: Vec<SizeNode>) -> SizeNode {
    SizeNode {
        name: name.into(),
        size: children.iter().map(|child| child.size).sum(),
        is_dir: true,
        children,
    }
}

fn file(name: &str, size: u64) -> SizeNode {
    SizeNode {
        name: name.into(),
        size,
        is_dir: false,
        children: Vec::new(),
    }
}

fn volume_tree(android: &str, data: &str) -> SizeNode {
    dir(
        "0",
        vec![
            dir(
                android,
                vec![
                    dir(data, vec![dir("own", vec![file("cache.bin", 100)])]),
                    dir("media", vec![file("song.mp3", 200)]),
                ],
            ),
            dir(
                "DCIM",
                vec![file("a.jpg", 590), file(&aggregate_name(3), 10)],
            ),
            file("big.bin", 100),
        ],
    )
}

fn segments(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn totals(used: u64, other: u64) -> PlatformTotals {
    PlatformTotals {
        volume_used_bytes: Some(used),
        other_apps_bytes: Some(other),
    }
}

#[test]
fn android_background_task_view_adds_other_app_data_and_rest_at_a_volume_root() {
    let tree = volume_tree("Android", "data");
    let place = VolumeRoot::from_segments(&[], true);
    let approx = Approximations::compute(&tree, &place, totals(5000, 1100), true);

    let root = node_view(&tree, &[], &approx, 500).expect("root");
    assert_eq!((root.size, root.measured), (5000, 1000));
    let rows: Vec<(&str, u64, NodeKind)> = root
        .children
        .iter()
        .map(|child| (child.name.as_str(), child.size, child.kind))
        .collect();
    assert_eq!(
        rows,
        [
            (UNCAPTURED_NAME, 3000, NodeKind::Rest),
            ("Android", 1300, NodeKind::Dir),
            ("DCIM", 600, NodeKind::Dir),
            ("big.bin", 100, NodeKind::File),
        ]
    );

    let android = node_view(&tree, &segments(&["Android"]), &approx, 500).expect("android");
    assert_eq!((android.size, android.measured), (1300, 300));
    assert_eq!(android.children[0].name, "data");
    assert_eq!(android.children[0].size, 1100);

    let data = node_view(&tree, &segments(&["Android", "data"]), &approx, 500).expect("data");
    assert_eq!((data.size, data.measured), (1100, 100));
    assert_eq!(data.children[0].name, OTHER_APP_DATA_NAME);
    assert_eq!(data.children[0].kind, NodeKind::Protected);
    assert_eq!(data.children[0].size, 1000);
    assert!(!data.children[0].is_dir);
    assert_eq!(data.children[1].name, "own");

    let dcim = node_view(&tree, &segments(&["DCIM"]), &approx, 500).expect("dcim");
    assert_eq!(dcim.size, 600);
    assert_eq!(dcim.children[1].kind, NodeKind::Aggregate);
    assert!(node_view(&tree, &segments(&["gone"]), &approx, 500).is_none());
}

#[test]
fn android_background_task_view_rows_only_where_they_apply() {
    let tree = volume_tree("Android", "data");
    let whole = VolumeRoot::from_segments(&[], true);
    // A result with read errors gets no rest; other apps stay.
    let partial = Approximations::compute(&tree, &whole, totals(5000, 1100), false);
    assert_eq!(
        node_view(&tree, &[], &partial, 500).expect("root").size,
        2000
    );
    // Without usage access the other apps fall into the rest.
    let no_access = PlatformTotals {
        volume_used_bytes: Some(5000),
        other_apps_bytes: None,
    };
    let rest_only = Approximations::compute(&tree, &whole, no_access, true);
    let root = node_view(&tree, &[], &rest_only, 500).expect("root");
    assert_eq!(root.children[0].size, 4000);
    let data = node_view(&tree, &segments(&["Android", "data"]), &rest_only, 500).expect("data");
    assert_eq!(data.children.len(), 1);
    // A secondary volume reports no other apps' data; a measured total that
    // already exceeds Android's figure adds nothing.
    let sd = VolumeRoot::from_segments(&[], false);
    assert_eq!(sd.app_data, None);
    assert!(sd.whole_volume);
    let covered = Approximations::compute(&tree, &whole, totals(1000, 50), true);
    assert_eq!(covered, Approximations::default());
    // Below the volume root: no rest, other apps still under Android/data.
    let below = VolumeRoot::from_segments(&segments(&["Android"]), true);
    assert_eq!(below.app_data, Some(segments(&["data"])));
    assert!(!below.whole_volume);
    let android = tree.children[0].clone_tree();
    let approx = Approximations::compute(&android, &below, totals(5000, 1100), true);
    let view = node_view(&android, &[], &approx, 500).expect("android root");
    assert_eq!(view.size, 1300);
    assert_eq!(
        VolumeRoot::from_segments(&segments(&["android", "DATA"]), true).app_data,
        Some(Vec::new())
    );
    let elsewhere: [&[&str]; 3] = [
        &["DCIM"],
        &["Android", "data", "own"],
        &["Android", "media"],
    ];
    for elsewhere in elsewhere {
        assert_eq!(
            VolumeRoot::from_segments(&segments(elsewhere), true).app_data,
            None
        );
    }
}

#[test]
fn android_background_task_view_follows_tree_case_and_keeps_rows_at_the_cap() {
    let tree = volume_tree("android", "Data");
    let place = VolumeRoot::from_segments(&[], true);
    let approx = Approximations::compute(&tree, &place, totals(5000, 1100), true);
    let data = node_view(&tree, &segments(&["android", "Data"]), &approx, 500).expect("data");
    assert_eq!(data.children[0].name, OTHER_APP_DATA_NAME);
    // The cap leaves room for the approximate rows.
    let capped = node_view(&tree, &[], &approx, 2).expect("root");
    let names: Vec<&str> = capped
        .children
        .iter()
        .map(|child| child.name.as_str())
        .collect();
    assert_eq!(names, [UNCAPTURED_NAME, "android"]);
    let json = serde_json::to_value(&data).expect("json");
    assert_eq!(json["measured"], 100);
    assert_eq!(json["isDir"], true);
    assert_eq!(json["kind"], "dir");
    assert_eq!(json["children"][0]["kind"], "protected");
    assert_eq!(json["children"][0]["childCount"], 0);
    assert_eq!(json["children"][1]["kind"], "dir");
}

#[test]
fn android_background_task_view_recognizes_only_generated_aggregates() {
    assert!(is_aggregate_name(&aggregate_name(12)));
    for name in [
        "… x weitere Eintraege",
        "… weitere Eintraege",
        "12 weitere Eintraege",
        "a.bin",
    ] {
        assert!(!is_aggregate_name(name), "{name}");
    }
}

trait CloneTree {
    fn clone_tree(&self) -> SizeNode;
}

impl CloneTree for SizeNode {
    fn clone_tree(&self) -> SizeNode {
        SizeNode {
            name: self.name.clone(),
            size: self.size,
            is_dir: self.is_dir,
            children: self.children.iter().map(CloneTree::clone_tree).collect(),
        }
    }
}
