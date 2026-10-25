# B4 — statischer Code- und Primärquellenreview

Stand: 2026-10-25. Gegenstand ist der integrierte, nicht veröffentlichte Arbeitsstand für P15, P16/P17, P18a–d, P19a/b, P20 und P29. Ausgangs-HEAD der Untersuchung: `7442afc0`; die geprüften Änderungen lagen zusätzlich im Worktree. Änderungen der parallel bearbeiteten App-/Androidhost- und RDP-Adapter sind nicht als abgeschlossene B4-Funktion bewertet.

Auftrag des Nutzers: „aufgrund des umfangs der aufgaben, ohne dass das weiteres vorgehen blockt, will ich dass du danach n astra xhigh die sachen kritisch reviewn lässt. per Batch“; Präzisierung: „also rein codeseitig und gegen quellen reviewen lässt“. Belegte Startparameter des Reviewers: `gpt-6-astra`, `xhigh`. Antwortmodell-Metadaten stellt das verwendete Delegationswerkzeug nicht bereit.

Dieser Bericht bewertet Quellcode und externe Primärquellen. Es wurden keine Builds, Compiler, Tests, Formatter, Server, Installationen, Commits, Pushes oder Releases ausgeführt. Testquelltext ist ausschließlich als ergänzende Beschreibung beabsichtigter Fälle gelesen; daraus folgt kein bestandener Test. Laufzeitdarstellung, Bildschirmleserausgabe und tatsächlicher Kanalabbau bleiben ohne Ausführung ungeprüft.

**Abschlussergebnis:** Fünf belegte Defektgruppen wurden gemeldet: ein P1-Befund und vier P2-Befunde. Main hat die Korrekturen einschließlich zweier im Nachabgleich erkannter Restfälle vorgenommen. Die gemeldeten Fehlerabläufe sind im abschließend gelesenen Quelltext statisch geschlossen. B4 ist damit innerhalb der festgelegten Lesefläche kritisch gegen Code und Primärquellen bewertet; es liegt kein Build-, Test- oder Laufzeit-Pass vor. Die konkreten ungeprüften Integrationsgrenzen stehen unten.

## Prüfrahmen und Entscheidungen

Der Erstplan aus dem Auftrag war: die genaue Lesefläche und die B4-Verträge erfassen; anschließend Identität, Persistenz, Zulassung und Entzug, die CLI-Kopplung sowie die beiden Oberflächen kritisch vergleichen; schließlich ausschließlich diesen Bericht schreiben. Der aus dem Bestand abgeleitete Detailplan verfolgt diese Grenzen:

1. P18d: Version 1/2 und Legacyfelder, unbekannte Bedingungen, konkrete Direct-/Room-Principals, einmalige Zustimmung, Einschränkung und Ablauf laufender Rechte.
2. P18a–c/P19/P20: dieselbe Policyquelle in Hostansicht, Desktop und Android; Navigation aus bestehenden Einstiegen; neue Anzeige und Mutation bestehender Rechte; sicherheitsrelevante Beschriftungen.
3. P29: Besitz vor und nach Startbestätigung, Client-EOF/Cancel, parallele GUI-Beobachtung, PIN-Eingabe und Text-/JSON-Ausgabe.
4. P15–17: unveränderter Ordnerort, getrennte Geräte-/Ordnerpräferenz, Dialog-/Listeninnenfläche, Geometriespeicherung und plattformeigene Grenzen.

Die zweite Quellenprüfung nutzt die im Lockfile gebundene egui-Version 0.29.1 und aktuelle offizielle Rust-, Serde- und Android-Dokumentation. Ein Graphquery mit den im freigegebenen Graphbestand belegten Begriffen `access approval discovery exchange grant settings` diente zur Navigation. Der Graph lieferte keine ausreichenden Verhaltensbelege; die nachfolgenden Befunde beruhen auf den konkreten Quellen. Der Graph wurde nicht neu gebaut oder verändert. Ein angekündigter Paket-/Skillversionsunterschied wurde nicht durch eine im Review untersagte Installation beseitigt.

## Priorisierte Befunde

### B4-R01 — P1, während des Reviews statisch korrigiert: falsche Elternmodulpfade verhindern Rust-Namensauflösung

**Fundstellen:** `native/src/app/core/share_access_dialog_ui.rs:358`, `native/src/app/core/share_access_room_ui.rs:111`; Modulregistrierungen `native/src/app/core/share_exec_ui.rs:13` und `native/src/app/mod.rs:80`.

Beide Dialoge riefen `super::dialog_geometry::window` auf. Sie werden als Untermodule von `app::share_exec_ui` eingebunden; `dialog_geometry` liegt dagegen unter `app`. `super` erreicht deshalb `share_exec_ui`, das keinen solchen Namen bereitstellt. Das ist ein statisch belegter Namensauflösungsfehler, kein aus einer Compilerdiagnose abgeleiteter Befund. Die [Rust Reference zu `super`](https://doc.rust-lang.org/reference/paths.html#super) definiert den unmittelbaren Elternmodulpfad.

**Korrekturergebnis:** Main hat nach der Meldung beide Aufrufe auf `crate::app::dialog_geometry::window` geändert. Diese beiden aktuellen Zeilen wurden nachgelesen. Der konkrete falsche Pfad ist damit statisch korrigiert; Kompilierbarkeit des Gesamtstands ist weiterhin ungeprüft.

**Erweiterter Befund aus den anschließend freigegebenen Einbindungen:** Derselbe Fehler stand in `share_exec_log_ui.rs:115` (Elternmodul `share_exec_ui`), in `vault_actions_ui.rs:156`, `vault_add_ui.rs:102`, `vault_check_ui.rs:183`, `vault_compare_ui.rs:337` und `:365`, `vault_compare_detail_ui.rs:108`, `:149` und `:196`, `vault_create_ui.rs:231`, `vault_sync_editor_ui.rs:127`, `vault_unlock_ui.rs:163` und `:261`, `vault_settings_ui.rs:145` (Elternmodul `app::vault_modules`) sowie in `share_screen_codec_ui.rs:209`, `:215`, `:243` und `:251` (Elternmodul `app::screen_modules`). Alle Basenamen dieses Absatzes liegen unter `native/src/app/core/`. Die vollständig gelesenen `native/src/app/vault_modules.rs` und `screen_modules.rs` importieren `dialog_geometry` nicht. Die Re-Exporte ihrer Kinder nach `app` ändern deren lexikalisches Elternmodul nicht.

**Abschlussabgleich:** Sämtliche 20 gemeldeten Aufrufe verwenden jetzt `crate::app::dialog_geometry`. Die erste Fundstelle des Gerätedialogs liegt nach Folgeänderungen bei Zeile 360. Die Vermutung desselben Fehlers in `mount_ui.rs:222` und `mount_ui_draft.rs:139` wurde verworfen: Beide Module werden direkt unter `app` eingebunden; dort ist `super::dialog_geometry` korrekt. Dieser Abgleich belegt die Korrektur der gemeldeten Pfade, keinen erfolgreich ausgeführten Rust-Build.

### B4-R02 — P2, während des Reviews statisch korrigiert: Android behauptet wirksame Rechte trotz abgelaufener Zielbedingungen und hält alte Snapshots fest

**Fundstellen:** `android/app/src/main/java/app/smartexplorer/android/ui/share/DeviceSettingsPage.kt:54`, `:72`, `:195`, `:210`; `native/src/share/core/access_relation_view.rs:136`; `native/src/share/core/access_constraints.rs:178`.

`sourceRows` bezeichnet ein eingeschaltetes Recht als wirksam, solange nur der gesamte Grant nicht abgelaufen und die Beziehung aktiv ist. Die einzelnen `valid_until`-Werte für `app_launch`, `login_access` und `tcp_forward` werden dabei nicht ausgewertet. Ein aktiver, global unbegrenzter Grant mit `app_launch=allow` und abgelaufener App-Bedingung erscheint daher auch in einem frisch geladenen Snapshot als „Erlaubt · wirksam: Erlaubt“. Der Host entfernt dieselbe Fähigkeit in `append` mittels `constraints.available_at(capability, now)` bereits aus `reachable`. Bei TCP gilt der Widerspruch, sobald sämtliche Zielregeln abgelaufen sind. Der Desktop berücksichtigt diese Verfügbarkeit in `device_rights_ui.rs`.

Zusätzlich lädt der Android-Effekt nur für `selector` und den manuellen Zähler `reload`. Änderungen von `vm.status`, Policyrevision oder Uhrzeit starten keinen neuen Abruf. Ein auf der geöffneten Seite erreichter Ablauf oder ein über CLI vorgenommenes Entziehen kann daher in `relations` und `reachable` unbefristet als alter Zustand stehen bleiben. Die [offizielle Compose-Dokumentation zu `LaunchedEffect`](https://developer.android.com/develop/ui/compose/side-effects#launchedeffect) bestätigt den Neustart bei geänderten Effektschlüsseln; sie verspricht keine Aktualisierung durch nicht verwendete Zustandswerte.

**Auswirkung:** Eine sicherheitsrelevante Anzeige widerspricht dem autoritativen Hostzustand. Ein tatsächlicher Autorisierungsbypass ist daraus nicht belegt.

**Notwendiges Korrekturergebnis:** Der angezeigte wirksame Zustand muss die konkreten Bedingungen und Fristen auswerten oder direkt eine autoritative Projektion verwenden. Eine geöffnete Geräteseite muss Policy-/Beziehungsänderungen und Ablauf mit definiertem Aktualisierungsverhalten übernehmen; bei fehlgeschlagenem Abruf darf sie einen alten Stand nicht als aktuellen wirksamen Zugang ausgeben.

**Erster Korrekturabgleich:** `DeviceSettingsPage.kt:206` verwendet jetzt ausschließlich `reachable` des Workers für den wirksamen Zustand; `:221` bindet den Eintrag an das exakte `source.target` und die Fähigkeit. Die Effektschlüssel bei `:56` umfassen `execTargets`, `rooms`, `writeGrants` und `exports`; der Fehlerpfad bei `:66` löscht den alten Snapshot. Der Revisionswechsel ist bis zum serialisierten `policyRevision` in `native/src/mobile/os/shared/domains/share_exec.rs:85` und zum Kotlin-Datenklassenfeld in `api/ShareExecApi.kt:39` nachvollzogen. `sourceDeadlines` bei `:239` erfasst Grant-, App-, Login- und TCP-Fristen.

**Zusätzlicher Ablaufrest dieser ersten Korrektur:** Der einmalige Timer bei `DeviceSettingsPage.kt:71` war nur an `snapshot` gebunden. Bei zwei TCP-Regeln mit Fristen `t1 < t2` bleibt die Ansicht nach `t1` strukturell unverändert: `access_relation_view.rs:142` behält TCP wegen Regel 2 in `reachable` und serialisiert bei `:145` weiterhin die vollständigen Bedingungen. Ein identischer Snapshot startet den Effekt nicht neu. `t2` wurde damit ohne weiteren Statuswechsel nicht eingeplant. Notwendiges Ergebnis ist eine erneute Fristplanung nach dem ausgelösten Abruf auch bei strukturell gleichem Ergebnis, beispielsweise mit einer erfolgreichen Abrufgeneration. Dieser konkrete Fall wurde Main zusätzlich gemeldet.

**Abschlussabgleich:** `DeviceSettingsPage.kt:53` hält nun zusätzlich `fetched`; `:65` erhöht diesen Wert nach jedem erfolgreichen Abruf. Der Timer bei `:73` verwendet `LaunchedEffect(snapshot, fetched)`. Dadurch wird nach dem Abruf bei `t1` auch bei identischem JSON die nächste noch zukünftige Frist `t2` eingeplant. Die Snapshotaktualisierung liegt jetzt bei `:57`, der Fehlerpfad bei `:68`, `sourceRows` bei `:208` und `sourceDeadlines` bei `:241`. Die beiden ursprünglichen Fehler und der beschriebene Mehrfristfall sind statisch geschlossen; eine Ausführung auf Android liegt nicht vor.

### B4-R03 — P2, während des Reviews statisch korrigiert: der Desktopeditor verändert kontextuelle Rechte, die er nicht zeigt

**Fundstellen:** `native/src/app/core/share_exec_programs_ui.rs:244`, `:315`, `:358`; `native/src/app/core/share_access_dialog_ui.rs:86`, `:151`, `:191`; `native/src/app/core/share_access_room_ui.rs:56`, `:89`; `native/src/share/core/access_catalog.rs:64`.

Der Desktopeditor verwendet den Basiskatalog für Befehle, Terminal, Bildschirm ansehen und Bildschirm steuern. `AccessDraft::of` übernimmt jedoch alle sieben Entscheidungen einschließlich Appstart, Anmeldezugang und TCP-Weiterleitung. Das gemeinsame Kontrollkästchen „Mit Rückfrage“ iteriert über sämtliche Entscheidungen. Wird es abgeschaltet, setzt es auch eine nicht dargestellte kontextuelle Entscheidung von `Ask` auf `Allow`. `change` sendet anschließend die vollständige Map und bewahrt die alten Zielbedingungen. Der Nutzer kann daher eine Rückfrage für einen vorhandenen App-/Anmelde-/TCP-Grant entfernen, ohne dessen Zeile oder Bedingungen im bearbeiteten Dialog zu sehen.

Der Raum-Sammeldialog hat den umgekehrten Effekt: Er erzeugt einen Entwurf aus `AccessGrant::default()`, übernimmt damit die kontextuellen Entscheidungen als `Off` und kopiert die vollständige Änderung auf jedes ausgewählte Mitglied. Das Bearbeiten der sichtbaren Basisrechte entzieht bestehende kontextuelle Rechte, die der Dialog nicht anzeigt. Dies ist aus `planned` und `AccessDraft::change` ableitbar; es setzt keinen erfolgreich laufenden neuen RDP-Adapter voraus.

**Auswirkung:** Versteckte Erweiterung von `Ask` auf `Allow` beziehungsweise versteckter Entzug bei einer Sammeländerung. Der Vertrag „Bearbeiten ändert genau die angezeigte Policyquelle“ reicht als Quellenidentität allein nicht aus: auch der tatsächliche Mutationsumfang muss erkennbar sein. Primärbeleg für diesen projektspezifischen Vertrag sind die genannten produktiven Rust-Funktionen und die dort definierte vollständige Fähigkeitsmenge; keine externe UI-Dokumentation kann die beabsichtigte Projektpolicy ersetzen.

**Notwendiges Korrekturergebnis:** Entweder zeigt der Editor alle von seiner Aktion betroffenen Fähigkeiten samt Bedingungen, oder er begrenzt seine Änderungen auf tatsächlich dargestellte Fähigkeiten. Die Raum-Sammelaktion muss nicht angezeigte Fähigkeiten pro Mitglied erhalten oder den Entzug dieser Fähigkeiten ausdrücklich darstellen. Eine Basisrechteänderung darf vorhandene kontextuelle Rückfragen nicht still abschalten.

**Erster Korrekturabgleich:** `AccessDraft::select` bei `:146`, `set_ask` bei `:154` und `change` bei `:194` filtern `requires_context()`. Die Änderungsmap enthält nur Befehle, Terminal, Bildschirm ansehen und Bildschirm steuern; `constraints: None` bleibt erhalten. `AccessGrant::apply` in `native/src/share/core/access_grant.rs:361` ändert nur aufgeführte Fähigkeiten. Dadurch bleiben die kontextuellen Entscheidungen sowohl beim Einzelgerät als auch über `RoomDialog::planned` erhalten. Die beiden ursprünglichen Mutationsfehler sind statisch geschlossen.

**Zusätzlicher Beschriftungsrest dieser Korrektur:** `confirm_label` bei `share_access_dialog_ui.rs:211` zeigte bei vier ausgeschalteten Basisfähigkeiten weiterhin „Zugriff entziehen“; `share_access_room_ui.rs:142` entsprechend „Für n Mitglieder entziehen“. Appstart, Anmeldezugang und TCP-Weiterleitung bleiben nach der korrigierten Mutation ausdrücklich erhalten. Notwendiges Ergebnis ist ein sichtbarer Scopehinweis und eine Bestätigung, die den tatsächlichen Umfang der Basisrechteänderung benennt. Ein vollständiger Entzug darf nicht versprochen werden. Dieser konkrete Rest wurde Main zusätzlich gemeldet.

**Abschlussabgleich:** `share_access_dialog_ui.rs:214` benennt den Entzug jetzt ausdrücklich für Befehle, Terminal und Bildschirmrechte; `share_access_room_ui.rs:146` tut dies für die ausgewählten Mitglieder. Die gemeinsame Steuerungsansicht erläutert bei `share_exec_programs_ui.rs:358`, dass die Entscheidungen für Appstart, Anmeldezugang und TCP-Weiterleitung erhalten bleiben. Damit sind der Mutationsumfang und der gemeldete falsche Entzugstext statisch korrigiert. Die sichtbare globale Grantgültigkeit bleibt ein gemeinsamer Parameter; die Korrektur ist kein neuer separater Gültigkeitszeitraum je Fähigkeit.

### B4-R04 — P2, während des Reviews statisch korrigiert: der schmale Desktop-Rechteeditor verliert die zugängliche Fähigkeitsbeschriftung

**Fundstellen:** `native/src/app/core/share_exec_programs_ui.rs:359`, `:365`, `:395`, `:411`.

Im breiten Layout erhält jedes Radioelement sowohl die Fähigkeitszeile als auch die Entscheidungsspalte über `labelled_by`. Im schmalen Zweig steht die Fähigkeitsbezeichnung separat; die Radioelemente tragen nur „Aus“, „Fragen“ beziehungsweise „Erlauben“. `capability_row` verknüpft die ID der Fähigkeitsbezeichnung nicht mit ihnen. Der Hovertext ist keine explizite AccessKit-Beschriftungsverknüpfung.

Der [egui-0.29.1-Quelltext von `Response::labelled_by`](https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/egui/src/response.rs) verwendet `push_labelled_by`. Zwei Aufrufe im breiten Zweig sind daher korrekt und überschreiben sich nicht. Der schmale Zweig führt diese API nicht aus.

**Auswirkung:** Der neue responsive Layoutwechsel entfernt genau die Zuordnung, die bei Tastatur-/Bildschirmleserbedienung für die Auswahl des richtigen Rechts benötigt wird. Eine tatsächlich gesprochene Ausgabe wurde nicht ausgeführt.

**Notwendiges Korrekturergebnis:** Auch im schmalen Layout muss jedes Radioelement seine konkrete Fähigkeit und Entscheidung als zugängliche Beschriftung besitzen. Die reine räumliche Nachbarschaft eines Textlabels reicht für diese Quellcodeanforderung nicht.

**Abschlussabgleich:** `share_exec_programs_ui.rs:399` hält jetzt die Antwort des Fähigkeitslabels fest; `:401` verknüpft jedes Radioelement über `labelled_by(capability_label.id)`. Der statisch belegte Unterschied zum breiten Layout ist behoben. Tatsächliche Fokusreihenfolge und Bildschirmleserausgabe bleiben ungeprüft.

### B4-R05 — P2, während des Reviews statisch korrigiert: mehrere GUI-Prozesse überschreiben gespeicherte Größen fremder Dialogtypen

**Fundstellen:** `native/src/app/os/shared/dialog_geometry_store.rs:23`, `:44`, `:54`, `:63`; `native/src/desktop/os/shared/open_target.rs:191` und `:220` als vorhandener Startpfad weiterer GUI-Prozesse.

`store()` lädt `dialog_geometry.json` genau einmal in eine pro Prozess gehaltene `OnceLock<Mutex<Store>>`. `save()` ersetzt anschließend die komplette Datei durch `state.file` dieses Prozesses. Ein erneutes Lesen oder eine prozessübergreifende Zusammenführung findet nicht statt. Beispiel: Zwei GUI-Prozesse laden denselben Anfangsstand; Prozess A speichert eine vergrößerte Rechteansicht; Prozess B speichert danach die Größe des Einstellungsdialogs. B serialisiert seinen älteren Snapshot samt eigener Änderung und entfernt dadurch As neue Rechteansichtsgröße. Gleichzeitige Schreibzugriffe sind für diesen Ablauf nicht erforderlich.

Der atomare Schreibhelfer kann einen vollständigen Dateiersatz liefern, aber keine bereits vor dessen Aufruf aus dem veralteten Snapshot verlorenen Einträge rekonstruieren. Die [Rust-Dokumentation von `OnceLock::get_or_init`](https://doc.rust-lang.org/std/sync/struct.OnceLock.html#method.get_or_init) beschreibt die einmalige Initialisierung; der weitere Kontrollfluss ist unmittelbar im Store belegt. Abgerufener Rust-Dokumentationsstand: 1.99.1, keine Behauptung über die installierte lokale Toolchain.

**Auswirkung:** Die zugesagte Größe pro Dialogtyp bleibt bei mehreren regulären Smart-Explorer-Fenstern nicht zuverlässig über Neustarts erhalten.

**Notwendiges Korrekturergebnis:** Ein Schreiben darf ausschließlich den betroffenen stabilen Dialogtyp ändern und muss die zuletzt gespeicherten Größen sämtlicher übriger Typen erhalten. Dafür ist ein nachweislich serialisierter Lese-/Zusammenführungs-/Schreibvorgang oder eine voneinander unabhängige Speicherung je Typ erforderlich; ein weiterer rein prozesslokaler Mutex schließt den Auslöser nicht.

**Abschlussabgleich:** `save` ruft jetzt `merge_save` auf. `dialog_geometry_store.rs:70` öffnet eine separate `dialog_geometry.lock`; nach `lock.lock()` liest es die aktuelle JSON-Datei, prüft deren Version, ersetzt nur den betroffenen Schlüssel und ruft den atomaren Schreibhelfer noch unter dem gehaltenen Lock auf. Der Cache wird erst nach erfolgreichem Schreiben übernommen. `native/src/support_dirs.rs:22` liefert ein `std::fs::File` und öffnet dieses bei `:26` beziehungsweise `:29` schreibbar. Die OS-Auswahl in `native/src/creds/mod.rs:23` und beide `private_storage`-Adapter wurden nachgelesen.

Die [Rust-Dokumentation von `File::lock`](https://doc.rust-lang.org/std/fs/struct.File.html#method.lock) beschreibt die exklusive Sperre und ihre Freigabe beim Schließen des Handles; diese API ist seit Rust 1.89 stabil. Die lokale Variable hält das Handle bis nach Lesen, Zusammenführen und Promotion. Der separate Lockpfad vermeidet den Austausch des gesperrten JSON-Dateiobjekts während der atomaren Promotion. Damit ist der gemeldete Verlust durch zwei veraltete Prozesssnapshots im Kontrollfluss geschlossen. Prozessübergreifendes Verhalten und Wiederanlauf nach tatsächlichem I/O-Fehler wurden nicht ausgeführt.

## Weitere Codebefunde und widerlegte Verdachte

### P18d — Persistenz, konkrete Identität, Zustimmung und Entzug

`access_grant_wire.rs` und `exec_policy_wire.rs` behandeln unbekannte Access-Versionen beziehungsweise nicht verstandene Bedingungen als unbekannte Records. Der Originalwert bleibt erhalten; die ausführbare Sicht ist gesperrt. V2 wird beim Schreiben nicht unbemerkt auf V1 zurückgestuft; die Legacyprojektion deaktiviert den vollständigen Grant für einen alten Leser. Bei V1 bleiben die bestehenden Basisfähigkeiten erhalten, während neue kontextuelle Fähigkeiten standardmäßig ausgeschaltet sind. Ein Verdacht auf stilles Ignorieren unbekannter Bedingungsfelder wurde anhand der `deny_unknown_fields`-Typen in `access_constraints.rs` und des äußeren Fehlerpfads verworfen. Das entspricht der [Serde-Dokumentation zu Containerattributen](https://serde.rs/container-attrs.html); die gebundene Serde-Version ist 1.0.228. Die eigene Recordlogik, nicht Serde allein, liefert das Verhalten bei unbekannter Version.

`access_authorize.rs` löst den Grant aus dem vollständigen `ProvenPrincipal` der konkreten Beziehung auf. Die Gerätedarstellung in `access_relation_view.rs:88` und `:110` sammelt passende Direct- und Room-Beziehungen getrennt nach Geräteschlüssel; `ReachableAccess` behält die jeweilige Quelle und ihre Bedingungen. Es entsteht keine kombinierte Maximalberechtigung aus mehreren Räumen. `AccessGrant::decision_at` erlaubt kontextuelle Fähigkeiten nicht ohne Anfrage; die dafür bestimmten Anfragepfade prüfen App-/Benutzer-/Sitzungsbedingungen beziehungsweise TCP-Richtung, Initiator, Ziel, Adresse und Port. `LoginAccess` ist auf die dafür zugelassene direkte Beziehung begrenzt. Die tatsächlichen neuen Apps-/RDP-/Tunneladapter sind Gegenstand B7/B8; ihre Aufruferabdeckung wird hier nicht aus der Existenz dieser Kernfunktionen abgeleitet.

Die einmalige Zustimmung in `access_revocation.rs:18` ist an Principal, konkrete Anfrage und Policyrevision gebunden. Der Zustimmungswert ist kein frei klonbarer globaler Freischalter. `access_approval_view.rs` transportiert den vollständigen Kontext zu Desktop, Android und CLI. Die geprüften Android-Benachrichtigungsaktionen erteilen kontextuelle Anfragen nicht allein anhand der verkürzten Benachrichtigung; der vollständige Dialogpfad ist erforderlich. Die Einmaligkeit und die konkrete Bindung wurden im Produktionscode verfolgt; die bloße Existenz von Testfällen wurde nicht als Ausführungsbeleg gewertet.

Die anfangs fehlende Verbindung vom persistierten Rechteentzug zum aktiven Registryzustand ist im nachträglich freigegebenen Produktionspfad belegt:

1. `native/src/daemon/os/shared/exec_grant_journal.rs:320` wendet die exakte gespeicherte Änderung an und prüft bei `:348` die Bestätigung; `drive_steps` bei `:426` unterscheidet Persistieren/Anwenden und den vorherigen Sperrschritt einer Einschränkung.
2. `native/src/share/core/service.rs:101` sendet `ApplyExecGrant`; `signal_commands_local.rs:33` reicht das aktive `access_registry` an `apply_exact_with_access` weiter.
3. `exec_grant_runtime.rs:94` verifiziert Ziel und Revision unter dem Auth-Mutex, erstellt den Kandidaten und prüft die registrierten Vorgänge vor dessen Veröffentlichung neu. `configuration_runtime.rs:55` behandelt Profilwechsel entsprechend; `signal_commands_local.rs:60` bezieht den Direct-Online-Zustand ein.
4. `node_access.rs:33` hält beim Zulassen die Lebenszyklus- und Auth-Grenze bis zur Registrierung und Ressourcenannahme. Die Ablaufüberwachung startet in `node.rs` unabhängig von einem geöffneten Bildschirmfenster. `node_access.rs:118` abonniert Änderungen vor dem Lesen der nächsten Frist. `access_revocation.rs:156` sendet den Entzugsgrund an den registrierten Vorgang; der Eintrag bleibt bis zum Drop seines Handles erhalten.

Der Verdacht, ein vor dem Warten gesendetes Watchsignal müsse verloren gehen, wurde verworfen. Der abonnierte Receiver und die verwendete Aktualisierungsart passen zur [Tokio-1.52.3-Implementierung von `watch`](https://raw.githubusercontent.com/tokio-rs/tokio/tokio-1.52.3/tokio/src/sync/watch.rs). Das ist ein Kontrollflussbeleg für die Benachrichtigung. Tatsächlich geschlossene Sockets, beendete Prozesse, freigegebene Eingabetasten und abgeschlossene Betriebssystemsitzungen sind damit noch nicht nachgewiesen. Der untersuchte Legacyweg `mutate_exec_grant` ändert nur die Execfähigkeiten; daraus wurde kein unbelegter fehlender kontextueller Entzug konstruiert.

### P18a–c, P19a/b und P20 — Einstellungswege und lesbare Rechtequellen

Desktop und Android benutzen stabile Geräte-/Schlüsselidentitäten für die neue Geräteseite. Anzeigenamen dienen als Überschrift, nicht als Policyselektor. Der Desktop hält die getrennten Quellen mit ihrem eigenen Bearbeitungsziel; Android gleicht nach R02 dieselben exakten Targets mit dem Workergebnis ab. Bestehende direkte Zugänge, Raumzugänge und Schreibrechte bleiben über die geprüften Einstiege erreichbar. Die numerischen Altansichten werden ausdrücklich auf Geräte, Räume, dieses Gerät, Verbindungen beziehungsweise Diagnose abgebildet; der Storage-Diagnoseweg bleibt benannt erhalten.

Die geprüften Einstellungsseiten trennen die üblichen Aufgaben von technischen Detailfeldern. Gerätefingerabdruck, Direktcode, Programmliste, Prozessende und Serverkonfiguration sind weiterhin auffindbar. Eine fehlende tatsächliche Routenmessung wird auf Android nicht als Relay-Verbindung ausgegeben. Die übergreifende Direct-/Relay-Messung gehört laut Vertrag zu B5 und wurde hier nicht als geliefert gewertet. Die Codecansicht fasst Standardentscheidungen zusammen und hält Lizenz-/Statistikdetails gesondert erreichbar.

Die explizite Beschriftung des Rechteeditors wurde einschließlich des engen Layoutzweigs geprüft; ihr Defekt ist R04. Allgemeine Layouthelfer oder das Vorhandensein einer `LazyColumn` beweisen keine vollständige Tastatur-, TalkBack- oder Bildschirmleserbedienung aller Seiten. Die [Compose-Dokumentation zur standardmäßigen Zugänglichkeit](https://developer.android.com/develop/ui/compose/accessibility/api-defaults) wurde zur Einordnung von Semantik und Berührungsflächen herangezogen; tatsächliche Trefferflächen, Fokuspfade und gesprochene Namen wurden nicht gemessen.

### P15 — Ordneröffnung und erhaltene Orte

`native/src/desktop/core/folder_open.rs:6` definiert die von der Gerätepräferenz getrennte Ordnerpräferenz mit `App` als Default; `config.rs`, Desktop-Einstellungen und CLI transportieren diese Einstellung. `desktop_surface_actions.rs` und `desktop_surface_menu_ui.rs` führen Ordneraktivierung über denselben Helfer. `desktop/os/shared/folder_open.rs:17` übernimmt den Locator unverändert in `StartRequest::OpenLocation`. Die Systemdateimanagerwahl akzeptiert über `local_endpoint_path` nur einen lokalen Ort. `desktop_entry` belässt Dateien und den ausdrücklichen Auswahldialog im bisherigen Pfad; ein nicht verlustfrei darstellbarer Ordnerpfad ergibt einen Fehler statt eines beschädigten Locators. Die Auswahl eines Links wird nicht vorab zum Ziel kanonisiert.

Damit ist am geänderten Übergang weder das Entfernen einer Remoteidentität noch ein Ersatz des gewählten Unterordners durch den Root belegt. Die tatsächliche Plattformklassifikation von Verzeichnislinks, ein Netzlaufwerk ohne Verbindung und das Öffnen eines bereits verschwundenen Ordners wurden nicht ausgeführt. Der Bericht behauptet keinen Laufzeitnachweis für sämtliche lokalen, UNC-, SFTP-, FTP-, WebDAV-, Drive- und Share-Backends.

### P16/P17 — Innenfläche, Persistenz und messbare Grenze

`picker_impl.rs:151` ermittelt die verfügbare Höhe vor dem Wechsel in die horizontale Aufteilung und reserviert die Aktionsfläche. Die Listenhöhe wird aus dem tatsächlichen Scrollbereich zurückgemeldet. `dialog_geometry.rs:155` begrenzt die Resthöhe anhand des Cliprechtecks; `:75` korrigiert die erste Pickerhöhe anhand des ermittelten Inhaltsanteils. Der Wrapper ist ein egui-Fenster innerhalb des Viewports, kein zusätzliches natives Betriebssystemfenster. Die gebundene [egui-0.29.1-Window-Implementierung](https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/egui/src/containers/window.rs) und der zugehörige [Resize-Container](https://raw.githubusercontent.com/emilk/egui/0.29.1/crates/egui/src/containers/resize.rs) wurden gegen die verwendeten Größen- und Scroll-APIs geprüft.

Die initiale Soll-Listenhöhe ist im Code `160.0` Punkte. Das verdoppelt die frühere Untergrenze `80.0`, nicht ein im Review gemessenes `H_alt`. `umsetzung.md` bezeichnet diese Basis ausdrücklich als Annahme für den bisherigen Fehlerfall. Der umfassendere V7-Vertrag `min(2 × H_alt, H_arbeitsfläche − H_dialogchrome)` ist deshalb durch den Codewert allein nicht bewiesen. Auch eine passende äußere Fensterhöhe wäre kein solcher Beleg.

Gespeichert werden ausschließlich stabile Dialogtyp-Schlüssel und begrenzte numerische Größen. `dialog_geometry_store.rs` liest höchstens 64 KiB, verwirft unbekannte Schlüssel/ungültige Größen aus der benutzten Ansicht und überschreibt unbekannte Versionen oder beschädigte Dateien nicht. Titel, Pfade und vollständige egui-Widgetzustände werden nicht in diesem Store persistiert. R05 schließt den gefundenen Verlust zwischen mehreren GUI-Prozessen. Der Wrapper macht den Gesamtinhalt vertikal scrollbar; daraus folgt nicht, dass jede Aktionszeile bei jeder Fensterhöhe permanent angeheftet sichtbar ist. Erreichbarkeit bei sehr kleinen Viewports, großer Schrift, Monitorwechsel, hoher Skalierung und Android-Rotation bleibt ungeprüft.

### P29 — tatsächlicher Besitzer und CLI-/MCP-Abbruch

Der Austauschbesitz ist im Produktionscode vorhanden und wird nicht aus einer IPC-Antwort abgeleitet. `discovery_exchange_owner.rs:26` erzeugt Owner und Lease gemeinsam; der Owner kann den Zustand beenden, während Leases den optionalen Operationsretainer halten. `discovery_pairing_ipc.rs:222` erzeugt den Besitzer vor dem Startauftrag. `ConnectionWatch` bei `:150` beobachtet eine geklonte Verbindung unabhängig vom Warten auf die Startbestätigung. EOF, Abbruchnachricht oder Lesefehler setzen die Cancellation. `OwnedExchange::drop` bei `:129` beendet den Austausch des ursprünglichen Services. Der Dispatch- und Wartungspfad entfernt besitzerlose ausstehende beziehungsweise aktive Austausche; ein fehlgeschlagener Netzwerksendeversuch verhindert das lokale Aufräumen nicht.

Die IPC-Beobachter werden vor dem jeweiligen Auftrag registriert und halten Ergebnisse je Austausch-/Discovery-ID. Der GUI-Drain konsumiert deren Ergebnis nicht. Die Beobachtung hat einen begrenzten Ergebnispuffer und einen ausdrücklichen Fehlerpfad bei Überlauf. Die tatsächliche Haltedauer entspricht den Besitzverhältnissen des [Rust-`Arc`](https://doc.rust-lang.org/std/sync/struct.Arc.html); die Verbindungsbeobachtung verwendet den dokumentierten gemeinsamen Socket eines [`TcpStream::try_clone`](https://doc.rust-lang.org/std/net/struct.TcpStream.html#method.try_clone). Die Abfolge wurde gelesen, nicht mit einem abstürzenden CLI-Prozess ausgeführt.

`discover_input.rs:22` liest den PIN bis EOF, begrenzt die Eingabe und erhält Leerzeichen sowie Zeilenenden. Eingabefehler werden nicht als nutzbarer Teil-PIN übernommen. CLI-Argumente bieten keinen PINwert; vorgesehen sind Standardeingabe oder verdeckte interaktive Eingabe. Ausgaben und strukturierte Fehler enthalten keinen PIN. `cli/mcp/cli_tool.rs:29` verlangt für den erkannten Kopplungsaufruf `--pin-stdin` und einen separaten stdin-Wert; der Jobtitel enthält nur die Argumente. Die Pipe schreibt die gelieferten UTF-8-Bytes und schließt anschließend die Eingabe. Die Abbruchschleife bei `:104` fordert zunächst Stop an und wartet auf das Kind; der Deadlinepfad kann es beenden. `jobs.rs:95` übernimmt einen bereits angeforderten Abbruch auch bei später installierter Steuerung; `finish` gibt die Operation erst am Jobende frei.

Der Retainerbezug zur allgemeinen Prozessgeneration ist ergänzend in `review-b3.md` dokumentiert. B3 ersetzt den hier gelesenen Discovery-/EOF-Pfad nicht. Die vollständige MCP-Sitzungsbeendigung oberhalb von `cli_tool` und `jobs` sowie die OS-spezifischen Kindprozessmethoden sind kein erneut geprüfter Teil dieses B4-Berichts; für deren Integrationsnachweis gilt die B3-Abgrenzung.

## Quellenstand und nicht ausgeführte Nachweise

Sämtliche externen Abrufe erfolgten am 2026-10-25. Versionsgebundene Primärquellen: egui/eframe 0.29.1 und Tokio 1.52.3 entsprechend `native/Cargo.lock`. Serde-Attribute wurden gegen die offizielle Dokumentation bei gebundener Version 1.0.228 geprüft. Die abgerufenen aktuellen Rust-Standardbibliotheksseiten zeigen 1.99.1; dies bezeichnet die Dokumentation und keinen festgestellten lokalen oder CI-Compiler. Zusätzlich gelesen wurden die offiziellen [Serde-Feldattribute](https://serde.rs/field-attrs.html), [Serde-Variantenattribute](https://serde.rs/variant-attrs.html) und die [Kotlin-Dokumentation zum Erzeugen von Collections](https://kotlinlang.org/docs/constructing-collections.html). Nicht erfolgreich geladene docs.rs-Seiten wurden nicht als Beleg verwendet.

Die folgenden Nachweise fehlen ausdrücklich, weil der Auftrag ausschließlich statisches Review erlaubt:

- Rust-/Kotlin-Kompilierbarkeit des integrierten Stands einschließlich der parallel bearbeiteten B7-Adapter.
- Physischer Abbau laufender Datei-, Exec-, Terminal-, Screen-, App-, Login- und TCP-Vorgänge nach Entzug, Ablauf, Sessionende oder Cancel; der Bericht belegt nur die gelesenen Policy-/Registrierungs-/Benachrichtigungswege.
- Darstellung und Bedienung der Dialoge bei schmalen/kurzen Viewports, DPI-/Schriftänderung, Monitorentfernung und Android-Rotation; insbesondere V7-Messung der alten und neuen tatsächlichen Listeninnenhöhe sowie erreichbare Bestätigungs-/Abbruchaktionen.
- Android-Aktualisierung auf einem laufenden Gerät einschließlich mehrfacher Bedingungsfristen und Statuswechsel während offener Dialoge; Desktop-Bildschirmleserausgabe und Android-TalkBack-Ausgabe.
- Gleichzeitige GUI-Prozesse mit realem Dateilock und I/O-Fehlern; tatsächliche Ordneraktivierung über lokale, UNC-, SFTP-, FTP-, WebDAV-, Drive- und Share-Orte.
- Kopplungsabbruch vor dem Start-ACK, während parallelem GUI-Drain und bei Prozess-/MCP-Sitzungsende mit realen Sockets und Kindprozessen.

Diese fehlenden Ausführungsbelege sind weder bestandene Tests noch ein zusätzlicher vom Reviewer eingeführter Releasevorbehalt. Suite und Veröffentlichung gehören dem Main-Auftrag.

## Vollständiges Lese- und Schreibinventar

Die folgende Liste enthält alle 169 im Review benutzten Repositorydateien der freigegebenen Lesefläche. „Gelesen“ umfasst vollständige Module sowie gezielte Definitionen, Registrierungen, Diffs und Suchtreffer; die Liste behauptet keine vollständige Zeilenprüfung jeder Datei. Graphlektüre war auf B4-Navigation begrenzt. In den geteilten UI-Dateien wurden die laufenden B7-RDP-/App-Adapteränderungen nicht bewertet.

```text
AGENTS.md
android/app/src/main/java/app/smartexplorer/android/api/ShareAccessApi.kt
android/app/src/main/java/app/smartexplorer/android/api/ShareApi.kt
android/app/src/main/java/app/smartexplorer/android/api/ShareExecApi.kt
android/app/src/main/java/app/smartexplorer/android/service/AccessApprovalNotifier.kt
android/app/src/main/java/app/smartexplorer/android/service/AccessApprovalReceiver.kt
android/app/src/main/java/app/smartexplorer/android/ui/AppRoot.kt
android/app/src/main/java/app/smartexplorer/android/ui/picker/LocationPickerDialog.kt
android/app/src/main/java/app/smartexplorer/android/ui/settings/DocumentsProviderSection.kt
android/app/src/main/java/app/smartexplorer/android/ui/settings/SettingsForm.kt
android/app/src/main/java/app/smartexplorer/android/ui/settings/SettingsNavigation.kt
android/app/src/main/java/app/smartexplorer/android/ui/settings/SettingsScreen.kt
android/app/src/main/java/app/smartexplorer/android/ui/settings/ShareServerSettings.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/AccessDialog.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/DeviceSettingsPage.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/ExecAuditDialog.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/ExecHostSection.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/ShareActionDialog.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/ShareConnectDialog.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/ShareConnectionDialog.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/ShareDeviceCard.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/ShareDialogHost.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/ShareDialogs.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/SharePeers.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/ShareRightsDialogs.kt
android/app/src/main/java/app/smartexplorer/android/ui/share/ShareScreen.kt
docs/ARCHITEKTUR.md
docs/oberflaechen/desktop-launcher.md
docs/oberflaechen/einstellungen.md
docs/oberflaechen/share-exec.md
docs/plaene/2026-10-24-batches/batches.md
docs/plaene/2026-10-24-batches/recherche.md
docs/plaene/2026-10-24-batches/review-b3.md
docs/plaene/2026-10-24-batches/umsetzung.md
docs/plaene/2026-10-24-batches/vertraege.md
graphify-out/.graphify_python
graphify-out/graph.json
native/Cargo.lock
native/Cargo.toml
native/src/app/access_modules.rs
native/src/app/core/access_consent_ui.rs
native/src/app/core/analytics_access.rs
native/src/app/core/analytics_accessibility.rs
native/src/app/core/analytics_ui.rs
native/src/app/core/bisync_ui.rs
native/src/app/core/connect_dialog_ui.rs
native/src/app/core/copy_dialog.rs
native/src/app/core/desktop_surface_actions.rs
native/src/app/core/desktop_surface_menu_ui.rs
native/src/app/core/desktop_surface_settings_ui.rs
native/src/app/core/device_rights_model.rs
native/src/app/core/device_rights_ui.rs
native/src/app/core/dialog_geometry.rs
native/src/app/core/dialogs.rs
native/src/app/core/host_key_ui.rs
native/src/app/core/merge_ui.rs
native/src/app/core/mount_ui.rs
native/src/app/core/mount_ui_draft.rs
native/src/app/core/picker_async.rs
native/src/app/core/picker_impl.rs
native/src/app/core/picker_types.rs
native/src/app/core/settings_ai_plugin.rs
native/src/app/core/settings_background.rs
native/src/app/core/settings_cloud.rs
native/src/app/core/settings_devices_ui.rs
native/src/app/core/settings_form.rs
native/src/app/core/settings_navigation.rs
native/src/app/core/settings_ui.rs
native/src/app/core/settings_updates.rs
native/src/app/core/share_access_accounts_ui.rs
native/src/app/core/share_access_approvals_ui.rs
native/src/app/core/share_access_dialog_ui.rs
native/src/app/core/share_access_lists_tests.rs
native/src/app/core/share_access_room_ui.rs
native/src/app/core/share_connections_ui.rs
native/src/app/core/share_device_page_ui.rs
native/src/app/core/share_diagnostics_ui.rs
native/src/app/core/share_drain.rs
native/src/app/core/share_exec_jobs_ui.rs
native/src/app/core/share_exec_log_ui.rs
native/src/app/core/share_exec_programs_ui.rs
native/src/app/core/share_exec_ui.rs
native/src/app/core/share_exports_ui.rs
native/src/app/core/share_lan_ui.rs
native/src/app/core/share_pages_ui.rs
native/src/app/core/share_policy_ui.rs
native/src/app/core/share_room_page_ui.rs
native/src/app/core/share_screen_codec_ui.rs
native/src/app/core/share_screen_tab_ui.rs
native/src/app/core/share_this_device_ui.rs
native/src/app/core/share_window_ui.rs
native/src/app/core/shell_commands.rs
native/src/app/core/sync_versions_ui.rs
native/src/app/core/theme.rs
native/src/app/core/transfer_window.rs
native/src/app/core/vault_actions_ui.rs
native/src/app/core/vault_add_ui.rs
native/src/app/core/vault_check_ui.rs
native/src/app/core/vault_compare_detail_ui.rs
native/src/app/core/vault_compare_ui.rs
native/src/app/core/vault_create_ui.rs
native/src/app/core/vault_settings_ui.rs
native/src/app/core/vault_sync_editor_ui.rs
native/src/app/core/vault_unlock_ui.rs
native/src/app/desktop_modules.rs
native/src/app/drive_modules.rs
native/src/app/mod.rs
native/src/app/os/shared/dialog_geometry_store.rs
native/src/app/os/shared/sync_exit_gate.rs
native/src/app/screen_modules.rs
native/src/app/vault_modules.rs
native/src/cli/desktop.rs
native/src/cli/mcp/catalog.rs
native/src/cli/mcp/cli_tool.rs
native/src/cli/mcp/jobs.rs
native/src/cli/share/discover.rs
native/src/cli/share/discover_input.rs
native/src/cli/share/discover_output.rs
native/src/cli/share/exec_approvals.rs
native/src/cli/share/grants_device_view.rs
native/src/creds/mod.rs
native/src/creds/os/private_storage_unix.rs
native/src/creds/os/private_storage_windows.rs
native/src/daemon/os/shared/discovery_pairing_client.rs
native/src/daemon/os/shared/discovery_pairing_ipc.rs
native/src/daemon/os/shared/discovery_pairing_protocol.rs
native/src/daemon/os/shared/exec_grant_journal.rs
native/src/desktop/core/config.rs
native/src/desktop/core/folder_open.rs
native/src/desktop/mod.rs
native/src/desktop/os/shared/folder_open.rs
native/src/desktop/os/shared/open_target.rs
native/src/mobile/os/shared/domains/mod.rs
native/src/mobile/os/shared/domains/share_access.rs
native/src/mobile/os/shared/domains/share_exec.rs
native/src/mobile/os/shared/domains/share_status.rs
native/src/share/access_modules.rs
native/src/share/core/access_approval.rs
native/src/share/core/access_approval_view.rs
native/src/share/core/access_authorize.rs
native/src/share/core/access_catalog.rs
native/src/share/core/access_constraints.rs
native/src/share/core/access_grant.rs
native/src/share/core/access_grant_runtime.rs
native/src/share/core/access_grant_wire.rs
native/src/share/core/access_relation_view.rs
native/src/share/core/access_revocation.rs
native/src/share/core/access_runtime_tests.rs
native/src/share/core/configuration_runtime.rs
native/src/share/core/discovery_exchange_owner.rs
native/src/share/core/discovery_signal_cancellation.rs
native/src/share/core/discovery_signal_commands.rs
native/src/share/core/discovery_signal_dispatch.rs
native/src/share/core/discovery_signal_maintenance.rs
native/src/share/core/discovery_signal_state.rs
native/src/share/core/discovery_signal_types.rs
native/src/share/core/exec_grant_runtime.rs
native/src/share/core/exec_policy.rs
native/src/share/core/exec_policy_wire.rs
native/src/share/core/node.rs
native/src/share/core/node_access.rs
native/src/share/core/node_policy.rs
native/src/share/core/node_sessions.rs
native/src/share/core/relation_rights.rs
native/src/share/core/service.rs
native/src/share/core/signal_commands_local.rs
native/src/share/mod.rs
native/src/share/os/shared/consent_launch.rs
native/src/support_dirs.rs
```

Zusätzlich gelesen: `/tmp/smartexplorer-review-b4-scope.json`, `/root/.agents/skills/arbeitsweise/SKILL.md`, `/root/.codex/skills/arbeitsweise/SKILL.md`, `/root/.codex/skills/graphify/SKILL.md` und `/root/.codex/skills/graphify/references/query.md`. Die über das Arbeitsweise-MCP gelieferten Anweisungen wurden geladen. Gitstatus, letzter Commit und die auf freigegebene Pfade begrenzten Diffs dienten als Metadaten; es erfolgte keine repositoryweite Ordnererkundung.

Die angefragten Namen `native/src/share/core/access_admission.rs`, `native/src/share/core/relation_runtime.rs` und `native/src/share/core/relation_transition.rs` bezeichneten keine vorhandenen Quelldateien. Daraus wurde kein Produktdefekt abgeleitet. Die tatsächlichen Definitionen wurden über `mod.rs`/`access_modules.rs` und die anschließend freigegebenen `node_access.rs`, `configuration_runtime.rs`, `node_policy.rs` und `exec_grant_runtime.rs` verfolgt. Der tatsächliche Consentadapter liegt in `native/src/share/os/shared/consent_launch.rs`.

**Schreibinventar dieses Reviewers:** ausschließlich `docs/plaene/2026-10-24-batches/review-b4.md`, neu erstellt und während des Reviews fortgeschrieben. Die in R01–R05 genannten Produktkorrekturen stammen von Main. Dieser Reviewer hat keine Produktdatei, Testdatei, Graphdatei oder Konfiguration geändert.

**Entscheidungen:** Prioritäten richten sich nach reproduzierbarem Kontrollfluss: R01 verhindert Namensauflösung; R02/R03 betreffen die wahrheitsgemäße Anzeige beziehungsweise Änderung von Rechten; R04 entfernt die zugängliche Fähigkeitszuordnung; R05 verliert persistierte Benutzergrößen. Widerlegte Verdachte sind gesondert dokumentiert. Aus Graphkanten, Plantext, Testquelltext oder einem syntaktischen Parserergebnis wurde kein Funktionsnachweis abgeleitet. Die Nachprüfung der Main-Korrekturen ist Bestandteil desselben Reviews.

**Gezielt verbleibende Abgrenzungen:** Für die gemeldeten fünf Defektgruppen fehlt nach dem Lockhelfer-Abgleich keine angefragte Typ- oder Moduldefinition mehr. Nicht erneut verfolgt wurden `crate::local_access::DirectoryHandle::open_private_child` und `crate::vfs::replace_local_file` hinter den bestehenden privaten Windows-Datei-/Promotionshelfern; die B4-Korrektur nutzt deren vorhandene API. Ebenso liegen `cli/mcp/session.rs` für den übergeordneten Sitzungsabbau und die OS-Kindprozessadapter außerhalb dieser Lesefreigabe. Die neuen B7/B8-Ressourcenadapter bleiben ausdrücklich ausgeschlossen. Das begrenzt die Aussagen zu tatsächlichem Plattformschutz und Ressourcenabbau, nicht die belegten B4-Aufrufer- und Besitzketten.
