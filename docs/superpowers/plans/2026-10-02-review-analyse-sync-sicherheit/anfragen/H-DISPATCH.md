# H-DISPATCH – konkrete Übergaben und Restgrenzen

Stand: 2026-10-03. Keine erneute Freigabe für bereits implementierte Source-Anschlüsse nötig. Dies sind konkrete Abhängigkeiten außerhalb des exklusiven Änderungsumfangs und ausstehende Ausführungsnachweise; keine neue Projekt-Review-Liste.

## R1 – Hauptanschluss des schnellen lokalen H-ANALYSIS-Consumers

Bereitgestellt und in eigenem Local-Delete/Destructive-Preflight verwendet:

```rust
crate::share::fs::ensure_local_share_handle_allowed(
    handle: &crate::local_access::DirectoryHandle,
) -> std::io::Result<()>
```

Der Hauptagent hat zugesagt, diesen Hook an den schon geöffneten Root/Child des schnellen lokalen H-ANALYSIS-Wegs anzuschließen. Er braucht keine freie Pfad-Auflösung oder pauschale Provider-Deaktivierung. Normale Daten-Reparsepunkte und Broker/Consent behalten ihr bestehendes Verhalten. Die bereits von H-ANALYSIS integrierten exakten Privatnamen-/Stage-Filter bleiben vor dem Öffnen.

Erwartetes Signal: Öffnen eines privaten Verzeichnis-Alias über Local/UNC bzw. Bind-Mount wird vor dessen Auslesen abgewiesen; normales lokales Verzeichnis bleibt auf dem schnellen Weg. Ohne diesen Hauptanschluss behauptet dieser Block keine vollständige physische Aliasgrenze für die außerhalb seines Scopes liegenden schnellen Walk-Consumer.

## R2 – allgemeine LocalBackend-Mutationen bleiben pfadbasiert

Die vom Hauptagenten bestätigte vorhandene LocalBackend-Grenze enthält FolderGuard-Komponentenprüfungen, aber Stage-/Rename-/Create-/Read-Operationen sind nicht durchgehend an denselben gehaltenen Exportroot gebunden. H-DISPATCH stellt den richtigen physischen Root her, prüft aktuelle Rechte/Privat-/Systempfade und benutzt für rekursives Delete eine tatsächliche handlegebundene OS-Grenze. Das stellt keine vollständige TOCTOU-Härtung aller anderen LocalBackend-Methoden her.

Die konkrete Restgrenze ist der Zeitraum zwischen erfolgreicher Pfad-/Ancestorprüfung und späterem pfadbasiertem LocalBackend-I/O. Ein lokaler gleichzeitiger Root-/Ancestor-Swap kann dort das effektive Objekt ändern. Eine vollständige Behebung müsste die nicht dem Block zugewiesenen LocalBackend-Consumer auf gehaltene Root-/Parent-Operationen umstellen. Der Hauptagent hat ausdrücklich keine weitere Scope-Ausweitung für diesen Worker beauftragt.

Erwartetes Signal für die vorhandene Delete-Grenze: Root-Swap beeinflusst die gehaltene Eltern-Löschung nicht und entfernt kein außerhalb liegendes Linkziel. Allgemeine Stage-/Rename-/Read-Swap-Härtung darf im Gesamtbericht nicht aus diesem Signal abgeleitet werden.

## R3 – private Datei-Hardlinks außerhalb privater Ancestry

Der neue typed Handle-Hook erkennt private Verzeichnisse und deren physische Vorfahren einschließlich UNC-/Bind-Aliase. Ein regulärer Datei-Hardlink in einem erlaubten öffentlichen Verzeichnis besitzt dagegen öffentliche Directory-Ancestry. Der allgemeine LocalBackend-Read besitzt an dieser Stelle keine zu einer privaten Datei gespeicherte Objektidentität als Vergleichsgrenze. Dieser Block meldet diese konkrete Grenze und behauptet keinen Schutz aller solchen Datei-Aliase.

Eine weitergehende Behebung braucht eine file-object-gebundene Privacy-Grenze im LocalBackend/LocalAccess-Consumer, außerhalb des freigegebenen Directory-/Delete-Anschlusses. Der Worker erweitert diesen Scope nicht.

## R4 – Provider-Listenallokation vor dem rekursiven Remote-Delete-Budget

Der iterative Remote-Walker budgetiert den behaltenen Stack, Vec-Kapazitäten, Metadaten und Strings und benutzt keinen rekursiven Backend-Delete. Die bestehende Provider-Methode `Backend::list_dir(&str) -> VfsResult<Vec<VfsMeta>>` liefert jedoch erst nach Allokation ihrer gesamten Liste zurück. Das Budget kann daher diese erste Provider-Allokation nicht vorweg begrenzen.

Der lokale Walker benutzt den bestehenden gehaltenen begrenzten Directory-Iterator. Für eine entsprechende harte Vorabgrenze beim Remote-Provider wäre ein tatsächlich unterstützter inkrementeller Listing-Hook erforderlich. Dieser Worker erfindet dafür keinen vermeintlich fertigen Stub und deaktiviert keine vorhandene Providerfunktion.

Erwartetes Signal: budgetierte Remote-DFS bleibt bei behaltenen Listen begrenzt und retrybar; ein einzelnes Provider-Listing wird im Gesamtbericht nicht als streaming-budgetiert bezeichnet.

## R5 – gemeinsame Wire-/Root-Integration

H-DISPATCH implementiert `SyncChildPath`, `ChildPath`, `literal_children_v1`, Host-/Peer-Hooks und die ausdrücklich freigegebenen exhaustiven Consumer in `peer_request.rs`/`peer_fs_logging.rs`. Der Hauptagent besitzt zusätzliche Registrierungen und Consumer-/Fixture-Konstruktoren außerhalb des Manifests. Er übernimmt deren Anschluss an die neue Wire-Variante und das Feature im gemeinsamen Kandidaten.

Die konkreten neuen Guard-Hooks `sync_child_path`, `replace_staged_reversible` und `previous_state_identities` sind gegen die realen VFS-APIs verbunden. Der Recovery-Sibling folgt dem finalen Root-Vertrag `.se-replace-<16lowerhex>` ohne Dateibasename. Der Hauptagent verbindet anschließend seine Engine-Consumer. S09-LINK-Node/ALPN/Service-Registrierungen sind ebenfalls sein späterer Anschluss.

## R6 – Remote-Abnahme und Graph

Der Worker hat keine lokale Suite, keinen Compiler, keinen Formatter und keinen Server gestartet. Die Source-Fixtures und Integrationssignale in [abnahme/H-DISPATCH.md](../abnahme/H-DISPATCH.md) gehören in die eine vom Hauptagenten orchestrierte Remote-Suite, einschließlich echter Windows-/UNC-/Bind-Mount-/TLS-/Drive-Fälle.

Root-Graph-Aktualisierung, Commit/Push, Auswertung/Fixes dieser einen Suite und abschließender Remote-Release liegen beim Hauptagenten. Vor deren Ergebnis ist H-DISPATCH source-seitig übergeben, aber nicht laufzeitabgenommen oder veröffentlicht.

## Aufgelöste frühere Abhängigkeiten

V2-Host-Entry-Points von H-ANALYSIS sind konkret angeschlossen; es gibt keinen Stub-Einstieg. Legacy-Snapshot verwendet den realen vierargumentigen Aufruf. `PeerPrincipal::device_identity`, `FsAccess::policy_key/retained_snapshot/is_dynamic/check_read/register_cancel` sind vorhanden und von H-ANALYSIS genutzt. Aktuelle Rechte werden bei Wiederaufnahme und Ausgabe geprüft; Transportverlust allein invalidiert behaltene Ergebnisse nicht.

`ShareIrohNode::invalidate_restrictions` ist real implementiert und erfüllt den S-REVOKE-Aufrufervertrag. Exec-Transition bleibt vor Auth-Snapshotübernahme. Relay-Pins und URL-Filter sind am tatsächlichen Iroh-Builder/Sitzungspfad. Die bereits vorhandene B21-Dateimigration wird nicht dupliziert.

Dateiinventar und Fundzuordnung stehen vollständig in [abnahme/H-DISPATCH.md](../abnahme/H-DISPATCH.md), die genauen Signaturen in [api-delta/H-DISPATCH.md](../api-delta/H-DISPATCH.md). Der Worker stoppt nach dieser Übergabe.
