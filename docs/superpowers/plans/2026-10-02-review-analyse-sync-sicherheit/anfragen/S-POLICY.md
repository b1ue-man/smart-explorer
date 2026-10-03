# Anfragen S-POLICY

Stand: 2026-10-03. Die eigene FC1-Konfigurationsoberfläche ist implementiert und selbst geprüft.
Keine neue Review-Runde; nur konkrete Owner-Integration des bestehenden Plans.

| Owner | Präziser Restauftrag / Signal |
|---|---|
| Hauptagent / spätere gemeinsame Remote-Suite | Die im Abnahmebericht benannten FC1-Signale gemeinsam abnehmen; API-/Struct-Abgleich außerhalb dieses Scopes und Worker-/V5-Zustellung für echte Desktop-/CLI-/Mobile-Aktionen. `merge_user_edits` liefert jetzt `Result<(),String>`; eigene Produktaufrufer propagieren, externe Aufrufer müssen das Result erhalten. Kein lokaler Lauf durch diesen Block. |
| Hauptagent | Bekannte `load_checked(None)`-Produktionsaufrufer `native/src/cli/doctor.rs:148` und `native/src/cli/connections/output.rs:3` erhalten den Home-Fakt bzw. propagieren die neue Missing-Home-Fehlersemantik. Dateien wurden von diesem Block nicht gelesen oder geändert; exakte Stellen stammen vom Hauptagenten. |
| S-LOCAL | Der bekannte Produktionsaufrufer `native/src/share/os/shared/lan_uplink_evidence.rs:30` benötigt dieselbe Home-Fakt-/Resultsemantik. Private Anbindung des bestehenden einzigen `direct_policy_store.rs` bleibt bei S-LOCAL; keine zweite Policydatei. Diese Aufgabe wurde vom Hauptagenten zugewiesen. |
| AND-SHARE-UI | Native Delta `../api-delta/S-POLICY.md` in den bestehenden Android-Share-Bedienweg und das zentrale `api.md` übernehmen: RO/RW je Root, einzelne gespeicherte Konten mit Warnung, Kontaktwrite mit vollen Pins, Raumrechte, Share-back und Home-Hinweis/Wiederfreigabe. Kotlin/Android-UI ist außerhalb dieses Scopes und wurde nicht gelesen. |

Erledigte Vertragslücken: `creds::load_connections_checked()->Result<Vec<SavedConnection>,String>`
ist der strikte Migrations-/Bedienloader; S-LOCALs private Datei-/Lock-/Stage-API ist implementiert und
eingebunden. Die freigegebenen Room-/Removed-APIs wurden gegen ihre aktuellen Signaturen abgeglichen.
Removed-Denials scheitern weder an der direkten noch an der Legacy-64er-Requestgrenze und werden
nie abgeschnitten; die Gesamtdateigrenze meldet einen Fehler unter Erhalt der bisherigen Datei.

Keine offene eigene Implementierung, kein Stub als abgeschlossene Host-/Android-Integration gewertet.
