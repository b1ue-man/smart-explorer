# H-REPLACE – API-Delta

Stand: 2026-10-03. Additiver ausgehandelter Share-Anschluss des vorhandenen Y140/Y142-VFS-Vertrags. Source-seitig fertig; gemeinsame Remote-Abnahme offen.

## Wire und Aushandlung

```rust
// native/src/share/core/fs_request.rs
pub(crate) struct FsReversibleReplace {
    pub(crate) staged: String,
    pub(crate) destination: String,
    pub(crate) retained: String,
}

FsRequest::ReplaceStagedReversible(FsReversibleReplace)

// native/src/share/core/fs_response.rs
FsResponse::ReversibleReplaced { replaced: bool }

// native/src/share/core/wire_capabilities.rs
FsHostFeatures::reversible_replace_v1: bool
```

JSON-Request hat `op: "replace_staged_reversible"` und die drei Pfade als Felder. Antwort hat `r: "reversible_replaced"` und ein verpflichtendes `replaced`-Boolean. Fehlendes Boolean ist ein Decodefehler, kein implizites `false`.

Die Anfrage ist ausdrücklich `FsEffect::Write`, also `mutates_filesystem() == true`. Sie ist eine eigene ausgehandelte Fähigkeit und wird nicht vom alten Transfer-v1-Schalter abhängig gemacht. Alt-Hosts lassen das neue Capability-Feld aus; serde-default ist `false`. Aktueller Host advertisiert `true`, unabhängig davon, ob der konkrete Provider selbst den Hook unterstützt. Provider-`false` bleibt mutationsfrei.

## PeerBackend-Extension

Der bestehende VFS-Trait-Hook wird nun tatsächlich von PeerBackend implementiert:

```rust
BackendExtensions::replace_staged_reversible(
    &self,
    staged: &str,
    destination: &str,
    retained: &str,
) -> io::Result<bool>

peer_extensions::reversible_replace::client(
    backend: &PeerBackend,
    staged: &str,
    destination: &str,
    retained: &str,
) -> io::Result<bool>
```

Für einen gültigen Vertrag liefert fehlendes `reversible_replace_v1` `Ok(false)`, bevor ein Mutationsframe gesendet wird. Bei ausgehandeltem Feature muss `backend.owns_stage(staged)` gelten. Stage, Ziel und Retained müssen verschiedene Namen desselben virtuellen Parents sein, Stage muss ein vorhandenes Stageformat haben und Retained exakt `.se-replace-<16lowerhex>` ohne Dateibasename.

Die Prüfung interpretiert Share-Pfadkomponenten ausschließlich mit der vorhandenen `split_clean`-Syntax. Sie dekodiert keine Backendnamen und schreibt keinen Locator um. `root_display() == "/"` bedeutet den virtuellen Share-Namensraum; der vorhandene Lease-Token und Host-FsAccess bestimmen die konkrete Mount-/Verbindungsbindung.

Der Client benutzt einen einmaligen `call_once`, der genau ein `PeerBackend::request_once` mit aktuellem Endpoint und bestehendem Lease-Kontext ausführt. `request_once` ist dafür intern `pub(super)`; Antwortzuordnung, Abort und Sessioninvalidierung bleiben seine vorhandenen Funktionen. Es wird keine Attempts-Schleife benutzt.

Zusätzlich schließen beide allgemeinen Kontroll-Attempts ausschließlich für diese neue Requestvariante ihren Idle-Zusatzretry aus. Die Variante steht nicht im Retry-Read-Match. Andere Requests behalten ihre bestehenden Verträge.

Nur eine bestätigte `ReversibleReplaced { replaced: true }`-Antwort ruft `release_stage(staged)`. `false`, transport-/providerseitiger Fehler, falsche Antwortart oder verlorene Antwort behalten die Stage-Registrierung. Es gibt keine automatische Löschung oder Legacy-/Overwrite-Ausweichmutation. Die generische Telemetrie meldet nur Operation/Antwortart und Boolean.

## Autorisierter Hostaufruf

```rust
FsAccess::replace_staged_reversible(
    &self,
    request: &FsReversibleReplace,
) -> io::Result<bool>

fs_access::reversible_replace::validate(
    request: &FsReversibleReplace,
) -> io::Result<()>

fs_access::reversible_replace::serve(
    send: SendStream,
    request: FsReversibleReplace,
    access: FsAccess,
    authorization: Option<MountLeaseAuthorization>,
    principal: PeerPrincipal,
) -> impl Future<Output = io::Result<()>>
```

Der Dispatcher übergibt dieselbe nach Session-Zulassung entstandene FsAccess und gegebenenfalls Lease-Rezulassung. `serve` nutzt den bestehenden fairen Kontrollworker und `run_authorized`, ohne dessen Autoritäts-/Wake-Lifetime zu verändern.

FsAccess validiert den virtuellen Vertrag, prüft aktuelle Schreibrechte und löst alle drei Pfade einzeln mit `resolve_write` auf. `require_same_backend(stage, destination)` und `require_same_backend(stage, retained)` bewahren die echte Export-/Verbindungs-/Lease-Bindung. Danach delegiert es konkret:

```rust
crate::vfs::replace_staged_reversible(
    &*stage.backend,
    &stage.path,
    &destination.path,
    &retained.path,
) -> VfsResult<bool>
```

Dieser Backendzugriff bleibt der bestehende GuardedBackend. Der Guard prüft Stage, Destination und Retained unmittelbar vor dem echten Provider-Hook. Aktuelle Rechte werden nach dem Hook sowie nach dem Worker vor der Antwort erneut geprüft. Es gibt keinen rohen Backend-Escape.

`Ok(false)` bedeutet Provider-unverfügbar ohne Mutation. `Ok(true)` bestätigt Veröffentlichung und Erhalt des alten Originals am bekannten Retained-Sibling. Fehler bzw. Antwortverlust lässt Zustand und Evidenz zur bestehenden Provider-/Engine-Recovery offen; er wird nicht in `false` umgedeutet. Der Providervertrag verspricht keine atomare Ersetzung.

## Kohäsive Kontrollhelfer-Extraktion

Die zusätzlich exakt freigegebene `server_fs_control.rs` enthält ausschließlich die vorherigen `simple`, `answer`, `reply_unit` und `control` mit erforderlicher interner Sichtbarkeit und Imports. Die bestehenden Poolwahl-, Lease-Admission- und Antwortwirkungen bleiben unverändert. Die Registrierung ist lokal in `server_fs.rs`; kein gemeinsamer `share/mod.rs`-Umbau.

Die ebenso exakt freigegebene `peer_request_policy.rs` übernimmt ausschließlich `is_retryable_read(&FsRequest) -> bool` und `response_matches(&FsRequest, &FsResponse) -> bool`, mit unveränderten Funktionskörpern und interner Sichtbarkeit. `peer_request.rs` registriert und importiert dieses Pure-Wire-Modul lokal. Die allgemeine Retrylogik wird nicht umgebaut; die bestehende Datei liegt dadurch bei 443 Zeilen und das neue Modul bei 58 Zeilen.

Neue Peer-/Hostmodule sind lokale Submodule in `peer_extensions.rs` und `fs_access.rs`. Die Source-Fixturedatei ist ein Testsubmodul des neuen Peer-Helfers. Es gibt keine Node-/ALPN-/Provider-Neuregistrierung.

## Caller-Handoff

Die Engine persistiert ihren Intent vor dem ersten Aufruf, bestätigt tatsächliche Veröffentlichung getrennt und behält bei Fehler/Antwortverlust alle bekannten Recoverypfade. E-APPLY setzt den vorhandenen Cleanupmarker inzwischen vor dem Publish-Versuch; `Staged::Drop` verwirft dessen Stage bei Fehler oder verlorener Bestätigung nicht automatisch. E-ENGINE übernimmt den dauerhaften Reversible-Intent und Recovery. Ein Cleanup-Schutz ersetzt keine erfolgreiche Baselinebestätigung.

Der Hauptagent übernimmt den additiven `wire.rs`-Reexport von `FsReversibleReplace` und repräsentative gemeinsame Wire-Roundtrip-Fixtures. Außerhalb des Scopes liegende exhaustive Fixture-/Feature-Konstruktoranschlüsse bleiben ebenfalls beim Hauptagenten. Der eigentliche Peer-/Host-Hook importiert den DTO bereits direkt und benötigt diese Fixture-Anschlüsse nicht.

Exakte Abnahmesignale/Dateiinventar: [abnahme/H-REPLACE.md](../abnahme/H-REPLACE.md). Konkrete Hauptanschlüsse: [anfragen/H-REPLACE.md](../anfragen/H-REPLACE.md). Der Worker erweitert weder LocalBackend-/Hardlink-/Listen-Grenzen noch Providerstrategien.
