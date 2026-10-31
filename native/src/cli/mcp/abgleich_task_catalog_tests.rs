//! G3 (review 2026-10-31): the server instructions name the rights of the
//! connect flow as a change of Share trust, within Claude Code's 2 048
//! bytes.
use super::INSTRUCTIONS;

#[test]
fn abgleich_task_instructions_name_connection_rights_as_trust() {
    let trust = INSTRUCTIONS
        .split("Change Share trust")
        .nth(1)
        .and_then(|rest| rest.split("only when the user").next())
        .expect("the trust sentence");
    for named in [
        "access requests",
        "connection rights",
        "share grants confirm|set",
        "--preset",
        "Exec grants",
        "discoverability",
    ] {
        assert!(trust.contains(named), "{named} missing in: {trust}");
    }
    assert!(INSTRUCTIONS.len() <= 2_048, "{}", INSTRUCTIONS.len());
}
