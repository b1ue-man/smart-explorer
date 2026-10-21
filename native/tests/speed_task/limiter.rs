//! "Wer begrenzt" (M-5 row "Engpass-Urteil", M-6 "Engpass"): each case builds
//! one bottleneck and checks that the verdict the engine reports in the last
//! five seconds of the job names it. The stage creates the bottleneck around
//! the test program: a link, a throttled drive (`systemd-run --scope -p
//! IOReadBandwidthMax/IOWriteBandwidthMax`), a throttled server container
//! (`docker run --device-write-bps`), a CPU quota (`-p CPUQuota=25%`) or a
//! slow source server (a rate limit inside the container's network).
use crate::data::{random_file, Scratch, GIB};
use crate::remote::{connect, Remote, Way};
use crate::run::run_job;
use crate::support::{
    check_profile, fast_dir, finish, guarded, job, retry_once, slash, var, Case, ReportLine, Trial,
    CASE_LIMIT,
};
use crate::tools::drop_caches;
use smart_explorer::transfer::{Endpoint, Limiter};
use std::path::PathBuf;

/// The window at the end of the job in which the verdict must hold.
const TAIL_MS: u64 = 5000;

/// What one case builds and expects.
struct Plan {
    /// Verdicts that name the built bottleneck.
    expected: &'static [Limiter],
    up: bool,
    bytes: u64,
    /// The local folder is on the throttled drive (`SE_SPEED_LIMITER_DIR`).
    local_on_limited_drive: bool,
    /// The source file is read from the device, not the page cache.
    cold_source: bool,
}

fn local_root(plan: &Plan) -> Result<PathBuf, String> {
    if plan.local_on_limited_drive {
        var("SE_SPEED_LIMITER_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| {
                "SE_SPEED_LIMITER_DIR fehlt (Ordner auf dem gedrosselten Laufwerk)".into()
            })
    } else {
        fast_dir()
    }
}

fn names(list: &[Limiter]) -> String {
    list.iter()
        .map(|item| item.code())
        .collect::<Vec<_>>()
        .join(" oder ")
}

fn run_plan(case: Case, plan: Plan) -> Result<(), String> {
    check_profile(&case)?;
    let remote: Remote = connect(Way::Sftp)?;
    let scratch = Scratch::new(&local_root(&plan)?, case.name);
    let source = scratch.path().join("large.bin");
    random_file(&source, plan.bytes);
    let seeded = remote.guarded_folder("limiter-seed");
    let remote_file = remote.join(&seeded.path, "large.bin");
    if !plan.up {
        remote.seed_file(&seeded.path, &source, "large.bin")?;
        let _ = std::fs::remove_file(&source);
    }
    let throttle = var("SE_SPEED_THROTTLE_BPS").and_then(|text| text.parse::<f64>().ok());
    let trial = retry_once(|| {
        if plan.cold_source {
            drop_caches().expect("the source must come from the device");
        }
        let run = if plan.up {
            let target = remote.guarded_folder("limiter-up");
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
        assert_eq!(run.bytes, plan.bytes, "{}: every byte arrives", case.name);
        let (verdict, count, window) = run.tail_verdict(TAIL_MS);
        let mut line = ReportLine::new(&case, "B/s");
        line.size_bytes = plan.bytes;
        line.files = run.files;
        line.se_rate = run.rate();
        line.requirement = format!("Verdikt der letzten 5 s ∈ {{{}}}", names(plan.expected));
        if let Some(limit) = throttle {
            line.capacity = limit;
            line.share = line.se_rate / limit;
            line.reference_tool = "Drosselwert der Stufe".into();
            line.reference_rate = limit;
        }
        line.limiter = verdict.code().to_string();
        line.notes.push(format!(
            "Verdikt {} in {count} von {window} Meldungen der letzten 5 s; Linkrate {} B/s; \
             Endurteil {}",
            verdict.code(),
            run.link_bps,
            run.limiter.code()
        ));
        let holds = window > 0 && plan.expected.contains(&verdict) && count * 2 >= window;
        let failure = (!holds).then(|| {
            format!(
                "{}: Verdikt {} ({count} von {window} Meldungen der letzten 5 s), erwartet {}",
                case.name,
                verdict.code(),
                names(plan.expected)
            )
        });
        Trial { line, failure }
    });
    finish(&trial.line, trial.failure.as_deref());
    Ok(())
}

pub fn run_limiter(name: &'static str, profile: &'static str) -> Result<(), String> {
    let case = Case::new(name, "sftp", "both", profile);
    let plan = match name {
        "speed_task_limiter_link_wan_large" => Plan {
            // The runner's default route is not the shaped loopback: when its
            // link rate is unknown the engine can only say "server or link".
            expected: &[Limiter::Link, Limiter::Remote],
            up: true,
            bytes: GIB,
            local_on_limited_drive: false,
            cold_source: false,
        },
        "speed_task_limiter_target_disk_throttled" => Plan {
            expected: &[Limiter::Target],
            up: false,
            bytes: 3 * GIB,
            local_on_limited_drive: true,
            cold_source: false,
        },
        "speed_task_limiter_source_disk_throttled" => Plan {
            expected: &[Limiter::Source],
            up: true,
            bytes: GIB,
            local_on_limited_drive: true,
            cold_source: true,
        },
        "speed_task_limiter_remote_server_throttled" => Plan {
            expected: &[Limiter::Remote],
            up: true,
            bytes: 3 * GIB,
            local_on_limited_drive: false,
            cold_source: false,
        },
        "speed_task_limiter_se_cpu_quota" => Plan {
            expected: &[Limiter::SeCpu],
            up: true,
            bytes: 2 * GIB,
            local_on_limited_drive: false,
            cold_source: false,
        },
        _ => Plan {
            expected: &[Limiter::Idle],
            up: false,
            bytes: GIB / 64,
            local_on_limited_drive: false,
            cold_source: false,
        },
    };
    run_plan(case, plan)
}

macro_rules! limiter_cases {
    ($($name:ident => $profile:literal),* $(,)?) => {
        $(
            #[test]
            #[ignore = "needs the bottleneck the stage builds around the test program"]
            fn $name() {
                guarded(stringify!($name), || run_limiter(stringify!($name), $profile));
            }
        )*
    };
}

limiter_cases! {
    speed_task_limiter_link_wan_large => "wan",
    speed_task_limiter_target_disk_throttled => "raw",
    speed_task_limiter_source_disk_throttled => "raw",
    speed_task_limiter_remote_server_throttled => "raw",
    speed_task_limiter_se_cpu_quota => "raw",
    speed_task_limiter_idle_when_source_slow => "raw",
}
