//! Per-file publication receipts. A private stage's object identity and
//! content digest are recorded durably before publication; equal size never
//! establishes ownership during recovery.
use super::job_store::{endpoint, snapshot, with_backend, JobRun};
use super::ops::{At, OpError, Outcome};
use super::queue::FileWork;
use super::record_io::{read_record, write_record, RecordContext, RecordKind};
use super::view::{JobView, Side};
use super::Engine;
use crate::transfer::job_record::{JobRecord, SourceGuard, StoredEndpoint};
use ring::digest::SHA256;
use super::file_evidence::{observe, observe_source, verify, Proof};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Receipt {
    version: u32,
    job: String,
    source_endpoint: StoredEndpoint,
    target_endpoint: StoredEndpoint,
    target_root: String,
    key: String,
    source: SourceGuard,
    relative: String,
    destination: String,
    proof: Option<Proof>,
    #[serde(default)]
    quarantine: Option<Proof>,
    #[serde(default)]
    staged: Option<Proof>,
    confirmed: bool,
    #[serde(default)]
    discarded: bool,
    #[serde(default = "authorized_default")]
    publication_authorized: bool,
    #[serde(default)]
    previous: Option<PreviousTarget>,
}
fn authorized_default() -> bool { true }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum PreviousTarget { Absent, File(Proof), Unconfirmed }


#[derive(Serialize, Deserialize)]
struct Index { job: String, source: StoredEndpoint, target: StoredEndpoint, root: String, count: u64, #[serde(default)] pending: Option<Receipt> }

fn failure(message: impl Into<String>) -> io::Error { io::Error::other(message.into()) }
fn active(engine: &Engine<'_>) -> io::Result<()> {
    if engine.stopped() { Err(io::Error::new(io::ErrorKind::Interrupted, super::super::cancel::CANCELED_ERROR)) } else { Ok(()) }
}
fn digest(bytes: &[u8]) -> String {
    ring::digest::digest(&SHA256, bytes).as_ref().iter().map(|byte| format!("{byte:02x}")).collect()
}
fn key(file: &FileWork) -> String {
    file.receipt_key.clone().unwrap_or_else(|| format!("{}\0{}", file.source, file.rel.split_once('/').map_or("", |(_, tail)| tail)))
}
fn directory(job_path: &Path) -> PathBuf { job_path.with_extension("files") }
pub(super) fn initialize(job_path: &Path, record: &JobRecord, context: Option<&RecordContext>) -> io::Result<()> {
    crate::support_dirs::ensure_private_dir(&directory(job_path))?;
    match read_index(job_path, record, context) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound && !record.mutation_started => write_record(&directory(job_path).join("index.json"), RecordKind::Job, &record.id, context, &Index { job: record.id.clone(), source: record.source.clone(), target: record.target.clone(), root: record.target_dir.clone(), count: 0, pending: None }),
        Err(error) => Err(error),
    }
}
fn read_index(job_path: &Path, record: &JobRecord, context: Option<&RecordContext>) -> io::Result<Index> {
    let index: Index = read_record(&directory(job_path).join("index.json"), RecordKind::Job, &record.id, u64::MAX, context.is_some())?;
    if index.job != record.id || index.source != record.source || index.target != record.target || index.root != record.target_dir { return Err(failure("Dateijournalindex gehört zu einem abweichenden Auftrag")); }
    Ok(index)
}
fn receipt_path(job_path: &Path, key: &str) -> PathBuf { directory(job_path).join(format!("{}.json", digest(key.as_bytes()))) }

fn repair_new_entry(job_path: &Path, record: &JobRecord, context: Option<&RecordContext>, index: &mut Index) -> io::Result<()> {
    let Some(intent) = index.pending.clone() else { return Ok(()); };
    validate(&intent, record)?;
    let candidate = receipt_path(job_path, &intent.key);
    let mut unapproved = intent.clone(); unapproved.publication_authorized = false;
    let mut committed = 0u64;
    for entry in std::fs::read_dir(directory(job_path))? {
        let entry = entry?;
        if entry.file_name() == "index.json" || entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") { continue; }
        let receipt: Receipt = read_record(&entry.path(), RecordKind::Job, &record.id, u64::MAX, context.is_some())?;
        validate(&receipt, record)?;
        if receipt_path(job_path, &receipt.key) != entry.path() { return Err(failure("Abweichender Dateijournalschlüssel")); }
        if entry.path() == candidate {
            if receipt != intent && receipt != unapproved { return Err(failure("Unbestätigter Journalneueintrag wurde verändert")); }
        } else { committed = committed.checked_add(1).ok_or_else(|| failure("Dateijournalzähler übergelaufen"))?; }
    }
    // The intent identifies exactly one never-authorized new key. Every
    // previously committed key must still be present; never recount loss.
    if committed != index.count { return Err(failure("Bestehende Dateibelege fehlen während Journalwiederaufnahme")); }
    write_record(&candidate, RecordKind::Job, &record.id, context, &unapproved)?;
    index.count = index.count.checked_add(1).ok_or_else(|| failure("Dateijournalzähler übergelaufen"))?;
    index.pending = None;
    write_record(&directory(job_path).join("index.json"), RecordKind::Job, &record.id, context, index)
}

fn previous_target(side: Side<'_>, path: &str, cancel: &AtomicBool) -> io::Result<PreviousTarget> {
    match with_backend(side, |backend| backend.stat(path)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(PreviousTarget::Absent),
        Err(error) if matches!(error.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported) => Ok(PreviousTarget::Unconfirmed),
        Err(error) => Err(error),
        Ok(meta) if meta.is_dir || meta.is_symlink || meta.special => Ok(PreviousTarget::Unconfirmed),
        Ok(_) => match observe(side, path, cancel) {
            Ok(proof) => Ok(PreviousTarget::File(proof)),
            Err(error) if matches!(error.kind(), io::ErrorKind::PermissionDenied | io::ErrorKind::Unsupported) => Ok(PreviousTarget::Unconfirmed),
            Err(error) => Err(error),
        },
    }
}

fn previous_matches(receipt: &Receipt, side: Side<'_>, cancel: &AtomicBool) -> io::Result<bool> {
    match &receipt.previous {
        Some(PreviousTarget::Absent) => match with_backend(side, |backend| backend.stat(&receipt.destination)) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true), Err(error) => Err(error), Ok(_) => Ok(false),
        },
        Some(PreviousTarget::File(proof)) => match verify(proof, side, cancel, true) {
            Ok(()) => Ok(true), Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error), Err(_) => Ok(false),
        },
        _ => Ok(false),
    }
}

fn publication_state(receipt: &Receipt, side: Side<'_>, cancel: &AtomicBool) -> io::Result<bool> {
    // A pending new-key intent, failed preparation, or live-proven failed
    // publication can write this durable negative acknowledgement.
    if !receipt.publication_authorized { return Ok(false); }
    let Some(proof) = &receipt.proof else {
        return if receipt.quarantine.is_some() { Ok(false) } else { Err(failure(format!("Veröffentlichungsergebnis bleibt unbestätigt: {}", receipt.destination))) };
    };
    if receipt.publication_authorized && proof.object.is_none() {
        return Err(failure(format!("Dateieigentum ohne stabile Objektidentität ist unbestätigt: {}", proof.path)));
    }
    match verify(proof, side, cancel, true) {
        Ok(()) if receipt.publication_authorized => return Ok(true),
        Err(error) if receipt.confirmed || error.kind() == io::ErrorKind::Interrupted => return Err(error),
        _ => {},
    }
    let stage = receipt.staged.as_ref().ok_or_else(|| failure("Vorpublikationszustand ohne eigene Stage"))?;
    let stage_present = match with_backend(side, |backend| backend.stat(&stage.path)) {
        Ok(_) => { verify(stage, side, cancel, true)?; true },
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if (!stage_present && receipt.publication_authorized) || !previous_matches(receipt, side, cancel)? {
        return Err(failure(format!("Zielersatz oder unbestätigte Publikationsphase: {}", receipt.destination)));
    }
    Ok(false)
}

fn destination_parts(record: &JobRecord, destination: &str) -> io::Result<(String, String)> {
    let display = |path: &str| if record.target.local { crate::copy::native_path_text(&super::super::walk_listers::native(path)) } else { Ok(path.to_string()) };
    let root = display(&record.target_dir)?;
    let path = display(destination)?;
    let relative = path.strip_prefix(&format!("{}/", root.trim_end_matches('/'))).ok_or_else(|| failure("Dateibeleg liegt außerhalb des Zielroots"))?.to_string();
    let destination = super::super::engine_names::join_rel(&record.target_dir, &relative);
    Ok((relative, destination))
}


fn validate(receipt: &Receipt, record: &JobRecord) -> io::Result<()> {
    if receipt.version != 1 || receipt.job != record.id || receipt.source_endpoint != record.source || receipt.target_endpoint != record.target || receipt.target_root != record.target_dir {
        return Err(failure("Dateibeleg gehört zu einer abweichenden Verbindung oder einem abweichenden Auftrag"));
    }
    let allow_backslash = record.target.local && super::super::platform::backslash_is_name_char();
    if receipt.relative.is_empty() || receipt.relative.starts_with('/') || receipt.relative.split('/').any(|part| super::super::engine_names::name_problem(part, allow_backslash).is_some()) || super::super::engine_names::join_rel(&record.target_dir, &receipt.relative) != receipt.destination || receipt.proof.as_ref().is_some_and(|proof| proof.path != receipt.destination) || receipt.previous.as_ref().is_some_and(|previous| matches!(previous, PreviousTarget::File(proof) if proof.path != receipt.destination)) {
        return Err(failure("Dateibeleg enthält einen ungültigen Zielpfad"));
    }
    Ok(())
}

fn visit_saved(record: &JobRecord, job_path: &Path, context: Option<&RecordContext>, mut visit: impl FnMut(Receipt) -> io::Result<()>) -> io::Result<()> {
    let mut index = read_index(job_path, record, context)?;
    repair_new_entry(job_path, record, context, &mut index)?;
    let mut count = 0u64;
    let entries = match std::fs::read_dir(directory(job_path)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound && !record.mutation_started => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_name() == "index.json" || entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") { continue; }
        count = count.checked_add(1).ok_or_else(|| failure("Dateijournalzähler übergelaufen"))?;
        let receipt: Receipt = read_record(&entry.path(), RecordKind::Job, &record.id, u64::MAX, context.is_some())?;
        validate(&receipt, record)?;
        if receipt_path(job_path, &receipt.key) != entry.path() { return Err(failure("Dateibeleg hat einen abweichenden Speicherschlüssel")); }
        visit(receipt)?;
    }
    if count != index.count { return Err(failure("Dateijournal ist unvollständig oder enthält unbestätigte Belege")); }
    Ok(())
}

pub(super) type FileRoots = std::collections::BTreeMap<String, (SourceGuard, crate::transfer::ResolvedRoot)>;

fn selected_key(record: &JobRecord, receipt: &Receipt) -> String {
    match &record.items { crate::transfer::JobItems::Pairs(_) => receipt.key.clone(), _ => receipt.source.path.clone() }
}

pub(super) fn verify_saved(view: &JobView<'_>, record: &JobRecord, job_path: &Path, context: Option<&RecordContext>, cancel: &AtomicBool) -> io::Result<FileRoots> {
    if endpoint(view.source) != record.source || endpoint(view.target) != record.target { return Err(failure("Dateiwiederaufnahme hat eine abweichende Quellen-/Zielverbindung")); }
    let mut roots = FileRoots::new();
    let selected: std::collections::HashSet<&str> = record.guards.iter().filter(|guard| !guard.directory).map(|guard| guard.path.as_str()).collect();
    let mut recovery = Vec::new();
    // Validate every receipt before any quarantine cleanup or restoration.
    visit_saved(record, job_path, context, |receipt| {
        if receipt.discarded { return Ok(()); }
        let published = publication_state(&receipt, view.target, cancel)?;
        let quarantine_exists = match &receipt.quarantine {
            Some(quarantine) => match with_backend(view.source, |backend| backend.stat(&quarantine.path)) {
                Ok(_) => { verify(quarantine, view.source, cancel, true)?; true },
                Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                Err(error) => return Err(error),
            },
            None => false,
        };
        match with_backend(view.source, |backend| backend.stat(&receipt.source.path)) {
            Ok(_) => verify_source(&receipt, view.source, cancel)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound && record.moving && (published || quarantine_exists) => {},
            Err(error) => return Err(error),
        }
        if !published {
            if let Some(stage) = &receipt.staged {
                match with_backend(view.target, |backend| backend.stat(&stage.path)) {
                    Ok(_) => verify(stage, view.target, cancel, true)?,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {},
                    Err(error) => return Err(error),
                }
            }
        } else if selected.contains(receipt.source.path.as_str()) {
            let key = selected_key(record, &receipt);
            if roots.insert(key.clone(), (receipt.source.clone(), crate::transfer::ResolvedRoot { source: key, rel: receipt.relative.clone() })).is_some() { return Err(failure("Mehrdeutige Dateizielbelege in der Auswahl")); }
        }
        if quarantine_exists || !published { recovery.push((receipt, published, quarantine_exists)); }
        Ok(())
    })?;
    for (receipt, published, quarantine_exists) in recovery {
        if cancel.load(Ordering::Acquire) { return Err(io::Error::new(io::ErrorKind::Interrupted, "Dateiwiederaufnahme abgebrochen")); }
        if quarantine_exists {
            if !view.source.is_local() { return Err(failure("Movequarantäne ist nicht lokal")); }
            let quarantine = receipt.quarantine.as_ref().ok_or_else(|| failure("Movequarantäne fehlt"))?;
            let identity = quarantine.object.as_deref().ok_or_else(|| failure("Movequarantäne ohne Objektidentität"))?;
            if published {
                let proof = receipt.proof.as_ref().ok_or_else(|| failure("Publiziertes Moveziel ohne Beleg"))?;
                if !view.target.is_local() { return Err(failure("Moveziel ist nicht lokal")); }
                crate::copy::confirm_transfer_publication(&super::super::walk_listers::native(&proof.path), proof.object.as_deref().ok_or_else(|| failure("Moveziel ohne Objektidentität"))?, &proof.sha256, cancel)?;
                verify(proof, view.target, cancel, true)?;
                let root = with_backend(view.target, |backend| crate::vfs::directory_identity(backend, &record.target_dir))?;
                if root.is_none() || root != record.target_object { return Err(failure("Movezielroot während der Dauerhaftigkeitsbestätigung geändert")); }
            }
            crate::copy::recover_transfer_quarantine(&super::super::walk_listers::native(&receipt.source.path), &super::super::walk_listers::native(&quarantine.path), identity, published)?;
        }
        if !published {
            verify_source(&receipt, view.source, cancel)?;
            if let Some(stage) = &receipt.staged {
                match with_backend(view.target, |backend| backend.stat(&stage.path)) {
                    Ok(_) if view.target.is_local() => crate::copy::discard_transfer_stage(&super::super::walk_listers::native(&stage.path), stage.object.as_deref().ok_or_else(|| failure("Private Stage ohne Objektidentität"))?)?,
                    Ok(_) => return Err(failure(format!("Private Remotestage bleibt bis zur bestätigten providergebundenen Bereinigung erhalten: {}", stage.path))),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {},
                    Err(error) => return Err(error),
                }
            }
            let mut receipt = receipt;
            receipt.discarded = true;
            receipt.proof = None; receipt.staged = None; receipt.quarantine = None;
            write_record(&receipt_path(job_path, &receipt.key), RecordKind::Job, &record.id, context, &receipt)?;
        }
    }
    Ok(roots)
}

fn verify_source(receipt: &Receipt, side: Side<'_>, cancel: &AtomicBool) -> io::Result<()> {
    let after = observe_source(side, &receipt.source.path, receipt.source.id.as_deref(), cancel)?;
    if after.id != receipt.source.id || after.object != receipt.source.object || after.size != receipt.source.size || after.mtime_ms != receipt.source.mtime_ms {
        return Err(failure(format!("Quelle des Dateibelegs wurde geändert: {}", receipt.source.path)));
    }
    if let Some(proof) = receipt.proof.as_ref().or(receipt.quarantine.as_ref()) {
        if after.sha256 != proof.sha256 { return Err(failure(format!("Quellinhalt des Dateibelegs wurde geändert: {}", receipt.source.path))); }
    }
    Ok(())
}


impl JobRun {
    fn read_file(&self, file: &FileWork) -> io::Result<Option<Receipt>> {
        let data = super::lock(&self.data);
        self.read_file_unlocked(file, &data)
    }
    fn read_file_unlocked(&self, file: &FileWork, data: &JobRecord) -> io::Result<Option<Receipt>> {
        let path = receipt_path(&self.path, &key(file));
        match read_record::<Receipt>(&path, RecordKind::Job, &data.id, u64::MAX, self.context.is_some()) {
            Ok(receipt) => { validate(&receipt, &data)?; if receipt.key != key(file) || receipt.source.path != file.source { return Err(failure("Dateibeleg passt nicht zur Auswahl")); } Ok(Some(receipt)) }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
    fn write_file(&self, receipt: &Receipt) -> io::Result<()> {
        write_record(&receipt_path(&self.path, &receipt.key), RecordKind::Job, &receipt.job, self.context.as_ref(), receipt)
    }
    fn start_file(&self, file: &FileWork, source: SourceGuard, staged: Option<Proof>, destination: String, quarantine: Option<Proof>, previous_target: PreviousTarget) -> io::Result<()> {
        let data = super::lock(&self.data);
        let (relative, destination) = destination_parts(&data, &destination)?;
        let mut index = read_index(&self.path, &data, self.context.as_ref())?;
        repair_new_entry(&self.path, &data, self.context.as_ref(), &mut index)?;
        let previous = self.read_file_unlocked(file, &data)?;
        let new_key = previous.is_none();
        let stage = staged.clone();
        let mut proof = staged;
        if let Some(proof) = &mut proof { proof.path = destination.clone(); }
        let mut previous_target = previous_target;
        if let PreviousTarget::File(proof) = &mut previous_target { proof.path = destination.clone(); }
        let receipt = Receipt { version: 1, job: data.id.clone(), source_endpoint: data.source.clone(), target_endpoint: data.target.clone(), target_root: data.target_dir.clone(), key: key(file), source, relative, destination, proof, quarantine: quarantine.or_else(|| previous.and_then(|receipt| receipt.quarantine)), staged: stage, confirmed: false, discarded: false, publication_authorized: true, previous: Some(previous_target) };
        validate(&receipt, &data)?;
        if new_key {
            index.pending = Some(receipt.clone());
            write_record(&directory(&self.path).join("index.json"), RecordKind::Job, &data.id, self.context.as_ref(), &index)?;
        }
        self.write_file(&receipt)?;
        if new_key {
            index.pending = None;
            index.count = index.count.checked_add(1).ok_or_else(|| failure("Dateijournalzähler übergelaufen"))?;
            write_record(&directory(&self.path).join("index.json"), RecordKind::Job, &data.id, self.context.as_ref(), &index)?;
        }
        Ok(())
    }
}

pub(super) fn resume(engine: &Engine<'_>, file: &FileWork) -> Result<Option<Outcome>, OpError> {
    let Some(record) = &engine.job_record else { return Ok(None); };
    let Some(receipt) = record.read_file(file).map_err(|error| OpError::at(At::Unknown, error))? else { return Ok(None); };
    if receipt.discarded { return Ok(None); }
    if !receipt.publication_authorized { return Err(OpError::at(At::Unknown, failure("Journalneueintrag hat keine bestätigte Publikationserlaubnis"))); }
    let result = (|| {
        let proof = receipt.proof.as_ref().ok_or_else(|| failure("Dateipublikation bleibt unbestätigt"))?;
        verify(proof, engine.view.target, engine.stop_flag(), true)?;
        verify_source(&receipt, engine.view.source, engine.stop_flag())?;
        Ok(())
    })();
    result.map_err(|error| OpError::at(At::Unknown, error))?;
    if super::super::engine_names::parent_rel(&file.rel).is_none() { engine.folders.record_alias(&file.rel, super::super::engine_names::base_name(&receipt.destination)); }
    Ok(Some(Outcome::Skipped))
}

pub(super) fn before(engine: &Engine<'_>, file: &FileWork, staged: &str, destination: &str) -> io::Result<()> {
    let Some(record) = &engine.job_record else { return Ok(()); };
    active(engine)?;
    let source = snapshot(engine.view.source, &file.source);
    if !source.verified || source.directory || source.link { return Err(failure("Quelle des Dateibelegs ist unbestätigt")); }
    let proof = observe(engine.view.target, staged, engine.stop_flag())?;
    record.start_file(file, source, Some(proof), destination.to_string(), None, previous_target(engine.view.target, destination, engine.stop_flag())?)?;
    active(engine)
}

pub(super) fn after(engine: &Engine<'_>, file: &FileWork, destination: &str) -> io::Result<()> {
    after_checked(engine, file, destination, None)
}

fn after_checked(engine: &Engine<'_>, file: &FileWork, destination: &str, held: Option<&std::fs::File>) -> io::Result<()> {
    let Some(record) = &engine.job_record else { return Ok(()); };
    let mut receipt = record.read_file(file)?.ok_or_else(|| failure("Dateipublikation hat keinen vorbereiteten Beleg"))?;
    if let Some(proof) = &receipt.proof {
        let held = held.filter(|_| engine.is_move() || engine.view.conflict == crate::types::Conflict::Overwrite);
        if let (Some(held), Some(identity)) = (held, proof.object.as_deref()) {
            crate::copy::confirm_transfer_publication_handle(&super::super::walk_listers::native(destination), held, identity, &proof.sha256, engine.stop_flag())?;
        } else { verify(proof, engine.view.target, engine.stop_flag(), false)?; }
    }
    else {
        let proof = observe(engine.view.target, destination, engine.stop_flag())?;
        let (relative, destination) = destination_parts(&super::lock(&record.data), destination)?;
        receipt.destination = destination;
        receipt.relative = relative;
        receipt.previous = None;
        receipt.proof = Some(proof);
        verify_source(&receipt, engine.view.source, engine.stop_flag())?;
    }
    receipt.confirmed = receipt.proof.as_ref().is_some_and(|proof| proof.object.is_some());
    record.write_file(&receipt)
}

pub(super) fn prepare_batch(engine: &Engine<'_>, file: &FileWork) -> io::Result<()> {
    prepare_direct(engine, file, &engine.folders.path_of(&file.rel))
}

pub(super) fn prepare_direct(engine: &Engine<'_>, file: &FileWork, destination: &str) -> io::Result<()> {
    let Some(record) = &engine.job_record else { return Ok(()); };
    active(engine)?;
    let source = snapshot(engine.view.source, &file.source);
    if !source.verified || source.directory || source.link { return Err(failure("Paketquelle des Dateibelegs ist unbestätigt")); }
    record.start_file(file, source, None, destination.to_string(), None, previous_target(engine.view.target, destination, engine.stop_flag())?)?;
    active(engine)
}

pub(super) fn unpublished(engine: &Engine<'_>, file: &FileWork) -> io::Result<()> {
    let Some(record) = &engine.job_record else { return Ok(()); };
    let receipt = {
        let data = super::lock(&record.data);
        let mut index = read_index(&record.path, &data, record.context.as_ref())?;
        repair_new_entry(&record.path, &data, record.context.as_ref(), &mut index)?;
        record.read_file_unlocked(file, &data)?
    };
    let Some(mut receipt) = receipt else { return Ok(()); };
    if let Some(quarantine) = &receipt.quarantine {
        match with_backend(engine.view.source, |backend| backend.stat(&quarantine.path)) {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
    }
    receipt.discarded = true;
    receipt.proof = None; receipt.staged = None; receipt.quarantine = None;
    record.write_file(&receipt)
}

pub(super) fn preparation_failed(engine: &Engine<'_>, file: &FileWork) -> io::Result<()> {
    revoke(engine, file, None)
}

fn revoke(engine: &Engine<'_>, file: &FileWork, committed: Option<(&Path, &Path)>) -> io::Result<()> {
    let Some(record) = &engine.job_record else { return Ok(()); };
    let data = super::lock(&record.data);
    let mut index = read_index(&record.path, &data, record.context.as_ref())?;
    repair_new_entry(&record.path, &data, record.context.as_ref(), &mut index)?;
    if let Some(mut receipt) = record.read_file_unlocked(file, &data)? {
        if let Some((stage, target)) = committed {
            let stage = crate::copy::native_path_text(stage)?;
            let target = crate::copy::native_path_text(target)?;
            if receipt.confirmed || receipt.discarded || receipt.destination != target
                || receipt.staged.as_ref().map_or(true, |proof| proof.path != stage) {
                return Err(failure("Fehlgeschlagene Publikation passt nicht zum unbestätigten Stage-/Zielbeleg"));
            }
        }
        receipt.publication_authorized = false;
        receipt.confirmed = false;
        record.write_file(&receipt)?;
    } else if committed.is_some() { return Err(failure("Fehlgeschlagene Publikation hat keinen vorbereiteten Dateibeleg")); }
    Ok(())
}

pub(super) struct LocalObserver<'e, 'a> { engine: &'e Engine<'a>, file: &'e FileWork, source: SourceGuard }
impl<'e, 'a> LocalObserver<'e, 'a> {
    pub fn new(engine: &'e Engine<'a>, file: &'e FileWork) -> Self { Self { engine, file, source: snapshot(engine.view.source, &file.source) } }
}
impl crate::copy::PublicationObserver for LocalObserver<'_, '_> {
    fn publication_preparation_failed(&self, _: &Path, _: &Path) -> io::Result<()> {
        preparation_failed(self.engine, self.file)
    }
    fn publication_failed(&self, staged: &Path, target: &Path) -> io::Result<()> {
        revoke(self.engine, self.file, Some((staged, target)))
    }
    fn before_source_quarantine(&self, source: &Path, quarantine: &Path) -> io::Result<()> {
        let Some(record) = &self.engine.job_record else { return Ok(()); };
        active(self.engine)?;
        let source = crate::copy::native_path_text(source)?;
        let mut proof = observe(Side::Local, &source, self.engine.stop_flag())?;
        proof.path = crate::copy::native_path_text(quarantine)?;
        let destination = self.engine.folders.path_of(&self.file.rel);
        record.start_file(self.file, self.source.clone(), None, destination.clone(), Some(proof), previous_target(self.engine.view.target, &destination, self.engine.stop_flag())?)?;
        active(self.engine)
    }
    fn before_publish(&self, staged: &Path, target: &Path) -> io::Result<()> {
        let Some(record) = &self.engine.job_record else { return Ok(()); };
        active(self.engine)?;
        if !self.source.verified || self.source.directory || self.source.link { return Err(failure("Lokale Quelle des Dateibelegs ist unbestätigt")); }
        let staged = crate::copy::native_path_text(staged)?;
        let target = crate::copy::native_path_text(target)?;
        let proof = observe(Side::Local, &staged, self.engine.stop_flag())?;
        let previous = previous_target(self.engine.view.target, &target, self.engine.stop_flag())?;
        record.start_file(self.file, self.source.clone(), Some(proof), target, None, previous)?;
        active(self.engine)
    }
    fn after_publish(&self, target: &Path) -> io::Result<()> { after(self.engine, self.file, &crate::copy::native_path_text(target)?) }
    fn after_publish_handle(&self, target: &Path, held: &std::fs::File) -> io::Result<()> {
        after_checked(self.engine, self.file, &crate::copy::native_path_text(target)?, Some(held))
    }
}
