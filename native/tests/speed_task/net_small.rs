//! Many small files over each way (M-5 rows "klein Netz", "gemischt Netz",
//! "WebDAV Anfragen je Datei"; M-6 "Kleine Dateien Netz" and "Gemischt"):
//! 10 000 files of 4096 bytes in 100 folders, files per second against
//! `B_klein = C_tcp ÷ (4096 + 1024)`, and against `rclone` on the same data.
use crate::data::{mixed_tree, small_tree, Scratch, MIXED_FILES, MIXED_SEED};
use crate::net_large::link_capacity;
use crate::reference::{rclone_files_per_second, rclone_unavailable};
use crate::remote::{connect, FolderGuard, Remote, Way};
use crate::run::{run_job, Run};
use crate::support::{
    capacity, check_profile, fast_dir, finish, guarded, job, need, retry_once, rtt_seconds,
    shortfall, slash, Case, ReportLine, Trial, CASE_LIMIT,
};
use smart_explorer::transfer::Endpoint;
use std::collections::BTreeMap;
use std::path::Path;

const FILES: u64 = 10_000;
const FOLDERS: usize = 100;
const FILE_BYTES: usize = 4096;
/// The protocol cost per small file that M-5 assumes (A-S8).
const PROTOCOL_BYTES: f64 = 1024.0;

/// Where the downloads find the tree: a guarded folder and the tree below it.
fn remote_tree<'a>(remote: &'a Remote, tree: &Path) -> Result<(FolderGuard<'a>, String), String> {
    let guard = remote.guarded_folder("seed");
    let place = remote.join(&guard.path, "tree");
    remote
        .backend
        .mkdir_all(&place)
        .map_err(|error| format!("Ordner auf dem Server anlegen: {error}"))?;
    remote.seed_tree(&place, tree)?;
    Ok((guard, place))
}

/// One engine job that moves `tree` to the server (`up`) or `place` from it.
fn move_tree(case: &Case, remote: &Remote, scratch: &Scratch, tree: &Path, place: &str) -> Run {
    if case.direction == "up" {
        let target = remote.guarded_folder("up");
        run_job(
            job(
                Endpoint::Local,
                Endpoint::Remote(remote.backend.clone()),
                vec![slash(tree)],
                target.path.clone(),
                case.name,
            ),
            CASE_LIMIT,
        )
    } else {
        let target = scratch.sub(&format!("down-{}", crate::support::unique("d")));
        run_job(
            job(
                Endpoint::Remote(remote.backend.clone()),
                Endpoint::Local,
                vec![place.to_string()],
                slash(&target),
                case.name,
            ),
            CASE_LIMIT,
        )
    }
}

fn tcp_capacity(way: Way, profile: &str) -> Result<f64, String> {
    let all = capacity(profile)?;
    let bps = if way.proxied() {
        all.proxy_tcp_bps
    } else {
        all.tcp_bps
    };
    if bps <= 0.0 {
        return Err(format!("keine TCP-Kapazität der Strecke {profile}"));
    }
    Ok(bps)
}

/// `rclone copy` of the same data on the same server, once per case.
fn rclone_rate(
    case: &Case,
    remote: &Remote,
    scratch: &Scratch,
    tree: &Path,
    place: &str,
    transfers: u32,
) -> Result<f64, String> {
    let way = remote.way;
    let server = remote
        .server
        .as_ref()
        .ok_or("rclone kennt das Share-Protokoll nicht")?;
    if case.direction == "up" {
        let target = remote.guarded_folder("rclone");
        rclone_files_per_second(way, server, tree, &target.path, true, transfers, FILES)
    } else {
        let target = scratch.sub("rclone-down");
        rclone_files_per_second(way, server, &target, place, false, transfers, FILES)
    }
}

pub fn run_small(case: Case) -> Result<(), String> {
    check_profile(&case)?;
    let way = Way::parse(case.way);
    let c_tcp = tcp_capacity(way, case.profile)?;
    let b_small = c_tcp / (FILE_BYTES as f64 + PROTOCOL_BYTES);
    let remote = connect(way)?;
    let rtt = if way == Way::Ftp {
        rtt_seconds().ok_or("SE_SPEED_RTT_MS fehlt (Rundreisezeit der Strecke)")?
    } else {
        0.0
    };
    let scratch = Scratch::new(&fast_dir()?, case.name);
    let tree = small_tree(&scratch.sub("tree"), FOLDERS, FILES as usize, FILE_BYTES);
    let source = format!("iperf3 -P 8, Strecke {}", case.profile);
    let up = case.direction == "up";
    let seeded = if up {
        None
    } else {
        Some(remote_tree(&remote, &tree)?)
    };
    let place = seeded
        .as_ref()
        .map(|(_, place)| place.clone())
        .unwrap_or_default();
    let mut rclone: Option<Result<f64, String>> = None;
    let trial = retry_once(|| {
        let run = move_tree(&case, &remote, &scratch, &tree, &place);
        assert_eq!(run.files, FILES, "{}: every file arrives", case.name);
        let rate = run.files_per_second();
        let mut line = ReportLine::new(&case, "files/s");
        line.size_bytes = FILES * FILE_BYTES as u64;
        line.files = run.files;
        line.se_rate = rate;
        line.limiter = run.limiter.code().to_string();
        line.notes.push(format!(
            "parallel höchstens {}, {:.1} s",
            run.max_parallel,
            run.seconds()
        ));
        let mut failure = None;
        if way == Way::Ftp {
            let k = run.max_parallel.max(1) as f64;
            let ceiling = k / (5.0 * rtt);
            line.against(ceiling, 0.6, "≥ 0,6 × K ÷ (5 × RTT)");
            line.reference_tool = format!("K = {k} (parallel), RTT = {:.0} ms", rtt * 1000.0);
            failure = shortfall(
                case.name,
                rate,
                ceiling,
                0.6,
                "K ÷ (5 × RTT)",
                &line.reference_tool,
                "files/s",
            );
        } else {
            line.against(b_small, 0.5, "≥ 0,5 × B_klein und ≥ 1,0 × rclone");
            failure = failure.or(shortfall(
                case.name, rate, b_small, 0.5, "B_klein", &source, "files/s",
            ));
            if way == Way::Share {
                line.notes.push(
                    "rclone kennt das Share-Protokoll nicht: die Forderung gegen rclone entfällt"
                        .into(),
                );
            } else {
                let transfers = run.max_parallel.max(16);
                let measured = rclone
                    .get_or_insert_with(|| {
                        rclone_rate(&case, &remote, &scratch, &tree, &place, transfers)
                    })
                    .clone();
                match measured {
                    Ok(other) => {
                        line.reference_tool = format!("rclone --transfers={transfers}");
                        line.reference_rate = other;
                        failure = failure.or(shortfall(
                            case.name,
                            rate,
                            other,
                            1.0,
                            "rclone",
                            &line.reference_tool,
                            "files/s",
                        ));
                    }
                    Err(reason) => rclone_unavailable(&mut line, &reason),
                }
            }
        }
        Trial { line, failure }
    });
    finish(&trial.line, trial.failure.as_deref());
    Ok(())
}

/// The mixed tree (300 files, 1 KiB to 16 MiB, seed 20261021): M-5 allows
/// 15 % over the sum of the transfer of the bytes and of the file starts.
pub fn run_mixed(case: Case) -> Result<(), String> {
    check_profile(&case)?;
    let way = Way::parse(case.way);
    let link = link_capacity(way, case.profile)?;
    let b_small = tcp_capacity(way, case.profile)? / (FILE_BYTES as f64 + PROTOCOL_BYTES);
    let remote = connect(way)?;
    let scratch = Scratch::new(&fast_dir()?, case.name);
    let tree = scratch.sub("mixed");
    let bytes = mixed_tree(&tree, MIXED_SEED);
    let seeded = if case.direction == "up" {
        None
    } else {
        Some(remote_tree(&remote, &tree)?)
    };
    let place = seeded
        .as_ref()
        .map(|(_, place)| place.clone())
        .unwrap_or_default();
    let ideal = bytes as f64 / link.bps + MIXED_FILES as f64 / b_small;
    let ideal_rate = bytes as f64 / ideal;
    let required = 1.0 / 1.15;
    let trial = retry_once(|| {
        let run = move_tree(&case, &remote, &scratch, &tree, &place);
        assert_eq!(run.files, MIXED_FILES as u64, "{}: every file", case.name);
        let mut line = ReportLine::new(&case, "B/s");
        line.size_bytes = bytes;
        line.files = run.files;
        line.se_rate = run.rate();
        line.against(
            ideal_rate,
            required,
            "Dauer ≤ 1,15 × (Bytes ÷ C + Dateien ÷ B_klein)",
        );
        line.reference_tool = link.source.clone();
        line.reference_rate = link.bps;
        line.limiter = run.limiter.code().to_string();
        line.notes
            .push(format!("Dauer {:.1} s, Soll {:.1} s", run.seconds(), ideal));
        let failure = shortfall(
            case.name,
            line.se_rate,
            ideal_rate,
            required,
            "Bytes ÷ Soll-Dauer",
            &link.source,
            "B/s",
        );
        Trial { line, failure }
    });
    finish(&trial.line, trial.failure.as_deref());
    Ok(())
}

/// Requests per file at the WebDAV server, counted in the access log of
/// `httpd` (M-5: at most 3.2); the byte counters (`%I %O`) also give the real
/// protocol cost per file that replaces the assumption A-S8.
pub fn run_requests_per_file() -> Result<(), String> {
    let case = Case::new("speed_task_webdav_requests_per_file", "webdav", "up", "lan");
    check_profile(&case)?;
    let log = need("SE_SPEED_WEBDAV_ACCESS_LOG", "Zugriffsprotokoll von httpd")?;
    let remote = connect(Way::Webdav)?;
    let scratch = Scratch::new(&fast_dir()?, case.name);
    let tree = small_tree(&scratch.sub("tree"), FOLDERS, FILES as usize, FILE_BYTES);
    let before = std::fs::read_to_string(&log)
        .map(|text| text.lines().count())
        .unwrap_or(0);
    let run = move_tree(&case, &remote, &scratch, &tree, "");
    std::thread::sleep(std::time::Duration::from_secs(2));
    let text = std::fs::read_to_string(&log).map_err(|error| format!("{log}: {error}"))?;
    let mut methods: BTreeMap<String, u64> = BTreeMap::new();
    let (mut requests, mut octets) = (0u64, 0f64);
    for line in text.lines().skip(before) {
        requests += 1;
        let method = line
            .split('"')
            .nth(1)
            .and_then(|request| request.split_whitespace().next())
            .unwrap_or("?");
        *methods.entry(method.to_string()).or_default() += 1;
        let mut tail = line.split_whitespace().rev();
        let (out, received) = (tail.next(), tail.next());
        if let (Some(out), Some(received)) = (out, received) {
            octets += out.parse::<f64>().unwrap_or(0.0) + received.parse::<f64>().unwrap_or(0.0);
        }
    }
    let per_file = requests as f64 / FILES as f64;
    let overhead = octets / FILES as f64 - FILE_BYTES as f64;
    let mut line = ReportLine::new(&case, "Anfragen/Datei");
    line.size_bytes = FILES * FILE_BYTES as u64;
    line.files = run.files;
    line.se_rate = per_file;
    line.against(3.2, 1.0, "≤ 3,2 Anfragen je Datei");
    line.reference_tool = "httpd-Zugriffsprotokoll".into();
    line.limiter = run.limiter.code().to_string();
    line.notes.push(format!(
        "Anfragen {methods:?}; Protokollaufwand je Datei {overhead:.0} Byte (Annahme A-S8: {PROTOCOL_BYTES:.0})"
    ));
    let failure = (per_file > 3.2).then(|| {
        format!(
            "{}: {per_file:.2} Anfragen je Datei > 3,2 (Anfragen aus dem Zugriffsprotokoll von httpd: {methods:?})",
            case.name
        )
    });
    finish(&line, failure.as_deref());
    Ok(())
}

macro_rules! small_cases {
    ($($name:ident => ($way:literal, $direction:literal, $profile:literal)),* $(,)?) => {
        $(
            #[test]
            #[ignore = "needs the stage's server, shaped loopback and capacity file"]
            fn $name() {
                let case = Case::new(stringify!($name), $way, $direction, $profile);
                guarded(case.name, || run_small(case));
            }
        )*
    };
}

small_cases! {
    speed_task_sftp_small_up_lan => ("sftp", "up", "lan"),
    speed_task_sftp_small_up_wan => ("sftp", "up", "wan"),
    speed_task_sftp_small_down_lan => ("sftp", "down", "lan"),
    speed_task_sftp_small_down_wan => ("sftp", "down", "wan"),
    speed_task_ftp_small_up_lan => ("ftp", "up", "lan"),
    speed_task_ftp_small_up_wan => ("ftp", "up", "wan"),
    speed_task_ftp_small_down_lan => ("ftp", "down", "lan"),
    speed_task_ftp_small_down_wan => ("ftp", "down", "wan"),
    speed_task_webdav_small_up_lan => ("webdav", "up", "lan"),
    speed_task_webdav_small_up_wan => ("webdav", "up", "wan"),
    speed_task_webdav_small_down_lan => ("webdav", "down", "lan"),
    speed_task_webdav_small_down_wan => ("webdav", "down", "wan"),
    speed_task_smb_small_up_lan => ("smb", "up", "lan"),
    speed_task_smb_small_up_wan => ("smb", "up", "wan"),
    speed_task_smb_small_down_lan => ("smb", "down", "lan"),
    speed_task_smb_small_down_wan => ("smb", "down", "wan"),
    speed_task_share_small_up_lan => ("share", "up", "lan"),
    speed_task_share_small_up_wan => ("share", "up", "wan"),
    speed_task_share_small_down_lan => ("share", "down", "lan"),
    speed_task_share_small_down_wan => ("share", "down", "wan"),
    speed_task_agent_small_up_lan => ("agent", "up", "lan"),
    speed_task_agent_small_up_wan => ("agent", "up", "wan"),
    speed_task_agent_small_down_lan => ("agent", "down", "lan"),
    speed_task_agent_small_down_wan => ("agent", "down", "wan"),
}

macro_rules! mixed_cases {
    ($($name:ident => ($way:literal, $direction:literal)),* $(,)?) => {
        $(
            #[test]
            #[ignore = "needs the stage's server, shaped loopback and capacity file"]
            fn $name() {
                let case = Case::new(stringify!($name), $way, $direction, "wan");
                guarded(case.name, || run_mixed(case));
            }
        )*
    };
}

mixed_cases! {
    speed_task_sftp_mixed_up_wan => ("sftp", "up"),
    speed_task_sftp_mixed_down_wan => ("sftp", "down"),
    speed_task_share_mixed_up_wan => ("share", "up"),
    speed_task_share_mixed_down_wan => ("share", "down"),
}

#[test]
#[ignore = "needs the stage's WebDAV server with its access log"]
fn speed_task_webdav_requests_per_file() {
    guarded("speed_task_webdav_requests_per_file", run_requests_per_file);
}
