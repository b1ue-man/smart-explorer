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
        apps: Vec::new(),
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
        ..PlatformTotals::default()
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

/// A volume root whose `Android/obb` the walk could read besides the own
/// app's `Android/data` folder: both are inside the app figures as well.
fn apps_tree() -> SizeNode {
    dir(
        "0",
        vec![
            dir(
                "Android",
                vec![
                    dir("data", vec![dir("app.own", vec![file("cache.bin", 100)])]),
                    dir("obb", vec![dir("com.game", vec![file("main.obb", 400)])]),
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

type App<'a> = (&'a str, &'a str, u64, u64, u64);

/// Own app 200, a game 1700 (its OBB folder inside), an app without bytes
/// and a repeated package.
const APPS: [App<'static>; 4] = [
    ("app.own", "Smart Explorer", 60, 140, 20),
    ("com.game", "Game", 1400, 300, 50),
    ("com.empty", "Leer", 0, 0, 0),
    ("com.game", "Game (doppelt)", 9, 9, 9),
];

fn with_apps(used: u64, other: Option<u64>, apps: &[App<'_>]) -> PlatformTotals {
    let mut totals = PlatformTotals {
        volume_used_bytes: Some(used),
        other_apps_bytes: other,
        apps: Vec::new(),
    };
    for (package, label, app, data, cache) in apps {
        totals.add_app(package.to_string(), label.to_string(), *app, *data, *cache);
    }
    totals
}

fn rows(view: &NodeView) -> Vec<(&str, u64, NodeKind)> {
    view.children
        .iter()
        .map(|child| (child.name.as_str(), child.size, child.kind))
        .collect()
}

#[test]
fn android_background_task_view_lists_the_apps_at_a_primary_volume_root() {
    let tree = apps_tree();
    let place = VolumeRoot::from_segments(&[], true);
    let approx = Approximations::compute(&tree, &place, with_apps(5000, Some(1100), &APPS), true);

    // The app folders the walk measured (100 + 400) count once: 1400 measured
    // + 1900 apps − 500 + 2200 rest = the used 5000.
    let root = node_view(&tree, &[], &approx, 500).expect("root");
    assert_eq!((root.size, root.measured), (5000, 1400));
    assert_eq!(
        rows(&root),
        [
            (SYSTEM_AND_OTHER_NAME, 2200, NodeKind::Rest),
            (APPS_NAME, 1900, NodeKind::Apps),
            ("Android", 700, NodeKind::Dir),
            ("DCIM", 600, NodeKind::Dir),
            ("big.bin", 100, NodeKind::File),
        ]
    );
    assert!(root.children[1].is_dir);
    assert_eq!(root.children[1].child_count, 2);

    // Other apps' data is part of the apps: no extra row under Android/data.
    let data = node_view(&tree, &segments(&["Android", "data"]), &approx, 500).expect("data");
    assert_eq!((data.size, data.measured), (100, 100));
    assert_eq!(rows(&data), [("app.own", 100, NodeKind::Dir)]);
    let android = node_view(&tree, &segments(&["Android"]), &approx, 500).expect("android");
    assert_eq!(android.size, 700);

    // The apps row opens the list: one row per package, largest first.
    let list = node_view(&tree, &segments(&[APPS_NAME]), &approx, 500).expect("apps");
    assert_eq!(
        (list.name.as_str(), list.size, list.measured, list.kind),
        (APPS_NAME, 1900, 0, NodeKind::Apps)
    );
    assert!(list.is_dir);
    assert_eq!(
        rows(&list),
        [
            ("Game", 1700, NodeKind::App),
            ("Smart Explorer", 200, NodeKind::App)
        ]
    );
    let game = list.children[0].app.as_ref().expect("figures");
    assert_eq!(
        (
            game.package.as_str(),
            game.app_bytes,
            game.data_bytes,
            game.cache_bytes
        ),
        ("com.game", 1400, 300, 50)
    );
    assert!(node_view(&tree, &segments(&[APPS_NAME, "Game"]), &approx, 500).is_none());

    let json = serde_json::to_value(&list).expect("json");
    assert_eq!(json["kind"], "apps");
    let game = &json["children"][0];
    assert_eq!(game["kind"], "app");
    assert_eq!(game["name"], "Game");
    assert_eq!(game["package"], "com.game");
    let bytes = ["size", "appBytes", "dataBytes", "cacheBytes"].map(|key| game[key].as_u64());
    assert_eq!(bytes, [Some(1700), Some(1400), Some(300), Some(50)]);
    assert_eq!(game["isDir"], false);
    assert!(game.get("label").is_none());
    let root_json = serde_json::to_value(&root).expect("json");
    assert_eq!(root_json["children"][1]["kind"], "apps");
    assert_eq!(root_json["children"][1]["childCount"], 2);
    assert!(root_json["children"][1].get("package").is_none());
    assert!(root_json["children"][2].get("appBytes").is_none());
}

#[test]
fn android_background_task_view_lists_apps_only_at_a_whole_primary_volume() {
    let tree = apps_tree();
    let whole = VolumeRoot::from_segments(&[], true);
    let figures = || with_apps(5000, Some(1100), &APPS);

    // A secondary volume lists no apps; its rest keeps the former name.
    let sd = VolumeRoot::from_segments(&[], false);
    let sd = Approximations::compute(&tree, &sd, figures(), true);
    let root = node_view(&tree, &[], &sd, 500).expect("sd root");
    assert_eq!(rows(&root)[0], (UNCAPTURED_NAME, 3600, NodeKind::Rest));
    assert!(root
        .children
        .iter()
        .all(|child| child.kind != NodeKind::Apps));
    assert!(node_view(&tree, &segments(&[APPS_NAME]), &sd, 500).is_none());

    // Below the volume root: no apps, other apps' data as before.
    let android = tree.children[0].clone_tree();
    let below = VolumeRoot::from_segments(&segments(&["Android"]), true);
    let below = Approximations::compute(&android, &below, figures(), true);
    let data = node_view(&android, &segments(&["data"]), &below, 500).expect("data");
    assert_eq!(
        rows(&data)[0],
        (OTHER_APP_DATA_NAME, 1000, NodeKind::Protected)
    );

    // A result with read errors keeps the apps but has no rest.
    let partial = Approximations::compute(&tree, &whole, figures(), false);
    let root = node_view(&tree, &[], &partial, 500).expect("partial root");
    assert_eq!(root.size, 2800);
    assert_eq!(root.children[0].kind, NodeKind::Apps);
    assert!(root
        .children
        .iter()
        .all(|child| child.kind != NodeKind::Rest));

    // Apps without bytes make no list: other apps' data and the former rest.
    let empty = with_apps(5000, Some(1100), &APPS[2..3]);
    let empty = Approximations::compute(&tree, &whole, empty, true);
    let root = node_view(&tree, &[], &empty, 500).expect("root");
    assert_eq!(rows(&root)[0], (UNCAPTURED_NAME, 2600, NodeKind::Rest));
    let data = node_view(&tree, &segments(&["Android", "data"]), &empty, 500).expect("data");
    assert_eq!(data.children[0].name, OTHER_APP_DATA_NAME);

    // The counted-once part never exceeds the apps; a blank label shows the
    // package and the cache is at most the data it belongs to.
    let mut tiny = PlatformTotals::default();
    tiny.add_app("com.tiny".into(), " ".into(), 10, 20, 50);
    let tiny = Approximations::compute(&tree, &whole, tiny, true);
    assert_eq!(node_view(&tree, &[], &tiny, 500).expect("root").size, 1400);
    let list = node_view(&tree, &segments(&[APPS_NAME]), &tiny, 500).expect("apps");
    let tiny_app = list.children[0].app.as_ref().expect("figures");
    assert_eq!(
        (list.children[0].name.as_str(), tiny_app.cache_bytes),
        ("com.tiny", 20)
    );
}

#[test]
fn android_background_task_view_folds_the_smallest_apps_at_the_cap() {
    let tree = apps_tree();
    let whole = VolumeRoot::from_segments(&[], true);
    let apps: [App<'_>; 3] = [
        ("a", "A", 100, 0, 0),
        ("b", "B", 60, 0, 0),
        ("c", "C", 50, 0, 0),
    ];
    let approx = Approximations::compute(&tree, &whole, with_apps(5000, None, &apps), true);
    let list = node_view(&tree, &segments(&[APPS_NAME]), &approx, 2).expect("apps");
    let folded = aggregate_name(2);
    assert_eq!(
        rows(&list),
        [
            (folded.as_str(), 110, NodeKind::Aggregate),
            ("A", 100, NodeKind::App)
        ]
    );
    assert_eq!(list.size, 210);
    // The capped root keeps both approximate rows (rest: 5000 − 1400).
    let root = node_view(&tree, &[], &approx, 2).expect("root");
    assert_eq!(
        rows(&root),
        [
            (SYSTEM_AND_OTHER_NAME, 3600, NodeKind::Rest),
            (APPS_NAME, 210, NodeKind::Apps)
        ]
    );
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
