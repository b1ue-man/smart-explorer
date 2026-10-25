# B1 – statischer Nachreview der Reparaturen R01–R06

Stand: 2026-10-25. Erste Nachprüfung: **20:14:30 UTC**; zusätzliche Nachprüfung der integrierten F1-Reparatur: **20:40:03 UTC**. Die Befundstatus unten ersetzen den ursprünglichen Review vom selben Tag, 19:28 UTC. Frühere Gegenbeispiele und Hashstände bleiben ausdrücklich als historische Belege erhalten.

**Aktuelles Ergebnis: R01–R06 einschließlich B1-R05-F1 sind für die dokumentierten Gegenbeispiele und geprüften Übergänge statisch geschlossen. Kein bestätigter Befund bleibt in diesem Nachreview offen.** Kein Build, Compiler, Test, Server, Livekonto oder Mount wurde ausgeführt. Laufzeitverhalten, Plattformintegration und Crashfestigkeit bleiben unbestätigt.

## F1-Nachprüfung nach Integration – 20:40 UTC

**B1-R05-F1 ist für beide bestätigten Trigger und die geprüften Desktop-/Androidübergänge statisch geschlossen.** Die doppelten versiegelten Upserts können im Offlineaudit keine falschen Map-Einträge mehr bilden; das Crashfenster oberhalb 1 MiB erreicht dort keine Kompaktierung mehr. Es wurde kein verbleibender F1-Defekt bestätigt. Ein während der Nachprüfung gefundener Signaturfehler in zwei Testaufrufern wurde dem Hauptagenten gemeldet, von ihm korrigiert und anschließend anhand des Quelltextes geschlossen.

Diese zweite Nachprüfung umfasst ausschließlich F1 und seine betroffenen Aufrufer. Die präzise begrenzten Schließungsbelege für R01, R02, R03, R04 und R06 aus der ersten Nachprüfung bleiben bestehen; ihre gesamte Fläche wurde nicht erneut gelesen. Die R05-Exportkorrektur bleibt der separate erste Teil der nun geschlossenen Kette. Die alten F1-Gegenbeispiele weiter unten dienen ausschließlich als historischer Befundbeleg.

### Kontrollflussbeleg

| Übergang / Gegenbeispiel | Geprüfter jetziger Kontrollfluss und Ergebnis |
| --- | --- |
| Offlineaufruf → Cachebesitz | Windows und Linux verwenden `CacheLease::acquire_existing`: vorhandene Verzeichnisse und vorhandene Lockdatei öffnen; kein `create`, kein Anlegen von `files`, keine Änderung des Journals. Die Lockdatei wird nur lesend geöffnet, unter Linux exklusiv/nichtblockierend per `flock`, unter Windows mit Sharemode 0. Fehlende oder belegte Lease wird als Fehler weitergereicht, nicht als leerer Cache. Nur ein tatsächlich fehlendes Mountverzeichnis ergibt vorab `Some(0)`. |
| Cachebesitz → Inventur | `spool.rs::audit_pending_state` (292) traversiert über `DirectoryHandle`, öffnet die ausgewählte Journalgeneration nur regulär/lesend und begrenzt den Stream auf 64 MiB + 1. Auswahlreihenfolge `journal.jsonl → compact-old → compact-new` entspricht der Autoritätsreihenfolge des Live-Recoverypfads. Der Audit ruft weder `WholeFileSpool::open[_with]` noch `Journal::open`, Rotation, Truncate oder Cleanup auf; ein Spoolobjekt samt mutierendem Drop wird nicht erzeugt. |
| Versiegelter Bestand → unbekannt | Ein `spool-key.wrapped`-Eintrag führt zu `None`. Auch ohne diesen Marker lässt `spool_audit.rs::pending` nur Aktionen mit erkennbaren absoluten Klartextschlüsseln zu `apply` durch. `JournalPathSealer` speichert Base64url ohne Padding; dessen Alphabet enthält kein `/`. Daher scheitern versiegelte Upsert-/Forget-/Move-/Namespacepfade vor `apply` am `plain_paths`-Gate. `ForgetDelete` verwendet ausschließlich seinen numerischen Token und erzeugt keinen Ciphertextschlüssel. |
| F1-Trigger A: zwei Updates desselben versiegelten Pfads/Spools | Der Audit beendet sich mit `None`, bevor er zwei Ciphertextschlüssel als Einträge rekonstruiert oder eine doppelte Spoolreferenz prüft. Nach autorisiertem Öffnen verwendet `WholeFileSpool::open_with` weiterhin `storage.paths`; `Journal::replay` entsiegelt vor `apply`. Gleichlautende Klartextpfade überschreiben sich wieder in korrekter Reihenfolge. |
| F1-Trigger B: gesynctes Journal ab 1 MiB vor Kompaktierung | Die Offlineprüfung hat keinen schreibenden Journaleinstieg und keine 1-MiB-Wartungsschwelle. Ein großer versiegelter Bestand bleibt unbekannt und byteweise erhalten. Erst der autorisierte Liveopener darf mit dem richtigen Sealer reparieren/kompaktieren. Eine unvollständige Schlusszeile liefert im Audit `None` statt Truncate; Decoder-, Größen- und Lesefehler bleiben Fehler. |
| Klartextcache → exakte Zahl | Vollständige Klartextaktionen werden nur im RAM in Journalreihenfolge angewandt. Eintrags-, Lösch- und Namespacezustände werden gezählt; erkannte doppelte Spoolreferenzen und unzulässige Cleanzustände werden abgewiesen. Fehlendes Journal bei vorhandenen Cachedateien wird nicht als leer ausgegeben. Die Bestandszählung behauptet keine erfolgreiche Payloadentschlüsselung oder Remoteveröffentlichung. |
| Desktopaudit → gesperrter Tresor | `mount_manager_start_cache::prepare` trägt `Option<usize>` weiter. `None` oder ein Auditfehler ergibt `MountRecovery::Unknown`; der Fehler bricht den autorisierten Start nicht vorzeitig ab. `defer_until_unlock` (213) erhält `None` und meldet die unbekannte Zahl ausdrücklich. `waits_for_unlock` erkennt den `vault_locked`-Status auch ohne numerisches `RequiresUnlock`; `restore_vault` kann den Start nach Entsperren erneut betreten. Google Drive behält sein separates historisches Identitätsgate. |
| Androidzählung → lebender/gehaltener widerrufener Slot | `docs_pending::keys` nimmt offene Slots und noch lebende `Weak<Slot>`-Einträge aus `retired` auf. `waiting` verwendet die Öffnungssperre und danach `held`; ein vorhandener Slot wird ausschließlich über seinen privaten rohen `pending_engine.dirty_entries()` gezählt. Root-/Sessionentzug bleibt für Dokumentoperationen wirksam. Ein widerrufener Deskriptor darf den Slot/Lease halten, ohne dass die Pendinganzeige einen zweiten Lease erwirbt oder das Journal öffnet. |
| Androidzählung → ungeöffneter Cache / unvollständige Rootzuordnung | `cache_path` berechnet nur den Pfad. Der Audit bleibt lesend. Lesefehler, unbekannte Zähler, fehlerhafte Marker und Mountverzeichnisse ohne gültige Rootzuordnung erzeugen `unknown`; die Antwort enthält dann `pending: null` sowie den gesonderten bekannten Teil `knownPending`. Ein Fehler wird nicht mit `unwrap_or(0)` versteckt. |
| Android-Retry → autorisiertes Öffnen | Nur ein **ungeöffneter** Slot mit nachgewiesenem `Some(0)` wird als leer übersprungen. `None` oder Auditfehler erlaubt bei freigegebenem Root den normalen `engine`-Opener; dessen Öffnungslease, konkreter Session-/Rootguard und versiegelter Storage bleiben erhalten. Ohne Freigabe bleibt ein nicht nachweislich leerer Root in `failed`. Eine noch gehaltene widerrufene Generation wird nicht umgangen: ihr Lease kann einen neuen Opener weiter ablehnen, während ihr RAM-Pendingzustand erhalten bleibt. |
| Android-Retryantwort → Queueergebnis und Anzeige | `docs.rs::retry_pending` (484) berechnet nach Retry die Pendingantwort erneut und ergänzt `failed`. Deshalb gehen auch Fehler der vollständigen Cacheinventur nicht dadurch verloren, dass `docs_pending::retry_pending` nur bekannte Rootkeys durchläuft. Kotlin `DocsPending.isClean` verlangt `pending == 0 && unknown.isEmpty() && failed.isEmpty()`. `DocsCommitWorker` meldet nur dann Erfolg; unbekannter Zustand bleibt Retry beziehungsweise nach dem bestehenden Versuchslimit Fehler. `SeDocumentsProvider.watch` plant auch bei unbekannter Pendingantwort Arbeit ein; die Einstellungsansicht zeigt Unbekannt, bekannten Teil und fehlgeschlagene Roots getrennt. |

Der Nurlesebeleg wurde bis zu den Plattformopens verfolgt: Linux-`DirectoryHandle` verwendet für diesen Pfad `O_RDONLY`/`openat` ohne `O_CREAT`; Windows-`DirectoryHandle` ruft den direkten Reader mit `ReadKind::{Metadata,Directory,PinChild,File}` auf. `windows/read.rs::open_direct` fordert ausschließlich Lese-/Attribut-/Verzeichnisrechte an und setzt weder Create noch Truncate. `open_root_consented` und ein privilegierter Readbroker werden vom Audit nicht verwendet. **Abgrenzung:** Der umgebende Desktop-Start/Registrycode darf weiterhin den globalen `mount-cache`-Basisordner vorbereiten; der Nachweis betrifft die neue Offlineinventur und den bestehenden Bestand eines Mounts, nicht völlige Mutationsfreiheit jedes Desktop-Startschritts.

### Während dieser Nachprüfung gefundener und geschlossener Integrationsfehler

`mount_termination_tests.rs` enthielt zunächst zwei vierargumentige Aufrufe von `start_cache::insert`, während die aktuelle Funktion fünf Argumente einschließlich `startup_lease` verlangt. Der Hauptagent ergänzte nach Meldung das fünfte Argument `None` an beiden Stellen (142 und 162). Die abschließend gelesenen Aufrufe passen damit zur Signatur. Die ebenfalls sichtbaren `defer_until_unlock(..., Some(0))` und `Some(2)` passen zum neuen Optiontyp. **Dies ist nur ein statischer Aufrufabgleich; die Testdatei wurde weder kompiliert noch ausgeführt.** Der Befund wird nicht als offen geführt.

### Erneut abgeglichene Primärquellen und APIstände

Abrufdatum sämtlicher Quellen in dieser Ergänzung: **2026-10-25**.

| Quelle / Stand | Bestätigte Grenze |
| --- | --- |
| [RFC 4648 §5](https://www.rfc-editor.org/rfc/rfc4648#section-5), Oktober 2006 | Base64url verwendet `-` und `_` statt `+` und `/`; die aktuelle Journalverschlüsselung kann deshalb keinen absoluten `/`-Klartextpfad vortäuschen. Dies belegt Formatunterscheidung, keine Authentifizierung beliebiger Cachebytes. |
| [Cryptomator Vault encryption](https://docs.cryptomator.org/security/vault/), abgerufener Webstand | Randomisierte Header-/Inhaltsverschlüsselung bleibt erhalten. Die Korrektur verzichtet auf schlüssellose Interpretation statt die Verschlüsselung deterministisch zu machen. |
| [flock(2)](https://man7.org/linux/man-pages/man2/flock.2.html), Linux man-pages 6.19, 2026-02-08 | Exklusiver nichtblockierender Lock und Bindung an die offene Dateibeschreibung; auf lokalen Linuxdateisystemen ist ein nur lesend geöffnetes Lockfile zulässig. NFS hat abweichende Voraussetzungen für exklusive Locks; ein solcher Fehler bleibt im geprüften Code unbekannt und wird nicht zu Clean. |
| [CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew), Dokumentstand 2023-02-09, Win32 API | Sharemode 0 schließt konkurrierende Lese-/Schreib-/Deleteopens aus; ein bestehendes Objekt kann ohne Create geöffnet werden. Kein Beleg für eine tatsächlich ausgeführte Windows-Leaseprobe. |
| [AndroidX ListenableWorker.Result](https://developer.android.com/reference/androidx/work/ListenableWorker.Result), `success/retry/failure` seit 1.0.0, Dokumentstand 2026-06-24 | `success` bedeutet erfolgreich erledigt; `retry` fordert Wiederholung mit Backoff, `failure` beendet den Lauf als Fehler. Der neue `isClean`-Guard passt diese Semantik an den expliziten unbekannten Inventurzustand an. Die tatsächlich eingebundene AndroidX-Version wurde in dieser Lesefläche nicht ermittelt. |

Journalformat v1 und die im ersten Nachreview ermittelten Rust-/Lockstände bleiben die Bezugsstände; keine neue Formatmigration wurde aus F1 abgeleitet. Neue Runtime-, Compiler-, ABI-, Kotlinserialisierungs-, Windows-/Linux-/Android- oder WorkManagerausführung ist nicht Teil dieses Nachweises.

### Grenzen und Entscheidungen

- Die beiden ursprünglichen F1-Fälle sind im geprüften Kontrollfluss geschlossen. Ein bereits **vor** dieser Reparatur semantisch falsch kompaktiertes Journal wird dadurch nicht automatisch rekonstruiert.
- Unbekannt kann auch bei einem tatsächlich leeren, aber versiegelten oder nicht lesbaren Cache bestehen bleiben. Ein exakter Count ist erst mit dem berechtigten Liveopener möglich; das ist kein nachgewiesener Verlust von Änderungen.
- Ein noch gehaltener widerrufener Deskriptor darf die alte Generation weiter exklusiv besitzen. Pendingzählung funktioniert über RAM; erneutes Öffnen kann bis zur Freigabe dieses Besitzers scheitern. Kein Umgehen der terminalen Quarantäne wurde hinzugefügt.
- Der regelmäßige manuelle Android-Retry sowie der Desktop-Unlockwiederanlauf sind statisch erreichbar. Eine garantierte automatische Androidveröffentlichung allein durch jedes Unlockereignis wird nicht behauptet; der bestehende Worker hat ein endliches Versuchslimit.
- Die Prüfung bestätigt keine atomare globale Pendingaufnahme bei gleichzeitig neu entstehenden Änderungen, keine Crashsimulation und keine plattformübergreifende Ausführung. Kein bestätigter verbleibender Defekt wurde daraus konstruiert.

### Inhaltsanker und vollständiges Leseinventar der F1-Nachprüfung

Hashzeit `2026-10-25T20:40:03.099979+00:00`, HEAD `3c24152abbc46a65884cd4d8e7b0b7aa62442e72`. Der Worktree enthält ungetrackte Reparaturdateien und parallele Änderungen; die Hashes, nicht HEAD allein, identifizieren die gelesenen Inhalte. Die folgende Tabelle enthält die tatsächlich gelesenen Dateien beziehungsweise gezielten Quelltextausschnitte; sie behauptet keine Volllektüre jeder Datei. Zusätzlich nur gezielt durchsucht wurden `native/src/mount/mod.rs`, `native/src/mount/core/frontend_engine.rs` und `native/src/vault/os/shared/session.rs`. Gelesene Dokumentbasis war dieser bestehende Bericht; Projektanweisungen, Architektur und Verträge bleiben die bereits im ersten Nachreview gelesene Basis.

| Datei / gelesener Ausschnitt | SHA-256 |
| --- | --- |
| `native/src/mount/core/spool.rs` | `949aced304867406b2fbb7b97f377fb943189d0b5fab199b9bcb1efc5e282982` |
| `native/src/mount/core/spool_audit.rs` | `e442c1297c28b39f39caf83c3da2ef358f6ac36c8a4ae3bf5e054aee06fd8c3f` |
| `native/src/mount/core/journal.rs` | `427cd8ed37c45988476d2969e82eee35172d2da36786ab6bc09b99505dbc506d` |
| `native/src/mount/core/journal_record.rs` | `2c242209c5e7a53e75087174fb45b10d905291863f598751d327fd10ee495cdc` |
| `native/src/mount/core/recovery_state.rs` | `2309d6d5742f9cbb1620227ee5422c673449d1d85ff15de3a60b3a1849b58af3` |
| `native/src/mount/core/engine.rs` | `72cf3fad584946f86566a68f25ebd9b687bf62338b712213f9b49f0a18d75286` |
| `native/src/mount/core/recovery.rs` | `b1ad1d758f0d313c975e9c30acf7dbd1d036c89ec74c40823ee62ee4894ff890` |
| `native/src/mount/os/linux_os/cache_lease.rs` | `66dfab6c54dfe0b77e75cc6a60a9df73ecb153cff7ee54d0b40f3396732119ed` |
| `native/src/mount/os/windows/cache_lease.rs` | `7df8187e9e7c7cae0187f0710e8dcb6151692b639f10a77dadfef77713c02f9d` |
| `native/src/mount/os/linux_os/mod.rs` | `be0142bbfad851b43e466dad30b21e813666ba2b3d29df671b4240df3ea8d735` |
| `native/src/mount/os/windows/mod.rs` | `95aec739fdce39a22ead69745ee26b78529ffe0d3b217616b22d4da88e523c23` |
| `native/src/daemon/os/shared/mount_manager_start_cache.rs` | `8a3fe9cf1727930871abab537f7d03cf9d8128e4c5cdd23f25f81f40f67b5f7c` |
| `native/src/daemon/os/shared/mount_manager_start.rs` | `2d2b3f8140035bc97818ad692d089a6b94f4ce484e24a048499b8f151819f5bc` |
| `native/src/daemon/os/shared/mount_termination.rs` | `a3c1751bfe7e5749a1d434d8f73f0aca826a5fa4725ac282cf7392509274cc8d` |
| `native/src/daemon/os/shared/mount_termination_tests.rs` | `17e427108a8b90d781612ff45866a61c4e019b3d91ec8ccac44cb466896255a8` |
| `native/src/mobile/os/shared/domains/docs_pending.rs` | `38d5e75cabc01af65c55ead4c0828dc28453fb2a87048f8e24d75611b3d2250a` |
| `native/src/mobile/os/shared/domains/docs_engines.rs` | `3ec9262548a9aca511ef62b32fa2c6055b15f90e41b1ac6a9d5e27b3dce0a8d2` |
| `native/src/mobile/os/shared/domains/docs.rs` | `5c239b0869f2ad911d935a52d4daa66d4efbc2d97d92ab9f015f5cfe448e3839` |
| `native/src/mobile/os/shared/domains/docs_vault.rs` | `feae9089fea78bf112c31673e4ce1713c5a08f134c7a1b482002aa8ab33e19e5` |
| `native/src/mobile/os/shared/domains/docs_access.rs` | `b4a45925536ca941d811adc59102181872c3a4212fafcfda0d478e16560b00fe` |
| `native/src/mobile/os/shared/domains/docs_io.rs` | `67426f30e9f59c01358c8108d74ab118246aa6471fc05b7fd2fe83ba70576eb0` |
| `native/src/local_access/os/linux/directory_handle.rs` | `820dc0d24c968d509898a714b718f017e0384369d6d6ce4ea3fc548ffeb94730` |
| `native/src/local_access/os/windows/directory_handle.rs` | `19d63db427893048d5ea4d747b085c4ad7ebedfd4dd46e0ec76ef4316dd50417` |
| `native/src/local_access/os/windows/read.rs` | `e465d1da6be2fb8d4944ee2cda1f1c9f4c3115a64797ed1491b2678fb714e3b2` |
| `native/src/local_access/core/protocol.rs` | `49f6a678a1f4ec5d9ff83d3accb6256b7fa5d0216b6fd655ad12ba36025713cf` |
| `native/src/vault/os/shared/journal_sealer.rs` | `0892e476159f67159cbfa531b058ca34d527ed5dff5f0458139e7bddd6097d6b` |
| `native/src/vault/os/shared/session_seal.rs` | `e22a0a7a6f3e677c61581a0da3684366070d7b526e9a85ac6771eba92c04c1a2` |
| `android/app/src/main/java/app/smartexplorer/android/api/DocsApi.kt` | `091bc561ec79aafb3a0b2e5f183c4e4111e2baff658c57c91f07d7b5690dc0a0` |
| `android/app/src/main/java/app/smartexplorer/android/ui/settings/DocumentsProviderSection.kt` | `cd107df44536e87a011389d4a5627aadf7e481cb6ba20687d84a235a9779c965` |
| `android/app/src/main/java/app/smartexplorer/android/system/documents/SeDocumentsProvider.kt` | `21658283be6c1434cdd21d459a1fe7f5f58a36d1ab5dfe4a983d902b5834c57a` |
| `android/app/src/main/java/app/smartexplorer/android/system/documents/DocsCommitWorker.kt` | `3d43871b30ea8c88a1e26f54a9be4b3ca9c8665abca661af89f98cc1b39e870a` |

Der Hauptagent gab `native/src/local_access/os/windows/{directory_handle.rs,read.rs}` und `native/src/local_access/core/protocol.rs` zusätzlich lesend frei. Keine repositoryweite Suche. **Einzige durch den Reviewagenten geänderte Datei bleibt dieser Bericht.** Die beiden Testkorrekturen stammen vom Hauptagenten. Keine Produktionsreparatur, Tests, Builds, Compiler, Formatter, Server, Installationen, Commits, Pushes, Graphneubauten oder Unteragenten durch den Reviewagenten. Die F1-Nachprüfung endet mit dieser Ergänzung.



## Auftrag, Stand und Prüfgrenze

Nutzerauftrag vom 2026-10-25: „aufgrund des umfangs der aufgaben, ohne dass das weiteres vorgehen blockt, will ich dass du danach n astra xhigh die sachen kritisch reviewn lässt. per Batch“ und „also rein codeseitig und gegen quellen reviewen lässt“. Der abgegrenzte Nachauftrag umfasst die sechs B1-Reparaturen, ihre historischen Gegenbeispiele und die verbundenen Identitäts-, Recovery-, Android-, Mountinventar- und Authübergänge. Er autorisiert ausschließlich die Aktualisierung dieses Berichts, keine Reparatur.

Der Erstplan war der Gegenbeispielabgleich R01–R06 gegen die behaupteten Reparaturen. Nach Bestandslektüre konkretisierte sich der Detailplan auf historische → frisch bestätigte Driveidentität, schlüssellose → entschlüsselte Journalverarbeitung, Rootentzug → gecachte Provideroperationen, Mountinkarnation → UI-/Indexinvalidierung sowie strukturierte Providerfehler → persistierten Authzustand. Die zweite Quellenprüfung klärte Drive-403-Gründe, Rootalias/Principal, wiederverwendbare Mount-IDs, Android-Callbacks und DAV-Property-404. Der verbleibende Offlineaudit-Kontrollfluss wurde bis zum verändernden Journalpfad verfolgt; zusätzliche Leseflächen wurden jeweils vom Hauptagenten freigegeben.

Beim Hashabgleich war HEAD `16052ba5314a69474b3cfff88adb0edd4a2e6571`. Der gemeinsame Worktree enthält parallele Implementierungen und teilweise ungetrackte Dateien; **HEAD ist deshalb kein vollständiger Inhaltsbeleg**. Die Hashliste fixiert die entscheidenden tatsächlich gelesenen Inhalte. B2, B3, B4, B5, B6 sowie deren Gesamtintegration sind nicht Gegenstand dieses Nachreviews. Graphdateien waren außerhalb der zugewiesenen Lesefläche; es gab weder Graphabfragen noch Graphneubauten.

`native/Cargo.toml` nennt `smart_explorer 0.5.184`, Rust-Edition 2021, ohne `rust-version`. Die TOML-Dateien wurden statisch gelesen; dies ist keine Release- oder Compilerbestätigung. Relevante Lockstände: `ureq 2.12.1`, `roxmltree 0.20.0`, `serde 1.0.228`, `serde_json 1.0.150`, `libc 0.2.186`, `fuser 0.18.0`; die Lockdatei enthält mehrere `windows-sys`-Versionen. Geprüft wurden das Recoverymanifest v1, die neue Recoverybackendidentität v1 und das Journalformat v1.

## Historischer F1-Befund um 20:14 UTC – durch die Nachprüfung oben geschlossen

### B1-R05-F1 — ursprünglicher P1, jetzt statisch geschlossen: Schlüsselloser Offlineaudit

Die folgenden Trigger und Kontrollflüsse beschreiben ausschließlich den inzwischen reparierten Stand um 20:14 UTC. Sie sind keine aktuellen offenen Befunde.

**Betroffene Stellen:** `native/src/mount/core/spool.rs::audit_pending` (282–284), `WholeFileSpool::open` (84–85), `open_with` (99–125); `spool_storage.rs::MountStorage::plain`; `path_sealer.rs::PlainPathSealer` (36–43); `journal.rs::Journal::open` (34–58), `append` (94–127), `replay` (290–291), `compact` (131–180), `write_snapshot` (196–213). Aufrufer: `daemon/os/shared/mount_manager_start_cache.rs::audit_registered` (24–40), `prepare` (88–112), `mount_manager_start.rs::start_locked` (132 vor 143); `mobile/os/shared/domains/docs_engines.rs::pending` (422–440) und `retry_pending` (446–467). Die Windows-/Linux-`cache_lease.rs::audit_pending` reichen nach Leaseerwerb an denselben Kernpfad weiter.

**Trigger A, ohne großen Bestand oder Beschädigung:** Ein korrekt versiegeltes Tresorjournal unterhalb 1 MiB enthält zwei Upserts desselben Remotewegs mit demselben `spool_name`. Ein regulärer Entstehungsweg ist ein persistierter Dirtyeintrag, dessen Flush später einen Konflikt feststellt und denselben Spool mit Konfliktzustand erneut persistiert (`file_commit.rs::flush_entry`, 30–34). Nach Prozessende ist kein Provider-/Mountslot geöffnet. Die Offlineprüfung öffnet das Journal mit `PlainPathSealer`. Die beiden zufällig versiegelten Pfadstrings unterscheiden sich; `apply` baut zwei Map-Einträge auf. Die anschließende Spoolvalidierung weist den gültigen Bestand als „two recovery entries reference the same mount spool“ ab.

**Auswirkung A:** Desktopstart und Retry führen diesen Audit vor der Backendauflösung aus und brechen bei diesem Fehler ab; ein vorhandener gültiger Tresorschlüssel wird noch nicht benutzt. Der registrierte Bestandsaudit meldet Unknown. Android wandelt beim ungeöffneten Slot denselben Fehler mit `unwrap_or(0)` in null ausstehende Änderungen um. `retry_pending` überspringt danach den Slot, obwohl er veröffentlichbare oder konfliktbehaftete Änderungen besitzt. Der korrigierte private Inmemory-Pendingcount eines **bereits geöffneten** Androidslots betrifft diesen Pfad nicht.

**Trigger B, mit zulässigem Crashfenster:** Ein versiegeltes Journal erreicht mindestens 1 MiB. Ein Prozessabbruch nach dem gesyncten Append und vor der anschließenden Kompaktierung lässt diesen regulären Zwischenstand zurück (`append`, 108 vor 122). Eine erfolglose Wartungskompaktierung kann denselben Zustand erhalten. Danach läuft der schlüssellose Audit. `Journal::open` verdichtet bereits bei 1 MiB, **bevor** `WholeFileSpool::open_with` doppelte Spoolreferenzen oder die Einträge prüft.

**Auswirkung B:** Die Verdichtung verwendet den mit Ciphertextschlüsseln falsch rekonstruierten Zustand. `write_snapshot` schreibt die HashMap-Werte als Upserts, nicht die ursprünglichen Forget-/Moveaktionen; der Plainsealer lässt ihre verschlüsselten Pfadfelder unverändert. Bei `UpsertEntry("/a") → ForgetEntry("/a")` bleibt ein bereits vergessener Eintrag im Snapshot. Bei mehreren Updates desselben Klartextpfads geht die zeitliche Reihenfolge zugunsten der HashMap-Iterationsreihenfolge verloren. Die Rotation ersetzt das ursprüngliche Journal und entfernt dessen alte Generation (178). Ein späterer korrekt entschlüsselter Replay sieht dadurch eine veränderte Historie. Ein anschließender Auditfehler stellt das ursprüngliche Journal nicht wieder her.

Das ist ein statisch belegter Verlust semantischer Recoveryinformation, kein ausgeführter Datenverlustversuch und keine Behauptung eines bereits beobachteten Remoteoverwrites. Voraussetzung ist ein beschreibbarer, korrekt zugänglicher Cache; der Lease verhindert parallele Eigentümer, aber keine falsche Interpretation innerhalb seines Besitzers. Die Korrektur des Exportdecoders verursacht diesen Fehler nicht; sie schließt jedoch diese unmittelbar verbundene schlüssellose Auswertung noch nicht.

**Quelle und Widerspruch:** Die lokale `PathSealer`-Schnittstelle verlangt Entsiegelung vor semantischer Verwendung. `JournalPathSealer::seal_text` benutzt Inhaltsverschlüsselung mit frischem Header; Gleichheit des Klartextpfads ist daraus nicht als Gleichheit der gespeicherten Strings ableitbar. Der Kommentar in `spool.rs` (279–281), wonach versiegelte Pfade beim Replay nur Datensätze benennen, ist deshalb falsch. [Cryptomator: Vault encryption](https://docs.cryptomator.org/security/vault/) beschreibt die zufälligen Header-/Inhaltschlüssel; die konkrete Verwendung für Journalfelder ist durch `vault/os/shared/journal_sealer.rs` und `session_seal.rs` belegt.

**Erforderliche Ergebnisgrenze einer Reparatur:** Ein schlüsselloser Audit muss vorhandene versiegelte Recoverydaten unangetastet lassen und einen unbekannten beziehungsweise entsperrpflichtigen Zustand ausdrücken. Exakte Pendingzahlen und jede semantische Kompaktierung benötigen den richtigen Sealer. Ein Auditfehler darf Android-Retry nicht als bewiesenes „keine Änderungen“ überspringen lassen. Plaincache-Recovery, versiegelte Konflikte, Forget-/Movehistorien und der bereits korrigierte Inmemorycount müssen erhalten bleiben. In diesem Auftrag wurde keine Reparatur vorgenommen.

## Schließungsbelege und geänderte Übergänge

### B1-R01 — ursprünglicher P1 statisch geschlossen; historische Drivebindung fehlt nicht mehr stillschweigend

**Erhaltenes Gegenbeispiel:** Eine neue Dirtydatei mit `Baseline::Missing` wird aus Konto A offline exportiert, auf einer Installation mit Konto B importiert, reaktiviert und erneut gestartet. Der UI-Selektor `cloud:gdrive` genügte zuvor als scheinbare Identität.

**Jetzt belegter Übergang:** Ein neuer Drivecache erhält `RecoveryBackendIdentity` mit Version, vollständiger Source, Provider, bestätigtem Principalhash, Namespace, Backendzustand und tatsächlicher Rootobjekt-ID. `mount/os/shared/recovery_identity.rs::{preflight_mount_recovery_identity,bind_mount_recovery_identity}` erfinden bei bestehender Historie oder Payload keinen Ursprungsbeleg; die Bindung benötigt einen `RecoveryCacheLease`, validiert vorhandene Bindungen und veröffentlicht die neue Metadatendatei über einen synchronisierten Zwischenstand. `daemon/os/shared/mount_recovery_identity.rs::authorize` erwirbt den Lease, wiederholt den Preflight, vergleicht die frischen Backend-/Namespacewerte **vor** der Rootobjektauflösung und danach die vollständige historische Identität.

`mount_manager_start_cache.rs` nimmt Google Drive ausdrücklich vom alten schlüssellosen Audit aus. `mount_manager_start.rs::backend_of` autorisiert die frische Generation vor Rootwrapper/Hoststart. `gdrive/core/state.rs::connect` bestätigt den Principal aus `about.user.permissionId`; `backend.rs::{state_identity,namespace_identity,item_id}`, `identity.rs::actual_parent_id` und `resolution.rs` verbinden diesen mit der konkreten Root-/Ordnerobjektauflösung. Der Alias `root` wird als gespeicherter Rootobjektbeweis abgewiesen.

Der Export liest die historische Bindung aus dem **kopierten** Cache. Fehlende oder fehlerhafte historische Bindung verhindert die Offlinepaketerhaltung nicht, wird aber nicht mit dem heute angemeldeten Konto ergänzt. Importprüfung bindet strukturiertes Manifest und separate Identitätsdatei zusammen; Legacybestände bleiben lesbar, ihre Reaktivierung ohne historischen Beleg wird gesperrt. `mount_recovery_transaction.rs::recovery_reactivate` und `restore_recovery_cache` erhalten den Paketbestand und ersetzen keine abweichenden Cachebytes.

**Statische Schließung:** Konto B erfüllt die historische Gleichheit von A nicht; der ursprüngliche `Baseline::Missing`-Pfad erreicht den schreibenden Replay nicht. Gleichbleibende `cloud:gdrive`-UI-Konfiguration wird weiterhin unterstützt. [Drive v3: User.permissionId](https://developers.google.com/workspace/drive/api/reference/rest/v3/User), [About](https://developers.google.com/workspace/drive/api/reference/rest/v3/about), [Rootalias](https://developers.google.com/workspace/drive/api/guides/folder).

**Grenze:** Die Schließung betrifft Replay-/Schreibautorisierung, nicht die unzutreffende Forderung „vor jedem Netzwerkzugriff“: Zur frischen Identitätsbestätigung sind OAuth-/Providerlesezugriffe notwendig. Altcaches ohne Ursprungsbeleg sind absichtlich nicht automatisch wiederanlaufbar; ein zusätzlicher manueller Zuordnungsdialog wurde nicht nachgewiesen. Crashdurabilität der Metadatenveröffentlichung und echte Konto-/Rootwechsel wurden nicht ausgeführt.

### B1-R02 — ursprünglicher P1 statisch geschlossen; Mountinkarnation von Medienbindung getrennt

**Erhaltenes Gegenbeispiel:** `41 25 0:52 / /mnt/work rw - fuse.example provider rw` wird durch `74 25 0:87 / /mnt/work rw - fuse.example provider rw` ersetzt.

**Jetzt belegter Übergang:** `app/os/linux_os/drive_inventory.rs` bildet eine Mountinkarnation aus Mount-ID, `major:minor`, Dateisystemtyp, Quelle und Subroot. FUSE-/Netzquellen erhalten aus ihrem Quellenlabel **keine** persistente Medienidentität. Bei Blockquellen werden UUID beziehungsweise ein belegter Diskseq-Fallback samt Subroot verwendet; der bloße `/dev/sdX`-Name reicht nicht. `DriveEntry` hält beide Begriffe getrennt. `drive_snapshot.rs::diff` (41) vergleicht Root, Medienidentität und Inkarnation; `apply_drive_snapshot` sowie `drain_drive_index` invalidieren die betroffene Ansicht beziehungsweise verwerfen Resultate der alten Generation. Automatisches Wiederanknüpfen einer getrennten Bindung verlangt einen vorhandenen übereinstimmenden Persistenzbeleg. Der Windowsproduzent füllt das neue Inkarnationsfeld aus seiner vorhandenen Identität.

**Statische Schließung:** Das konkrete FUSE-Gegenbeispiel liefert jetzt Entfernung und Zugang; ein unverändertes Quellenlabel unterdrückt die Invalidierung nicht mehr. Dies stimmt mit der Unterscheidung der Kernelbegriffe überein. [Linux man-pages 6.19: proc_pid_mountinfo(5)](https://man7.org/linux/man-pages/man5/proc_pid_mountinfo.5.html).

**Grenze:** Mount-IDs dürfen nach Unmount wiederverwendet werden. Ein vollständiger Austausch zwischen zwei Beobachtungen mit Wiederverwendung **aller** beobachteten Merkmale ist hiermit nicht ausgeschlossen. Ereignislatenz, verdeckte gestapelte Mounts und reale Windows-/Linuxwechsel wurden nicht zur Laufzeit geprüft. Fehlende persistente Medienidentität bleibt eine echte Unbekannte.

### B1-R03 — ursprünglicher P2 statisch geschlossen; Guard schützt gecachte Provideroperationen

**Erhaltenes Gegenbeispiel:** Nach erstem Öffnen eines freigegebenen lokalen Tresors wird dessen Rootbindung ersetzt oder neu bestätigt, während ein Android-Engineslot beziehungsweise Proxydateideskriptor weiterlebt.

**Jetzt belegter Übergang:** `vault/os/shared/open_policy.rs::authorize_provider` gibt Backend **und** `MountAccess` zurück. `docs_vault.rs::source` hält zusätzlich die konkrete `Arc<VaultSession>`. `docs_access.rs::Access::check` prüft diese Sitzung sowie den Rootguard und setzt bei Entzug eine terminale Quarantäne. Der `MountFilesystem`-Wrapper prüft vor Zugriffen auf Klartextspool, Metadaten, Größen, Schreiben, Flush, Retry und Lockvorbereitung. Die Slotrückgabe in `docs_engines.rs::engine` prüft auch einen vorhandenen Slot. `docs_io.rs::get` prüft jeden bestehenden Proxyzugriff, entzieht bei Fehler den Deskriptor und schließt ohne autorisierten Flush. `finish` prüft vor Flush und schließt anschließend; `close` darf Ressourcen auch nach Entzug freigeben.

Der aktive Slot hält den rohen `MountEngine` ausschließlich privat für `pending_count`; dessen `dirty_entries` zählt bereits gehaltene Änderungen, Löschungen und Namespacekonflikte ohne erneuten Journalzugriff, Backend-I/O oder zweiten Cachelease. `docs_vault.rs::released` bindet auch Dokument-/Verzeichnisabfragen an den Slotguard. Der vorhandene explizite Entzug über Lockparticipant/withdraw bleibt erhalten. Die Lektüre von `entry_lifecycle.rs` und `clean_cache.rs` zeigt beim Close keinen Remotecommit; ausstehende Recoveryreferenzen werden nicht durch das Close des Providers vergessen.

**Statische Schließung:** Ein gecachter Spool oder eine neue Sitzung umgeht den Guard des bereits geöffneten Slots nicht. Die Quarantäne bleibt terminal; eine neu registrierte Sitzung ersetzt die im alten Slot gehaltene Sitzung nicht. [Android ProxyFileDescriptorCallback, API 26](https://developer.android.com/reference/android/os/ProxyFileDescriptorCallback).

**Grenze:** Belegt ist die Eintrittsprüfung je Operation, kein atomarer Abbruch eines bereits zugelassenen I/O bei gleichzeitigem Entzug. Android-/JNI-/Kotlin-Verhalten wurde nicht ausgeführt. Das Wiederfreigeben eines terminal quarantänisierten Slots benötigt dessen Lebenszykluswechsel. Der um 20:14 UTC noch betroffene **ungeöffnete** Androidslot ist durch die gesonderte F1-Nachprüfung oben ebenfalls statisch geschlossen.

### B1-R04 — ursprünglicher P2 statisch geschlossen; temporärer Drive-403 bleibt wiederholbar

**Erhaltenes Gegenbeispiel:** Nach erfolgreichem Tokenrefresh antwortet `about?fields=user(permissionId)` mit HTTP 403 und `rateLimitExceeded` oder `userRateLimitExceeded`.

**Jetzt belegter Übergang:** `cloud/os/oauth_http.rs::AuthError::drive_identity_response` begrenzt den Fehlerbody auf 64 KiB und liest strukturierte `error.errors[].reason`-Werte. Die drei Identitätswege `oauth_http::identify`, `gdrive/core/state.rs::load_drive_account_key` und `gdrive/core/auth.rs::identity_json` verwenden denselben Decoder. Die Allowlist umfasst `rateLimitExceeded`, `userRateLimitExceeded`, `sharingRateLimitExceeded` und `quotaExceeded`. `cloud/core/auth_state.rs::AuthError::provider` klassifiziert die passenden 403 sowie 429/5xx als `NetworkError`; beliebige Providerbeschreibungen werden nicht als Wiederholbarkeitsbeweis benutzt.

`cloud/os/coordinator.rs` lässt Retry und die Prüfung eines noch gültigen Bearers bei `NetworkError` zu; die terminalen Identitäts-/Reauthzustände bleiben gesperrt. `gdrive/core/auth.rs` verwendet für den Ressourcenread den vorhandenen begrenzten Backoff; daraus folgt keine zusätzliche automatische Schleife für jeden OAuth-/Connectweg. Die Persistenz rotierter Refreshtokens bei fehlgeschlagener Identitätsabfrage bleibt über `record_failure`/`token_persistence` erhalten. [Drive v3: Resolve errors](https://developers.google.com/workspace/drive/api/guides/handle-errors), [OAuth 2.0, RFC 6749 §6](https://www.rfc-editor.org/rfc/rfc6749.html#section-6).

**Statische Schließung:** Die beiden ursprünglichen 403-Gründe führen künftig nicht mehr zum persistierten terminalen `ProviderError`. Die sichere Authfehlerklassifikation wird nicht auf sämtliche 403 erweitert.

**Grenze:** Eine bereits früher gespeicherte generische `ProviderError/provider_error`-Sperre enthält ihren damaligen Drivegrund nicht mehr; eine automatische Reparatur solcher Altzustände ist nicht belegt. Retryzeiten, Rate-Limits und Refreshrotation wurden gegen keinen Liveanbieter ausgeführt.

### B1-R05 — ursprünglicher Exportfehler korrigiert; F1-Nachbefund ebenfalls statisch geschlossen

**Erhaltenes Gegenbeispiel:** Ein versiegelter `UpsertEntry("/a") → ForgetEntry("/a")`-Verlauf wurde im Export als zwei unterschiedliche Ciphertextpfade interpretiert; der regulär entfernte Spool erschien als fehlende Nutzlast.

**Jetzt belegter Übergang:** `mount/core/recovery_export.rs::inspect_journal` (35) beendet den versiegelten Zweig vor Decoder oder `apply` mit `JournalInventoryUnknown` und leeren semantischen Inventurlisten. `inspect_stages` behandelt versiegelte Stagepfade ebenso. Rohdateien und ihre Hashes bleiben im Paket; ein großes versiegeltes Journal muss für diese Sicherung nicht durch den begrenzten Klartextdecoder.

`sealed_inventory_view` (10) entfernt die früher aus Ciphertextaktionen erfundenen Einträge und davon abgeleiteten MissingPayload-/RemoteStage-/RemoteQuarantine-Lücken ausschließlich aus der gelesenen Manifestansicht. `mount/os/shared/recovery_import.rs::read_recovery_manifest` berechnet den Manifestdigest zuerst aus den ursprünglichen Bytes und normalisiert danach die Ansicht. Kopie und Paketprüfung erhalten die ursprünglichen Bytes. Inaktivenliste, GUI und CLI führen die unbekannte Inventur getrennt von bewiesenen ungesicherten Datenbezügen; die CLI trennt `inventory-unknown-count` und `not-backed-up-count`.

**Statische Schließung des Exportgegenbeispiels:** Der neue Export spielt ein versiegeltes Journal nicht mehr semantisch ab; historische Paketbytes werden durch die Ansichtsnormalisierung nicht umgeschrieben. Unbekannte Inventur behauptet weder vollständige semantische Sicherung noch eine bewiesene fehlende Datei.

**Nachgeprüfte Integration:** Der um 20:14 UTC noch schreibfähige `spool::audit_pending`-Pfad wurde durch die oben belegte reine Offlineinventur ersetzt. Deshalb ist R05 nun einschließlich B1-R05-F1 statisch geschlossen. Ein verlorenes oder bereits früher falsch kompaktiertes Ursprungsjournal wird durch diese Reparatur oder die Manifestansichtsnormalisierung nicht rekonstruiert.

### B1-R06 — ursprünglicher P2 statisch geschlossen; Property-404 wird quotaspezifisch behandelt

**Erhaltenes Gegenbeispiel:** Ein gültiges DAV-Multistatus meldet `quota-available-bytes` und `quota-used-bytes` jeweils im Property-`propstat` als 404, weil der Server diese Werte nicht anbietet.

**Jetzt belegter Übergang:** `webdav/core/capacity.rs::quota_numbers` (41) wertet die beiden DAV-Properties gesondert aus. Ein Property-404 liefert unbekannte Werte; bei ausschließlich unbekannten Werten müssen die beiden angefragten Properties als gesehen vorliegen. Zahlenfehler, doppelte angefragte Properties, nicht passende Properties und ungültige Status bleiben Fehler. `query_inner` prüft DAV-Namespace, genau eine Antwort und passende Root-href. `[None, None]` ergibt eine unbekannte `CapacitySnapshot` ohne Providerfehler. Ressourcen-404 und 401/403 werden damit nicht in erfolgreich unbekannte Kapazität umgedeutet. Der allgemeine `multistatus::successful_props`-Pfad für Listings bleibt streng.

**Statische Schließung:** Die dokumentierte Nichtverfügbarkeit optionaler Quotaeigenschaften wird nicht mehr als kaputte Providerantwort gemeldet; unbekannt wird nicht mit unbegrenzt gleichgesetzt. [RFC 4331 §3, Februar 2006](https://www.rfc-editor.org/rfc/rfc4331#section-3).

**Grenze:** Dies ist kein Beleg für Kompatibilität mit beliebigem Server-XML; PROPFIND, Authfehler und tatsächliche Kapazitätsanzeigen wurden nicht zur Laufzeit geprüft.

## Quellenstand dieses Nachreviews

Alle folgenden Primärquellen wurden am **2026-10-25** abgerufen. Die Implementierung wurde an diesen Verträgen geprüft; die Abrufe sind keine Beobachtung des Laufzeitverhaltens eines Benutzerkontos.

| Quelle / Version | Konkreter Prüfbezug |
| --- | --- |
| [Drive API v3: Resolve errors](https://developers.google.com/workspace/drive/api/guides/handle-errors), Seitenstand 2026-09-03 | Strukturierte 403-Gründe, `rateLimitExceeded`/`userRateLimitExceeded`, Backoff. |
| [Drive API v3: User](https://developers.google.com/workspace/drive/api/reference/rest/v3/User), Seitenstand 2025-07-23; [About v3](https://developers.google.com/workspace/drive/api/reference/rest/v3/about) | Bestätigter Principal über `about.user.permissionId`. |
| [Drive API v3: Folders](https://developers.google.com/workspace/drive/api/guides/folder) | `root` als Alias, keine historische kontenübergreifende Bindung. |
| [RFC 6749 §6](https://www.rfc-editor.org/rfc/rfc6749.html#section-6), Oktober 2012 | Rotation und Ersatz des bisherigen Refreshtokens. |
| [proc_pid_mountinfo(5)](https://man7.org/linux/man-pages/man5/proc_pid_mountinfo.5.html), Linux man-pages 6.19, Seitendatum 2026-02-08 | Mount-ID wiederverwendbar; `major:minor` und dateisystemspezifische Quelle; Inkarnation kein Persistenzbeweis. |
| [ProxyFileDescriptorCallback](https://developer.android.com/reference/android/os/ProxyFileDescriptorCallback), seit API 26, Seitenstand 2026-08-03 | Eigene Callbackprüfung für Größe, Lesen, Schreiben, Fsync; `onRelease` räumt Ressourcen auf. |
| [Cryptomator Vault encryption](https://docs.cryptomator.org/security/vault/), aktueller Webstand ohne hier behauptete Paketversion | Randomisierte Inhaltsverschlüsselung; Journalpfadverwendung durch den Projektcode belegt. |
| [RFC 4331 §3](https://www.rfc-editor.org/rfc/rfc4331#section-3), Februar 2006 | Quota-Property kann mit 404 fehlen; daraus folgt nicht unbegrenzter Platz. |

## Entscheidende Dateihashes der ersten Nachprüfung um 20:14 UTC

SHA-256 der gelesenen Arbeitskopie am `2026-10-25T20:14:30.061244+00:00`. Die Liste ist ein Inhaltsanker für die sechs Reparaturentscheidungen und den damals offenen Offlineauditbefund, keine Behauptung eines atomaren Repositorysnapshots. Für die F1-Reparatur gilt die neue Hashliste um 20:40 UTC oben.

| Datei | SHA-256 |
| --- | --- |
| `native/src/app/core/drive_snapshot.rs` | `482d376d58c2d93c19ee6810ae750b22331127a8bde3bc1ce621c3255b5191cb` |
| `native/src/app/os/linux_os/drive_inventory.rs` | `31170b4ad91e7e31711f80302d8aae0259a4b1c1c37bee68a580bbb9fdb22fa2` |
| `native/src/app/os/windows/drive_inventory.rs` | `fac19e3d7b48027f409d9d73ef35ec2da3dc855cdbb7e7553fc7b469038ed1da` |
| `native/src/app/core/mount_recovery_ui.rs` | `13bf8dd1c9a90ff47c42202c1069c3d4d7fbfdd2a45d942730a357c9bf8b97d3` |
| `native/src/cli/drive_recovery.rs` | `57f89253c552bb08955f883c50123898afaf3d3de1bbc6186bd3a830b41ddf5f` |
| `native/src/mount/core/recovery_identity.rs` | `42caa07108bc092b5d42a96b563e235c54c0ee75b9198afa543db6d653c01407` |
| `native/src/mount/core/recovery_manifest.rs` | `0abee51c7906d538fd9348b982b761ba2d0ef350157b90ff435af35a690d204d` |
| `native/src/mount/core/recovery_export.rs` | `4d134b3b1d466380a4eb10500091b1c01a659aadbe8fd276e7dc79092eb87ba2` |
| `native/src/mount/core/recovery_import.rs` | `38c976a71fe09b2c46d282d5cf852f6502fb11c52a4859bb7dafdf4722d2d792` |
| `native/src/mount/os/shared/recovery_identity.rs` | `43ebaaad23c496ee716be7505d6d31ea546f40b4c3cfd93a52b7ab31d1bb0e26` |
| `native/src/mount/os/shared/recovery_export.rs` | `1aae7a8d7479eb5979ba7bb3f7efe5da6b09cab1c236555f29674a7e4a6a5440` |
| `native/src/mount/os/shared/recovery_import.rs` | `70653aaf57dcb9772d544409f9e0843b431fabc571abdbd8419cfe125460215f` |
| `native/src/daemon/os/shared/mount_recovery_identity.rs` | `620f24888596a1f0b1abe306d0307c236026d8109a9c13be28873e2a39f4347b` |
| `native/src/daemon/os/shared/mount_manager_start.rs` | `2d2b3f8140035bc97818ad692d089a6b94f4ce484e24a048499b8f151819f5bc` |
| `native/src/daemon/os/shared/mount_manager_start_cache.rs` | `53e3f204671118b49af0afea6ed9d9430b2a06861cdeb820410884cecfd46103` |
| `native/src/daemon/os/shared/mount_recovery_transaction.rs` | `74f55fa7d344e1ed16e92e5566ed5aca8f30e7ac5a478370589343289681e516` |
| `native/src/daemon/os/shared/mount_inactive_recovery.rs` | `52e42cc097adfc085cdef9046e987251d9deebf91d124366439d2e4f40a71e45` |
| `native/src/gdrive/core/backend.rs` | `97e016c81203afbb301736c1af41e6bbb1c7f9398cc9dfb26bbaced866a46dc3` |
| `native/src/gdrive/core/identity.rs` | `ba71f4c31d77b4bab8ebbfb39318069ef05d467a24921a21a0afb30fb4448dc4` |
| `native/src/gdrive/core/resolution.rs` | `fc5c86bed10995b2f28b21b6bb4347aec1a5b821c2b4d581a83d5e0ad473ddc4` |
| `native/src/gdrive/core/state.rs` | `5ea42dfc5038ec30a0adb5311a2bca802c65cf01a9a246ced1edc09badfeb216` |
| `native/src/gdrive/core/auth.rs` | `c6c762d21ac8273dfa3f16e67fcbf6c17015981c969759f9c7cd2e66280a3c52` |
| `native/src/cloud/core/auth_state.rs` | `3135763e8d32ab1f0fcafb63e2abc49b21ee09e7a08e74137a0581444480fef9` |
| `native/src/cloud/os/oauth_http.rs` | `acf74b7c9e551adf72996fc97a3569526e1544761de12b15bb3d227c8862aa85` |
| `native/src/cloud/os/coordinator.rs` | `4a66b893dcd7fe090df12aa6459a42208f659b9856b447943b0113445fa620af` |
| `native/src/mobile/os/shared/domains/docs_access.rs` | `b4a45925536ca941d811adc59102181872c3a4212fafcfda0d478e16560b00fe` |
| `native/src/mobile/os/shared/domains/docs_engines.rs` | `61ad68962821fde69a395a63dc2b79047232eb456a475061496f595c574a46b6` |
| `native/src/mobile/os/shared/domains/docs_io.rs` | `67426f30e9f59c01358c8108d74ab118246aa6471fc05b7fd2fe83ba70576eb0` |
| `native/src/mobile/os/shared/domains/docs_vault.rs` | `feae9089fea78bf112c31673e4ce1713c5a08f134c7a1b482002aa8ab33e19e5` |
| `native/src/vault/os/shared/open_policy.rs` | `28d661e289730f6cfe1d930881696506e61cf6335191156348e953127c846ccf` |
| `native/src/vault/os/shared/journal_sealer.rs` | `0892e476159f67159cbfa531b058ca34d527ed5dff5f0458139e7bddd6097d6b` |
| `native/src/mount/core/spool.rs` | `a942f19d9705188003740b1aefa7513f9ea8d87a5a1237d04b3cd10307e4b6ee` |
| `native/src/mount/core/spool_storage.rs` | `e773c7c96ba76b747bc665e34e73fe52c10f2462f377e6f0b7c461be09535a4e` |
| `native/src/mount/core/spool_recovery.rs` | `7f402e090fae0150b599d88eb5e3a9800d54a9730415cd9d079f20d860476886` |
| `native/src/mount/core/path_sealer.rs` | `a5e6280df5ff6b57dab2fd3836c700e3dd64bfc469bf9385918654b0413061e9` |
| `native/src/mount/core/journal.rs` | `427cd8ed37c45988476d2969e82eee35172d2da36786ab6bc09b99505dbc506d` |
| `native/src/mount/core/journal_record.rs` | `2c242209c5e7a53e75087174fb45b10d905291863f598751d327fd10ee495cdc` |
| `native/src/mount/core/file_commit.rs` | `17946e19892278dcf321d0d05304b741323306506d2961ed6a4afcf2081c0374` |
| `native/src/mount/core/entry_lifecycle.rs` | `21d76dfa5654937fb207db7a3728eb2f959f90d2f0d1bf0ef5178271c21880ed` |
| `native/src/webdav/core/capacity.rs` | `3825aa9307ca9a137284634c0f74bb73c876bc27b81a86e3928ed53a34cc10d2` |
| `native/src/webdav/core/multistatus.rs` | `6e615dc8bd299cfadc3be5c8f9d7f63682a9c012bf3ff14c9a833aefdc07c89d` |
| `native/src/vfs/core/capacity.rs` | `34ddf07bc93c8700cfdfd9bf215e4774c741cf9502ea4a5e37955d6b8fbab149` |

## Vollständiges Lese- und Schreibinventar dieses Nachreviews

Die folgende Liste zählt **tatsächlich gelesene Dateien beziehungsweise gezielt gelesene Ausschnitte**; sie behauptet keine Volllektüre jeder großen Datei. Das nachfolgende Suchinventar trennt reine Suchtreffer davon. Historische Lektüre des Erstberichts steht ausschließlich im historischen Anhang.

Projektanweisungen: der vollständig übergebene Inhalt von `AGENTS.md`. Architektur und Verträge: `docs/ARCHITEKTUR.md` (Architekturüberblick), `docs/plaene/2026-10-24-batches/batches.md` (B1/P10/P21/P22/P23/P26), `vertraege.md` (V4/V5), `recherche.md` (B1-Quellen), `review-b1.md` (bestehender Bericht). `native/Cargo.toml` und `native/Cargo.lock` wurden als TOML gelesen; die oben genannten Metadaten wurden gezielt ausgewertet.

```text
native/src/app/core/drive_snapshot.rs
native/src/app/core/mount_recovery_ui.rs
native/src/app/os/linux_os/drive_inventory.rs
native/src/app/os/windows/drive_inventory.rs
native/src/cli/drive_recovery.rs
native/src/mount/mod.rs
native/src/mount/core/recovery_identity.rs
native/src/mount/core/recovery_manifest.rs
native/src/mount/core/recovery_export.rs
native/src/mount/core/recovery_import.rs
native/src/mount/core/recovery.rs
native/src/mount/core/spool.rs
native/src/mount/core/journal.rs
native/src/mount/core/journal_record.rs
native/src/mount/core/engine.rs
native/src/mount/core/case_semantics.rs
native/src/mount/core/frontend.rs
native/src/mount/core/frontend_engine.rs
native/src/mount/core/file_io.rs
native/src/mount/core/file_commit.rs
native/src/mount/core/spool_storage.rs
native/src/mount/core/path_sealer.rs
native/src/mount/core/spool_recovery.rs
native/src/mount/core/clean_cache.rs
native/src/mount/core/entry_lifecycle.rs
native/src/mount/os/shared/recovery_identity.rs
native/src/mount/os/shared/recovery_export.rs
native/src/mount/os/shared/recovery_import.rs
native/src/mount/os/linux_os/cache_lease.rs
native/src/mount/os/linux_os/recovery_storage.rs
native/src/mount/os/windows/cache_lease.rs
native/src/mount/os/windows/recovery_storage.rs
native/src/daemon/os/shared/mount_recovery_identity.rs
native/src/daemon/os/shared/mount_recovery_transaction.rs
native/src/daemon/os/shared/mount_inactive_recovery.rs
native/src/daemon/os/shared/mount_manager_start.rs
native/src/daemon/os/shared/mount_manager_start_cache.rs
native/src/daemon/os/shared/mount_source.rs
native/src/vault/os/shared/open_policy.rs
native/src/vault/os/shared/session.rs
native/src/vault/os/shared/session_seal.rs
native/src/vault/os/shared/journal_sealer.rs
native/src/vault/os/shared/backend_identity.rs
native/src/vault/core/open_policy.rs
native/src/mobile/os/shared/domains/mod.rs
native/src/mobile/os/shared/domains/docs.rs
native/src/mobile/os/shared/domains/docs_access.rs
native/src/mobile/os/shared/domains/docs_engines.rs
native/src/mobile/os/shared/domains/docs_io.rs
native/src/mobile/os/shared/domains/docs_vault.rs
native/src/cloud/mod.rs
native/src/cloud/core/auth_state.rs
native/src/cloud/core/token_persistence.rs
native/src/cloud/os/coordinator.rs
native/src/cloud/os/oauth_http.rs
native/src/cloud/os/session_store.rs
native/src/gdrive/core/state.rs
native/src/gdrive/core/auth.rs
native/src/gdrive/core/overload.rs
native/src/gdrive/core/identity.rs
native/src/gdrive/core/extensions.rs
native/src/gdrive/core/resolution.rs
native/src/gdrive/core/backend.rs
native/src/webdav/core/capacity.rs
native/src/webdav/core/multistatus.rs
native/src/vfs/core/capacity.rs
```

Zusätzlich gezielt durchsucht, ohne daraus eine vollständige Codelektüre abzuleiten:

```text
docs/oberflaechen/laufwerke.md
docs/oberflaechen/tresore.md
native/src/app/core/mount_ui_helpers.rs
native/src/mount/core/recovery_state.rs
native/src/daemon/mod.rs
native/src/daemon/os/shared/rooted_backend.rs
native/src/daemon/os/shared/rooted_backend_extensions.rs
native/src/gdrive/core/metadata.rs
```

Die Lesefreigaben für `gdrive/core/backend.rs`, `mount/core/{spool_storage.rs,path_sealer.rs,spool_recovery.rs,clean_cache.rs,entry_lifecycle.rs}` wurden vom Hauptagenten zusätzlich erteilt. Keine Ausweitung auf repositoryweite Suche. Die freigegebenen, aber hier nicht als gelesen aufgeführten Dateien waren keine Nachweisbasis dieses Nachreviews.

Angewandte Arbeitsweise: `/root/.codex/skills/arbeitsweise/SKILL.md`, `/root/.agents/skills/arbeitsweise/SKILL.md`, delegierter zentraler MCP-Stand Revision 337. Der Auftrag nennt Astra/xhigh; eine vererbte MCP-Modellkennung ist kein Beleg der tatsächlichen Modellausführung. Modellbelege und Orchestrierung liegen beim Hauptagenten.

**Einzige durch den Reviewagenten geänderte Datei:** `docs/plaene/2026-10-24-batches/review-b1.md`. Keine Produktions-, Test- oder Konfigurationsdatei verändert. Keine Commits, Pushes, CI-/Releaseaufrufe, Builds, Compiler, Tests, Formatter, Installationen, Server oder Subagenten. Die erste Nachprüfung schloss fünf ursprüngliche Befunde und den R05-Exportfehler; der damals offengehaltene F1-Befund ist durch die zusätzliche Nachprüfung oben ebenfalls statisch geschlossen. Die Laufzeitbestätigung sämtlicher sechs Reparaturbereiche bleibt unbestätigt und gehört nicht zu diesem Nachreviewauftrag.

## Historischer Erstbericht – Status und Hashes durch den Nachreview oben ersetzt

Der folgende vollständig erhaltene Bericht dokumentiert ausschließlich den Stand **19:28 UTC vor Integration der Reparaturen**. Seine sechs damaligen offenen Befunde, alten Zeilennummern, Hashes, Leseinventare und Folgeschritte sind historische Evidenz; sie sind kein zweites aktuelles Statusboard.

<details>
<summary>Erstbericht vom 2026-10-25, 19:28 UTC, vollständig anzeigen</summary>

## B1 – ursprünglicher statischer kritischer Review


Stand der Codelektüre: 2026-10-25, Abschlussabgleich 19:28 UTC. Auftrag: P10, P21, P22, P23 und P26 rein am Code und an einschlägigen Primärquellen prüfen. Keine Implementierung.

## Ergebnis und geprüfter Stand

**Sechs belegte Defekte bleiben im geprüften Stand offen: zwei P1-Befunde und vier P2-Befunde.** Der schwerste Befund erlaubt, importierte Google-Drive-Schreibreste beim ausdrücklichen Wiederanlauf einem abweichenden Konto zuzuordnen. Die Befunde sind statische Gegenbeispiele, keine ausgeführten Laufzeitversuche.

Vergleichsbasis ist `bea312ea`; der OAuth-Meilenstein ist `b27d0549`. Zu Beginn wurde `a1b44770`, beim Abschluss der Codelektüre `060f156bc92ff686b6e12782e21ce14adca059c1` auf dem gemeinsamen `main`-Worktree gesehen. **HEAD allein beschreibt den Reviewstand nicht:** Die B1-Neudateien sind teilweise untracked; `docs_engines.rs` und `docs_vault.rs` enthalten Arbeitskopieänderungen. Die Dateihashes unten legen die entscheidenden gelesenen Inhalte fest.

Die parallelen B2-, B3-, B5- und B6-Arbeiten wurden nicht geprüft. Die bereits sichtbaren P11-Opticalreader-Leases in ConfinedLocal wurden als angrenzender Iststand gelesen; ihre Schedulerimplementierung gehört nicht zu diesem Review. Spätere Änderungen an den unten genannten Befundflächen sind nicht durch diesen Bericht bewertet.

Der Erstplan war die Prüfung der fünf B1-Verträge an ihren gemeinsamen Identitäts-, Mount-, Recovery- und Authgrenzen. Nach der Bestandslektüre konzentrierte sich der Detailplan auf gehaltene Rootobjekte, laufende Freigabeprüfung, Export/Import/Wiederanlauf, Provider-/Cachetrennung und Refreshrotation einschließlich Fehlerzuständen. Zusätzliche Lesefreigaben des Hauptagents deckten die konkreten Android-, Mountresolver- und WebDAV-Übergänge ab. Graphdateien lagen außerhalb der zugewiesenen Lesefläche; es erfolgten weder Graphabfragen noch Graphneubauten.

## Priorisierte Defekte

### B1-R01 — P1: Google-Drive-Recovery besitzt keine historische Kontobindung

**Stellen:** `native/src/daemon/os/shared/mount_recovery_transaction.rs::save_remove` (Zeilen 24–28), `recovery_reactivate` (75), `validate_vault_identity` (113); `native/src/mount/core/recovery_import.rs::validate_manifest` (15); `native/src/mount/core/mount_source.rs::MountSource::validate`; `native/src/daemon/os/shared/mount_source.rs::resolve`.

**Konkreter Trigger:** Auf Installation A bleibt eine neue Datei im Google-Drive-Mountcache als `Dirty` mit `Baseline::Missing`. Das Recoverypaket wird auf Installation B importiert; B ist bereits mit einem abweichenden Google-Konto verbunden. Der ursprüngliche Mount zeigte auf den Drive-Root, die Mount-ID ist auf B noch frei. Anschließend erfolgen die ausdrücklich angebotenen Schritte `recovery reactivate` und `drive retry`.

**Beleg und Auswirkung:** Für jede Nicht-Vaultquelle serialisiert der Export nur `MountSource` als `backend_identity`. Bei Google Drive kann dessen `account` ausschließlich `cloud:gdrive` sein. Der Import prüft lediglich die Gleichheit dieser zwei Kopien derselben Konfiguration. Die einzige Identitätsprüfung vor Reaktivierung kehrt bei Nicht-Vaultquellen sofort mit Erfolg zurück. Der Resolver öffnet danach das aktuell lokal angemeldete Drive-Konto. Dessen in `gdrive/core/state.rs::connect` bestätigte Kontokennung wird nicht mit dem Ursprungsmanifest verglichen. `mount/core/recovery.rs::retry_pending_changes` veröffentlicht Dirtyeinträge; `file_commit.rs::detect_conflict` akzeptiert bei einer neuen Datei ausdrücklich `(Baseline::Missing, None)`. Die fehlende Datei auf Konto B gilt deshalb nicht als Kontokonflikt. Der Inhalt aus A kann in B veröffentlicht werden.

Das ist keine Behauptung, dass ein normaler OAuth-Kontowechsel innerhalb derselben Installation P26 umgeht: Das Gegenbeispiel verwendet den vorgesehenen Paketimport in ein getrenntes Profil. Der Fehler liegt an der fehlenden historischen Bindung des Pakets.

**Quellbeleg:** Der lokale Vertrag V4/P26 verbietet das stille Neuinterpretieren gespeicherter Drivewurzeln bei Kontowechsel. Google dokumentiert `root` als Rootalias und den benutzerspezifischen My-Drive-Zielordner; dieser Alias ist kein kontenübergreifender Identitätsbeweis. [Drive API v3: Create and populate folders](https://developers.google.com/workspace/drive/api/guides/folder)

**Korrekturrichtung und Änderungsgrenze:** Beim ursprünglichen Mount eine bestätigte, persistente Bindung aus Provider, Principal/Kontokennung, Namespace und tatsächlicher Rootobjekt-ID zusammen mit dem Cache festhalten; diese beim Offlineexport übernehmen. Vor dem ersten Remotezugriff nach Import gegen den frisch aufgelösten Backendhandle prüfen. Unbekannte historische Bindung muss den Bestand erhalten und eine ausdrückliche Zuordnung verlangen. Der allgemeine Bezeichner `cloud:gdrive` darf weiter der UI-Auswahl dienen. Betroffen sind Mount-/Cachebindungsmetadaten, Recoveryformat/-validierung und Startgate; ein neues Mehrkonten-UI ist hierfür nicht erforderlich.

### B1-R02 — P1: Linux verwechselt neue FUSE-/Netzmounts mit dem bisherigen Mount

**Stellen:** `native/src/app/os/linux_os/drive_inventory.rs::read_drive_inventory` (6, insbesondere 21–28); `native/src/app/core/drive_snapshot.rs::diff` (38) und `App::apply_drive_snapshot` (131).

**Konkreter Trigger:** Ein FUSE-Mount unter `/mnt/work` wird zwischen zwei Bestandsaufnahmen ersetzt. Dateisystemtyp, gemeldete Quelle und Subroot bleiben gleich; Mount-ID und `major:minor` wechseln. Zwei dafür zulässige mountinfo-Zeilen sind beispielsweise:

```text
41 25 0:52 / /mnt/work rw - fuse.example provider rw
74 25 0:87 / /mnt/work rw - fuse.example provider rw
```

**Beleg und Auswirkung:** Beide Zeilen ergeben `identity = "mount:fuse.example:provider:/"`. Die OS-Mount-ID und `major:minor` gehen bei Nicht-Blockquellen nicht in die Identität ein. `diff` liefert damit weder Entfernung noch Zugang. Die dadurch gesteuerte Invalidierung von Scanresultaten, Tabellenzeilen, Auswahl und Indexgeneration bleibt aus. Ein späterer pfadbasierter Vorgang aus der alten Ansicht kann den gleichnamigen Eintrag des Ersatzmounts adressieren.

Das Beispiel benötigt keine Annahme über eine konkrete FUSE-Anwendung: Die beiden Mountinstanzen sind verschieden, während das gelesene Quellenfeld nach seinem dokumentierten Vertrag kein eindeutiges Medienmerkmal ist.

**Quellbeleg:** Linux man-pages 6.19 beschreibt Mount-ID als Kennung der Mountinstanz, `major:minor` als `st_dev` und `mount source` als dateisystemspezifische Angabe beziehungsweise `none`. Es beschreibt auch gestapelte Mounts am gleichen Pfad. [proc_pid_mountinfo(5)](https://man7.org/linux/man-pages/man5/proc_pid_mountinfo.5.html)

**Korrekturrichtung und Änderungsgrenze:** Persistente Medienidentität und aktuelle Mountinkarnation getrennt erfassen. Ein Wechsel der Inkarnation muss die betroffenen UI-/Indexgenerationen ungültig machen, auch wenn der Quellenname identisch bleibt. Die Mount-ID allein ist kein dauerhaft gespeicherter Medienbeweis, da Linux ihre Wiederverwendung nach Unmount erlaubt. Die Korrektur betrifft Linuxinventar und die gemeinsame Snapshot-/Bindinglogik; unveränderte Roots sollen ihre Indexdaten behalten.

### B1-R03 — P2: Android verwirft die laufende Tresor-Rootprüfung nach dem ersten Öffnen

**Stellen:** `native/src/vault/os/shared/open_policy.rs::authorize_provider` (171), `MountAccess::check` (123); `native/src/mobile/os/shared/domains/docs_vault.rs::source` (97); `docs_engines.rs::engine` (129); `docs_io.rs::get` (65), `open` (82), `doc_read`, `doc_write`, `doc_fsync` und `finish`.

**Konkreter Trigger:** Ein freigegebener, entsperrter lokaler Tresor wurde im Android-Dokumentanbieter geöffnet. Danach wird sein bisheriger Rootordner bei weiter bestehender Sitzung umbenannt und am registrierten Pfad durch einen neuen Root ersetzt. Alternativ wird die gebundene Rootfreigabe bei weiter bestehendem Slot neu bestätigt. Der bestehende Proxydateideskriptor oder ein neuer `doc_open` auf dem bereits gecachten Enginekey wird weiter benutzt.

**Beleg und Auswirkung:** `authorize_provider` berechnet einen `MountAccess`, führt einmal `check` aus und gibt danach den ursprünglichen Backendhandle zurück. Der Guard wird weder gewrappt noch im Slot gespeichert. `engine` gibt vorhandene Slots ohne erneute Prüfung zurück. `docs_io::get` prüft ausschließlich `revoked`; `open` prüft die Rootfreigabe und den Entsperrstatus, nicht die gebundene Rootidentität. Der Kommentar in `authorize_provider`, wonach der Provider Policy und Freigabe bei jeder Deskriptoroperation prüfe, stimmt für die Policyprüfung nicht mit diesem Pfad überein.

Ein gehaltener lokaler Backendroot verhindert zwar das Umlenken seiner I/O auf den Ersatzordner. Trotzdem bleibt der vorherige Root über den Dokumentanbieter les- und schreibbar, obwohl dieselbe aktuelle Rootbindung im Desktop-`MountAccess` bereits abgewiesen würde. Bei gecachten Spooldaten erreicht eine Deskriptoroperation zudem nicht zwingend überhaupt eine Backendprüfung.

**Abgrenzung:** Der ausdrücklich ausgeführte Entzug über `set_root_trust(false)` ruft den vorhandenen Lockparticipant auf; `withdraw` entzieht dabei Deskriptoren. Dieser funktionierende Weg wird nicht als generell defekt bewertet. Der Befund betrifft den laufenden Root-/Bindingwechsel ohne diesen expliziten Entzug.

**Quellbeleg:** Der Projektvertrag V4/P21 umfasst ausdrücklich die Android-Rootprüfung und Deskriptorkette. Androids seit API 26 verfügbare Proxydateideskriptoren delegieren Lesen, Schreiben und Flush an diese Anwendungscallbacks; Fehler müssen dort als passende `ErrnoException` zurückgegeben werden. Die Plattform kennt die private Tresorbindung nicht. [ProxyFileDescriptorCallback](https://developer.android.com/reference/android/os/ProxyFileDescriptorCallback)

**Korrekturrichtung und Änderungsgrenze:** Den konkreten `MountAccess` für die Lebensdauer des Providerslots behalten und auch vor Zugriffen auf gecachte Klartextdaten, Größenangaben sowie Flush/Release prüfen; bei Abweichung Slot und Deskriptoren entziehen, versiegelte Änderungen erhalten. Ein Wrapper nur um Netzwerk-/Backend-I/O reicht wegen Spoolzugriffen nicht aus. Die bestehende Sitzungssperre und der versiegelte Spool bleiben erhalten.

### B1-R04 — P2: Vorübergehende Drive-403-Fehler sperren die gespeicherte Anmeldung dauerhaft

**Stellen:** `native/src/cloud/os/oauth_http.rs::identify` (51); `native/src/gdrive/core/state.rs::load_drive_account_key` (384); `gdrive/core/auth.rs::refresh_for_account` (44); `cloud/core/auth_state.rs::AuthError::provider` (56); `cloud/os/coordinator.rs::refresh` (113, 131–140), `record_failure` (157) und `validate_account` (207).

**Konkreter Trigger:** Nach einem gültigen Refresh beantwortet Drive die Abfrage `about?fields=user(permissionId)` mit HTTP 403 und `reason = userRateLimitExceeded` oder `rateLimitExceeded`.

**Beleg und Auswirkung:** Die Identitätsabfrage verwirft den Fehlerbody beziehungsweise dessen strukturierten Grund und ruft `AuthError::provider(403, "provider_error")` auf. Diese Klasse wird zu `ProviderError`, gespeichert und bei jedem zukünftigen Refresh bereits vor dem Netzwerkzugriff abgewiesen. Auch die Prüfung eines noch gültigen gecachten Bearertokens weist diesen Zustand ab. `status` meldet `signed_in = false`. Das Abklingen des Ratelimits oder ein Prozessneustart hebt die gespeicherte Sperre nicht auf. Der Refreshtoken bleibt vorhanden, ist über den normalen Retrypfad aber nicht nutzbar.

**Quellbeleg:** Google verlangt bei Drive-403 die Auswertung von `reason` und nennt für die beiden genannten Ratelimitgründe ausdrücklich Wiederholung mit exponentiellem Backoff. Das sind Ressourcen-API-Fehler, kein OAuth-`invalid_grant`. [Drive API v3: Resolve errors](https://developers.google.com/workspace/drive/api/guides/handle-errors)

**Korrekturrichtung und Änderungsgrenze:** Einen begrenzten Decoder für die tatsächlichen Drivefehlergründe an sämtlichen drei Identitätsabfragen verwenden. Wiederholbare Ressourcenfehler dürfen die bestehende Authidentität nicht terminal sperren. Sichere Fehlercodes können gespeichert werden; Providerbeschreibungen und Tokens dürfen weiterhin nicht ins Diagnosemodell gelangen. Die bestehende atomare Speicherung rotierter Refreshtokens auch bei fehlgeschlagener Identitätsabfrage muss erhalten bleiben.

### B1-R05 — P2: Der Recoveryexport spielt versiegelte Journalpfade als Klartextschlüssel ab

**Stellen:** `native/src/mount/core/recovery_export.rs::inspect_journal` (7, insbesondere 29–30); `mount/core/journal_record.rs::Action::sealed` (83) und `apply` (187); `vault/os/shared/journal_sealer.rs::JournalPathSealer::seal_text` (42). Gegenbeleg für den vorgesehenen Ablauf: `mount/core/journal.rs::replay` (230), dort `decode_record(...).unsealed(sealer)` vor `apply`.

**Konkreter Trigger:** Ein versiegeltes Tresorjournal unterhalb der Kompaktierungsschwelle enthält `UpsertEntry("/a")` und später `ForgetEntry("/a")`; der saubere Spoolinhalt wurde danach regulär entfernt. Ein solcher Bestand wird offline exportiert. Entsprechend betroffen sind mehrere Updates oder `MoveEntry` beziehungsweise `ForgetNamespaceConflict`.

**Beleg und Auswirkung:** Jeder Persistenzvorgang versiegelt denselben Pfad mit einem frischen Header. Die beiden Ciphertextstrings für `/a` sind daher verschieden. Der Export ruft `apply` direkt auf den noch versiegelten Aktionen auf. Dessen HashMap ist nach `remote_path` beziehungsweise Konfliktpfad indiziert: Vergessen und Verschieben treffen den früheren Eintrag nicht, Updates werden als zusätzliche Einträge behandelt. Der Export meldet deshalb bereits erledigte Einträge als weiterhin vorhanden und erzeugt nach deren regulärer Cachebereinigung falsche `MissingPayload`-Lücken. GUI und CLI zeigen diese als ungesicherte Datenbezüge an.

Die Originaljournalbytes werden kopiert; dieser Befund beweist keinen Verlust der Rohsicherung und keinen fehlerhaften normalen Journalreplay. Defekt ist die als rekonstruierter Zustand ausgegebene Recoveryinventur.

**Quellbeleg:** Der Projektcode benennt im `JournalPathSealer` ausdrücklich frische Header pro Feld. Das verwendete Inhaltsformat nutzt zufällige Headernonce und Inhaltschlüssel; es darf nicht mit der deterministischen Dateinamenverschlüsselung verwechselt werden. [Cryptomator: Vault Cryptography, File Header Encryption](https://docs.cryptomator.org/security/vault/)

**Korrekturrichtung und Änderungsgrenze:** Bei einem Offlineexport ohne Schlüssel den versiegelten Journalzustand als nicht semantisch ausgewertet kennzeichnen und keine pfadbasierten Zustands-/MissingPayloadbehauptungen daraus ableiten. Alternativ benötigt das Persistenzformat einen ausdrücklich entworfenen stabilen, nicht verräterischen Datensatzschlüssel oder eine autorisierte Auswertung im Schlüsselhalter. Die sichere kurzfristige Grenze liegt im Recoverydecoder/-manifest; die Journalverschlüsselung nicht zur Herstellung gleicher Ciphertexte abschwächen.

### B1-R06 — P2: Nicht angebotene WebDAV-Quotaproperties werden als defekte Providerantwort gemeldet

**Stellen:** `native/src/webdav/core/capacity.rs::query_inner` (12), Aufruf von `successful_props` (21); `webdav/core/multistatus.rs::successful_props` (35, Abweisung ohne erfolgreiche Properties); `native/src/vfs/core/capacity.rs::capacity_failure`; `mount/core/capacity.rs::MountCapacityReport::display_lines`.

**Konkreter Trigger:** Ein erreichbarer WebDAV-Server antwortet auf die Quota-PROPFIND mit gültigem 207-Multistatus, passendem `href` und beiden angefragten Eigenschaften in einem `propstat` mit Status 404. Er bietet diese optionalen Quotaangaben nicht an.

**Beleg und Auswirkung:** Die Quotaabfrage fordert ausschließlich diese beiden Properties an. Der gemeinsam genutzte Propertyparser verlangt mindestens eine erfolgreiche Propertygruppe und erzeugt für die gültige Antwort `InvalidData`. Daraus werden `CapacityState::Unavailable` und `CapacityFailure::InvalidResponse`; die Statusdarstellung nennt den Remotezustand „nicht erreichbar“. Der Zustand müsste `Unknown` ohne Providerfehler sein. Der Cachefallback funktioniert weiterhin; ein Schreibstopp wird hier nicht behauptet.

**Quellbeleg:** RFC 4331 §3 erlaubt bei fehlender/unbegrenzter Quote ausdrücklich das Weglassen von `quota-available-bytes` mit Property-404 und verbietet die Schlussfolgerung, die Quote sei dadurch sicher unbegrenzt. Das rechtfertigt `Unknown`, nicht `InvalidResponse`. [RFC 4331 §3](https://www.rfc-editor.org/rfc/rfc4331#section-3)

**Korrekturrichtung und Änderungsgrenze:** Im quota-spezifischen Decoder vollständige 404-Propertyantworten als unbekannte Quote behandeln. Echte Responsefehler, Authfehler und fehlerhafte Zahlen bleiben getrennt. Die strengere Vollständigkeitsprüfung des allgemeinen Verzeichnislistings darf hierfür nicht global abgeschwächt werden.

## Abgleich mit Primärquellen und versionsgebundenen APIs

Alle externen Quellen wurden am **2026-10-25** abgerufen. Die folgenden positiven Aussagen gelten nur für die genannten Code-/APIübergänge:

| Grenze | Festgeschriebener Stand / Primärquelle | Ergebnis der statischen Prüfung |
|---|---|---|
| Refreshrotation | OAuth 2.0, [RFC 6749 §6](https://www.rfc-editor.org/rfc/rfc6749#section-6); `ureq 2.12.1` im Lock | Der Coordinator hält Prozess-/Dateisperre über Refresh und Persistenz. Ein vom Provider ersetzter Refreshtoken bleibt auch bei anschließendem Identitätslesefehler gespeichert. B1-R04 betrifft die Fehlerklassifikation. |
| Google-Kapazität | Drive API v3, [About.storageQuota](https://developers.google.com/workspace/drive/api/reference/rest/v3/about) | Die Abfrage nutzt `usage` über die Dienste und `limit`; Shared-Drive-Corpus erhält keine erfundene eigene Accountquote. Ein fehlendes Limit wird als unbegrenzt dargestellt. |
| SFTP-Kapazität | `russh-sftp 2.3.0`; [veröffentlichtes Cratearchiv](https://static.crates.io/crates/russh-sftp/russh-sftp-2.3.0.crate), SHA-256 stimmt mit `native/Cargo.lock` überein; [OpenSSH V_9_9_P2 PROTOCOL §4.2/4.4](https://raw.githubusercontent.com/openssh/openssh-portable/V_9_9_P2/PROTOCOL) | `SftpSession::new_with_config` akzeptiert statvfs nur mit Versionsstring 2; `fs_info` liefert sonst `None`. Der Adapter verwendet `fragment_size` und den für den Nutzer verfügbaren Blockwert. Das Archiv wurde nur im Speicher gelesen, nicht installiert oder ausgeführt. |
| WebDAV-Kapazität | RFC 4331; `roxmltree 0.20.0`, `ureq 2.12.1` | Quota wird gezielt angefragt; URLpfad und Propertyantworten werden geprüft. Die vollständige Abwesenheit beider Properties scheitert gemäß B1-R06. |
| Linux-Laufwerksereignisse | `zbus 5.16.0`, [MessageStream::for_match_rule](https://docs.rs/zbus/5.16.0/zbus/struct.MessageStream.html); Linux mountinfo, man-pages 6.19 | UDisks2-Ereignisse und periodische Inventur sind im Code getrennt von Kapazitätsabfragen. Die Quellenname-/Instanzverwechslung bleibt B1-R02. |
| Windows-Laufwerksereignisse | direktes `windows-sys 0.59.0`; [WM_DEVICECHANGE](https://learn.microsoft.com/en-us/windows/win32/devio/wm-devicechange) | Der Adapter erstellt ein verborgenes Top-Level-Fenster und betreibt eine Message-Loop; er verwendet kein reines Message-only-Fenster für die Broadcastannahme. |
| Windows-Datei-/Recoveryveröffentlichung | [CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew), [MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw) | Gehaltene Verzeichnishandles, relative Childöffnungen und No-follow-Prüfungen sind vorhanden. Ein statischer Abgleich beweist keine universelle Crashdurabilität für NTFS, ReFS, FAT, exFAT, Netzwerkredirectoren und Filtertreiber. |
| Android-Dokumentdeskriptoren | ProxyFileDescriptorCallback seit API 26 | Der versiegelte Spool und der vorhandene explizite Deskriptorentzug sind erhalten; die dauerhafte Bindungsprüfung fehlt gemäß B1-R03. |
| Mountkapazität | `fuser 0.18.0` im Lock; gemeinsame `MountCapacity`-/`MountCapacityReport`-Projektion | Remotequote und tatsächliches lokales Budget bleiben getrennte Felder. Unknown/Unbounded/Unavailable projizieren auf ein als CacheCompatibility bezeichnetes OS-Budget; bekannte Remoteverfügbarkeit wird mit dem Cachebudget begrenzt. Kein umfassender FUSE-/Dokany-Verträglichkeitsnachweis. |

Zusätzlich aus den gelesenen Recoveryflächen belegt: Der Export kopiert Originaldateien einschließlich Journal-/Stageartefakten in ein privates Paket, bindet sie über Länge und SHA-256, veröffentlicht das Paket vor dem Besitzwechsel und entfernt den Originalcache nicht. Aktive Mountregistrierung und inaktiver Recoverybesitz werden im selben Registrydokument geändert. Das sind begrenzte Codeeigenschaften; B1-R01 und B1-R05 verhindern eine pauschale Aussage, der Recoveryvertrag sei vollständig erfüllt.

## Ungeprüfte Aussagen und punktuelle Folgearbeit

Keine Builds, Compiler, Tests, Formatter, Server, Installationen, Commits, Pushes, Remote-CI-Läufe oder Releases wurden ausgelöst. Es gibt daher keinen Nachweis für Kompilierbarkeit, reale Hotpluglatenz, Windows-/Android-Laufzeit, Crash-/Stromausfallverhalten oder konkrete Providerkonten. Fehlende reale Konten sind hier weder Pass noch zusätzliche Releasebedingung.

Die erweiterten Sicherheitsverzeichnisse wurden gezielt an den genannten Übergängen gelesen, nicht vollständig auditiert. Außerhalb der Dateiliste stehende UI-/MCP-/IPC-Verbraucher und die Kryptoprimitiven sind nicht umfassend geprüft. Für die Desktop-/Android-Sicherheitsgleichheit genügt eine Korrektur ausschließlich am Desktop-Mountgate nicht.

Die sechs Korrekturen und ihre Gegenbeispiele gehen an den Hauptagenten. Dieser Review führt keine Reparaturen und keine neue Testsuite ein. Die konkrete nachfolgende Bewertung muss die geänderten Befundflächen erneut lesen; die vom Hauptagenten besessene eine Remote-Task-Suite kann die sechs beschriebenen Trigger aufnehmen.

`native/src/ftp/core/capacity.rs` existiert nicht; dies wurde dem Hauptagenten gemeldet. `ftp/core/extensions.rs` wurde stattdessen lesend freigegeben. Der fehlende Dateiname ist für sich kein Defekt. Der direkte docs.rs-Abruf der russh-sftp-Quelldatei scheiterte; das versionsgebundene, gegen Cargo.lock geprüfte Cratearchiv schloss diese Quellenlücke.

## Entscheidende Dateihashes

SHA-256 der tatsächlich gelesenen Arbeitskopien; diese Tabelle ersetzt keine Aussage über den vollständigen Worktree.

| Datei | SHA-256 |
|---|---|
| `native/src/app/core/drive_snapshot.rs` | `edd5da8861a20c355a567919059770971625ccfe06fdeb015fcd07e1457ea0ef` |
| `native/src/app/os/linux_os/drive_inventory.rs` | `72ef4277868dbeb371a94430123bd7b6ec53853899578395b8871a08d43ba8ce` |
| `native/src/cloud/core/auth_state.rs` | `2e9d6378975c8854bf2fdf5a39cf88a8e25970193ce982863061f84597499c19` |
| `native/src/cloud/os/coordinator.rs` | `4a66b893dcd7fe090df12aa6459a42208f659b9856b447943b0113445fa620af` |
| `native/src/cloud/os/oauth_http.rs` | `20ed60a1e0a8a3500f1aee291dedbb204d3e396b34b4147684c8c11d210e81e4` |
| `native/src/gdrive/core/auth.rs` | `92ddf3e580d730305f7e0a04eeeb836bf55ec1fd50971b930d1312ba8f07064f` |
| `native/src/gdrive/core/state.rs` | `1fcd02ba593570968201ce56c9133c323a999a632ae4c056b3f2a765c3bcedbb` |
| `native/src/mount/core/recovery_export.rs` | `b632ba8259c928521e2cf97267ac54ee4291a6ea053c1f55038705f5704911cb` |
| `native/src/mount/core/recovery_import.rs` | `0d535d0c0d57fa8767a72427c0f2f2d9f15c4d27b238bf4c6d6b822050fc39ef` |
| `native/src/mount/core/journal_record.rs` | `2c242209c5e7a53e75087174fb45b10d905291863f598751d327fd10ee495cdc` |
| `native/src/mount/core/journal.rs` | `427cd8ed37c45988476d2969e82eee35172d2da36786ab6bc09b99505dbc506d` |
| `native/src/mount/core/file_commit.rs` | `17946e19892278dcf321d0d05304b741323306506d2961ed6a4afcf2081c0374` |
| `native/src/daemon/os/shared/mount_recovery_transaction.rs` | `0d2c7488d7cd4698b45f58dbfc931fa6a80b791eecc884debc529753f920fd8c` |
| `native/src/daemon/os/shared/mount_source.rs` | `a27eb7549a633f4aa7a3a826c0a9d14bfe2049dcf9b24a4c7f3f96c10f950c73` |
| `native/src/vault/os/shared/open_policy.rs` | `e0b142fbfa3b5fe3d6964f60193a8abb25419d50bb4b65e9f9c727be092d1133` |
| `native/src/vault/os/shared/journal_sealer.rs` | `0892e476159f67159cbfa531b058ca34d527ed5dff5f0458139e7bddd6097d6b` |
| `native/src/mobile/os/shared/domains/docs_vault.rs` | `7961351a1b0f1953cd5ef91945029c009db9d322d8fc20a226ec0f4f0f2435c2` |
| `native/src/mobile/os/shared/domains/docs_engines.rs` | `031eb409a6a0f80d3266cf15eb09564368ec9af22636bab65825f09c2a31a777` |
| `native/src/mobile/os/shared/domains/docs_io.rs` | `ca5f51f13842e74a4058d857f01d5cdd4b7a66542435e129bd493135026cf149` |
| `native/src/webdav/core/capacity.rs` | `208f50b7e68f9fb48ae94449c52fefa90c030863e1dbc8d316b9a56281902393` |
| `native/src/webdav/core/multistatus.rs` | `6eed439719aab24faa0276def91dcbea0d40b8dc2d9b244014226ace7414b1a9` |
| `native/src/vfs/core/capacity.rs` | `34ddf07bc93c8700cfdfd9bf215e4774c741cf9502ea4a5e37955d6b8fbab149` |
| `native/src/mount/core/capacity.rs` | `236ef8abe84ab452676cfa9e041750a02081a6d40ebbc55364210e61af75c17c` |

## Lese- und Schreibinventar des Reviewagents

Vollständig gelesen wurden die folgenden Dateien. „Vollständig gelesen“ bedeutet keine vollständige funktionale Abnahme sämtlicher darin enthaltener Bestandsfunktionen.

```text
AGENTS.md
docs/oberflaechen/INDEX.md
native/src/app/core/drive_snapshot.rs
native/src/app/core/mount_recovery_ui.rs
native/src/app/core/vault_open_policy_ui.rs
native/src/app/os/shared/drive_events.rs
native/src/app/os/shared/drive_inventory.rs
native/src/app/os/shared/vault_open.rs
native/src/app/os/linux_os/drive_events.rs
native/src/app/os/linux_os/drive_inventory.rs
native/src/app/os/windows/drive_events.rs
native/src/app/os/windows/drive_inventory.rs
native/src/app/os/windows/watchers.rs
native/src/cli/drive_recovery.rs
native/src/cli/vault/open_policy.rs
native/src/cloud/core/auth_state.rs
native/src/cloud/core/token_persistence.rs
native/src/cloud/os/coordinator.rs
native/src/cloud/os/oauth_flow.rs
native/src/cloud/os/oauth_http.rs
native/src/cloud/os/session_store.rs
native/src/daemon/os/shared/mount_inactive_recovery.rs
native/src/daemon/os/shared/mount_manager_start_cache.rs
native/src/daemon/os/shared/mount_paths.rs
native/src/daemon/os/shared/mount_recovery_transaction.rs
native/src/daemon/os/shared/mount_source.rs
native/src/gdrive/core/auth.rs
native/src/gdrive/core/capacity.rs
native/src/local_access/os/linux/confined.rs
native/src/local_access/os/linux/directory_handle.rs
native/src/local_access/os/windows/confined.rs
native/src/local_access/os/windows/confined_native.rs
native/src/local_access/os/windows/directory_handle.rs
native/src/mobile/os/shared/domains/docs_vault.rs
native/src/mobile/os/shared/domains/docs_engines.rs
native/src/mobile/os/shared/domains/docs_io.rs
native/src/mobile/os/shared/domains/vault_open_policy.rs
native/src/mount/core/capacity.rs
native/src/mount/core/cache_space.rs
native/src/mount/core/file_commit.rs
native/src/mount/core/journal.rs
native/src/mount/core/journal_record.rs
native/src/mount/core/mount_source.rs
native/src/mount/core/recovery.rs
native/src/mount/core/recovery_cache_task_tests.rs
native/src/mount/core/recovery_export.rs
native/src/mount/core/recovery_import.rs
native/src/mount/core/recovery_manifest.rs
native/src/mount/core/stage_ledger.rs
native/src/mount/os/shared/capacity_refresh.rs
native/src/mount/os/shared/host_capacity.rs
native/src/mount/os/shared/recovery_export.rs
native/src/mount/os/shared/recovery_import.rs
native/src/mount/os/linux_os/cache_lease.rs
native/src/mount/os/linux_os/recovery_storage.rs
native/src/mount/os/windows/cache_lease.rs
native/src/mount/os/windows/recovery_storage.rs
native/src/sftp/core/capacity.rs
native/src/smb/core/capacity.rs
native/src/vault/core/capabilities.rs
native/src/vault/core/open_policy.rs
native/src/vault/core/sealed.rs
native/src/vault/os/shared/backend.rs
native/src/vault/os/shared/backend_identity.rs
native/src/vault/os/shared/backend_extensions.rs
native/src/vault/os/shared/journal_sealer.rs
native/src/vault/os/shared/open.rs
native/src/vault/os/shared/open_policy.rs
native/src/vault/os/shared/unlock.rs
native/src/vfs/core/capacity.rs
native/src/vfs/os/shared/confined_local.rs
native/src/vfs/os/shared/confined_local_backend.rs
native/src/vfs/os/linux_os/confined_local.rs
native/src/vfs/os/windows/confined_local.rs
native/src/webdav/core/capacity.rs
native/src/webdav/core/metadata_request.rs
android/app/src/main/java/app/smartexplorer/android/ui/vaults/VaultOpenPolicy.kt
```

Gezielte Auszüge, Suchtreffer oder nicht vollständig ausgegebene Dateien wurden gelesen; für diese Dateien wird keine Volllektüre behauptet:

```text
docs/ARCHITEKTUR.md
docs/plaene/2026-10-24-batches/batches.md
docs/plaene/2026-10-24-batches/vertraege.md
docs/plaene/2026-10-24-batches/recherche.md
docs/plaene/2026-10-24-batches/umsetzung.md
native/Cargo.toml
native/src/daemon/os/shared/mount_manager_start.rs
native/src/daemon/os/shared/mount_manager_host.rs
native/src/daemon/os/shared/rooted_backend.rs
native/src/daemon/os/shared/rooted_backend_gate.rs
native/src/ftp/core/extensions.rs
native/src/gdrive/core/state.rs
native/src/mobile/os/shared/domains/vaults.rs
native/src/mount/core/engine.rs
native/src/sftp/core/extensions.rs
native/src/smb/core/extensions.rs
native/src/vault/os/shared/local_store.rs
native/src/vfs/os/shared/confined_local_extensions.rs
native/src/webdav/core/multistatus.rs
```

`native/Cargo.lock` wurde vollständig maschinell als TOML gelesen; die relevanten Pakete wurden gezielt ausgewertet. Zusätzlich erfolgten begrenzte Symbol- und Dateinamenssuchen innerhalb der ausdrücklich freigegebenen Verzeichnisse `native/src/mount/` und `native/src/vault/`; Trefferlisten wurden nicht als Volllektüre gezählt.

Angewandte Arbeitsweise: `/root/.agents/skills/arbeitsweise/SKILL.md`, `/root/.codex/skills/arbeitsweise/SKILL.md` sowie der delegierte zentrale MCP-Abruf, Revision 337. Der explizit übergebene Hoststart lautet `gpt-6-astra/xhigh`; vererbte Modellkennungen des Arbeitsweise-MCP wurden nicht als tatsächliche Modellausführung gewertet.

**Einzige erstellte/geschriebene Datei:** `docs/plaene/2026-10-24-batches/review-b1.md`. Keine native Quelle verändert. Entscheidungen: sechs belegte Defekte priorisiert; nicht bewiesene Verdachtsmomente verworfen; Code-/APIgrenzen ausdrücklich begrenzt. Offen bleiben die sechs beschriebenen Korrekturen und ihre vom Hauptagenten zu koordinierende Prüfung. Der Reviewauftrag ist mit diesem Bericht abgeschlossen.

</details>

