# AND-SHARE-UI – Owner-Grenzen und erledigte Anfragen

Stand: 2026-10-03. Android-Consumer im zugeordneten Scope sind quellseitig abgeschlossen.
Es fehlt keine native Methode für die hier angeschlossene Bedienung. Keine fremde Datei wurde geändert.
Die vollständige Datei-Inventur steht in [abnahme/AND-SHARE-UI.md](../abnahme/AND-SHARE-UI.md).

## Erledigte Integrationsanfragen

| Anfrage an den Hauptagenten / nativen Owner | Gelieferter Vertrag und tatsächlicher Android-Anschluss |
|---|---|
| Inaktive lokale Direct-Grants wieder zulassen | `share.allowGrantAgain {deviceId,name?,publicKey,nodeId,fingerprint}`; volle eindeutige Pins in Profil-CAS, `{persisted:true,changed}`. Eigener Bestätigungsdialog; kein Exec. |
| Pending-Mitglieder zulassen beziehungsweise sperren/wieder zulassen | `share.setRoomMember {profileId,roomId,deviceId,name?,publicKey,nodeId,fingerprint,action:"admit|block|allow"}`; alle Statuspins geliefert. Eigener Dialog lehnt unvollständige Pins ab, native Grenze prüft aktuelle vollständige Identität. |
| Ask/AutoAccept über vorhandenen Präferenzstore speichern | `share.setPolicy {requests:"Ask|AutoAccept"}` → `{requests,persisted:true}`. Bestätigung für AutoAccept, sichtbare Getter-Warnung, kein weiterer Store. |
| Incoming-only Grant entziehen, ohne ausgehenden Kontakt zu erfinden | `share.withdrawGrant {deviceId,name?,publicKey,nodeId,fingerprint}` → `{persisted:true,changed:true}`. Der native Owner nutzt `withdraw_direct_key` unter derselben CAS einschließlich Schlüssel-/Knoten-/Alias-/Legacy-/Exec-Entzug. Explizite UI-Bestätigung und Retry angeschlossen. |
| Systemwrite allein darf paralleles RW→RO nicht überschreiben | Additives `expectedAccess:"read_only|read_write"?` an `share.setExportAccess`; gleiche native Profil-CAS prüft aktuelles Root-Recht. Android sendet es bei unverändertem Root-Recht und geänderten Systemwrite-Feld. Hauptagent meldet `a7014641`/`fc687a56`; aktueller Source bestätigt den Vertrag. |
| Kontorechteänderung darf extern entzogene Auswahl nicht neu anlegen | Additives `expectedShared:Boolean?` an `share.setConnectionExport`; gleiche Profil-CAS prüft aktuellen Auswahlzustand. Android sendet `expectedShared:true` für Rechteänderungen bestehender Auswahlen. Stale Fehler bleibt retrybar statt erneuter Freigabe. |
| H-TRASH-WINDOWS und S-LOCAL Delta-Handoff | Beide fertigen API-Deltas sind gelesen und in die zentrale API aufgenommen. Windows-Papierkorb heißt Smart-Explorer-Papierkorb; keine neue mobile Restore-Route. Private native IO-Fassade benötigt keine Kotlin-Datei oder zweite Persistenzimplementierung. |

## Verbleibende Owner-Aufgaben

| Owner | Exakte Restgrenze | Benötigtes Ergebnis |
|---|---|---|
| Hauptagent / SUITE | Android-/Native-Typprüfung und Laufzeit, Remote-Task-Suite, Integration/Commit/Push, terminaler Release | Die Signale `AND-FC1-*`, `AND-FC2-*`, `AND-FC3-SERVER`, `AND-FC5-*`, `AND-ERROR-RETRY` und `AND-COMPAT` aus dem Abnahmebericht in die eine gemeinsame Remote-Suite aufnehmen. Source-/Textchecks dieses Blocks sind keine Laufzeitabnahme. Keine gesonderte Pipeline durch diesen Block. |
| AND-SYNC | `docs/superpowers/plans/2026-09-25-android-apk/api.md`, bestehende Synchronisationsabschnitte | Eigene fertige Sync-/Job-/Versionsdeltas später in dieselbe zentrale API eintragen; dieser Block hat keine neue Sync-Bedienung oder konkurrierende zentrale Doku angelegt. |
| S-LOCAL / S09-LINK | Echter Iroh-Kanal-/Pfadnachweis für LAN-Uplink, wie in `api-delta/S-LOCAL.md` benannt | Die gemeinsame native Autoritätsgrenze bleibt ohne echten authentisierten Pfad ablehnend. Keine neue Android-Uplink-Freigabe ist erforderlich oder hier behauptet; keine mDNS-Dialinformation wird zum Opt-in. |

Keine zusätzliche Freigabe oder Rückfrage an den Nutzer erforderlich. Nach Fertigstellung dieser
Berichte stoppt der Block; die genannten Owner-Schritte erfolgen außerhalb seines Scopes.
