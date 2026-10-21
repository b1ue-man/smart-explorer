//! One large file over each way and shaped link (M-5 rows 1 to 3, M-6 "Große
//! Dateien Netz"): the rate of the engine job against the capacity of the same
//! path. `run_large` is the one generic function; each `#[test]` names its
//! way, direction and link.
use crate::data::{random_file, Scratch, GIB, MIB};
use crate::remote::{connect, Way};
use crate::run::run_job;
use crate::support::{
    capacity, check_profile, fast_dir, finish, guarded, job, retry_once, shortfall, size_for,
    slash, Case, ReportLine, Trial, CASE_LIMIT,
};
use smart_explorer::transfer::Endpoint;

/// The capacity a way is measured against (M-1): UDP for the Share (QUIC),
/// TCP through the published port for the ways behind Docker, TCP on the
/// loopback for FTP.
pub struct Link {
    pub bps: f64,
    pub name: &'static str,
    pub source: String,
}

pub fn link_capacity(way: Way, profile: &str) -> Result<Link, String> {
    let all = capacity(profile)?;
    let (bps, name, tool) = if way == Way::Share {
        (all.udp_bps, "C_udp", "iperf3 -u")
    } else if way.proxied() {
        (
            all.proxy_tcp_bps,
            "C_tcp",
            "iperf3 -P 8 über den veröffentlichten Port",
        )
    } else {
        (all.tcp_bps, "C_tcp", "iperf3 -P 8")
    };
    if bps <= 0.0 {
        return Err(format!(
            "Kapazität {name} der Strecke {profile} fehlt ({tool} hat keinen Wert geliefert)"
        ));
    }
    Ok(Link {
        bps,
        name,
        source: format!("{tool}, Strecke {profile}"),
    })
}

/// The required share of the capacity (M-5): TCP ways 0.90 on `lan`/`wan`,
/// 0.85 on `fat`/`loss`; the Share 0.85 on every link.
pub fn large_required(way: Way, profile: &str) -> f64 {
    if way == Way::Share || matches!(profile, "fat" | "loss") {
        0.85
    } else {
        0.90
    }
}

pub fn run_large(case: Case) -> Result<(), String> {
    check_profile(&case)?;
    let way = Way::parse(case.way);
    let link = link_capacity(way, case.profile)?;
    let remote = connect(way)?;
    let size = size_for(link.bps, 256 * MIB, 3 * GIB);
    let required = large_required(way, case.profile);
    let scratch = Scratch::new(&fast_dir()?, case.name);
    let source = scratch.path().join("large.bin");
    random_file(&source, size);
    let up = case.direction == "up";
    let seeded = remote.guarded_folder("seed");
    let remote_file = remote.join(&seeded.path, "large.bin");
    if !up {
        remote.seed_file(&seeded.path, &source, "large.bin")?;
        let _ = std::fs::remove_file(&source);
    }
    let trial = retry_once(|| {
        let run = if up {
            let target = remote.guarded_folder("up");
            run_job(
                job(
                    Endpoint::Local,
                    Endpoint::Remote(remote.backend.clone()),
                    vec![slash(&source)],
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
                    vec![remote_file.clone()],
                    slash(&target),
                    case.name,
                ),
                CASE_LIMIT,
            )
        };
        assert_eq!(run.bytes, size, "{}: every byte arrives", case.name);
        let mut line = ReportLine::new(&case, "B/s");
        line.size_bytes = size;
        line.files = run.files;
        line.se_rate = run.rate();
        line.against(
            link.bps,
            required,
            &format!("≥ {required:.2} × {}", link.name),
        );
        line.reference_tool = link.source.clone();
        line.reference_rate = link.bps;
        line.limiter = run.limiter.code().to_string();
        line.notes.push(format!(
            "parallel höchstens {}, Bereiche höchstens {}, {:.1} s",
            run.max_parallel,
            run.max_segments,
            run.seconds()
        ));
        let failure = shortfall(
            case.name,
            line.se_rate,
            link.bps,
            required,
            link.name,
            &link.source,
            "B/s",
        );
        Trial { line, failure }
    });
    finish(&trial.line, trial.failure.as_deref());
    Ok(())
}

/// A copy from one server to another through the app (M-6 `r2r`). Both legs
/// leave the machine over the one shaped loopback queue (netem shapes the sum
/// of both directions), so the capacity of the pair is the slower leg's half.
pub fn run_r2r(case: Case, from: Way, to: Way) -> Result<(), String> {
    check_profile(&case)?;
    let (from_link, to_link) = (
        link_capacity(from, case.profile)?,
        link_capacity(to, case.profile)?,
    );
    let capacity = from_link.bps.min(to_link.bps) / 2.0;
    let (source, target) = (connect(from)?, connect(to)?);
    let size = size_for(capacity, 256 * MIB, 3 * GIB);
    let required = large_required(from, case.profile).min(large_required(to, case.profile));
    let scratch = Scratch::new(&fast_dir()?, case.name);
    let local = scratch.path().join("large.bin");
    random_file(&local, size);
    let seeded = source.guarded_folder("seed");
    source.seed_file(&seeded.path, &local, "large.bin")?;
    let _ = std::fs::remove_file(&local);
    let origin = source.join(&seeded.path, "large.bin");
    let reference = format!(
        "min({}, {}) ÷ 2 (beide Strecken teilen die eine Warteschlange der Schleife)",
        from_link.name, to_link.name
    );
    let trial = retry_once(|| {
        let destination = target.guarded_folder("r2r");
        let run = run_job(
            job(
                Endpoint::Remote(source.backend.clone()),
                Endpoint::Remote(target.backend.clone()),
                vec![origin.clone()],
                destination.path.clone(),
                case.name,
            ),
            CASE_LIMIT,
        );
        assert_eq!(run.bytes, size, "{}: every byte arrives", case.name);
        let mut line = ReportLine::new(&case, "B/s");
        line.size_bytes = size;
        line.files = run.files;
        line.se_rate = run.rate();
        line.against(
            capacity,
            required,
            &format!("≥ {required:.2} × {reference}"),
        );
        line.reference_tool = format!("iperf3, {}", from_link.source);
        line.reference_rate = capacity;
        line.limiter = run.limiter.code().to_string();
        line.notes
            .push(format!("parallel höchstens {}", run.max_parallel));
        let failure = shortfall(
            case.name,
            line.se_rate,
            capacity,
            required,
            "C_tcp ÷ 2",
            &from_link.source,
            "B/s",
        );
        Trial { line, failure }
    });
    finish(&trial.line, trial.failure.as_deref());
    Ok(())
}

#[test]
#[ignore = "needs both servers, shaped loopback and capacity file"]
fn speed_task_r2r_sftp_to_ftp_large_wan() {
    let case = Case::new(
        "speed_task_r2r_sftp_to_ftp_large_wan",
        "sftp>ftp",
        "copy",
        "wan",
    );
    guarded(case.name, || run_r2r(case, Way::Sftp, Way::Ftp));
}

#[test]
#[ignore = "needs both servers, shaped loopback and capacity file"]
fn speed_task_r2r_share_to_sftp_large_wan() {
    let case = Case::new(
        "speed_task_r2r_share_to_sftp_large_wan",
        "share>sftp",
        "copy",
        "wan",
    );
    guarded(case.name, || run_r2r(case, Way::Share, Way::Sftp));
}

macro_rules! large_cases {
    ($($name:ident => ($way:literal, $direction:literal, $profile:literal)),* $(,)?) => {
        $(
            #[test]
            #[ignore = "needs the stage's server, shaped loopback and capacity file"]
            fn $name() {
                let case = Case::new(stringify!($name), $way, $direction, $profile);
                guarded(case.name, || run_large(case));
            }
        )*
    };
}

large_cases! {
    speed_task_sftp_large_up_lan => ("sftp", "up", "lan"),
    speed_task_sftp_large_up_wan => ("sftp", "up", "wan"),
    speed_task_sftp_large_up_fat => ("sftp", "up", "fat"),
    speed_task_sftp_large_up_loss => ("sftp", "up", "loss"),
    speed_task_sftp_large_down_lan => ("sftp", "down", "lan"),
    speed_task_sftp_large_down_wan => ("sftp", "down", "wan"),
    speed_task_sftp_large_down_fat => ("sftp", "down", "fat"),
    speed_task_sftp_large_down_loss => ("sftp", "down", "loss"),
    speed_task_ftp_large_up_lan => ("ftp", "up", "lan"),
    speed_task_ftp_large_up_wan => ("ftp", "up", "wan"),
    speed_task_ftp_large_up_loss => ("ftp", "up", "loss"),
    speed_task_ftp_large_down_lan => ("ftp", "down", "lan"),
    speed_task_ftp_large_down_wan => ("ftp", "down", "wan"),
    speed_task_ftp_large_down_loss => ("ftp", "down", "loss"),
    speed_task_webdav_large_up_lan => ("webdav", "up", "lan"),
    speed_task_webdav_large_up_wan => ("webdav", "up", "wan"),
    speed_task_webdav_large_up_loss => ("webdav", "up", "loss"),
    speed_task_webdav_large_down_lan => ("webdav", "down", "lan"),
    speed_task_webdav_large_down_wan => ("webdav", "down", "wan"),
    speed_task_webdav_large_down_loss => ("webdav", "down", "loss"),
    speed_task_smb_large_up_lan => ("smb", "up", "lan"),
    speed_task_smb_large_up_wan => ("smb", "up", "wan"),
    speed_task_smb_large_down_lan => ("smb", "down", "lan"),
    speed_task_smb_large_down_wan => ("smb", "down", "wan"),
    speed_task_share_large_up_lan => ("share", "up", "lan"),
    speed_task_share_large_up_wan => ("share", "up", "wan"),
    speed_task_share_large_up_fat => ("share", "up", "fat"),
    speed_task_share_large_up_loss => ("share", "up", "loss"),
    speed_task_share_large_down_lan => ("share", "down", "lan"),
    speed_task_share_large_down_wan => ("share", "down", "wan"),
    speed_task_share_large_down_fat => ("share", "down", "fat"),
    speed_task_share_large_down_loss => ("share", "down", "loss"),
    speed_task_agent_large_up_lan => ("agent", "up", "lan"),
    speed_task_agent_large_up_wan => ("agent", "up", "wan"),
    speed_task_agent_large_up_fat => ("agent", "up", "fat"),
    speed_task_agent_large_down_lan => ("agent", "down", "lan"),
    speed_task_agent_large_down_wan => ("agent", "down", "wan"),
    speed_task_agent_large_down_fat => ("agent", "down", "fat"),
}
