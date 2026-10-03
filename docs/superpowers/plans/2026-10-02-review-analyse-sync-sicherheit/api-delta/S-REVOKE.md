# S-REVOKE – API-Delta V5

Stand: 2026-10-03. Additive V5-Typen und ihre Reexports bleiben wie in `umsetzung.md`. Keine neue
Server-Nachricht, Profilformat-Heuristik oder Backend-Locator-Umdeutung.

## Beziehung und Bedienung

```rust
pub fn allow_direct_peer_again(
    default_home: Option<String>, device_id: &str,
) -> Result<RelationChange, String>;
pub fn set_direct_share_back(
    default_home: Option<String>, contact_id: &str, share_back: bool,
) -> Result<RelationChange, String>;
pub struct RelationChange { pub profiles: ShareProfiles, pub changed: bool }
```

Beide sind bereits unter `crate::share` reexportiert. Sie persistieren über die bestehende
Profiltransaktion. Wiederfreigabe betrifft den Schlüssel/Knoten einschließlich gespeicherter
Geräte-ID-Aliase und lässt Exec aus. Gegenseitig aus erhält vorhandene Grants; Gegenseitig an ist eine
bewusste Wiederfreigabe und legt neue Grants ohne Schreibrecht und Exec an.

CLI: `se share grants allow [selector] [--fingerprint <fingerprint>] [--json]`. Ohne Selector muss genau
ein inaktives Grant vorhanden sein. JSON liefert `action: "allowed"`, `device_id`, `persisted`,
`changed`, `exec_enabled: false`, `grants` und `worker_refresh`. Desktop besitzt denselben Weg und die
Gegenseitig-Wahl beim Direkt-Code-Hinzufügen und Verwalten. Android-Anschluss liegt beim Folgeblock;
keine zusätzliche Mobile-Dispatchregistrierung wurde vorgenommen.

Interner zentraler Entzug:

```rust
impl ShareProfiles {
    pub(crate) fn withdraw_direct_key(&mut self, peer: &DirectPeerIdentity, now: i64);
}
```

## Rechte und Exec

```rust
pub(crate) fn authorization_restrictions(
    current: &ShareAuthState, candidate: &ShareAuthState,
) -> RestrictionSet;
impl ShareIrohNode {
    pub(crate) fn invalidate_restrictions(
        &self, restrictions: &RestrictionSet,
    ) -> io::Result<usize>; // Implementierung H-DISPATCH; Aufruf in S-REVOKE
}
```

Die erste Funktion liefert jetzt konkrete Reduktionen. Anheben von Rechten, Hinzufügen neuer
Principals, Namen, Status, Zeitstempel, Präsenz und Routen liefern keine Restriktion. Direct-Key-
Rotation trifft Direct; lokale Geräteidentitätsänderung trifft alles. Entfernen, Sperren,
Wiederbestätigung, Export- oder Schreibentzug und Deaktivierung treffen die zugeordnete Beziehung.

`ExecRegistry` hält je exaktem Principal `revision`, `enabled`, `minimum_epoch`; die globale Epoche
ist nur die oberste bekannte Epoche. Launch-Tokens dürfen zwischen der Principal-Barriere und dieser
Epoche liegen. `restrict_authorization(epoch, &RestrictionSet)` beendet passende Starting-/Running-
Jobs, ohne unbeteiligte Reservierungen zu verwerfen. Eine gleiche deaktivierte Policyrevision darf
nicht wieder aktiviert werden. Vorübergehendes Offline betrifft Sitzungen, keine Policyrevision.

## Signierte Präsenzen und Legacy-Entscheidung

```rust
pub(crate) fn build_direct_decision_presence(
    relation_id: &str,
    requester_device_id: &str,
    accepted: bool,
    identity: &ShareIdentity,
    secret: &[u8],
    iroh: &ShareIrohNode,
) -> io::Result<PeerPresence>;
```

Pfad: `crate::share::signal_presence`. `send_direct_answer` in S-SIGNAL nutzt diesen Helfer statt einer
gewöhnlichen Direct-Präsenz. Das opake Nonce-Feld trägt `d1t`/`d1f`, 22 Base64url-Zeichen Empfängerdigest,
48 Bit Zufall, `.ps1.` und die Ed25519-Signatur: insgesamt 126 ASCII-Bytes. HMAC-v1 bleibt für alte
Server und Geräte erhalten. Die zusätzliche Signatur ist domänensepariert und längenkodiert; sie
deckt Namen, Fingerprint, Knoten und Routen mit ab. Gewöhnliche signierte Präsenzen können nicht als
Entscheidung verwendet werden. Der freie Legacy-Nachrichtentext bleibt unvertrauenswürdig und wird
nicht als Entscheidung übernommen.

Replay-Admission verweigert neue Nachrichten bei voller Menge, bis abgelaufene Einträge freigegeben
werden; kein noch gültiger Nonce wird verdrängt. Verifizierte Signaturen werden vor der Daemon-Übernahme
als begrenzter Schlüssel-/Knoten-Marker erinnert. Die dauerhaften `signed_presence`-Flags bleiben OR-
monoton. Unsignierte Altdaten behalten den eingeschränkten Pending-Kompatibilitätsweg.

## Reparatur, Removal und Serverdatei

`run_outgoing` erhält jetzt `OutgoingRepairPersistGate`. Der Ausgangsaufrufer ist real angeschlossen;
das Gate nimmt den Transition-Platz erst beim Schreiben, prüft Online-/Identitätsgeneration,
Contact-Pins und Relation-Secret erneut und gibt bei einem konkurrierenden Konfigurationswechsel
`Retryable` zurück. Der blockierende Store behält seinen Permit bis zum Ende, auch nach Async-Abbruch.
Ein früherer Replay-Receipt ersetzt keine aktuelle durable Policyprüfung. Der Daemon stellt Reload,
Neustart und ConfigureProfiles nicht mehr während des gesamten Reparaturaustauschs zurück;
die bestehende Runtime-Transition serialisiert ausschließlich den kurzen Store-Schritt.

Desktop: `remove_direct_peer_completely` und `remove_room_completely` liefern jetzt `bool`, das einen
erfolgreichen persistenten Entzug bezeichnet. `revoke_unconfirmed_pairing` vergisst seinen Hinweis
erst danach. Sekundäre Bereinigungshinweise bleiben sichtbar.

`ipc_host.rs::load_share_server` konsumiert nach der vorhandenen Regularfile-/16-KiB-Grenze
`crate::share::migrate_server_file(&path) -> Result<Option<String>, String>`. `Some` wird als canonical
Adresse übernommen; Fehler werden weitergereicht. Gespeicherte schema-lose Legacy-Adressen bleiben
über den S-SIGNAL-Helfer `tcp://`.
