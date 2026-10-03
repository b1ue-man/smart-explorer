# H-REPLACE – verbleibende Hauptanschlüsse

Stand: 2026-10-03. Der freigegebene Share-Source-Anschluss ist abgeschlossen; keine zusätzliche Implementierungsfreigabe für diesen Worker nötig. Die folgenden Anschlüsse bleiben beim Hauptagenten/E-ENGINE.

## R1 – Intent und Cleanup bei unbestätigtem Ergebnis

Der Caller muss seinen Recovery-Intent mit Stage, Destination und exakt `.se-replace-<16lowerhex>` im selben Parent vor dem ersten Hook-Aufruf persistieren. Der neue Peer-Client gibt nur nach vollständig erhaltenem `ReversibleReplaced { replaced: true }` seine Stage-Registrierung frei. Er wiederholt die Mutation nie und ruft bei `false`, Fehler oder Antwortverlust kein Cleanup auf.

E-APPLY hat den bestehenden Consumer `native/src/bisync/os/shared/apply_stage.rs::Staged::publish` inzwischen angeschlossen: Der Cleanupmarker wird vor dem Publish-Versuch gesetzt, sodass `Staged::drop` dessen Stage nach Fehler oder verlorener Bestätigung nicht über `discard_copy_stage` entfernt. Der aktuelle Source bestätigt diesen Anschluss. E-ENGINE übernimmt den dauerhaften Reversible-Intent mit allen drei Pfaden und dessen Recovery; dieser Worker ändert die Engine-Consumer nicht.

Unbestätigte Veröffentlichung ist kein bestätigter Erfolg: ein Cleanup-Schutz darf keine erfolgreiche Baseline-/Intent-/Publikationsbestätigung vortäuschen. Inhalt kann bereits am Ziel liegen und das Original am Retained-Pfad. Die drei bekannten Pfade, beobachtete Signaturen und unverbrauchte Intent-Evidenz gehören in die spätere kontrollierte Recovery, nicht in einen automatischen Replay-/Overwrite-Rückfall.

Der Hauptagent hat den erledigten E-APPLY-Anschluss und die E-ENGINE-Ownership bestätigt. Eine zusätzliche Freigabe für H-REPLACE ist nicht nötig.

## R2 – gemeinsame Wire-Konstruktoren und Fixture-Anschlüsse

Die neuen Varianten heißen:

```rust
FsRequest::ReplaceStagedReversible(FsReversibleReplace)
FsResponse::ReversibleReplaced { replaced: bool }
FsHostFeatures::reversible_replace_v1
```

Die neuen DTO-Felder sind in `fs_request.rs` definiert und intern direkt importiert. Der bestehende `wire.rs`-Reexport ist für den eigentlichen Hook nicht nötig und wurde nicht außerhalb der Änderungsfreigabe geändert. Der Hauptagent übernimmt vereinbarungsgemäß den additiven Wire-DTO-Reexport und repräsentative Roundtrip-Fixtures.

Außerhalb des Manifests liegende vollständige Feature-Literale und exhaustive Request-/Response-Fixtures gehören ebenfalls dem Hauptagenten. Die freigegebenen produktiven Peer-Logging-/Response-Matches sind implementiert. Legacy-Featureobjekte bleiben per serde-default `false`.

## R3 – Remote-Abnahme

Die exakten Fixture-Selektoren und echten Direct-/Room-/SFTP-/Lease-/Lost-ACK-Signale stehen in [abnahme/H-REPLACE.md](../abnahme/H-REPLACE.md). Der Hauptagent nimmt sie in die eine gemeinsame Remote-Suite auf. Echte Rechteänderung während Zulassung, alter Host ohne Feature sowie verlorene Antwort nach Provider-Publish werden dabei zusammen mit dem Engine-Intent-Consumer abgeglichen.

Es gibt keinen lokalen Compiler-/Laufzeitnachweis; der Worker führte weder Builds/Tests/Formatter/Server/Installationen noch Git-Mutationen, CI-, Graph- oder Releaseaktionen aus. Diese verbleiben beim Hauptagenten. Die statischen Checks dienen ausschließlich der Source-/Textkonsistenz.

Dateiinventar, Fundzuordnung und Entscheidungen stehen in [abnahme/H-REPLACE.md](../abnahme/H-REPLACE.md); exakte API-Semantik in [api-delta/H-REPLACE.md](../api-delta/H-REPLACE.md). Keine Erweiterung auf allgemeine LocalBackend-, Hardlink- oder Listing-Umbauten.

Die abschließend freigegebene `peer_request_policy.rs` enthält ausschließlich die unveränderten Pure-Wire-Funktionen `is_retryable_read` und `response_matches`; die lokale Registrierung liegt in `peer_request.rs`. Das reduziert dessen Größe auf 443 Zeilen ohne Änderung der allgemeinen Retrylogik.
