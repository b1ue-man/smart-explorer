//! The intents of a whole packet as one durable record (one sync for all
//! members). A member carries what is already known of it: the remote source
//! as a host reply described it (download packets), whether its destination
//! existed before (as the host reported it), and, for a local packet member
//! staged before the record (R4, 2026-10-28), its stage's evidence and its
//! destination, so a crash after the record leaves no stage the resumed job
//! does not know, and the acknowledgement compares through the stage handle.
use super::super::file_evidence::{HostEvidence, Proof};
use super::super::queue::FileWork;
use super::super::Engine;
use super::{active, failure, previous_known, source_guard};
use std::io;

/// One member of a packet whose intent is written with all the others.
pub(in crate::transfer::engine) struct PacketIntent<'f> {
    pub file: &'f FileWork,
    /// The remote source as a host reply described it (download packets).
    pub source: Option<&'f HostEvidence>,
    /// Whether the destination existed before, as the host reported it.
    pub existed_before: Option<bool>,
    /// The member's private stage, recorded by metadata as the single
    /// file's observer records it; `None`: no stage yet (remote packets).
    pub stage: Option<Proof>,
    /// The destination the stage is published at first, in the target's
    /// path form; `None`: the member's planned path.
    pub destination: Option<String>,
}

/// Writes the intents of `members`; nothing is written when any member
/// cannot be prepared.
pub(in crate::transfer::engine) fn prepare_batch_known(
    engine: &Engine<'_>,
    members: &[PacketIntent<'_>],
) -> io::Result<()> {
    let Some(record) = &engine.job_record else {
        return Ok(());
    };
    active(engine)?;
    let mut intents = Vec::with_capacity(members.len());
    for member in members {
        let source = source_guard(engine, member.file, member.source)?;
        if !source.verified || source.directory || source.link {
            return Err(failure("Paketquelle des Dateibelegs ist unbestätigt"));
        }
        let destination = match &member.destination {
            Some(destination) => destination.clone(),
            None => engine.folders.path_of(&member.file.rel),
        };
        let previous = previous_known(engine.view.target, &destination, member.existed_before)?;
        let stage = member.stage.clone();
        intents.push(record.intent(member.file, source, stage, destination, None, previous)?);
    }
    record.start_files(&intents)?;
    active(engine)
}
