# AND-SHARE-UI – Android-Consumer und zentrale API

Stand: 2026-10-03. Native Logik ausschließlich gelesen. Der Hauptagent liefert die neuen
nativen Facaden und Registrierungen; Android verwendet sie tatsächlich. Die einzige zentrale
API ist [2026-09-25-android-apk/api.md](../../2026-09-25-android-apk/api.md).
Genaue gelesene/geänderte/neue Pfade und Suite-Signale stehen in
[abnahme/AND-SHARE-UI.md](../abnahme/AND-SHARE-UI.md).

## Additive Statusmodelle und Legacyverhalten

`ShareApi.kt` ergänzt Felder am Ende bestehender Konstruktoren und behält bisherige Parameterreihenfolge:

| Modell / Feld | Native Form / Kotlin-Fallback |
|---|---|
| `ShareDevice.shareBack`, `write` | Boolean / false; Boolean? / null bei unbekannter oder nicht eindeutig verknüpfter lokaler Freigabe |
| `ShareMember.admission`, `publicKey`, `nodeId`, `fingerprint` | String? / null; fehlende Pins verhindern Mutation, ein tatsächlich geliefertes leeres Legacy-nodeId bleibt erhalten |
| `ShareRoom.policy` | `ShareRoomPolicy?` / null; enthaltene `membersMayWrite` und `confirmNewMembers` müssen beide vorhanden sein |
| `ShareExport.access`, `allowSystemWrites` | String? beziehungsweise Boolean? / null; fehlendes altes UI-Statusrecht bleibt sichtbar unbekannt und wird nicht automatisch als Schreibrecht interpretiert |
| `ShareStatus.connectionExports` | Direct-/Room-Listen, leerer kompatibler Fallback; Room-Key ist Profil-ID |
| `ShareStatus.writeGrants` | `List<ShareWriteGrant>?` / null; unbekannte alte Antwort ist unterscheidbar von neuer bekannter leerer Liste |
| `ShareStatus.autoHomeMigrations` | Liste `{scope,path}` / leer; nur native aufgezeichnete historische automatische Home-Migration erzeugt Wiederfreigabehinweis |

`SharePolicyApi.kt` stellt dafür `SHARE_READ_ONLY="read_only"`, `SHARE_READ_WRITE="read_write"`,
`ShareRoomPolicy`, `ShareConnectionExport(s)`, `ShareWriteGrant`, `ShareHomeMigration`,
`ShareSavedConnection`, `ShareConnections` und `ShareRequestPolicy` bereit. `ShareWriteGrant`
verlangt Geräte-ID, publicKey, nodeId, fingerprint sowie state/write/active/canSetWrite;
nur der Anzeigename hat einen leeren Fallback. Pins werden nicht aus Namen oder Locations abgeleitet.

Persistente Legacy-Defaults im nativen Profil bleiben davon getrennt: fehlendes altes Root-access
bedeutet RW, altes fehlendes Grant-write true, alte Raumpolicy erhält frühere Rechte. Neue native
Profile/Räume ohne Standardexports, neue Roots/Konten RO, neue Grants/Räume ohne Schreiben und neue
Kontakte ohne Share-back gelten wie in S-POLICY. Android kopiert diese Migration nicht in einen
zweiten Store und ersetzt keine alten Identitäten/Account-/Pfadbytes.

## Tatsächlich aufgerufene Rechte-/Policymethoden

Alle folgenden `SharePolicyApi`-Mutationen erwarten `persisted:true`. Fehlendes/false persisted
wird als `CoreException` weitergegeben; ein Dialog schließt erst nach erfolgreicher Antwort.
`changed` ist ein additives Responsefeld, keine Aussage über bereits beendete Transportsitzungen.

| Methode | Argumente | Antwort / Android-Verwendung |
|---|---|---|
| `share.connections` | `{scope}` | `{connections:[{account,label,shared,access?}],sharedConnections:[{account,access}],warning}`; strikter Saved-Lader, einzelne Auswahl mit Credential-Warnung |
| `share.setExportAccess` | `{scope,path,access,allowSystemWrites?,expectedAccess?}` | `{persisted:true,changed}`; Rootrechte/Home-Wiederfreigabe. Systemflag nur wenn geändert. `expectedAccess` nur bei Systemwrite-Änderung ohne bewusste access-Änderung. Gleiche native CAS lehnt abweichendes aktuelles Root-Recht ab. |
| `share.setConnectionExport` | `{scope,account,shared,access?,expectedShared?}` | `{persisted:true,changed}`; `shared:false` ohne access, auch für nicht mehr gespeichertes Konto. Bestehende Auswahlrechte mit `expectedShared:true`; neue bewusst ausgewählte Konten explizit RO/RW. Gleiche CAS lehnt zwischenzeitlichen Entzug ab. |
| `share.setContactWrite` | `{deviceId,name?,publicKey,nodeId,fingerprint,write}` | `{persisted:true,changed}`; volle aktuelle Pins aus Grant, Leerstring-Legacy-node mitgeben. Inaktiv nur altes Schreibflag entziehen, kein neues Recht. |
| `share.allowGrantAgain` | `{deviceId,name?,publicKey,nodeId,fingerprint}` | `{persisted:true,changed}`; ausdrücklich bestätigte bestehende inaktive Identität erneut zulassen, kein Exec |
| `share.withdrawGrant` | `{deviceId,name?,publicKey,nodeId,fingerprint}` | `{persisted:true,changed:true}`; ausdrücklich bestätigter Schlüssel-/Knoten-/Alias-/Legacy-/Exec-Entzug auch incoming-only ohne contactId, kein Secretcleanup |
| `share.setShareBack` | `{contactId,shareBack}` | `{persisted:true,changed}`; bewusster Opt-in kann wieder zulassen, false nimmt frühere ausdrückliche Freigabe nicht zurück |
| `share.setRoomPolicy` | `{profileId,roomId,membersMayWrite?,confirmNewMembers?}` | `{persisted:true,changed}`; mindestens ein Feld, nur tatsächlich geänderte Felder senden. Bestätigungsänderung spielt kein unverändertes altes Schreibflag zurück. |
| `share.setRoomMember` | `{profileId,roomId,deviceId,name?,publicKey,nodeId,fingerprint,action:"admit|block|allow"}` | `{persisted:true,changed}`; volle aktuelle Profil-/Raum-/Mitgliedsbindung, fehlende Pins bereits in UI gesperrt, native Mehrdeutigkeit/Stale wird erneut geprüft |
| `share.policy` | `{}` | `{requests:"Ask|AutoAccept",warning?}`; bestehender privater Präferenzstore; Warnung sichtbar |
| `share.setPolicy` | `{requests:"Ask|AutoAccept"}` | `{requests,persisted:true}`; AutoAccept braucht eigene Bestätigung, kein weiteres Preferencefile |

Kotlin-Helfersignaturen (alle suspend; Mutationen geben Unit zurück):

```kotlin
SharePolicyApi.connections(scope: String): ShareConnections
SharePolicyApi.policy(): ShareRequestPolicy
SharePolicyApi.setPolicy(requests: String)
SharePolicyApi.setExportAccess(scope: String, path: String, access: String,
    allowSystemWrites: Boolean? = null, expectedAccess: String? = null)
SharePolicyApi.setConnectionExport(scope: String, account: String, shared: Boolean,
    access: String? = null, expectedShared: Boolean? = null)
SharePolicyApi.setContactWrite(grant: ShareWriteGrant, write: Boolean)
SharePolicyApi.allowGrantAgain(grant: ShareWriteGrant)
SharePolicyApi.withdrawGrant(grant: ShareWriteGrant)
SharePolicyApi.setShareBack(contactId: String, shareBack: Boolean)
SharePolicyApi.setRoomPolicy(room: ShareRoom, membersMayWrite: Boolean? = null,
    confirmNewMembers: Boolean? = null)
SharePolicyApi.setRoomMember(room: ShareRoom, member: ShareMember, action: String)
```

## Server-/PIN-/Kopplungsconsumer

`ShareSecurityApi.kt` besitzt keine eigene Transport-/Persistenzlogik:

| Methode | Native Argumente / Antwort | Tatsächliche Bedienung |
|---|---|---|
| `serverInfo(): ShareServerInfo` | `share.serverInfo {}` → `{server,security:"encrypted|plaintext|none",summary,plaintext,ignoredPlaintext,migrated}` | Settings-Draft erst nach geladenen Fakten speicherbar. ServerCard verwendet Info nur zur identischen Statusadresse. server/security sind Pflichtfelder. |
| `setServer(server,allowPlaintext): ShareServerInfo` | `share.setServer {server,allowPlaintext}` → gleiche Form | Neue schemafreie Adresse TLS/WSS; ausdrücklicher Klartextopt-in. Legacy-TCP-Bedeutung bleibt sichtbar. Kein TLS-Klartext-Fallback behauptet. |
| `suggestPin(): String` | `share.suggestPin {}` → `{pin}` | Native sechsstellige Zufalls-PIN, erneut generieren, kein eigener Zufallsgenerator |
| `unconfirmedPairings(): List<UnconfirmedPairing>` | `share.unconfirmedPairings {}` → `{pairings:[{exchangeId,kind,contactId?,roomProfileId?,label,revocable}]}` | Antwortliste sowie Identität/kind sind Pflicht; fehlende/malformed Antwort gilt nicht als erfolgreich geprüfte leere Liste. Native Hinweise bleiben bis tatsächlicher Auflösung. |
| `resolvePairing(exchangeId,revoke)` | `share.resolvePairing {exchangeId,revoke}` → `{}` oder bestehender Endpoint-Cleanupbericht bei Entzug | Behalten schließt nur Hinweis; Widerruf tatsächlicher installierter Beziehung. Fehler erhält Hinweis/Retry. `roomShared` zeigt fehlende Rückholbarkeit. |

In `ShareApi` bleiben alte Aufrufsignaturen und Rückgabetypen erhalten:

```kotlin
setServer(server: String)                         // Unit; allowPlaintext=false
setServer(server: String, allowPlaintext: Boolean) // Unit
discoverable(target: String, alias: String, pin: String, minutes: Int) // Unit; false
discoverable(target: String, alias: String, pin: String, minutes: Int, allowWeakPin: Boolean)
connect(discoveryId: String, pin: String)          // Unit; shareBack=false
connect(discoveryId: String, pin: String, shareBack: Boolean)
addDirect(code: String, name: String): String      // contactId; shareBack=false
addDirect(code: String, name: String, shareBack: Boolean): String
```

Dialoge nutzen die neuen längeren Overloads mit ausdrücklichen Checkboxwerten. Weak-Opt-in ist
nach PINänderung zurückgesetzt; nichtleere fremde PIN bleibt unverändert, leere immer gesperrt.
Publizieren/Koppeln wird nativ eingereiht, nicht als synchron fertig installierte Transportsitzung
behauptet. Unbestätigte installierte Kopplung beziehungsweise unbekanntes Ergebnis nach fehlerhafter
Pairing-Abfrage bleibt sichtbar und kann neu abgeglichen werden. Keine implizite Rückfreigabe/Exec.

## Fehler- und Bestätigungssemantik

Core-Fehler werden als `CoreException(kind,message)` behandelt; PINregel enthält `weak_pin`,
Identitäts-/CAS-/Persistenz-/Ladefehler bleiben der native lesbare Text. Native falsche oder
fehlende persisted-Antwort und unvollständige Pinantwort werden nicht zum UI-Erfolg.

`ShareActionState.submit` führt genau einen gleichzeitigen Start je Dialog aus. Draft und Fehler
bleiben bis Erfolg. `ShareViewModel.act` hält mehrere `ShareActionFailure`-Einträge mit eigenem Retry;
unabhängige Aktionen/Polls löschen sie nicht. Ein Retry geschlossener Dialoge hat keine Bindung an
eine neu geöffnete Instanz. Retry eines Entzugs wiederholt dessen bestehende IDs/Pins und Rechte,
keine ersatzweise Neufreigabe. Bei Stale-Fehler kann der Nutzer schließen und aktuelle Tatsachen neu laden.

Raum-Bestätigung, Grantzulassung, Share-back und Entzug bleiben getrennt von bestehenden
`share.setExec`-Opt-ins mit Warnung und exaktem targetKey. Block/Pending erhält keine Öffnen-/Senden-/
Exec-Aktion. Alte fehlende admission-Felder blockieren keine etablierte Browse-Funktion.

## Übernommene fremde native Deltas in der zentralen API

- A-CLIENT/H-ANALYSIS: `analyze.start`, issue/note-Zahlen, Remote-Phasen, `remote`/Hostzahlen,
  vorgeschaltete node-/Speicherbudgets, separate Ergebnisfreigabe, verifizierte Duplikate und
  capability-gesteuerter Papierkorb. Hash-/Größenbindung, genauer moved-Teilerfolg bei Abbruch und
  erhaltene letzte Kopie sind dokumentiert. Fehlende Hostdaten werden als unbekannt beschrieben,
  ohne lokale Zahlen als Ersatz. Vorhandenes `fs.delete` bleibt für seine bisherige Oberfläche erhalten.
- Agent-Protokoll 11/IPC: special-Metadaten, `ReadyPartial`/WireChange-Kind 4, optionales
  node_budget, additive Share-Watch-complete false und Duplikat-more false mit Legacy-Fallbacks.
  Dies sind interne Wire-Verträge, keine neuen Kotlinmethoden oder Berechtigungsgrants.
- H-TRASH-WINDOWS: bestehende Recycle-Antworten bleiben unverändert. Die zentrale API verwendet
  den Namen Smart-Explorer-Papierkorb, ohne Windows-Systempapierkorb oder permanenten Fallback zu
  behaupten. Host-Wiederherstellung ist keine neue mobile Route.
- S-LOCAL: private gemeinsame native IO-Grenze mit Owner-/No-follow-/Hardlinkprüfung, exklusiver
  privater Stage, Dateisync und atomarem Ersatz; keine Android-Paralleldatei, neue Uplinkautorität
  oder unbelegte Windows-Verzeichnis-Dauerhaftigkeit. Authentisierte S09-LINK-Fakten bleiben Ownergrenze.
- S-POLICY/S-REVOKE/S-SIGNAL: Legacy-Defaults, aufgezeichnete Home-Migration, unveränderte
  Endpoint-/Exportbedeutungen, volle Pins, private Profil-CAS, tatsächliches 1-MiB-Profillimit,
  ungekürzte Widerrufshistorie, Ask/AutoAccept, ausdrückliches Share-back, TLS-/Plaintext-/PIN-Regeln.

Synchronisationsdeltas werden später durch AND-SYNC in dieselbe zentrale API aufgenommen.
Keine zentrale Doku-Alternative, kein nativer Eigentumswechsel, keine lokale Ausführung.
