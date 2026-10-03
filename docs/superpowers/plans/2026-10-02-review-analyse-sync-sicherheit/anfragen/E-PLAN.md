# E-PLAN – abgeschlossene Umsetzung und Owner-Anschlüsse

Stand: 2026-10-03. E-PLAN ist abgeschlossen. Diese Anschlüsse sind dem
Hauptagenten konkret übergeben und halten E-PLAN nicht offen. Fremde
Implementierungsdateien wurden ausschließlich gelesen.

## E-APPLY: bestehender V3-Vertrag

Voll-/inkrementeller Lauf, Vorschau-Einzelaktion und reguläres
`resolve_recorded` verwenden unmittelbar:

```rust
fn apply_planned_reporting(
    actions: &[Action], dirs: &[DirAction],
    planned_a: &Tree, planned_b: &Tree,
    endpoints: SyncEndpoints<'_>, opts: BisyncOptions,
    scope: &ApplyScope<'_>, errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
) -> ApplyReport;
```

`ApplyScope` bleibt `{ sink, versions, spellings }`. Die geplanten Trees
tragen tatsächliche Seitenschreibweisen; I/O verwendet
`scope.spellings.side_rel`. `CompletedAction` muss die Signaturen der
erfassten/kopierten Quelle und des tatsächlich veröffentlichten Ziels tragen.
Ein erneuter kompletter Ergebnis-Walk findet bei E-PLAN nicht statt.
Bytegebundene Hashes und wirksame Zielzeit sind somit Apply-Vertrag.
Auch nach Teil-/Backupfehlern oder Abbruch werden nur tatsächlich
abgeschlossene Teilaktionen gemeldet; fehlgeschlagene behalten alte Basis.

Neu additiv: `ApplySink::should_stop() -> bool`, Standard `false`.
E-APPLY prüft es vor Zulassung einer neuen Mutation und zwischen Aktionen.
Der E-PLAN-Sink setzt es bei Checkpointfehler oder `RunStop`; laufende
Ergebnisse dürfen noch gemeldet und abschließend gesichert werden.
Beobachter werden nach der internen Fortschrittserfassung benachrichtigt.

Ordner-Create erfolgt vor Kopien, Remove nach Dateilöschungen und nur leer.
Bei `durable == false` bestätigt E-PLAN vor dem Journal die betroffenen
Seiten über V1 `sync_filesystem`. Drift ist `deferred`; ungeeignete
Dateigröße/Name sind geschützte Auslassungen; Ziel voll/nur lesbar/wiederholt
nicht erreichbar ist typisierter Stopp. Versionen ausschließlich über
`scope.versions` und dessen Ziel-/App-Daten-Auswahl. Backupfehler verhindern
nachfolgende destruktive Teilaktionen.

## E-APPLY: Paar- und Kontroll-Snapshot

Die vorhandene `snapshot_pair::read_pair`-Signatur bleibt.
E-PLAN benötigt sein Ergebnis jetzt so:

```rust
struct PairSnapshot {
    a: SideSnapshot,
    b: SideSnapshot,
    repairs: Vec<Conflict>,
    conflicts: Vec<Conflict>,
}
```

Jede Seite hält `tree`, `filtered`, `dirs` und `omissions` getrennt.
Zusätzlich braucht `snapshot::Snapshot` die Felder `filtered: Tree`
und `dirs: DirSet`. Duplikatbeobachtung und Reparaturen im bestehenden
`read_pair`-Pfad erhalten. Die alte reine `PairSnapshot::plan`-Methode
wird von E-PLAN nicht mehr benötigt.

Der Hauptagent hat den folgenden kompatiblen Kontroll-Walk ausdrücklich
freigegeben; `incremental_collect.rs` ruft ihn bereits auf:

```rust
fn walk_snapshot_with_options(
    be: &dyn Backend, root: &str, cancel: &AtomicBool,
    filter: &WalkFilter<'_>, hash: HashMode, prev: Option<&Tree>,
    allow_duplicate_files: bool, fold_case: bool, opts: BisyncOptions,
) -> io::Result<Snapshot>;
```

Die bisherige Acht-Parameter-Signatur `walk_snapshot` bleibt kompatible
Hülle. Voll-/Kontroll-Quellwalks respektieren `cross_mounts`, `OwnFile`
und geschützte `Filtered`-Auslassungen wie der V3-Paarwalk.
Filterordner werden nicht betreten; stumme Auslassungen schützen trotzdem.
Links, Junctions und Mounts dürfen keine Gegenstück-Löschung auslösen.
Windows-Daten-Reparsepunkte bleiben normale Daten; gewöhnliches
`node_modules` wird nicht allein wegen seines Namens ausgefiltert.
Grenzen kommen aus gemeinsamen `SyncLimits`. Unvollständige Beobachtung
erlaubt keinen vollständigen inkrementellen Bootstrap; E-PLAN fällt bereits
auf Vollplanung zurück.

## E-APPLY: sichere Dedupe-Ergebnisse

Die Voll-Orchestrierung ruft einmal je vollständiger Kandidatenliste:

```rust
fn apply_dedupe_reporting(
    candidates: &[crate::vfs::DedupeCandidate], side: PairSide,
    planned_a: &Tree, planned_b: &Tree,
    endpoints: SyncEndpoints<'_>, opts: BisyncOptions,
    scope: &ApplyScope<'_>, errors: &mut Vec<(String, String)>,
    cancel: &AtomicBool,
) -> ApplyReport;
```

Dies ist der endgültige Anschluss; ein früher besprochenes
`io::Result<usize>` würde Teilergebnisse nicht ausreichend tragen.
Der direkte ungesicherte Backend-Apply wurde entfernt.
Einen normalisierten Planindex einmal für die ganze Liste aufbauen.
Jede Entfernung bindet die aufgezeichnete ID, revalidiert den geplanten
Zustand und sichert genau dieses Objekt über `scope.versions`, bevor sie
mutiert. Fehler schützen ihre Pfade; unabhängige Gruppen dürfen weiterlaufen.
ENOSPC, Nur-lesen, Abbruch und `sink.should_stop` beenden neue Mutationen.

`report.stats.deleted` zählt tatsächlich entfernte physische IDs.
Nach Extra-ID-Löschung bleibt die behaltene Primärdatei in der Basis:
`CompletedKind::Copied` mit ihrem unveränderten bestätigten Paarzustand
ist dafür zulässig. `Deleted { side }` erst melden, wenn der gesamte Pfad
auf dieser Seite fehlt. E-PLAN überspringt bereits vollständig gemeldete
Löschungen und schützt Deferred-/Omitted-Gruppen vor normalem Apply.
Kein Extra-ID-Erfolg darf die Primärbasis entfernen.
Vorab-Schutz zählt Primäraktion und zusätzliche IDs genau einmal;
Move-Jobs starten keine zusätzliche rekursive Dedupe.

## E-APPLY: bestehende Duplikat-Auflösung und Versionen

Automatische gemeinsame Varianten verwenden vorerst die bestehende
`duplicate_apply::resolve`-Signatur. E-PLAN journalisiert erst deren
erfolgreich zurückgegebenen exakten Paarzustand. Fehler schützen nur ihre
Gruppe. Die Variante von `resolve_recorded` hält dieselbe Paarsperre
und merged in ihren Owner-/Replika-Zustand.
Zwei konkrete Anforderungen bleiben beim Duplikat-Apply-Owner:

- Rückgabesignaturen an übernommene Bytes/IDs binden und alle erforderlichen
  Mutationen vor Erfolg dauerhaft bestätigen. Destruktive Schritte dürfen
  einem fehlgeschlagenen Backup nicht folgen.
- Scopefähige Verbindung zur vorhandenen `RunVersions` herstellen, damit
  `VersionsLocation::Auto`, Laufmetadaten und Beobachtermeldungen auch für
  automatische/manuelle Duplikatreparaturen gelten. Der bisherige
  `versions_dir`-/`app_data_dir`-Aufruf ist die kompatible Altgrenze
  und noch kein Anschluss an Ziel-Versionen. Keine ungesicherte zweite
  Versionsablage als Ersatz einführen.

Die Orchestrierung erstellt bereits eine `RunVersions` je Lauf und ruft
`finish`/`prune_after_run` auch nach Fehler/Abbruch unter der Paarsperre auf.
Reguläre Konfliktauflösung erhält gespeicherte Versions-/Retention-Optionen
des betreffenden Jobs. `versions.rs` bleibt E-APPLY-Besitz.

## T-JOBS, D-SYNCUI und AND-SYNC

K3-Anfragen 7/8 bleiben bestehen: gespeicherte Läufe mit
`RunSettings::for_job(job.id)`, Ignore-Faltung aus
`pair_key_policy(..).fold_case`, Kontroll-Lauf mit
`ScanDepth::VerifySources`, Konflikte über `resolve_recorded`,
Einzelaktionen über `apply_preview_action`.
Bestätigungen gelten nur für den angezeigten Block; Blockade ist kein
Nutzerabbruch. E-PLAN liefert die passenden `Outcome`-Felder und tatsächliches
`StateKey`. Job-Löschung/Retargeting bereinigt passende Basis-/Indexzustände;
Versionsbereinigung verwendet `bisync::versions` beim Versions-Owner.

## Hauptagent und Abschluss

Eigenen Block integrieren/committen; Root-Graph aktualisieren.
Formatierung, Kompilierung und Verhalten ausschließlich in der einen finalen
Remote-Task-Suite nach Fertigstellung aller Owner-Blöcke prüfen.
Fälle, Fundzuordnung und Dateilisten: [abnahme/E-PLAN.md](../abnahme/E-PLAN.md).

Die beschriebenen E-APPLY-Funktionen und Snapshotfelder fehlen derzeit;
die Gesamtquelle ist bis zu diesem Anschluss nicht als kompilierbar oder
abgenommen ausgewiesen. Im E-PLAN-Scope bleibt keine funktionale
Umsetzungslücke. E-PLAN stoppt mit dieser Übergabe.
