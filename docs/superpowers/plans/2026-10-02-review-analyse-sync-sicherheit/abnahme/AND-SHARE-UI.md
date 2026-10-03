# AND-SHARE-UI – Android-Bedienung und Abnahme

Stand: 2026-10-03. Enger Folgeblock der fertigen FC1–FC5-Verträge, kein neues Review.
Scope: `scopes/and-share-ui.json`. Native Logik wird ausschließlich gelesen. Die eine
gemeinsame Remote-Suite, Commit und Integration gehören dem Hauptagenten.

## Detaillierter Plan vor Änderungen

Grundlage: aktueller Stand `d3cced7`, bestehende Share-Compose-Callbacks, Core-Fehlervertrag,
S-POLICY/S-REVOKE/S-SIGNAL/A-CLIENT/H-ANALYSIS-Deltas und lokale Material3-Referenz
(`AlertDialog`, Checkbox/FilterChip, bestehende Kotlin-Muster). Der ursprüngliche Plan
bleibt maßgeblich; diese Lückenprüfung ist ausschließlich die Implementierungsplanung
des zugeordneten Android-Blocks. Keine neue Bibliothek oder Kritik-/Reviewrunde.

| Meilenstein | Dateien / Grenze | Erwartetes Signal für die gemeinsame Remote-Suite |
|---|---|---|
| Typed Share-Vertrag | `api/ShareApi.kt`, kohäsive Policy-/Security-API-Dateien daneben | Additive Rechte, volle Pins, Migrationshinweise, ServerInfo und unbestätigte Kopplungen werden dekodiert. Explizite Entscheidungen werden mit aktuellen IDs und unveränderten Account-/Pfadbytes gesendet. Fehlende Legacy-Felder erzeugen keinen Opt-in. |
| Wiederholbare Aktionen | `ShareViewModel.kt`, `ShareDialogHost.kt`, `ShareDialogs.kt`, eigener Aktionsdialog | Ein API-Fehler bleibt sichtbar; fehlgeschlagene Aktionen sind wiederholbar. Dialog und Eingaben bleiben bis erfolgreichem Commit erhalten; Statuspoll löscht keine Aktionsfehler. Keine optimistische Rechte-/Entzugsanzeige. |
| Rechte je Root und Konto | `ShareLists.kt`, neue Export-/Kontodialoge daneben | Neue Rootanlage explizit als RO angezeigt; bestehende Rechte werden separat gespeichert. Einzelne Saved-Konten, auch bestehende nicht mehr verfügbare Auswahlen, sind RO/RW beziehungsweise entziehbar. Historischer Home-Hinweis bietet bewusste Root-Wiederfreigabe, ohne Kontakt-/Raumrecht mitzuändern. |
| Kontakte und Räume | `SharePeers.kt`, `ShareScreen.kt`, neue Policy-Dialoge daneben | Kontaktwrite benötigt alle vier Identitätspins; Share-back bleibt eigenständige ausdrückliche Wahl. Raumpolicy bindet Profil-ID und Raum-ID. Pending und Blocked bleiben bis bewusster Zulassung inaktiv; Zulassung aktiviert kein Exec/Schreiben. Entfernte Kontakte zeigen Wiederzulassen mit unveränderter ID. |
| Sichere Kopplung | `ShareDialogs.kt`, `ShareConnectDialog.kt`, neuer Kopplungshinweis | Angebot bekommt native Zufalls-PIN, Neue PIN, Weak-Opt-in und maximal 30 min. Fremde PINbytes bleiben unverändert. Zusätzlicher eigener Zugriff nur auf Auswahl; fehlende Gegenbestätigung bleibt mit Behalten/Widerrufen sichtbar, Widerrufsfehler verlieren den Hinweis nicht. |
| Server und Policy sichtbar | `SettingsScreen.kt`, eigene Share-Server-Komponente, `ShareDeviceCard.kt` | TLS ohne Schema; Klartext nur nach ausdrücklicher Zustimmung; gespeicherte Legacy-TCP-Bedeutung bleibt mit Warnung. ServerInfo-Fehler blockieren Speichern bis geladen; Speichereingaben bleiben retrybar. Ask/AutoAccept sichtbar; Änderung nur über vorhandene/ergänzte native Persistenzgrenze. |
| Zentrale API und Self-Review | bestehendes `2026-09-25-android-apk/api.md`, eigene drei Berichte | Share-, Fernanalyse-, Papierkorb-/Hostzahlendeltas konsistent zentral dokumentiert. Syncreste gehören AND-SYNC. Nur statische Text-/Parsingprüfung; kein lokaler Build, Test oder Formatter. |

Kompatibilitätsentscheidungen: keine Änderung bestehender IDs/Locators oder expliziter Legacy-Rechte.
Neue RO/false-Wahlen werden explizit übermittelt; fehlende alte Rechtefelder werden sichtbar als
unbekannt behandelt. Kein Account-Präfix wird abgeschnitten, kein Pfad für Rechteänderungen
normalisiert. Neue Roots bleiben zunächst RO, statt RO-Anlage und RW-Opt-in in einer unsicheren
Zweischrittaktion zu vermischen. Root-, Kontakt-, Raum- und Exec-Rechte bleiben getrennte Entscheidungen.
Polling aktualisiert Tatsachen; Fehlermeldungen behaupten nicht, dass bereits gespeicherte
Teilaktionen rückgängig gemacht wurden.

## Lückenprüfung vor Änderungen

- Native `share.setExportAccess`, `connections`, `setConnectionExport`, `setContactWrite`,
  `setShareBack`, `setRoomPolicy`, `policy`, `serverInfo`, `suggestPin`,
  `unconfirmedPairings`, `resolvePairing` sind registriert; vorhandene Dialoge verwenden sie noch nicht.
- Pending-Raumzulassung/-Sperre sowie Wiederfreigabe inaktiver Grants fehlen im derzeitigen
  Mobile-Dispatcher. `share.readmit` deckt entfernte Geräte, nicht jeden inaktiven Grant ab.
  `share.policy` ist derzeit nur lesend. Der Hauptagent erhält dafür konkrete native Owner-Aufträge;
  keine UI-Aktion wird als erfolgreich abgeschlossen gezählt, bevor der echte Aufrufervertrag steht.
- `api-delta/H-TRASH-WINDOWS.md` und `api-delta/S-LOCAL.md` stehen beim ersten Abgleich noch aus.
  Bestätigte A-CLIENT-Hostfähigkeiten werden übernommen, fremde neue API-Felder nicht erfunden.

## Ergebnis und statischer Self-Review

Die zugeordnete Android-Bedienung ist quellseitig abgeschlossen. Alle neuen Dialoge sind an
reale native Routen angeschlossen. Native Dateien und Registrierungen wurden von diesem Block
nicht geändert. Die gemeinsame Remote-Abnahme steht aus; eine lokale Ausführung fand nicht statt.

Die Lücken aus der Vorplanung wurden geschlossen: Der Hauptagent ergänzt `share.allowGrantAgain`,
`share.setRoomMember`, `share.setPolicy` und den Entzug rein eingehender Grants über
`share.withdrawGrant`. Vollständige Mitgliedspins stehen im Status. Die beiden atomaren
Vorbedingungen `expectedAccess` und `expectedShared` sind in der nativen Persistenzgrenze vorhanden
(Hauptagent: `a7014641`, `fc687a56`) und werden von Android verwendet. Die fertigen Deltas
H-TRASH-WINDOWS und S-LOCAL wurden in die einzige zentrale API-Dokumentation übernommen.

### Umsetzung und Fundzuordnung

| Bestehender Vertrag | Tatsächlich angeschlossene Bedienung | Entscheidung und Fehlerverhalten |
|---|---|---|
| FC1 / S-POLICY / V2 | „Nur lesen“ beziehungsweise „Lesen und Schreiben“ je Root und einzeln ausgewähltem gespeichertem Konto, getrennt für Direct und Raum; Home-Migrationshinweis mit bewusster Root-Wiederfreigabe | Neue Rootanlage wird als RO bestätigt. Kein RW-Folgeaufruf als Teil einer halbfertigen Anlage. Unbekannte Legacy-Statusrechte bleiben unbekannt. Exakte Pfad-/Accountbytes und Scope bleiben erhalten. |
| FC1 / V5 | Kontaktwrite aus `writeGrants`, Raumwrite und Bestätigung neuer Mitglieder, ausdrückliches Share-back bei Code/PIN und bestehendem Kontakt | Kontaktmutationen senden Geräte-ID, Schlüssel, Knoten und Fingerabdruck; auch ein leerer Legacy-Knoten wird mitgesendet. Root-, Kontakt-, Raum- und Exec-Rechte sind eigenständige Entscheidungen. |
| FC1 / parallele Änderungen | Dialoge speichern nur Änderungen; Raumpolicy sendet nur geänderte optionale Felder | Systemwrite allein sendet `expectedAccess`; Rechteänderung eines bereits freigegebenen Kontos sendet `expectedShared:true`. Die native CAS lehnt einen zwischenzeitlichen Entzug ab. Ein unverändertes altes Recht wird dadurch nicht erneut freigegeben. |
| FC2 / S-SIGNAL / S-REVOKE | Native Zufalls-PIN, neue PIN, Weak-Opt-in, Laufzeit höchstens 30 Minuten; explizites Share-back; Behalten/Widerrufen unbestätigter Kopplungen | Leere PIN bleibt unzulässig. Fremde nichtleere PINbytes werden unverändert gesendet. Terminaler Status wird erst nach erfolgreichem Abgleich der unbestätigten Kopplungen als Ergebnis gezeigt. Fehlender Abgleich bietet Neuladen; installierte Kopplung mit fehlender Bestätigung wird sichtbar benannt. |
| FC3 / S-SIGNAL | Servereinstellung lädt `serverInfo`, zeigt TLS/Klartext sowie Legacy-Migration und ignorierte Klartexteinträge; neue Klartexteingabe braucht ausdrückliche Zustimmung | Keine Schemainterpretation aus dem UI-Status als Sicherheitsbeweis. Die native kanonische Antwort gilt. Ladefehler blockieren Speichern; Speicherfehler erhalten Adresse und Zustimmung als retrybaren Entwurf. Zustimmung wird bei Eingabeänderung zurückgesetzt. |
| FC4 / bestehender nativer Vertrag | Status und zentrale API beschreiben native TLS-/Schlüsselanmeldung und Zertifikatspin korrekt | Keine Android-Serverimplementierung oder zusätzliche Vertrauensentscheidung. Zertifikat-/Transportfehler bleiben native Fehler. Interne Protokollfelder sind keine neuen Kotlin-Berechtigungen. |
| FC5 / S-REVOKE / V5 | Pending-Mitglieder zulassen, sperren und wieder zulassen mit voller Raum-/Profil-/Mitgliedsbindung; inaktive lokale Grants wieder zulassen; eingehenden Zugriff ausdrücklich entziehen; entfernte Geräte erneut zulassen | Wiederzulassung und Share-back erfolgen nach eigener Bestätigung. Kein Exec-Opt-in. Entzug eines incoming-only Grants braucht keine erfundene `contactId` und nutzt die native Schlüssel-/Knoten-/Alias-/Legacy-Grenze. Persistenzfehler schließen keinen Dialog und löschen keinen Hinweis. |
| FC5 / S-POLICY | Ask/AutoAccept anzeigen und mit ausdrücklicher Bestätigung ändern | Nutzung des vorhandenen privaten Präferenzstores, keine zweite Policydatei. Ask-Warnung aus kaputten/fehlenden Präferenzen bleibt sichtbar. Status-/Transportflags löschen keine Widerrufshistorie. |
| FA1/FA2/FA6 / A-CLIENT / H-ANALYSIS / H-TRASH-WINDOWS | Zentrale API enthält Fernanalyse, Hostzahlen, Budgets, Release/Retention, verifizierte Duplikate, Papierkorbfähigkeit und Teilergebnisse | Hostzahlen werden nicht durch Clientzahlen ersetzt. Unbekannte Fähigkeit ermöglicht keine Löschaktion. Windows heißt Smart-Explorer-Papierkorb. H-TRASH-WINDOWS erweitert keine mobile Methode; Wiederherstellung dort ist eine Host-Bedienung. |

### Reale Callback- und Retry-Grenze

- `ShareDialogHost` registriert die Export-, Konto-, Kontakt-, Raum-, Wiederzulassungs-,
  Entzugs-, Policy- und Kopplungsdialoge. Listen und Menüs öffnen diese Dialoge mit dem aktuellen
  Objekt samt IDs/Pins. Ein lediglich angezeigter Snapshot löst keinen Mutationsaufruf aus.
- `ShareActionState` verhindert den doppelten Start während Speichern. Die Aktion läuft im
  ViewModel; die UI schließt erst nach Erfolg. Eine bereits entsorgte Dialoginstanz kann durch
  einen späten Retry keinen neu geöffneten Dialog schließen.
- `ShareViewModel.actionFailures` behält mehrere Fehler getrennt. Jeder hat eigene Wiederholung
  und explizites Schließen. Erfolgreiches Polling oder eine andere erfolgreiche Aktion löscht
  sie nicht; die erfolgreiche Wiederholung entfernt nur ihren eigenen Fehler.
- Konto-Ladefehler erlauben nur den Entzug bereits bekannter Auswahlen. Keine neue Freigabe aus
  einer still leeren oder veralteten Saved-Liste. Nicht mehr gespeicherte Konten bleiben einzeln
  entziehbar; Neu-Freigabe/RW-Auswahl für sie bleibt gesperrt.
- `persisted:true` bestätigt den nativen Profilcommit. Es behauptet keinen synchronen Abschluss
  aller Transportsitzungen. Rechteanzeigen folgen dem neu geladenen Core-Status; Workerfehler
  und Kopplungshinweise bleiben sichtbar. Endpoint-Cleanupberichte vorhandener Entfernen-Aktionen
  werden weiterhin angezeigt.
- Bestehende Exec-Dialoge, Warnung vor Exec-Host-Opt-in und exakte `targetKey`-Bindung bleiben
  erhalten. Neue Policy-/Share-back-/Wiederzulassungsaktionen rufen kein `setExec` auf.

### Exakte Datei-Inventur

Gelesene Arbeitsvorgaben: `AGENTS.md` als Sessionvorgabe und
`/root/.codex/skills/arbeitsweise/SKILL.md`. Kein Graphzugriff, keine breite Exploration.
Alle nachfolgend geänderten oder neuen Dateien wurden auch für den eigenen Self-Review gelesen.

Zusätzlich gelesene, unveränderte Dateien:

- `docs/ARCHITEKTUR.md`
- `docs/refs/compose-material3.md` (relevante Material3-/Checkbox-/Dialogabschnitte)
- `docs/refs/android-apis.md` (gezielter API-Abgleich)
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/scopes/and-share-ui.json`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/spec.md` (FC1–FC5)
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/umsetzung.md` (Block und bestehende Verträge)
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/review-befunde-sicherheit.md` (zugeordnete Befunde)
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/A-CLIENT.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/H-ANALYSIS.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/S-POLICY.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/S-REVOKE.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/S-SIGNAL.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/H-TRASH-WINDOWS.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/S-LOCAL.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/H-TRASH-WINDOWS.md`
- `android/app/src/main/java/app/smartexplorer/android/core/Core.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/share/ExecDialog.kt`
- `android/app/src/main/java/app/smartexplorer/android/ui/share/ExecHostSection.kt`
- `native/src/mobile/os/shared/domains/mod.rs`
- `native/src/mobile/os/shared/domains/share_peers.rs`
- `native/src/mobile/os/shared/domains/share_settings.rs`
- `native/src/mobile/os/shared/domains/share_requests.rs`
- `native/src/mobile/os/shared/domains/share_status.rs`
- `native/src/mobile/os/shared/domains/share_policy.rs`
- `native/src/mobile/os/shared/domains/share_policy_status.rs`
- `native/src/share/core/discovery_pin.rs`

Geänderte bestehende Dateien:

| Exakter Pfad | Inhalt |
|---|---|
| `android/app/src/main/java/app/smartexplorer/android/api/ShareApi.kt` | Additive Statusfelder; ausdrückliche Opt-in-Overloads ohne Änderung bisheriger Aufrufsignaturen/Rückgaben |
| `android/app/src/main/java/app/smartexplorer/android/ui/settings/SettingsScreen.kt` | Einbindung der eigenen Share-Server-Komponente |
| `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareConnectDialog.kt` | Explizites Share-back, unveränderte fremde PINbytes, unbestätigte/noch ungeklärte Terminalzustände |
| `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareDeviceCard.kt` | Tatsächlicher Server-Sicherheitsstatus aus passender nativer ServerInfo |
| `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareDialogHost.kt` | Eigene Dialogvarianten und echte Callback-Verkabelung; RO-Rootbestätigung; Schließen nach Erfolg |
| `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareDialogs.kt` | Fehlertolerante vorhandene Eingabedialoge; native PIN-Vorschläge, Weak-Opt-in und Share-back |
| `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareLists.kt` | Root-/Kontorechte, Home-Hinweis/Wiederfreigabe, entfernte Geräte |
| `android/app/src/main/java/app/smartexplorer/android/ui/share/SharePeers.kt` | Kontakt- und Raumpolicy, Pending/Blocked-Mitglieder, explizite neue Menüpunkte |
| `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareScreen.kt` | Policy-/Grant-/Kopplungsabschnitte und getrennte retrybare Fehler |
| `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareViewModel.kt` | Aktionsfehler mit eigener Retry-Grenze; Policy-/Security-/Pairing-Laden; vorhandene Send-/Exec-Abbruchfehler sichtbar |
| `docs/superpowers/plans/2026-09-25-android-apk/api.md` | Einzige zentrale API-Fassung mit Share-/Fernanalyse-/Host-/Papierkorbdeltas und Legacy-Defaults |

Neue kohäsive Featuredateien unmittelbar neben eigenen Dateien:

| Exakter Pfad | Verantwortung |
|---|---|
| `android/app/src/main/java/app/smartexplorer/android/api/SharePolicyApi.kt` | Typed Rechte-/Policy-/Pin-Modelle und persisted-geprüfte Mutationen |
| `android/app/src/main/java/app/smartexplorer/android/api/ShareSecurityApi.kt` | ServerInfo, PIN-Vorschlag und unbestätigte Kopplungen |
| `android/app/src/main/java/app/smartexplorer/android/ui/settings/ShareServerSettings.kt` | Share-Server-Draft, Klartextopt-in und Fehler/Retry |
| `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareActionDialog.kt` | Gemeinsame Aktions-/Dialogzustände und zugängliche explizite Checkboxwahl |
| `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareConfirmationDialogs.kt` | Gepinnte Mitglieds-/Grantentscheidungen, Entzug, Policy und Pairing-Auflösung |
| `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareConnectionDialog.kt` | Strikte Saved-Auswahl und Einzelkontorechte mit Entzugsretry |
| `android/app/src/main/java/app/smartexplorer/android/ui/share/SharePolicySections.kt` | Eingehende Grantrechte und unbestätigte Kopplungen anzeigen/bedienen |
| `android/app/src/main/java/app/smartexplorer/android/ui/share/ShareRightsDialogs.kt` | Root-/Kontakt-/Raumrechte und Share-back; nur geänderte Felder speichern |

Neue eigene Berichte:

- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/abnahme/AND-SHARE-UI.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/anfragen/AND-SHARE-UI.md`
- `docs/superpowers/plans/2026-10-02-review-analyse-sync-sicherheit/api-delta/AND-SHARE-UI.md`

### Self-Review und statische Nachweise

Die Doku-Kontextprüfung nutzte `git status --short`, `git log -1 --oneline`, gezielte Quellen-/Deltasuchen
und die freigegebenen nativen Dispatcher-/Facade-Dateien. Der Self-Review folgt den tatsächlichen
Dialogcallbacks bis zu den gespeicherten Methodenargumenten; insbesondere volle Pins, Nullable-
Legacyrechte, gezielte optionale Felder, paralleler Entzug, verspätete Dialogantworten und persistente
Fehlereinträge wurden abgeglichen. Kein separater Prüfer und kein neues Review.

Statische Text-/Parsingprüfungen: Kotlin-Kommentar-/String-/Interpolations- und Klammerbalance,
Dateigrößen und Zeilenlimits, doppelte Imports, nachlaufende Leerzeichen, Markdown-Codeblöcke,
`git diff --check` auf eigenen Pfaden sowie Abgleich aller eigenen `share.*`-API-Literale mit
dem nativen Dispatcher und der zentralen API. Alle bearbeiteten/neuen Kotlin-Dateien bleiben
unter 500 Zeilen und unter 50 KiB. Diese Prüfungen ersetzen keine Kotlin-Typprüfung oder
Android-/Netzwerk-Laufzeitabnahme.

Keine lokalen Builds, Tests, Compiler, Formatter, Server, Installationen, Commits, Pushes,
CI-, Graph- oder Releaseaktionen. Suite-Signale werden nur übergeben; ihre Ausführung gehört
der einen gemeinsamen späteren Remote-Suite des Hauptagenten.

### Konkrete Signale für diese gemeinsame Remote-Suite

| Signal | Auslöser / Erwartung |
|---|---|
| AND-FC1-ROOT | Frisches Profil hat keine Freigabe. Android-Rootbestätigung erzeugt RO im ausgewählten Scope. Bestehendes explizites Legacy-RW bleibt bis eigener Änderung erhalten. Fehlendes Statusrecht erscheint unbekannt, kein stiller Opt-in. |
| AND-FC1-HOME | Historischer Migrationshinweis bleibt sichtbar. „Schreiben wieder erlauben“ ändert nach Bestätigung nur den exakt gespeicherten Root; IDs/Locators, Kontakt-, Raum- und Exec-Rechte bleiben erhalten. |
| AND-FC1-DELTA | Unveränderte Rechte sind nicht speicherbar. Reine Systemwrite-Änderung sendet `expectedAccess`; zwischenzeitliches RW→RO wird abgelehnt. Raum-Bestätigungsänderung sendet kein unverändertes Schreibflag und erhält dessen neueren Wert. |
| AND-FC1-ACCOUNT | Zwei gespeicherte Konten mit gleichen relativen Pfaden bleiben verschieden. Neue Auswahl RO, bewusstes RW nur auf gewähltem Konto/Scope. Orphan-Auswahl kann entzogen werden. Saved-Ladefehler bietet Retry und erlaubt keine neue Freigabe. |
| AND-FC1-ACCOUNT-CAS | Bereits ausgewähltes Konto wird extern entzogen, während dessen Rechteeditor offen ist. Speichern sendet `expectedShared:true`, schlägt retrybar fehl und erzeugt keine neue Auswahl. |
| AND-FC1-CONTACT | Kontaktwrite sendet alle vier Pins einschließlich leerem Legacy-nodeId. Stale oder mehrdeutige Identität scheitert ohne Rechtserweiterung. Inaktives altes Schreibflag lässt sich entziehen, Schreiben nicht ohne ausdrückliche Wiederzulassung erlauben. |
| AND-FC1-ROOM | Raumpolicy bindet Profil- und Raum-ID. Pending bleibt ohne Zulassung inaktiv; Blocked bleibt gesperrt. Admit/Block/Allow senden volle Pins; fehlende Pins sperren den Mutationsknopf. Keine Aktion aktiviert Exec. |
| AND-FC1-SHAREBACK | Direct-Code und PIN-Kopplung senden ohne Auswahl `shareBack:false`; eigene Rückfreigabe nur per ausdrücklicher Wahl. Alte bewusste Grants werden durch false nicht gelöscht. Bei bereits angelegtem Kontakt und Rückfreigabefehler bleibt der native Teilresultattext mit Retry sichtbar. |
| AND-FC2-PIN | Native sechsstellige Zufalls-PIN und „Neue PIN“; kurze/triviale PIN nur nach neuer Zustimmung; leer auch dann gesperrt. Maximal 30 Minuten. Nichtleere fremde Legacy-/Unicode-PIN wird bytegetreu gesendet. |
| AND-FC2-CONFIRM | Kopplung bereits installiert, Gegenbestätigung fehlt: sichtbarer Unconfirmed-Hinweis statt Erfolg/Fehlschlag. Behalten und tatsächlicher Widerruf sind getrennt. Fehlgeschlagener Entzug erhält Hinweis und Retry. `roomShared` erklärt fehlende Rückholbarkeit. Terminalzustand mit fehlgeschlagener Pairing-Abfrage bietet Neuladen. |
| AND-FC3-SERVER | Neue nackte Adresse wird WSS/TLS, neue Klartexteingabe braucht Auswahl. Eingabeänderung setzt Zustimmung zurück. Alte nackte Adresse bleibt als kanonisches TCP mit Warnung erhalten; ignorierte Klartexteinträge einer TLS-Liste werden angezeigt. Kein Loadfehler wird zur leeren Serverlöschung. |
| AND-FC5-GRANT | Incoming-only Grant ohne Kontakt kann nach Bestätigung per `withdrawGrant` entzogen werden. Nativer Ignored-/Alias-/Legacy-/Exec-Entzug bleibt sichtbar. Inaktiver Grant und entfernte Geräte sind ausdrücklich wieder zulassbar, ohne Exec-Opt-in oder freie Secretlöschung. |
| AND-FC5-ASK | Ask/AutoAccept tatsächlich gespeichert über vorhandenen Store. AutoAccept braucht Bestätigung; kaputte Präferenz zeigt Ask und Warnung. Neustart/Poll/Runtimeflags kürzen oder löschen keine Withdraw-/Legacy-Historie. |
| AND-ERROR-RETRY | I/O-/CAS-/Identityfehler erhalten Draft/Dialog und eigenen Fehler. Wiederholter Tap während Speichern startet nicht doppelt. Anderer Erfolg/Poll löscht den Fehler nicht; eigener erfolgreicher Retry schon. Später Retry eines geschlossenen Dialogs schließt keinen neuen Dialog. |
| AND-COMPAT | Vorhandene Öffnen-/Senden-/Entfernen-/Exec-Bedienung behält Locators, Cleanupbericht, Konfliktmodus und explizite Exec-Warnung. PIN-Abbruch/Timeout verlieren keinen sichtbaren Unconfirmed-Endzustand. |

Für Fernanalyse-/Hostzahlen-/Duplikat-/Papierkorbverträge gelten zusätzlich die bereits konkreten
Signale in [A-CLIENT](A-CLIENT.md), [H-ANALYSIS](H-ANALYSIS.md) und
[H-TRASH-WINDOWS](H-TRASH-WINDOWS.md), gesammelt in derselben Suite. Keine zweite Ausführung.
Owner-Restgrenzen stehen in [anfragen/AND-SHARE-UI.md](../anfragen/AND-SHARE-UI.md),
die exakten Consumer-Verträge in [api-delta/AND-SHARE-UI.md](../api-delta/AND-SHARE-UI.md).
