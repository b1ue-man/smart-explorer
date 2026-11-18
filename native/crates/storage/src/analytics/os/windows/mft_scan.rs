//! Windows: admission of the MFT reader for the caller's own local analysis
//! and the size tree through it. Runs on the scanning thread (the admission
//! holds for that thread only).
use super::{attach_system_files, report, TreeVisitor, MFT_WAY};
use crate::analytics::{Progress, ScanPhase};
use crate::local_access::mft::{FileRef, MftDiagnostics, MftIndex, MftSection, ROOT_RECORD};
use crate::local_access::platform::mft::{LocalElevatedScan, MftControl, MftRefusal};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use super::super::{AnalyticsBudget, Diagnostics, Retention, SizeNode};

/// The tree through the MFT, or the note that names the directory walk
/// (with the reason where administrator rights would have enabled it).
pub(in super::super) fn scan_local(
    root: &Path,
    name: &str,
    progress: &Progress,
    diagnostics: &Diagnostics,
    budget: &AnalyticsBudget,
    excluded: &[PathBuf],
) -> Result<SizeNode, String> {
    let scan = match LocalElevatedScan::for_local_analysis(root) {
        Ok(scan) => scan,
        Err(refusal @ (MftRefusal::NotElevated | MftRefusal::NoBackupPrivilege)) => {
            return Err(format!(
                "Analyseweg: Verzeichnislauf (der schnelle NTFS-Weg braucht Administratorrechte: {refusal})"
            ))
        }
        Err(_) => return Err("Analyseweg: Verzeichnislauf".into()),
    };
    let canceled = || progress.cancel.load(Ordering::Relaxed);
    let empty = || SizeNode {
        name: name.into(),
        size: 0,
        allocated: 0,
        is_dir: true,
        children: Vec::new(),
    };
    progress.set_phase(ScanPhase::Scanning, "NTFS-MFT wird gelesen");
    let read = |done: u64, total: u64| {
        if Progress::directory_due() {
            progress.set_phase(
                ScanPhase::Scanning,
                format!(
                    "NTFS-MFT wird gelesen: {} von {} MiB",
                    done >> 20,
                    total >> 20
                ),
            );
        }
    };
    let control = MftControl {
        cancel: &progress.cancel,
        progress: &read,
    };
    let index = match scan.read_index(&control) {
        Ok(index) => index,
        Err(_) if canceled() => return Ok(empty()),
        Err(error) => {
            return Err(format!(
                "Analyseweg: Verzeichnislauf (MFT nicht lesbar: {error})"
            ))
        }
    };
    let mut visitor = TreeVisitor::new(root, progress, diagnostics, budget, excluded);
    let (mut tree, _, found) = match walk_section(
        &index,
        scan.root(),
        name,
        MftSection::Content,
        progress,
        &mut visitor,
    ) {
        Ok(result) => result,
        Err(_) if canceled() => return Ok(empty()),
        // A root the MFT does not hold as expected: nothing was counted yet.
        Err(error) => {
            return Err(format!(
                "Analyseweg: Verzeichnislauf (MFT-Ordner nicht lesbar: {error})"
            ))
        }
    };
    report(diagnostics, root, index.diagnostics());
    report(diagnostics, root, &found);
    if scan.root().record == ROOT_RECORD {
        visitor.begin_system_files();
        match walk_section(
            &index,
            scan.root(),
            super::SYSTEM_FILES_NAME,
            MftSection::SystemFiles,
            progress,
            &mut visitor,
        ) {
            Ok((system, retention, found)) => {
                report(diagnostics, root, &found);
                attach_system_files(&mut tree, system, retention);
            }
            Err(_) if canceled() => return Ok(empty()),
            Err(error) => diagnostics.note(format!("NTFS-Systemdateien nicht erfasst: {error}")),
        }
    }
    diagnostics.note(MFT_WAY);
    Ok(tree)
}

/// One walk of the index into the visitor's tree.
fn walk_section(
    index: &MftIndex,
    root: FileRef,
    name: &str,
    section: MftSection,
    progress: &Progress,
    visitor: &mut TreeVisitor<'_>,
) -> io::Result<(SizeNode, Retention, MftDiagnostics)> {
    let (_, found) = index.walk(root, name, section, &progress.cancel, visitor)?;
    let (tree, retention) = visitor
        .take()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "MFT-Baum unvollständig"))?;
    Ok((tree, retention, found))
}
