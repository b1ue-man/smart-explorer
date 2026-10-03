# S-SIGNAL – API-Delta für Android und Integrationen

Stand: 2026-10-03. Umsetzung des freigegebenen Vertrags FC2/FC3/FC4, kein neues
Review. Android-Aufrufe laufen durch `mobile::call`; die eigenen additiven
Registrierungen stehen in `native/src/mobile/os/shared/domains/mod.rs`.

## Server-Adresse

| Methode | Argumente | Ergebnis / Änderung |
| --- | --- | --- |
| `share.setServer` | `server: string`, optional `allowPlaintext: bool = false` | Neue Eingabe ohne Schema wird TLS auf Port 51820. Klartext braucht Opt-in. Leerer String entfernt den Server. Ergebnis wie `share.serverInfo`. |
| `share.serverInfo` | keine | Liest den gespeicherten Server, migriert nackte Altwerte atomar zu TCP; Ergebnis unten. |

```json
{"server":"wss://share.example:51820","security":"encrypted","summary":"…","plaintext":false,"ignoredPlaintext":0,"migrated":false}
```

`security` ist `encrypted`, `plaintext` oder `none`. `server` ist kanonisch mit
Schema und gegebenenfalls `#sha256=`. `plaintext` zeigt, ob die gespeicherte
Liste Klartexteinträge enthält; `ignoredPlaintext` zählt unter TLS unbenutzte
Alt-Einträge. Neue Eingaben dürfen TLS und Klartext nicht mischen. `summary`
ist für die sichtbare Statuszeile. `migrated` gilt nur für den jeweiligen
`serverInfo`-Aufruf, der die Altdatei tatsächlich geändert hat.

Validierungsfehler verwenden `invalid`; Dateifehler laufen über die bestehenden
I/O-Adapter. Die App muss vor einem Klartext-Speichern die
sichtbare Zustimmung an `allowPlaintext` binden. HTTPS- und HTTP-Adressen werden
als WSS bzw. WS gespeichert. Pins erlauben selbst signierte Zertifikate ohne
TLS-Klartext-Fallback.

## PIN-Kopplung

| Methode | Argumente / Ergebnis |
| --- | --- |
| `share.suggestPin` | keine; liefert `{"pin":"481902"}` mit sechs zufälligen, nicht trivialen Ziffern |
| `share.discoverable` | bestehende `target`, `alias`, `pin`, `minutes`; zusätzlich `allowWeakPin: bool = false`; Dauer 1–30 Minuten |
| `share.connect` | bestehende `discoveryId`, `pin`; `shareBack: bool = false` öffnet bei ausdrücklicher Wahl zusätzlich eigene Freigaben |
| `share.unconfirmedPairings` | keine; liefert `{"pairings":[…]}` für bereits installierte Kopplungen ohne Gegenbestätigung |
| `share.resolvePairing` | `exchangeId: string`, `revoke: bool`; `false` behält die Kopplung und entfernt nur den Hinweis, `true` entfernt den installierten Kontakt/Raum |

Beim Publizieren liefern kurze/triviale PINs ohne Opt-in `weak_pin`; leere PINs
liefern auch mit Opt-in `invalid`. Das Verbinden verbietet leere PINs, erlaubt
aber die exakten nichtleeren Bytes des fremden Angebots. `stopDiscoverable`,
`discover` und `cancelConnect` behalten ihre bisherigen Formen.

Ein unbestätigtes Ergebnis enthält `exchangeId`, `kind` (`direct`,
`roomInstalled`, `roomShared`), `contactId`, `roomProfileId`, `label` und
`revocable`. Unbenutzte IDs sind `null`. Raumdaten, die der Publisher bereits
weitergegeben hat (`roomShared`), sind nicht zurückholbar; die App zeigt den
Hinweis und kann ihn mit `revoke: false` verwerfen. Ein fehlgeschlagener Widerruf
bewahrt das Ergebnis/Hinweisobjekt. Unbekannte Exchange-ID ergibt `not_found`.

Die Android-Oberfläche liegt außerhalb des S-SIGNAL-Scope und muss diese
Methoden/Statuswerte in AND-SHARE-UI verwenden. Die nativen UI-/Port-Aufrufe und
JSON-Registrierungen sind fertig; es wurde kein Kotlin außerhalb des Scopes
untersucht oder geändert.

## Signaling- und Relay-Vertrag

Capability `key_login_v1` ergänzt die bisherigen drei Capabilities. Neue Clients
beantworten `hello_challenge` mit `hello_auth` vor `hello_ok`; der Server darf
Schlüsselanmeldung nur nach erfolgreicher Prüfung bestätigen. Alte Server
bestätigen unmittelbar und bleiben kompatibel. SHA-256-Login-Domain und
längenpräfixierter Inhalt sind auf beiden Seiten identisch; die bestehenden
`review_task_login_digest_*`-Vektoren sichern den Vertrag.

Nach bestätigtem Login senden neue Clients `publish_direct.access_hash`,
`watch_direct.access_proof` und `join_room.access_proof`. Die optionale Form
bleibt für alte Server/Clients erhalten. Die HMAC-Domain ist
`se-server-access-v1\0`, mit Art und Relation-ID gebunden; der Server speichert
für Direct nur den SHA-256 des Nachweises. Raum-Präsenz wird nach Nachweis-Hash
partitioniert. Strikter Serverbetrieb verlangt Schlüsselanmeldung; der
Legacy-Kompatibilitätsmodus erlaubt alte Beobachter ohne Nachweis und alte
Raum-Mitglieder über die Partitionen hinweg.

Die bestehende Legacy-Entscheidungsnachricht erhält in `signal_publish` eine
signierte, entscheidungs- und empfängergebundene Präsenz aus
`signal_presence::build_direct_decision_presence`; deren Empfangsprüfung
gehört S-REVOKE (B03/S46). Der Server prüft Ursprung und Lookup-Besitz zusätzlich.

`NodeTransportOptions::ca_tls_config()` und `accepts_relay_url()` sind die
fertigen Integrationshaken für H-DISPATCH. A2/A3 bleiben dort offen.
`migrate_server_file(&Path)` wird von Desktop, CLI, Android und laut Orchestrator
auch vom tatsächlichen Daemon-Dateilesepfad verwendet (A4 erledigt).

## Geprüfte API-Referenzen

Die primären TLS-/Iroh-/Tungstenite-Referenzen liegen lokal in
`docs/refs/share-server-tls-auth.md`. Ergänzender Abgleich am 2026-10-03:
[rcgen 0.14.8](https://docs.rs/rcgen/0.14.8/rcgen/fn.generate_simple_self_signed.html)
für `CertifiedKey { cert, signing_key }` in den TLS-Abnahmesignalen und
[Tokio Notify](https://docs.rs/tokio/latest/tokio/sync/struct.Notify.html) für das
Ausgabe-Wake des gemeinsamen Readiness-Reaktors. Keine neue Bibliotheks- oder
Protokollauswahl und keine zusätzliche Review-Runde.
