//! The reply `Decided` of a daemon of an older version carries no count
//! (E28, E59; TODO `SEC-ZUGANG-RUECKFRAGEN`): it reads as 0 answered too,
//! and a count that is sent is kept.
use super::AccessIpcReply;

#[test]
fn rueckfragen_task_decided_without_count_reads_as_zero() {
    let older: AccessIpcReply = serde_json::from_str(r#"{"kind":"decided"}"#).unwrap();
    assert_eq!(older, AccessIpcReply::Decided { also_answered: 0 });
    let counted: AccessIpcReply =
        serde_json::from_str(r#"{"kind":"decided","also_answered":2}"#).unwrap();
    assert_eq!(counted, AccessIpcReply::Decided { also_answered: 2 });
}
