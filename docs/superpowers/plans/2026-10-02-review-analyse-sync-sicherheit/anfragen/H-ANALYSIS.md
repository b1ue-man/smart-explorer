# H-ANALYSIS – Integrationsanfragen und Fähigkeitsgrenzen

Stand: 2026-10-03. Eigener Host-/Protokollblock statisch abgeschlossen; kein neuer Review.
Die [Abnahme](../abnahme/H-ANALYSIS.md) enthält Befundzuordnung, Quellen- und Änderungsmanifest.
Es gab keine lokale Ausführung und keine Änderungen außerhalb des zugewiesenen Scopes.

## 1. Windows-FA6: sicherer, auffindbarer Papierkorbanschluss – offen, Hauptagent

`native/src/analytics/os/windows.rs::publish_trash` gibt ausdrücklich Unsupported zurück.
`host_recycle_available()` ist auf Windows false; die zentrale
`native/src/share/core/wire_capabilities.rs::FsHostFeatures::host`-Maske bietet
`remote_trash_v1` dort deshalb nicht an. Der geprüfte Quarantäne-Einfang wird bei dieser
Veröffentlichungsgrenze ohne Ersetzung zurückgestellt. Restore-Konflikte erhalten den Inhalt
und melden `retained_location`.

Der Hauptagent schließt später den echten sicheren Wiederherstellungsanschluss: entweder eine native
Operation über den bestätigten Guard mit erhaltenem Windows-Papierkorb oder ein im bestehenden
Host-Store explizit auffindbarer Wiederherstellungsweg. Ein bloßer freier Pfad darf keine Shelloperation
auslösen, ein unauffindbarer Desktop-Apptrash-Store darf nicht Recycled behaupten. Es gibt keinen
Permanent-Fallback und keine globale Desktop-`apptrash::set_volumes`-Aktivierung.

Abnahmesignal: erwartete Identität/Länge/SHA bleiben bis zur reversiblen Veröffentlichung gebunden,
Restore ist auffindbar, Ersatzdateien werden weder verschoben noch überschrieben. Erst danach darf die
OS-API true melden. Diese Grenze ist kein offener Implementierungsschritt des eigenen abgeschlossenen
Host-/Protokollblocks; FA6 ist dadurch noch keine vollständige plattformübergreifende Abnahme.

## 2. Watch-Abdeckung – sicherer begrenzter Anschluss abgeschlossen, OS-Ausbau offen

T-JOBS hat `watch::watch_confined(&DirectoryHandle, &Path, WatchOptions, WatchFilter, WatchSink)`
additiv bereitgestellt. Exakter Host-Aufrufer:
`native/src/share/os/shared/host_watch.rs::setup`. Er öffnet über
`storage_roots::open_local` ausschließlich den zugelassenen Root, übergibt den gehaltenen Pin und
hält sowohl DirectoryHandle als auch WatchHandle für die gesamte Share-Sitzung.

Linux/Android beobachten über den gehaltenen FD nur direkte Kinder. Dies bleibt ehrlich
`Ready.complete=false` → `ChangeNotice::ReadyPartial`; Hybrid-Polling bleibt aktiv.
Windows ist Unsupported; eine fehlende angefragte Childroot ist Unavailable. Ein zukünftiger Ausbau
gehört dem Watch-Owner: sichere rekursive Handle-Abdeckung auf unterstützten Plattformen sowie ein
gehaltener Elternhandle und unverändert geprüfter Childname für das Wiedererscheinen einer Root.
H-ANALYSIS nutzt keinen freien Pfad als Wiederanlauf.

Abnahmesignal des jetzigen Vertrags: aktive partielle Hinweise erzeugen kein falsches Ended;
Reset/Transportende und Cancelled schließen den Strom sauber; Overflow/Ready werden bei Rückstau
nachgeliefert. Vollständige Ereignisabdeckung wird bei alten Meldungen ohne complete-Feld nie angenommen.

## 3. Android-Host-Figuren – konkrete Produzentenintegration offen, Hauptagent/Android-Owner

Die eigene Empfänger-/Hostseite benutzt
`native/src/analytics/core/host_figures.rs::remember_platform_totals(volume, totals)`
und bindet Figuren nur an die tatsächliche kanonische Host-Primärwurzel.
`native/src/analytics/os/android.rs` ermittelt Host-Volumenwerte; der Share-Scan übernimmt keine
Client-Zahlen. Der Android-Owner muss den vorhandenen Plattform-Produzenten beim erfolgreichen
Aktualisieren eigener App-/Speichertotals mit dieser API verbinden. Fremde Android-Quelldateien
wurden für diesen Anschluss nicht erkundet.

Abnahmesignal: Share-Analyse des Android-Primärvolumens enthält aktuelle eigene Apps/geschützte
Bereiche/nicht erfassten Speicher; eine andere SD-/Freigabewurzel erhält keine unpassenden
Primärvolumen-Appzahlen. Ergebnisretention berechnet beide
`PlatformFigures::estimated_heap_bytes()` und `Approximations::estimated_heap_bytes()`.

## Koordinierte abgeschlossene bzw. vom Integrationsowner zu bewahrende Handoffs

| Owner | Exakter Vertrag / eigene Anschlussdateien | Status |
| --- | --- | --- |
| V-LOCAL | DirectoryHandle ordinary/confined für `analytics_walk`, `finder_walk`, `host_hash_walk`, `storage_roots`; `open_root_consented` nur für erlaubte GUI-lokale Duplikatsuche | Bereitgestellt und konsumiert. Host erteilt niemals Elevation. |
| V-LOCAL | `quarantine_regular_child(name, &File)`, bestätigte Identität, `file/restore/move_to/retained_location`; private Child-Verzeichnisse/EXCL-Dateien | In `checked_recycle.rs`, `linux_trash.rs`, `apptrash/os/shared/quarantine.rs` konsumiert. .held.se-recycle-Namen werden zentral vom Owner geschützt. |
| H-DISPATCH | `FsAccess::{is_dynamic,policy_key,retained_snapshot,register_cancel,check_read}`, autorisierter physischer `LocalBackend::root_display` | Eigene Verbraucher angebunden. Retention entfernt nur Transport-Cancel und hält aktuelle Relation-/Export-/Lease-Bindung. |
| H-DISPATCH/V-LOCAL | FC1/FA3: `.se-versions`, erzeugte `.held.se-recycle-<16 lowerhex>` und `.se-private-<32 lowerhex>.tmp` | Lokaler Scanner/Finder/Hash-Walk schließen sie mit vorhandenen zentralen Engine-/Stage-Prüfern an jeder Tiefe vor Öffnen aus. Keine neue Linux-Literalnamenregel; direkte Windows-Aliasanfragen schützt H-DISPATCH. |
| H-DISPATCH | `analysis_tasks::cancel_principal`; registrierte Weak-Abbruchmarker für Task/Hash/List/Legacy/Watch | Eigene Marker und frische Frame-Prüfungen vorhanden. Restriktionen müssen nur betroffene Principals treffen; Disconnect allein ist kein Rechteverlust. |
| H-DISPATCH | `storage_snapshot::serve(send, root, access, principal)` (vier Argumente); alle neuen FsRequest-Dispatchzweige mit demselben autorisierten FsAccess/Principal | Exakte Signatur an Owner übergeben. Dispatcher bewahrt ReadRegular-, Stage-Ownership-, may_write- und Lease-Prüfungen. |
| H-DISPATCH | `server_capabilities.rs`: echte TargetLimits und beim Lease-Ziel `Some(target.access)` | Eigene Vorarbeit übergeben; Owner schließt relation-/leaseabhängige Rechte ab. OS-Recycle-Wahrheit bleibt zentral in wire_capabilities. |
| A-CLIENT | `Progress::node_budget/set_node_budget`, `AnalysisReceiver::with_node_budget`, `AnalyticsBudget::for_progress` | Bereitgestellt; neuer und Listing-/SSH-/WireNode-Rückfall benutzen dasselbe Budget einschließlich Aggregate/Container. |
| A-CLIENT | `PlatformFigures::estimated_heap_bytes`, `Approximations::estimated_heap_bytes`; privates `storage_retention.rs` additiv in storage_view | Bereitgestellt und dem Owner gemeldet. Kapazitäten saturierend, keine Konstruktor-Budgetlücke offen. |
| A-CLIENT | Walking→Scanning und `Progress::set_phase(Scanning)` löscht directories_unreported | Setter geschlossen; tatsächlicher Scan verliert die alte Legacy-Warnung. Alte build_from_listings/ChildMeta/parallel_tree_assembly-Pfade sind im eigenen Scanner-/Testbestand entfernt. |
| A-CLIENT/T-JOBS | Additive `FsDuplicateGroup.more` und `FsWatchEvent::Ready.complete` (serde false); `ChangeNotice::ReadyPartial` | Draht-/Peer-/Hostseiten konsumieren den Vertrag. Owner berücksichtigen zusätzliche Felder in eigenen Konstruktoren/Suite-Fixtures. |

Alle oben referenzierten eigenen neuen Featuremodule sind im Abnahmemanifest mit ihren genauen
Pfaden aufgeführt. Es werden keine zusätzlichen Module in fremden Dateien durch diesen Block angelegt.
Die einzige Remote-Suite und der Root-Graph-Refresh bleiben beim Hauptagenten.

Der Windows-Namensvertrag wurde von V-LOCAL bestätigt: lebende Quarantäne-/private Dateihandles mit
FILE_SHARE_READ verhindern externen Delete-/Rename-/Case-Rename; Enumeration liefert gespeicherte
OS-Namen. Dieser Vertrag gilt nicht nach Schließen der Handles und ersetzt keine H-DISPATCH-Prüfung
von DOS-/Hardlink-/Lookup-Alias-Anfragen. Der FC1/FA3-Anschluss benötigt keinen weiteren Helper.
