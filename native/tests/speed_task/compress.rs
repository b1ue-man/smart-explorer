//! Adaptive LZ4 and the QUIC controller (M-5 rows "LZ4" and "QUIC-Regler",
//! M-6 "Kompression und Regler"). The slow link (20 Mbit/s, 25 ms each way) is
//! set by the stage for these cases only. A case runs twice, in two processes:
//! first against a host that has LZ4 switched off (`SE_TEST_NO_LZ4=1`,
//! `SE_SPEED_LZ4_MODE=off`), then against one that offers it (`on`); the
//! second run reads the first one's rate from `SE_SPEED_STATE`. The controller
//! cases run once per controller (`SE_SPEED_CC=cubic|bbr3`, set on the hosts
//! as `SE_SHARE_QUIC_CC`) and only report; the decision flag is the stage's.
use crate::data::{lz4_ratio, random_file, text_file, Scratch, GIB, MIB};
use crate::net_large::link_capacity;
use crate::remote::{connect, Way};
use crate::run::run_job;
use crate::support::{
    check_profile, fast_dir, finish, guarded, job, need, retry_once, shortfall, size_for, slash,
    Case, ReportLine, Trial, CASE_LIMIT,
};
use serde_json::{json, Value};
use smart_explorer::transfer::Endpoint;
use std::path::Path;

/// 20 Mbit/s in bytes per second: the capacity the size of a slow-link case
/// follows (M-4 gives 256 MiB as the least size).
const SLOW_BYTES_PER_SECOND: f64 = 2_500_000.0;

fn state_path() -> Result<String, String> {
    need("SE_SPEED_STATE", "Zustandsdatei zwischen den beiden Läufen")
}

fn load_state() -> Value {
    state_path()
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}))
}

fn store(name: &str, key: &str, rate: f64) -> Result<(), String> {
    let mut state = load_state();
    state[name][key] = json!(rate);
    std::fs::write(state_path()?, state.to_string()).map_err(|error| error.to_string())
}

/// One download of `source` from the host `way`, in bytes per second.
fn download(case: &Case, way: Way, source: &Path, size: u64) -> Result<(f64, String), String> {
    let remote = connect(way)?;
    let seeded = remote.guarded_folder("compress");
    remote.seed_file(&seeded.path, source, "large.bin")?;
    let scratch = Scratch::new(&fast_dir()?, "compress-down");
    let file = remote.join(&seeded.path, "large.bin");
    let run = run_job(
        job(
            Endpoint::Remote(remote.backend.clone()),
            Endpoint::Local,
            vec![file],
            slash(&scratch.sub("down")),
            case.name,
        ),
        CASE_LIMIT,
    );
    assert_eq!(run.bytes, size, "{}: every byte arrives", case.name);
    Ok((
        run.rate(),
        format!(
            "{:.1} s, Verdikt {}, parallel höchstens {}",
            run.seconds(),
            run.limiter.code(),
            run.max_parallel
        ),
    ))
}

pub fn run_lz4(case: Case, text: bool) -> Result<(), String> {
    check_profile(&case)?;
    let mode = need("SE_SPEED_LZ4_MODE", "on oder off")?;
    let way = Way::parse(case.way);
    let size = size_for(SLOW_BYTES_PER_SECOND, 256 * MIB, 3 * GIB);
    let scratch = Scratch::new(&fast_dir()?, case.name);
    let source = scratch.path().join("large.bin");
    if text {
        text_file(&source, size);
    } else {
        random_file(&source, size);
    }
    let ratio = lz4_ratio(&source);
    let (required, words) = if text {
        (1.3, "≥ 1,3 × Rate ohne Kompression (Text)")
    } else {
        (0.95, "≥ 0,95 × Rate ohne Kompression (Zufallsdaten)")
    };
    if mode == "off" {
        let (rate, detail) = download(&case, way, &source, size)?;
        store(case.name, "off", rate)?;
        let mut line = ReportLine::new(&case, "B/s");
        line.size_bytes = size;
        line.se_rate = rate;
        line.requirement = words.into();
        line.notes
            .push(format!("Basislauf ohne LZ4 (SE_TEST_NO_LZ4=1): {detail}"));
        line.skipped
            .push("der Lauf mit LZ4 steht noch aus".to_string());
        finish(&line, None);
        return Ok(());
    }
    let plain = load_state()[case.name]["off"]
        .as_f64()
        .ok_or("der Basislauf ohne LZ4 fehlt in SE_SPEED_STATE")?;
    let trial = retry_once(|| {
        let (rate, detail) = download(&case, way, &source, size).expect("the download runs");
        let mut line = ReportLine::new(&case, "B/s");
        line.size_bytes = size;
        line.se_rate = rate;
        line.against(plain, required, words);
        line.reference_tool = "gleicher Lauf gegen einen Wirt ohne LZ4".into();
        line.reference_rate = plain;
        line.notes.push(format!(
            "LZ4-Verhältnis der Datei ≈ {ratio:.2}; mit LZ4: {detail}"
        ));
        let failure = shortfall(
            case.name,
            rate,
            plain,
            required,
            "Rate ohne Kompression",
            "Lauf gegen einen Wirt mit SE_TEST_NO_LZ4=1",
            "B/s",
        );
        Trial { line, failure }
    });
    finish(&trial.line, trial.failure.as_deref());
    Ok(())
}

/// A Share download on `loss` or `wan` with one controller; the case reports
/// both rates and their ratio once the second controller ran.
pub fn run_cc(case: Case) -> Result<(), String> {
    check_profile(&case)?;
    let cc = need("SE_SPEED_CC", "cubic oder bbr3")?;
    let link = link_capacity(Way::Share, case.profile)?;
    let size = size_for(link.bps, 256 * MIB, 3 * GIB);
    let scratch = Scratch::new(&fast_dir()?, case.name);
    let source = scratch.path().join("large.bin");
    random_file(&source, size);
    let (rate, detail) = download(&case, Way::Share, &source, size)?;
    store(case.name, &cc, rate)?;
    let state = load_state();
    let other = |name: &str| state[case.name][name].as_f64();
    let mut line = ReportLine::new(&case, "B/s");
    line.size_bytes = size;
    line.se_rate = rate;
    line.requirement = "nur Bericht und Entscheidungsflagge (U44-11)".into();
    line.reference_tool = link.source.clone();
    line.notes.push(format!("Regler {cc}: {detail}"));
    if let (Some(cubic), Some(bbr3)) = (other("cubic"), other("bbr3")) {
        line.capacity = cubic;
        line.share = bbr3 / cubic;
        line.notes.push(format!(
            "bbr3 {:.2} MB/s, cubic {:.2} MB/s, Verhältnis bbr3 ÷ cubic = {:.2}",
            bbr3 / 1e6,
            cubic / 1e6,
            bbr3 / cubic
        ));
    } else {
        line.skipped
            .push("der Lauf mit dem anderen Regler steht noch aus".to_string());
    }
    finish(&line, None);
    Ok(())
}

macro_rules! lz4_cases {
    ($($name:ident => ($way:literal, $text:literal)),* $(,)?) => {
        $(
            #[test]
            #[ignore = "needs the stage's host pair and the slow link"]
            fn $name() {
                let case = Case::new(stringify!($name), $way, "down", "slow");
                guarded(case.name, || run_lz4(case, $text));
            }
        )*
    };
}

lz4_cases! {
    speed_task_share_lz4_text_slow => ("share", true),
    speed_task_share_lz4_random_slow => ("share", false),
    speed_task_agent_lz4_text_slow => ("agent", true),
}

#[test]
#[ignore = "needs the stage's host pair and the lossy link"]
fn speed_task_share_cc_bbr3_vs_cubic_loss() {
    let case = Case::new(
        "speed_task_share_cc_bbr3_vs_cubic_loss",
        "share",
        "down",
        "loss",
    );
    guarded(case.name, || run_cc(case));
}

#[test]
#[ignore = "needs the stage's host pair and the wan link"]
fn speed_task_share_cc_bbr3_vs_cubic_wan() {
    let case = Case::new(
        "speed_task_share_cc_bbr3_vs_cubic_wan",
        "share",
        "down",
        "wan",
    );
    guarded(case.name, || run_cc(case));
}
