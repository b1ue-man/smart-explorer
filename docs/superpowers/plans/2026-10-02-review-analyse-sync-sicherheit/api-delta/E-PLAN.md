# E-PLAN – Schnittstellen für die gemeinsame Integration

Stand: 2026-10-03. E-PLAN ist umgesetzt; die abschließende Remote-Abnahme steht aus.
Die vollständigen Signaturen und Erhaltungserwartungen stehen in
[anfragen/E-PLAN.md](../anfragen/E-PLAN.md).

- Gespeicherte Läufe verwenden `RunSettings::for_job(job.id)` und die Paarregel aus
  `pair_key_policy(..).fold_case`; Kontroll-Läufe verwenden `ScanDepth::VerifySources`.
- `ApplySink::should_stop() -> bool` ist additiv, Standard `false`. Apply hält vor neuen
  Mutationen an, wenn ein Checkpointfehler oder ein typisierter Laufstopp dies verlangt.
- `PairSnapshot` liefert getrennte `SideSnapshot`-Werte, Reparaturen und Konflikte.
  Der optionsfähige Kontroll-Walk erhält die bisherige Signatur als kompatible Hülle.
- Reporting-Apply und Dedupe melden tatsächliche Quell-/Zielsignaturen, dauerhafte
  Teilaktionen und geschützte Auslassungen. E-APPLY verbindet diese beschriebenen APIs;
  fehlende Implementierungen dort gelten weiterhin als offene Owner-Anschlüsse.
- Konflikte gehen über `resolve_recorded`, Vorschauaktionen über `apply_preview_action`.
  Die gespeicherte Job-/Replika-Basis wird unter derselben Paarsperre aktualisiert.

Altjob-Migration, Replika-Rotation, Literalnamen, Backend-/Verbindungsidentität und
geschützte Gegenstücke bleiben erhalten. Kein neues Wire-Protokoll; keine lokale
Build- oder Testausführung. Dateien und konkrete gemeinsame Abnahmesignale sind in
[abnahme/E-PLAN.md](../abnahme/E-PLAN.md) aufgeführt.
