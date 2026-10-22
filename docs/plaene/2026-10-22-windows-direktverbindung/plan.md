# Windows-Direktverbindung (2026-10-22, Ziel 0.5.184)

## Auftrag

Kritischer Fehler nach 0.5.183: Zwei Windows-Geräte (Windows 10, Windows 11)
mit aktueller Version bekommen keine Direktverbindung. Die App meldet, die
Gegenseite reagiere nicht richtig, obwohl sie als online angezeigt wird. Ein
Linux-Host und ein Android-Gerät erreichen die Windows-Geräte. Auf einem
weiteren, neu installierten Windows-Gerät ist die Share-Identität nicht
verfügbar („Iroh-Identitaet fehlt im sicheren Speicher“). Der Nutzer konnte
nur das Worker-Log und die Grant-Liste eines Windows-Geräts liefern; den Rest
klärt der Code.

## Befund

1. **Meldung „Gegenstelle … nicht richtig reagiert“** ist der Windows-Text des
   Socket-Timeouts (os error 10060). Er stammt nicht von der Gegenseite,
   sondern von `open_share_backend`: App, Laufwerke und `se` warteten 3 s auf
   den eigenen Hintergrund-Worker, der eine Gegenstelle bis zu 45 s lang
   versucht. Unter Linux erscheint derselbe Fall als „Resource temporarily
   unavailable (os error 11)“. Die eigentliche Ursache des Workers ging
   verloren. Ein Test gegen den Worker des Linux-Hosts lieferte sie:
   „peer session handshake timed out after 19 seconds“.
2. **Das Windows-10-Gerät veröffentlichte keine Relay-Adresse**, nur seine
   LAN-Adresse. Seine Presence blieb 35 Minuten nach dem Start so. Das
   Signaling zum selben Server lief, das Relay desselben Servers antwortet
   anderen Geräten. Ohne Relay ist ein Gerät hinter NAT von außen nicht
   erreichbar, und zwei solche Windows-Geräte finden keinen Weg zueinander.
   Das Gerät lief zuletzt mit 0.5.180 (Relay erreichbar) und dann erst
   wieder mit 0.5.183.
3. **Zwischen 0.5.180 und 0.5.181** wechselte iroh von 1.0.1 auf 1.3.0, und
   iroh-dns 1.3.0 löst Namen mit dem neuen `n0-dns-resolver` 0.1.0 statt mit
   hickory auf. Das Signaling löst den Servernamen über das Betriebssystem
   auf. Die Relay-TLS-Prüfung ist plattformgleich (eingebettete
   Mozilla-Wurzeln), ein Systemproxy wird nicht gelesen (`hyper-util` ohne
   Windows-Registry). Damit bleibt die Namensauflösung von iroh der
   wahrscheinlichste Windows-spezifische Unterschied. Ein Beweis auf den
   Geräten fehlt.
4. **Eine Annahme wurde mit `lifetime_exceeded` dauerhaft verworfen.** Eine
   Quittung paart die Uhr des quittierenden Geräts mit dem Ablauf, den das
   andere Gerät nach seiner Uhr setzte. Geht die quittierende Uhr nach, liegt
   die Spanne über den erlaubten 30 Tagen. Die vorhandene Prüfung
   `review_task_decision_delivery_accepts_clock_skew_in_both_directions`
   nutzt 600 s Lebensdauer und erreicht die Grenze nie.
5. **Fehlender Iroh-Schlüssel bei vorhandener Identitätsdatei** führt in App
   und Worker in eine Sackgasse: Nur `se share identity --repair` hilft, und
   die App nennt den Weg nicht. Warum der Schlüssel fehlte, ist unbekannt.
   Der Code der Identität, des sicheren Speichers und `keyring` 3.6.3 sind
   seit 0.5.182 unverändert.

## Meilensteine

| # | Ergebnis | Dateien | Abnahme |
|---|---|---|---|
| M1 | App, Laufwerke und `se` warten bis 75 s auf die Antwort des Workers und melden dessen Ursache. Eine stille Gegenseite heißt „Der Hintergrund-Worker hat … s lang nicht gemeldet, ob share://… erreichbar ist“. Kein erneutes Öffnen nach einem Timeout | `daemon/os/shared/ipc_open_client.rs` (neu, aus `ipc_client.rs` ausgelagert) | `windows_direct_task_open_waits_for_the_worker_answer`, `…_open_names_a_silent_worker`, `…_open_retries_a_broken_connection` |
| M2 | Der Iroh-Endpunkt löst Relay-Namen über das Betriebssystem auf. iroh's eigener Auflöser läuft parallel und beantwortet TXT, die erste nicht leere Antwort gilt | `share/os/shared/relay_dns.rs` (neu), `transport_options.rs`, `share/core/node.rs` | `…_relay_names_resolve_through_the_system`, `…_first_answer_prefers_any_addresses`, `…_endpoint_resolves_relays_through_the_system` |
| M3 | Hat der Endpunkt nach 20 s kein Relay, steht „Relay nicht erreichbar (URL…)“ im Worker-Log. Erholt er sich, folgt „Relay erreichbar: URL“. Loopback-Relays von Testservern bleiben unerwähnt | `share/core/endpoint_routes.rs` | `…_unreachable_relays_are_named_except_loopback`, `…_unreachable_relay_is_named_in_the_log` |
| M4 | Die Lebensdauer getrackter Direkt-Umschläge darf 30 Tage plus die schon geltende Uhrtoleranz von 5 Minuten betragen | `share/core/direct_protocol.rs` | `…_receipts_accept_clocks_within_the_skew`, angepasst `overlong_and_i64_max_envelopes_fail_before_authentication` |
| M5 | Eine Identitätsdatei ohne Schlüssel im sicheren Speicher wird ersetzt, solange keine Kontakte, Freigaben, Anfragen oder Räume bestehen. Sonst nennt die Meldung `se share identity --repair` | `share/os/shared/identity_store.rs`, CLI-Hinweise ohne Doppelung | `…_unpaired_identity_without_key_is_replaced`, `…_paired_identity_without_key_names_the_repair` |

Erhaltenes Verhalten:
- Direkte Verbindungen ohne Relay bleiben unverändert.
- Gekoppelte Identitäten werden nie stillschweigend ersetzt.
- Grenzen für überlange und zukünftige Umschläge bleiben bestehen.
- Peer-QUIC behält den Post-Quanten-Schlüsseltausch.

## Suite

`native/test-windows-direct-task.py`, Workflow `windows-direct-task.yml`
(ubuntu-24.04 und windows-2025). Sie führt die elf Abnahmen mit dem Präfix
`windows_direct_task_` aus. Dazu laufen die direkt betroffenen Integrationen
mit: Lebensdauer- und Uhrprüfungen des Direktprotokolls, Identitätsgeneration,
Öffnen über den echten Worker (`direct_open_task_…`), Wiederverbinden
(`remote_drive_task_…`, `windows_remote_task_…`). Danach folgt ein
vollständiges Release über `build.yml`.

## Nicht gelöst

- Ob die Namensauflösung auf den Geräten des Nutzers die Ursache war, zeigt
  erst die neue Logzeile (M3) nach dem Update.
- Warum der Schlüssel auf dem neuen Gerät fehlte, ist unbekannt (M5 behebt
  nur die Sackgasse).
- Ein Windows-Gerät hatte keine einzige Freigabe mehr. Freigaben löscht der
  Code nur beim Entfernen eines Geräts oder einer Freigabe, und diese Stellen
  hat 0.5.183 nicht geändert.
