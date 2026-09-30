# Mount Recovery und Cache Reparatur

Stand der Untersuchung: 2026-09-30. Ausgangspunkt ist der gemeldete wiederkehrende
Startfehler in 0.5.167 (`Laufwerk-Recovery lokal pruefen`, Windows-Fehler 2) und
ein etwa 450 GB großer Mount-Cache. Der konkrete Cache und das betroffene Backend
sind hier nicht zugänglich; eine Zuordnung einzelner Dateien zu diesen 450 GB
ist deshalb noch kein gesicherter Befund. Dieses Dokument hält den Reparaturumfang
fest; offene Arbeit wird ausschließlich in `docs/TODO.md` geführt.

## Befunde und erster Plan

Die Fehlermeldung kommt aus `daemon/os/shared/mount_manager_start_cache.rs` vor
der Remote-Auflösung. `mount/core/spool.rs` bricht beim ersten fehlenden, vom
Journal referenzierten Spool ab. Der Dateiname fehlt im Fehler, der Status bleibt
`Unknown`, und Retry stößt erneut auf dieselbe Datei. Der gültige Rest des
Journals kann damit nicht wiederhergestellt werden. Die GUI meldet einen bereits
bekannten Daemon-Fehler bei ihrer ersten Statusabfrage erneut als App-Fehler.

`materialization.rs` lädt beim ersten Datenzugriff die gesamte Datei. Das
500-MiB-Limit in `cache_policy.rs` begrenzt nur geschlossene saubere Dateien.
Offene Dateien und ungesicherte Änderungen können es überschreiten; die
Arbeitsplatzreserve schützt lediglich die letzten 512 MiB freien Speicher.
Zusätzlich beendet `clean_cache.rs` die gesamte Bereinigung, sobald die älteste
Datei nicht gelöscht werden kann. Diese Mechanismen erklären unbeschränktes
Arbeitsdatenwachstum; welcher davon den gemeldeten Bestand erzeugte, ist ohne
den Cache nicht feststellbar.

Erster Plan: lokale Recovery von einzelnen fehlenden Nutzdaten entkoppeln,
große reine Lesezugriffe ohne vollständigen Plattencache bedienen und unabhängige
saubere Dateien auch nach einem einzelnen Bereinigungsfehler freigeben.

## Recherche und Entscheidungen

Primärquellen geprüft am 2026-09-30:

- [rclone VFS Cache](https://rclone.org/commands/rclone_mount/#vfs-file-caching):
  offene und ungesicherte Daten benötigen andere Regeln als entbehrliche Kopien;
  bedarfsgesteuerte Lesezugriffe vermeiden vollständige Vorabkopien.
- [Dokany Operationen](https://dokan-dev.github.io/dokany-doc/html/struct_d_o_k_a_n___o_p_e_r_a_t_i_o_n_s.html):
  Paging-I/O kann nach Cleanup stattfinden. Die bestehende Pin-Lebensdauer bis
  Close und das Verhalten offener Handles bei atomarem Ersetzen bleiben erhalten.
- [Rust Read](https://doc.rust-lang.org/std/io/trait.Read.html): kurze Reads,
  `Interrupted` und vorzeitiges EOF müssen korrekt behandelt werden.
- [Rust File](https://doc.rust-lang.org/std/fs/struct.File.html): Drop bestätigt
  keine erfolgreiche Persistenz. Journal-Synchronisation bleibt Voraussetzung
  für das Freigeben ungesicherter Daten.

Die zweite Recherche prüfte die konkreten Integrationsgrenzen: `Backend::open_read_at`
existiert bereits, Agent-Frames besitzen einen Offset und das Fallenlassen eines
Agent-Lesers sendet Cancel. MountProxy und RootedBackend reichen die Fähigkeit
bislang nicht durch. Der neue Pfad muss deren Request-Gate, Fehlerübersetzung,
Root-Prüfung und die Nichtweitergabe globaler Provider-IDs beibehalten. Backends
ohne Offset-Unterstützung behalten ihren bisherigen vollständigen Lesepfad.

Fehlende ungesicherte Bytes dürfen weder durch Remote-Inhalt ersetzt noch als
sauber verbucht werden. Ein vollständig gelesenes gültiges Journal bleibt die
Autorität: fehlende Nutzdaten werden als Konflikt im Arbeitsspeicher sichtbar,
der originale Eintrag bleibt unverändert auf Platte. Ein später wiederhergestellter
Spool kann dadurch beim nächsten Öffnen normal wiederaufgenommen werden. Ein
fehlendes Journal bei vorhandenen Nutzdaten ist dagegen keine Löschberechtigung.

## Endgültiger Meilensteinplan

| Meilenstein | Betroffene Grenze | Erwartetes Ergebnis der gemeinsamen Remote-Prüfung |
|---|---|---|
| Recovery nach Teilverlust | `mount/core/spool.rs`, neues Recovery-Hilfsmodul, Engine-Recovery | Eine fehlende Nutzdatei blockiert keine intakte Geschwisterdatei; Originaljournal und übrige ungesicherte Bytes bleiben erhalten; Konflikt nennt Remote-Pfad und lokalen Spool; wiederhergestellte Bytes sind erneut retrybar. Fehlendes/defektes Journal, Links und ungültige Namen erlauben keine Bereinigung. |
| Begrenzte reine Lesezugriffe | neuer Mount-Lesepfad, `file_io.rs`, RootedBackend und MountProxy | Kleine Reads aus einer synthetischen 450-GB-Datei auf RO-Mounts erzeugen keinen ganzen Spool; Offset, EOF, kurze Antworten, Identitätsänderungen und Abbruch werden korrekt behandelt; RW-/Konflikt-/atomare Speicherpfade behalten ihre Semantik. Backends ohne Offset-Unterstützung bleiben nutzbar. |
| Unabhängige Cache-Bereinigung | `clean_cache.rs`, `spool.rs` | Eine gesperrte oder verschwundene saubere Datei verhindert nicht die Freigabe anderer; fehlgeschlagene Löschung bleibt verbucht und retrybar; offene/dirty/konfliktbehaftete Dateien werden nicht verdrängt. |
| Verständlicher Sitzungsstart | `app/core/mount_ui.rs`, Hilfsmodul | Ein bei erster Abfrage bereits bestehender Fehler öffnet die Recovery-Details, statt bei jedem GUI-Start erneut als frischer App-Fehler aufzutauchen; neue Fehler und explizite Retry-Fehler bleiben sichtbar. |
| Integration und Lieferung | eine neue Remote-Task-Suite, bestehender Release-Workflow, Dokumentation, Root-Graph | Ein Windows-Library-Task prüft alle Erwartungen einschließlich bestehender Dirty-Retry-, Delete-, Replace-, Pin-, Root- und Rechte-Verträge. Kandidat committen/pushen, genau diese Suite auswerten, danach genau eine vollständige Remote-Release-Transaktion. |

Abhängigkeiten: Recovery vor ihrer UI-Darstellung; Offset-Weitergabe vor dem neuen
Lesepfad; sämtliche Implementierung vor dem gemeinsamen Suite-Aufruf. Keine lokalen
Builds oder Tests. Die vorhandenen fremden Dokumentänderungen bleiben unberührt.
Backend-/Verbindungsidentitäten und persistierte Locator bleiben unverändert;
Sync-, Transfer-, Backup- und Provider-Implementierungen werden nicht umgebaut.

## Abnahme

Die Implementierung ist im Kandidaten enthalten. Die gemeinsame Remote-Abnahme
läuft über `mount-recovery-cache-task.yml` und ausschließlich
`native/test-mount-recovery-cache-task.ps1`. Sie verwendet einen passenden
vorhandenen Windows-Library-Testbinärstand oder baut inkrementell nur dieses
Target. Die Auswahl umfasst die neuen Fälle und die unmittelbar betroffenen
bestehenden Cache-, Dirty-Retry-, Delete-, Replace-, Pin- und Reserve-Verträge.
Es gibt keinen lokalen Build/Test und keinen Dokany-/Installer-Build in der Suite.
Remote-Ergebnis und Veröffentlichung sind noch nicht bestätigt.
Der erste [Remote-Lauf](https://github.com/b1ue-man/smart-explorer/actions/runs/36699705886)
auf `7a906b0` bestätigte Teilverlust/Retry, begrenzte große Reads, Windows-Dateisperren
und die ausgewählten bestehenden Schreibverträge. Der neue Root-Grenzfall erwartete
eine Ablehnung erst beim Lesen; der Agent lehnte bereits im Open-Handshake korrekt
mit `InvalidInput` ab. Der Prüffall akzeptiert nun beide Fehlerzeitpunkte, verlangt
weiterhin die korrekte Ablehnung und prüft die anschließende Nutzbarkeit der Verbindung.
Die gleiche Suite wird mit diesem korrigierten Prüffall und der präzisierten
Recovery-Meldung wiederholt; daraus folgt noch keine Release-Freigabe.
Ein synthetischer Größenfall ersetzt keine Messung des ursprünglichen
450-GB-Verzeichnisses und keine Zertifizierung sämtlicher Remote-Anbieter.
