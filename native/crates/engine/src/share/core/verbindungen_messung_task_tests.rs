//! Measurements M1 to M4, M8 and M9 of plan 2026-11-14 (C7, P7), each case a
//! test of its own that prints one line
//! `VERBINDUNGEN-MESSUNG fall=<M> plattform=<..> variante=<..> profil=<..> <size>=<value> ...`
//! (`verbindungen_messung_support.rs`). All tests are `#[ignore]` and run only
//! in the part `verbindungen` of the suite `crates-task`, one after another
//! (`--test-threads=1`), because they measure the speed of the runner:
//! `cargo test --profile speed -p smart_explorer_engine --lib
//! verbindungen_messung_task_ -- --ignored --nocapture --test-threads=1`.
//! The link of a case is the netem rule the suite sets on `lo`
//! (`SE_SPEED_PROFILE` names it); without one the loopback runs unshaped.
//! A test fails only when it cannot measure; the thresholds (E2, E4, E7) are
//! judged by the suite from the lines. Each variant takes the profile
//! switches (`transport_purpose_overrides.rs`) it needs through `hold`, which
//! also keeps two measurements of one binary apart.
use std::num::NonZeroUsize;
use std::time::Duration;

use super::verbindungen_messung_support::bulk::{
    browsing, bulk_throughput, constancy, ConstancyCase,
};
use super::verbindungen_messung_support::tunnel::{exec_setup, tunnel_throughput};
use super::verbindungen_messung_support::{field, line, one_decimal, run_case};
use crate::share::keepalive::transport_purpose::overrides::{hold, ProfileOverrides};

/// Short jobs of M8.
const EXEC_JOBS: usize = 20;

fn count(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).expect("a send batch is never zero")
}

/// M1 and M9: `connections` connections of the `Files` profile with the send
/// batch `segments`/`datagrams`, the ACK frequency extension on or off.
fn bulk_case(
    fall: &str,
    variant: &str,
    connections: usize,
    (segments, datagrams): (usize, usize),
    ack_frequency: bool,
) -> Result<(), String> {
    let _plan = hold(ProfileOverrides {
        transmit: Some((count(segments), count(datagrams))),
        ack_frequency: Some(ack_frequency),
        ..ProfileOverrides::default()
    });
    let result = run_case(bulk_throughput(connections))?;
    println!(
        "{}",
        line(
            fall,
            variant,
            vec![
                field("verbindungen", connections),
                field("segmente", segments),
                field("datagramme", datagrams),
                field("ack", u8::from(ack_frequency)),
                field("gezaehlt_mib", result.counted_mib),
                field("mib_s", one_decimal(result.mib_s)),
            ]
        )
    );
    Ok(())
}

/// M1 "vorher": one connection, send batch 10/20, no ACK frequency.
#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m1_c1_b10x20() -> Result<(), String> {
    bulk_case("M1", "c1_b10x20", 1, (10, 20), false)
}

#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m1_c1_b40x80() -> Result<(), String> {
    bulk_case("M1", "c1_b40x80", 1, (40, 80), false)
}

#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m1_c4_b10x20() -> Result<(), String> {
    bulk_case("M1", "c4_b10x20", 4, (10, 20), false)
}

#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m1_c4_b40x80() -> Result<(), String> {
    bulk_case("M1", "c4_b40x80", 4, (40, 80), false)
}

/// M9: the ACK frequency extension on; "vorher" is M1 `c1_b40x80`.
#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m9_c1_b40x80_ack() -> Result<(), String> {
    bulk_case("M9", "c1_b40x80_ack", 1, (40, 80), true)
}

/// M2 with `switches` in force; `target` is the queueing delay target T.
fn constancy_case(
    variant: &str,
    switches: ProfileOverrides,
    case: ConstancyCase,
) -> Result<(), String> {
    let target = switches.target;
    let _plan = hold(switches);
    let result = run_case(constancy(case))?;
    println!(
        "{}",
        line(
            "M2",
            variant,
            vec![
                field("bild", u8::from(case.picture)),
                field("uebertragung", u8::from(case.transfer)),
                field("rueckzug", u8::from(switches.yielding != Some(false))),
                field("t_ms", target.map_or(0, |value| value.as_millis())),
                field("bilder", result.frames),
                field("bild_p50_ms", one_decimal(result.p50_ms)),
                field("bild_p95_ms", one_decimal(result.p95_ms)),
                field("bulk_mib_s", one_decimal(result.bulk_mib_s)),
            ]
        )
    );
    Ok(())
}

const PICTURE_ALONE: ConstancyCase = ConstancyCase {
    picture: true,
    transfer: false,
    presence: true,
};
const PICTURE_WITH_TRANSFER: ConstancyCase = ConstancyCase {
    picture: true,
    transfer: true,
    presence: true,
};
const TRANSFER_ALONE: ConstancyCase = ConstancyCase {
    picture: false,
    transfer: true,
    presence: false,
};

fn plain_cubic() -> ProfileOverrides {
    ProfileOverrides {
        yielding: Some(false),
        ..ProfileOverrides::default()
    }
}

fn yielding_to(millis: u64) -> ProfileOverrides {
    ProfileOverrides {
        yielding: Some(true),
        target: Some(Duration::from_millis(millis)),
        ..ProfileOverrides::default()
    }
}

/// The picture alone: the base of the picture latency.
#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m2_allein() -> Result<(), String> {
    constancy_case("allein", plain_cubic(), PICTURE_ALONE)
}

/// "vorher": Cubic without yielding next to a download and an upload.
#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m2_cubic() -> Result<(), String> {
    constancy_case("cubic", plain_cubic(), PICTURE_WITH_TRANSFER)
}

#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m2_rueckzug_t15() -> Result<(), String> {
    constancy_case("rueckzug_t15", yielding_to(15), PICTURE_WITH_TRANSFER)
}

#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m2_rueckzug_t25() -> Result<(), String> {
    constancy_case("rueckzug_t25", yielding_to(25), PICTURE_WITH_TRANSFER)
}

#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m2_rueckzug_t50() -> Result<(), String> {
    constancy_case("rueckzug_t50", yielding_to(50), PICTURE_WITH_TRANSFER)
}

/// Transfer alone with Cubic: the base of "at least 95 %" (E2).
#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m2_bulk_cubic() -> Result<(), String> {
    constancy_case("bulk_cubic", plain_cubic(), TRANSFER_ALONE)
}

/// Transfer alone with the yielding controller (no interactive session, so it
/// must behave like Cubic).
#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m2_bulk_rueckzug() -> Result<(), String> {
    constancy_case("bulk_rueckzug", yielding_to(25), TRANSFER_ALONE)
}

/// M3 with `switches` in force.
fn browsing_case(variant: &str, switches: ProfileOverrides) -> Result<(), String> {
    let priority = switches.transfer_priority != Some(false);
    let _plan = hold(switches);
    let result = run_case(browsing())?;
    println!(
        "{}",
        line(
            "M3",
            variant,
            vec![
                field("prioritaet", u8::from(priority)),
                field("anfragen", result.requests),
                field("liste_p50_ms", one_decimal(result.p50_ms)),
                field("liste_p95_ms", one_decimal(result.p95_ms)),
                field("bulk_mib_s", one_decimal(result.bulk_mib_s)),
            ]
        )
    );
    Ok(())
}

/// "vorher": transfer streams at priority 0, no yielding.
#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m3_vorher() -> Result<(), String> {
    browsing_case(
        "vorher",
        ProfileOverrides {
            yielding: Some(false),
            transfer_priority: Some(false),
            ..ProfileOverrides::default()
        },
    )
}

/// "nachher": transfer streams at priority -1 (E4).
#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m3_prio() -> Result<(), String> {
    browsing_case("prio", ProfileOverrides::default())
}

/// M4 with the copy mode `legacy` (before E7) or the production pump.
fn tunnel_case(variant: &str, legacy: bool) -> Result<(), String> {
    let _plan = hold(ProfileOverrides::default());
    let mib_s = run_case(tunnel_throughput(legacy))?;
    println!(
        "{}",
        line("M4", variant, vec![field("mib_s", one_decimal(mib_s))])
    );
    Ok(())
}

/// "vorher": 8 KiB per call, one route wrapper per operation.
#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m4_vorher() -> Result<(), String> {
    tunnel_case("vorher", true)
}

#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m4_nachher() -> Result<(), String> {
    tunnel_case("nachher", false)
}

/// M8: `EXEC_JOBS` short jobs, a connection each or one for all.
fn exec_case(variant: &str, fresh: bool) -> Result<(), String> {
    let _plan = hold(ProfileOverrides::default());
    let result = run_case(exec_setup(fresh, EXEC_JOBS))?;
    println!(
        "{}",
        line(
            "M8",
            variant,
            vec![
                field("auftraege", result.jobs),
                field("auftrag_p50_ms", one_decimal(result.p50_ms)),
                field("auftrag_p95_ms", one_decimal(result.p95_ms)),
                field("gesamt_ms", one_decimal(result.total_ms)),
            ]
        )
    );
    Ok(())
}

/// Today: each job makes its own connection (`node_routes.rs`).
#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m8_neue_verbindung() -> Result<(), String> {
    exec_case("neue_verbindung", true)
}

/// The condition of B3: all jobs on one connection.
#[test]
#[ignore = "Messung; Suite crates-task"]
fn verbindungen_messung_task_m8_bestehende_verbindung() -> Result<(), String> {
    exec_case("bestehende_verbindung", false)
}
