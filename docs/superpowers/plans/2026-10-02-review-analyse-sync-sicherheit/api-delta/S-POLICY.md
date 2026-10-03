# API-Delta S-POLICY

Stand: 2026-10-03. Additiver FC1-Konfigurationsvertrag auf bestehendem V2/V5 und S-REVOKE.
Keine Host-/Signaling-/Clienttransportänderung und keine neue Policydatei. Dieses Delta wird später
vom AND-SHARE-UI-Owner ins zentrale `api.md` übernommen.

## Persistenz / Rust

- Profilversion bleibt 8. Neu: `ShareProfiles.auto_home_migrations: Vec<AutoHomeMigration>` mit
  `{scope:String,path:String}`, serde Default leer und leer nicht geschrieben. Scope ist `direct`
  oder die vorhandene Raumprofil-ID. Dies ist ein dauerhafter historischer Migrationshinweis;
  das aktuelle Recht steht weiterhin in `SharedRoot.access`.
- Die vorhandenen Defaults bleiben: fehlendes Legacy-Root-`access` = RW, fehlendes
  Legacy-Grant-`write` = true, fehlende alte RoomPolicy = members_may_write true. Neue Root-/Account-
  Einträge sind RO, neue Grants/RoomPolicy ohne Schreiben und neue Kontakte ohne Share-back.
- Nur genaues altes Auto-Home wird eingeschränkt. Für alte Home-Kandidaten ohne Home-Fakt liefert
  `ShareProfiles::load_checked(None)` einen Fehler. Migrierte Daten werden vor Rückgabe gespeichert;
  fehlgeschlagene Migration ersetzt weder Datei noch Runtime-Profil. Der OS-Loader lädt bei einem
  tatsächlichen CAS-Konflikt bis zu fünfmal neu.
- `ShareProfiles::save/persist_replacement/mutate_persisted`: bounded JSON-Encoding und echte
  Dateigröße maximal 1 MiB; direkte und Legacy-Validierung auf allen Commitwegen. `Untracked` ist
  ausschließlich erstmalige Anlage, Replacement ist an die Revision des Ausgangssnapshots gebunden.
  Keine Ledger-Eviction als Größenreparatur. `load`-Kompatibilitätsfallback kann vorhandene Stores
  nicht überschreiben.
- Neue Methoden: `ShareProfiles::export_config_mut(&str)->Result<&mut ShareExportConfig,String>`,
  `set_direct_peer_write(&DirectPeerIdentity,bool,i64)->Result<bool,String>`,
  `set_room_policy(&str,Option<bool>,Option<bool>)->Result<bool,String>`;
  `ShareExportConfig::set_root_access(&str,ExportAccess,Option<bool>)->Result<bool,String>` und
  `set_connection_access(&str,Option<ExportAccess>)->Result<bool,String>`.
- Neues persistiertes Free-API:
  `share::set_direct_peer_write(Option<String>,&DirectPeerIdentity,bool)->Result<RelationChange,String>`.
  Es ändert nur einen bestehenden Grant mit allen aktuellen Pins; aktiver Grant ist Voraussetzung
  zum Einschalten. Es hebt keinen Widerruf auf und erzeugt keinen neuen Grant.
- `share::profile_edits::merge_user_edits` liefert jetzt `Result<(),String>`. Nur Benutzerfelder
  werden rebasiert; Erfolg ist atomar für den In-Memory-Kandidaten, Fehler propagieren zum Commit.
  Zugeordnete Desktop-/Mobile-Aufrufer sind angepasst.
- Interner Persistenzadapter: optionaler `saved_connection_accounts`-Hook; keine stillschweigende
  leere Liste bei nicht verfügbarem Provider. Systemquelle ist ausschließlich
  `creds::load_connections_checked()` und `SavedConnection::account()`. Legacyflag-Migration
  erhält aktuelle RW-Konten, bestehende RO-Ausnahmen und literal gespeicherte Endpoints.

## Mobile-Routen

Alle unten genannten Schreibaktionen speichern per bestehender CAS-Grenze und melden
`{persisted:true,changed:bool}`. Die bestehende Worker-Neukonfiguration folgt dem Commit;
deren Fehler werden wie bisher über Runtime-Log/Status behandelt. `persisted` ist kein synchroner
Transport-/Session-Abschlussnachweis. Fehler vor Commit liefern ein `ApiError`; keine neue
Autorisierung wird als Reparatur angelegt. Falsche Boolean-/Access-Typen werden abgewiesen.

| Route | Eingabe | Ergebnis / Semantik |
|---|---|---|
| `share.setExportAccess` | `scope`, exakter gespeicherter `path`, `access:read_only|read_write`, optional `allowSystemWrites:bool` | Bestehenden Root ändern; ausgelassenes Systemwrite-Feld erhält die vorherige bewusste Einstellung. |
| `share.connections` | `scope` | `connections:[{account,label,shared,access|null}]`, `sharedConnections:[{account,access}]`, `warning`. Saved-Storefehler sichtbar. |
| `share.setConnectionExport` | `scope`, exakter `account`, `shared:bool`, optional `access` | Neue Auswahl RO; ohne `access` bleibt eine bestehende Auswahl erhalten. `shared:false` entfernt genau dieses Konto (auch ein nicht mehr gespeichertes). Bei false darf kein access mitgegeben werden. |
| `share.setContactWrite` | `deviceId`, `publicKey`, `fingerprint`, `nodeId`, `write:bool`, optional `name` | Volle Pins stammen aus `writeGrants`; veränderte oder inaktive Identität kann nicht schreibend aktiviert werden. `nodeId` wird auch beim Legacy-leeren Wert mitgegeben. |
| `share.setShareBack` | `contactId`, `shareBack:bool` | Bestehendes S-REVOKE-API für die ausdrückliche Rückfreigabe; Ausschalten entfernt keine frühere bewusste Freigabe. |
| `share.setRoomPolicy` | `profileId`, `roomId`, mindestens eines von `membersMayWrite:bool`, `confirmNewMembers:bool` | Pinned Raum ändern; vorhandene Pending-/Blocked-Mitglieder werden nicht zugelassen. |
| `share.policy` | keine | `{requests:Ask|AutoAccept,warning:string|null}` aus dem bestehenden Gerätepräferenzstore. Fehlende/kaputte Präferenz wirkt als Ask. Nur lesend. |

`share.addDirect` akzeptiert optional `shareBack:bool`, Default false für die neue Kontaktanlage.
Die Antwort erweitert `{contactId}` um den kanonischen `shareBack`-Wert. Wiederholung erhält
bestehende bewusste Grants/Share-back. Scheitert die gewählte separate Rückfreigabe nach erfolgreicher
Kontaktanlage, meldet der Fehler „Kontakt … gespeichert“ samt retrybarem Folgebefehl.

Additive Felder in `share.status`, bisherige Arrays/Locators bleiben erhalten:

- `devices[]`: `shareBack:bool`, `write:bool|null` (null = keine eindeutig verknüpfte lokale Freigabe).
- `rooms[].policy`: `{membersMayWrite,confirmNewMembers}`; `rooms[].members[].admission`:
  `Admitted|Pending` neben dem bestehenden `blocked`.
- `exports.direct[]` und `exports.rooms[profileId][]`: zusätzlich `access`, `allowSystemWrites`.
- `connectionExports:{direct:[{account,access}],rooms:{profileId:[{account,access}]}}`.
- `writeGrants:[{deviceId,name,publicKey,nodeId,fingerprint,state:Accepted|Ignored|Reconfirm,
  write,active,canSetWrite}]`. `write` ist die gespeicherte Einstellung, `active` die lokale
  Grant-/Removed-Prüfung. Ein inaktives früheres Schreibrecht kann auf false zurückgenommen werden.
- `autoHomeMigrations:[{scope,path}]` für den historischen Hinweis und die bewusste Root-Wiederfreigabe.

Ältere Mobile-Clients können neue Felder/Routen ignorieren. Persistente Legacy-Rechte behalten ihre
vorhandene Defaultbedeutung; neue UI-Auswahlen verwenden explizit RO/false. Die Android-Ansicht und
das zentrale API-Dokument sind Restintegration von AND-SHARE-UI, hier keine Kotlin-/UI-Fertigmeldung.

## CLI / Desktop

Sichtbare CLI-Aliase `exports` für `export`, `contacts` für `grants`; bestehende Namen erhalten.
Neue Befehle:

```text
se share exports set PATH_OR_LABEL --write|--read-only [--room ROOM]
    [--allow-system-writes|--protect-system-files]
se share exports connections list [--room ROOM] [--json]
se share exports connections set ACCOUNT [--write|--read-only|--remove] [--room ROOM]
se share exports policy --room ROOM [--write|--read-only] [--confirm-new-members true|false]
se share contacts set NAME_OR_DEVICE --write|--read-only [--json]
```

Rechte-Schalter sind gegenseitig ausschließend. `connections set` ohne Rechteflag erzeugt nur eine
neue RO-Auswahl und erhält bestehende Rechte bei jedem CAS-Retry. Account-/Pfad-Identitäten werden
nicht normalisiert. Text/JSON der vorhandenen Export-/Grantlisten erhält Rechte, Raumpolicy,
historische Home-Hinweise und das vorhandene Ask-/AutoAccept-Gerätepräferenzsignal.

Desktop zeigt dieselben Flags in den vorhandenen Freigaben-/Raumansichten. „Schreiben wieder erlauben“
ändert das Root-Recht; tatsächliches Schreiben braucht zusätzlich das Kontakt-/Raumrecht und die
Providerberechtigung aus V2/V5. Der wirkungslose Symlink-Schalter und pauschale Verbindungswahl entfallen.
