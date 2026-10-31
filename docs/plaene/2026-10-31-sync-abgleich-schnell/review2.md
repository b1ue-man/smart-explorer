# Zweite Gesamtreview der Umsetzung „Sync-Abgleich schnell“ und „Rechte beim Verbinden“ (2026-10-31)

Gegenstand: `git diff aebf5436..HEAD -- native/ android/ docs/ README.md .github/ se-agent/` bei HEAD `7bf028a6` (32 Commits; 371 Dateien). Grundlage: Plan `plan.md` Abschnitte A, E, F (Nutzervorgaben N1–N6), erste Review `review.md`, `AGENTS.md`. Nichts wurde gebaut, kompiliert oder ausgeführt (Projektregel); alle Aussagen stammen aus dem Lesen des Quelltexts. Abkürzung: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`.

Zählung: **3 blockierend, 10 wesentlich, 8 gering.**

## Kompilierbarkeit nach Lesen

Geprüft ohne Befund (ohne Compiler, daher keine vollständige Typprüfung):
- Jede `#[path = "…"]`-Angabe in den 371 geänderten Dateien zeigt auf eine vorhandene Datei (Pfad relativ zum Ordner der deklarierenden Datei); jede neue `.rs`-Datei ist als Modul eingebunden.
- `FsRequest::WriteNew` (neues Feld `mtime_ms`): alle Literale setzen es (`N/share/core/server_fs.rs:215`, `N/share/core/peer_stages.rs:318` und die drei Testdateien). `FsHashWalk` (neue Felder `keys`, `known`, `keys_only`, `folders`): `N/share/core/peer_hash_walk.rs:119-132` setzt alle, `N/share/core/peer_content_digest.rs:28-35` nutzt `..Default::default()` (`Default` ist abgeleitet, `N/share/core/fs_request.rs:88`).
- Neue Variante `DiscoveryEvent::OfferRights`: beide erschöpfenden Matches behandeln sie (`N/share/core/discovery_offer_book.rs:63`, `N/share/os/shared/discovery_events.rs:316`); die übrigen Fundstellen konstruieren nur.
- Neue Varianten `FsHashWalkMessage::Run/Key`: die Matches der Clients haben einen Rest-Arm (`N/share/core/peer_content_digest.rs`, `N/share/core/content_digest_task_tests.rs:44`).
- Neue `AccessIpcRequest`-Varianten: `N/daemon/os/shared/access_ipc.rs:112` leitet alle übrigen an `rights_reply` weiter, das unbekannte ablehnt.
- Agent-Rahmenkennungen 85–90 (`N/agent_proto/core/walk_key_frames.rs:137-255`) kollidieren mit keiner vorhandenen Kennung (belegt: bis 84 in `folder_frames.rs`/`walk_meta_frames.rs`, 81 frei).
- Kotlin gegen Bridge: die Feldnamen `folders.kind/paths/connections`, `write`, `commands`, `terminal`, `screenView`, `screenControl`, `validUntil`, `account`, `target.kind/deviceId/publicKey/profileId`, `preset`, `rights`, `clear` in `A/api/ShareRightsApi.kt:19-45,118-123,186-241` stimmen mit `N/mobile/os/shared/domains/share_rights.rs:37-188` überein; die Methoden `share.rightsPresets/pendingRights/confirmRights/cancelRights/setCodeRights` sind registriert (`N/mobile/os/shared/domains/mod.rs:307-311`).

Befund dazu: B1 (eigenständiges `se-agent`-Crate baut nicht).

## Blockierend

### B1 — `se-agent` baut nicht: `agent_proto` greift auf `crate::vfs::KeyTally` und `crate::vfs::name_is_unusual` zu, die das Agent-Crate nicht hat (Kompilierfehler)

- Stellen: `N/agent_proto/core/folder_key.rs:40` (`use crate::vfs::KeyTally;`), `:266` (`crate::vfs::name_is_unusual`), `N/agent_proto/core/folder_key_tree.rs:12`, `N/agent_proto/core/walk_key_frames.rs:13`, `N/agent_proto/core/types.rs:333` (`tally: Option<crate::vfs::KeyTally>` im `Frame`), `N/agent_proto/os/shared/walk_keyed.rs:22`. Das Modul `folder_key` ist ohne `cfg` eingebunden (`N/agent_proto/mod.rs:35-36`).
- Das eigenständige Crate bindet `agent_proto` per Pfad ein (`se-agent/src/main.rs:5-6`) und stellt unter `crate::vfs` nur `fit_stage_name` und `is_unique_stage` bereit (`se-agent/src/vfs.rs:4-7`). `KeyTally` liegt in `N/vfs/core/key_tally.rs` und braucht `serde` (`:5`, `:16`), das `se-agent/Cargo.toml:19-26` nicht als Abhängigkeit führt; `name_is_unusual` braucht `crate::apptrash::TRASH_DIR_NAME` (`key_tally.rs:61`), das es im Agent-Crate nicht gibt.
- Fehlerfall: `cargo build --manifest-path se-agent/Cargo.toml` endet mit „unresolved import `crate::vfs::KeyTally`“ und „cannot find function `name_is_unusual` in module `crate::vfs`“. Damit scheitern der Suite-Schritt „Linux builds the embedded se-agent bundles from se-agent/ first“ (`.github/workflows/sync-abgleich-task.yml:14-15`), das Clippy-Gate `.github/workflows/build.yml:486` und der Release-Build der Agent-Payloads (`build.yml:661-662`).
- Abweichung: Projektgedächtnis „se-agent must keep building (suites never build it)“; `se-agent/src/main.rs:7-8` („without it this crate does not build“).
- Behebung: `KeyTally` (ohne `serde`-Ableitung im Agent-Teil, z. B. `serde` nur über einen Newtype auf App-Seite) und `name_is_unusual` (mit eigenem Namen des Papierkorbordners) nach `agent_proto` verlegen oder `se-agent/src/vfs.rs` um beide ergänzen, ohne neue Abhängigkeit.

### B2 — Shell-Injektion im entfernten Hash-Befehl, wenn die Login-Shell des SFTP-Kontos `fish` ist

- Stellen: `N/sftp/core/digest_command.rs:113-116` (`sh_quote`: `'` → `'\''`), `:125-131` (`sh -c '<Skript>' sh '<Pfad>'`); die Begründung `:46-47` („no login shell (sh, bash, zsh, fish, csh) reads anything into it“) und der Modulkopf `:10-13` behaupten Sicherheit für alle Login-Shells. sshd führt den Exec-Text mit der Login-Shell des Kontos aus (`$SHELL -c "<Text>"`).
- In `fish` sind innerhalb einfacher Anführungszeichen `\'` und `\\` Escape-Folgen. Ein Dateiname mit Backslash vor einem Apostroph bricht die Quotierung: Name `a\'; id >/tmp/x; #` wird zu `'a\'\''; id >/tmp/x; #'`; `fish` liest `'a` + `\'` (Zeichen `'`) + `\'` (Zeichen `'`) + `'` (Ende), danach steht `; id >/tmp/x; #'` ohne Quotierung, und `id >/tmp/x` läuft als das SSH-Konto. In POSIX-`sh` ergibt derselbe Text korrekt den Namen.
- Wer den Namen bestimmt: jeder, der in einem Baum, den der Sync im Prüfsummen-Modus vergleicht (`N/bisync/os/shared/snapshot_dir.rs:301`), den eine Kopie überprüft (`N/transfer/os/shared/engine/file_evidence.rs:157`) oder den ein Share-Host über eine freigegebene SFTP-Verbindung hasht (`N/share/os/shared/host_hash_file.rs:52`), eine Datei anlegen kann (geteilter Upload-Ordner, entpacktes Archiv, Gegenseite eines Syncs). Ein `\` in Pfaden wird nicht abgewiesen (`sendable`, `:134-136`, prüft nur NUL, CR, LF).
- Abweichung: Prüfauftrag „Shell-Injektion in entfernten Hash-Befehlen“; AGENTS.md „Treat recoverable failures as Result“ setzt voraus, dass ein fremder Name nie Befehle ausführt.
- Behebung: den Pfad nicht durch die Login-Shell schicken (z. B. fester Befehl `sh -c 'IFS= read -r f; …'` und den Pfad über stdin des Kanals senden, danach EOF), oder jeden Pfad mit `\` oder `'` ablehnen (`Ok(None)`, Übertragung wie bisher).

### B3 — Windows-OpenSSH-Server (Standard-Shell `cmd.exe`): jede Prüfsumme über SFTP endet mit einem Fehler statt mit dem bisherigen Download

- Stellen: Skript `N/sftp/core/digest_command.rs:48-52` und `:55-58`, Exit-Code 3 → `Reply::NotRegular` (`:32`, `:169`) → `Err(changed())` (`:259`, `:271-276`).
- Fehlerfall: `cmd.exe /c` zerlegt den Text an `||`: `sh -c 'f=$1; [ -f "$f" ]` scheitert (kein `sh` im Pfad: Fehlerstufe 9009; mit Git-`sh` im Pfad endet `[ -f "" ]` mit 1), danach läuft `exit 3; …` und beendet die Shell mit 3. `hash_by` meldet „file changed while its content digest was computed“. Die Aufrufer reichen den Fehler mit `?` weiter: im Sync mit Prüfsummen-Modus scheitert jede Datei der SFTP-Seite (`N/bisync/os/shared/snapshot_dir.rs:301`), das Überprüfen nach Kopien scheitert (`N/transfer/os/shared/engine/file_evidence.rs:157`), ein Share-Host mit freigegebener Windows-SFTP-Verbindung meldet Fehler (`N/share/os/shared/host_hash_file.rs:52`). In v0.5.189 hatte SFTP keinen `content_digest`/`content_md5` (neu in `N/sftp/core/extensions.rs:149-165`), der Inhalt wurde gelesen und gehasht.
- Abweichung: gebrochene Abwärtsverträglichkeit mit Windows-SFTP-Gegenstellen; AGENTS.md „preserve existing behavior“ (Prüfsummen-Sync und Überprüfen funktionierten).
- Behebung: Exit-Code 3 nur als „keine reguläre Datei“ werten, wenn die Ausgabe ein eindeutiges Kennzeichen des Skripts trägt (z. B. das Skript schreibt vor `exit 3` eine feste Marke auf stdout); jede andere Antwort ist `Reply::Unavailable` (Gate schließen, Download wie bisher).

## Wesentlich

### W1 — Hash-Cache: auf FAT/exFAT unter Linux kann ein Eintrag einer anderen Datei gelten; der Prüfsummen-Modus liest nicht mehr frisch

- Stellen: Schlüssel `(dev, ino, Algorithmus)` und Gültigkeit „Stempel gleich“ `N/agent_proto/os/shared/hash_cache.rs:136-148`; Stempel `N/agent_proto/os/linux_os/local_platform.rs:324-335` mit der Begründung „Every write changes the change time, and no call sets it back“; Modulkopf `hash_cache.rs:15-16` („a file without a stamp (a file system without file IDs) is not cached“).
- Fehlerfall 1: Der Linux-FAT-Treiber vergibt Inode-Nummern bei jedem Einlesen neu (`iunique`), nicht dauerhaft; `ctime` stammt dort aus den Feldern des Verzeichniseintrags (Änderungs- bzw. Erstellzeit), die ein Programm setzen kann, und `mtime` hat 2 s Auflösung. Nach erneutem Einhängen erhält Datei X die Inode-Nummer, unter der Datei Y gespeichert ist; haben X und Y dieselbe Größe und dieselbe 2-s-Zeit (z. B. Teile einer `split`-Ausgabe auf einem USB-Stick), liefert `digest` die MD5 von Y für X. Linux gibt immer einen Stempel (`change_stamp` liefert `Some`), die Ausnahme des Modulkopfs greift also nie.
- Fehlerfall 2: Der Prüfsummen-Modus ist laut `N/bisync/os/shared/snapshot_dir.rs:239-240` der Weg, der „a fresh hash“ verlangt; `hash_file_cached` (`N/bisync/os/shared/snapshot_hash.rs:40-55`), der Agent (`N/agent_proto/os/shared/walk_hashed.rs:452`) und der Share-Host (`N/share/os/shared/host_hash_walk_local.rs:207`) nehmen jetzt den gespeicherten Wert, solange der Stempel gleich ist. Eine Inhaltsänderung mit gleichem Stempel (Fehlerfall 1, still beschädigte Blöcke, eine Datei auf FAT mit gesetzter Zeit) erkennt der Prüfsummen-Modus damit nicht mehr. Keine Oberfläche (CLI, Desktop, Android, MCP) kann den Cache leeren oder für einen Lauf abschalten (Oberflächen-Dokument `docs/oberflaechen/uebertragungen.md`, `docs/oberflaechen/einstellungen.md` nennen ihn nicht).
- Abweichung: Prüfauftrag „falsches Vergleichsergebnis durch Hash-Cache“; Plan E8 verlangte die Racy-Regel nach Git, Git schließt Dateisysteme ohne stabile Inode-Nummern nicht stillschweigend ein; Oberflächenregel „all surfaces“.
- Behebung: keinen Stempel für Dateisysteme ohne dauerhafte Datei-ID liefern (`statfs`-Typ `MSDOS_SUPER_MAGIC`, `EXFAT_SUPER_MAGIC`, FUSE-Typen ohne `use_ino`); im Prüfsummen-Modus eine Option „frisch lesen“ auf allen Oberflächen, die den Cache für den Lauf umgeht.

### W2 — Der Hash-Cache wird nach jeder einzelnen Datei-Prüfsumme ganz neu geschrieben

- Stellen: `N/agent_proto/os/shared/ext_ops.rs:193-205` (`FILE_DIGEST` ruft nach jeder Datei `hash_cache::flush()`), `N/agent_proto/os/shared/hash_cache.rs:176-193` (Sortierung aller Einträge, dann `save`), `:247-273` (schreibt die ganze Datei); der globale Mutex bleibt während `save` gehalten (`:177`), alle Hash-Fäden des Prozesses warten so lange.
- Fehlerfall: Nach einem großen Durchlauf hält der Cache bis 500 000 Einträge zu je 130 Byte (1 + 5·16 + 16 + 1 + 32), also rund 65 MB. Das Überprüfen nach Kopien über einen se-agent (`content_digest` je Datei) fügt je Datei einen Eintrag hinzu und schreibt danach die 65 MB neu: 10 000 überprüfte Dateien schreiben rund 650 GB auf den Server.
- Abweichung: Nutzervorgabe N5 „so schnell wie möglich heißt so wenig wie möglich“.
- Behebung: `flush` nur am Ende einer Anfragefolge bzw. zeitgesteuert (z. B. höchstens alle 30 s) und außerhalb des Mutex schreiben (Kopie der Einträge unter dem Mutex, Schreiben danach).

### W3 — SFTP-Konten mit `ForceCommand internal-sftp`: jede der drei ersten Prüfsummen wartet mindestens 60 s

- Stellen: `N/sftp/core/digest_exec.rs:30-31` (Exec ohne `channel.eof()`), `:55-60` (Warten bis `wait`), `N/sftp/core/digest_command.rs:39`, `:42-44`, `:138-141` (`BASE_WAIT` 60 s plus Größe/8 MiB/s), `:243-247` (Zeitablauf zählt nur als Fehlversuch, Aufgabe erst nach drei).
- Fehlerfall: Bei SFTP-Konten mit erzwungenem `internal-sftp` (übliche chroot-Konten von NAS und Hostern) startet sshd für die Exec-Anfrage `internal-sftp`, das auf das SFTP-Init vom Kanal wartet; der Client sendet weder Daten noch EOF. Jede Prüfsumme wartet 60 s plus 1 s je 8 MiB Dateigröße (eine 10-GB-Datei: über 20 min), drei Versuche je Verbindung, bevor der Download wie in v0.5.189 beginnt. Der Gate ist je Verbindung (`digest_gate`, `md5_gate`), jede neue Verbindung wiederholt das.
- Abweichung: N5; gebrochenes Zeitverhalten gegenüber v0.5.189 bei verbreiteten Servern.
- Behebung: nach `request_exec` sofort `eof()` senden; eine leere Ausgabe mit Status 0 ergibt dann `Reply::Unavailable` (`digest_command.rs:189-193`) und schließt den Gate beim ersten Versuch.

### W4 — WebDAV: im Prüfsummen-Modus eine zusätzliche PROPFIND je Datei, die nie eine MD5 liefern kann

- Stellen: `N/bisync/os/shared/snapshot_dir.rs:244-283` (eine in der Auflistung mitgelieferte MD5 wird zuerst genommen, `content_hash` läuft nur ohne sie), `:296-303` (`content_md5` vor jedem Download), `N/webdav/core/content_digest.rs:91-101`, `N/webdav/core/extensions.rs:67-74`.
- Fehlerfall: Die Auflistung liest dieselbe Eigenschaft `oc:checksums` (`N/webdav/core/multistatus.rs:270-277`). Fehlt dort die MD5, fehlt sie auch in der Depth-0-PROPFIND derselben Datei; auf jedem WebDAV-Server ohne `oc:checksums` (Apache `mod_dav`, nginx, IIS) kostet jede Datei im Prüfsummen-Modus eine Anfrage mehr als in v0.5.189, ohne je eine Übertragung zu sparen.
- Abweichung: N5, AGENTS.md „Every … must answer a specific question“ sinngemäß für Netzanfragen; „verbleibende Arbeit ohne Bedarf“ des Prüfauftrags.
- Behebung: `content_md5` für WebDAV nur aufrufen, wenn die Auflistung `oc:checksums` überhaupt geführt hat (Merkmal der Verbindung), sonst `Ok(None)` ohne Anfrage.

### W5 — Beschleunigte Wege tun Arbeit ohne Bedarf: ausgeschlossene Dateien werden auf dem Host gehasht, lokale Seiten zweimal gelesen

- Stellen: `HashWalkRequest` trägt keine Filter (`N/vfs/core/extension_types.rs:232-245`); Filter, versteckte Dateien und Ausschlüsse wirken erst beim Client (`N/bisync/os/shared/snapshot_host_stream.rs`, Plan E11 „Filter … wirken weiter auf der Client-Seite“); der Listing-Weg schloss vor dem Hashen aus (`N/bisync/os/shared/snapshot_dir.rs:132`). Paarphase: `provide_local` (`N/bisync/os/shared/snapshot_pair_side.rs:165-196`) liest den lokalen Baum mit dem Agent-Durchlauf ein zweites Mal neben dem lokalen Schnappschuss-Durchlauf, im Prüfsummen-Modus mit MD5 jeder lokalen Datei (`md5 = true`).
- Fehlerfall: Ein Sync im Prüfsummen-Modus mit Ausschluss `*.vmdk` hasht auf dem Share-Host bzw. se-agent jede ausgeschlossene VM-Datei vollständig (Phase 1 `KeysOnly` braucht die Digests für die Schlüssel, `N/agent_proto/core/folder_key.rs:100-106`), in Phase 2 die gesendeten Ordner erneut (bei Share-Exporten über Verbindungen ohne Cache). Der lokale Partner liest und hasht seinen Baum zweimal.
- Abweichung: N1 „senden nur was muss“, N5.
- Behebung: Filter (Muster, versteckt, Mindestgröße) in die Anfrage aufnehmen und im Schlüssel festhalten (die Schlüssel bleiben gleichheitsrichtig, wenn der Filter Teil der Domäne ist); lokalen Schlüssel-Durchlauf und lokalen Schnappschuss aus einem Durchlauf bilden.

### W6 — Annahme einer neuen Anfrage eines zuvor widerrufenen oder abgelehnten Geräts gibt die alten Rechte ohne Bestätigung zurück

- Stellen: `N/share/core/direct_ledger_projection.rs:86-118` (gleiche Identität: Zustand wird `Accepted`, `write` und die Ordnerauswahl bleiben, `new_connection` bleibt `None`, also kein `settle_new_direct_grant`); Desktop-Annahme ohne Rechte `N/app/core/share_lifecycle_actions_ui.rs:106-113`; CLI `se share request accept` ohne `--preset` (`N/cli/share/requests.rs:268`).
- Fehlerfall: Ein Gerät mit vor F1 gespeicherter Freigabe (`write = true`, kein Eintrag in `device_folders` = alle Ordner, `N/share/core/export_device_folders.rs:36-40`) wird widerrufen (Zustand `Ignored`). Es sendet eine neue Anfrage, der Nutzer klickt „Akzeptieren“: das Gerät liest und schreibt sofort in allen Direkt-Ordnern; es entsteht kein „Rechte offen“ und kein Rechte-Fenster.
- Abweichung: F1 „Jede neue Verbindung (Annahme einer Direkt-Anfrage, …) legt Kontakt und Freigabe ohne jedes Recht an“; N6 „erst nachdem das bestätigt wurde werden berechtigungen gegeben“.
- Behebung: in `project_grant` auch beim Übergang `Ignored → Accepted` (gleiche Identität) `settle_new_direct_grant(…, RightsOrigin::Readmitted, …)` aufrufen.

### W7 — Engere Ordnerauswahl beendet laufende Arbeit nicht, wenn im selben Schritt Befehle/Terminal/Bildschirm enger werden

- Stellen: `N/share/core/relation_rights_diff.rs:66-80` (Reihenfolge der Arme: `ExecRevoked` vor dem neuen F2-Arm), `N/share/core/relation_rights.rs:25-27` (alle `ExecRevoked` werden entfernt und durch reine Fähigkeits-Einschränkungen ersetzt, `affects` übergeht diese, `:216-229`).
- Fehlerfall: „Dateien und Codeausführung“ (alle Ordner, Schreiben, Befehle) wird über „Rechte einstellen“ zu „Dateien lesen und schreiben“ mit nur Ordner A. `write` bleibt, Befehle werden enger → nur `ExecRevoked` → entfernt; die Einschränkung „Ordner B weg“ entsteht nie. Laufende Streams, Leases und Übertragungen des Geräts in Ordner B laufen bis zu ihrem Ende weiter (neue Streams prüft `authorize_state` neu).
- Abweichung: FA3 (`relation_rights.rs:4-7`: eingeschränkte Rechte beenden Sitzungen, Leases, Übertragungen); Prüfauftrag „Gerät erreicht mehr Ordner als bestätigt“.
- Behebung: die Ordnerprüfung je Gerät unabhängig von Schreib- und Exec-Arm ausführen und beide Einschränkungen eintragen.

### W8 — „Bestätigen“ setzt Exec-Rechte am Exec-Grant-Journal vorbei

- Stellen: `N/share/os/shared/connection_rights_actions.rs:44-54`, `N/share/core/connection_rights_apply.rs:240-247` (Exec-Änderung in `mutate_persisted`), Wirksamkeit nur über `reload_now` (`N/daemon/os/shared/access_ipc_rights.rs:49-53`) bzw. `worker_refresh` der CLI (`N/cli/share/grants_rights.rs:266`). Der bisherige Weg für dieselbe Änderung (`MutateGrant`) nimmt `exec_grant_lock`, verweigert bei offenem Journal-Eintrag und führt zweiphasig mit Zustand `PendingDeny` aus (`N/daemon/os/shared/exec_grant_journal.rs:274-316`, `:319-350`).
- Fehlerfall: Der Nutzer setzt über „Rechte einstellen“ ein Gerät von „Alles“ auf „Keine Rechte“. Scheitert das Neuladen (Worker beschäftigt, IPC-Zeitüberschreitung), meldet das Fenster „Rechte gespeichert, Worker nicht neu geladen“, der laufende Worker gewährt Befehle, Terminal und Bildschirm weiter; der Journal-Weg hätte die Sperre als ausstehend festgehalten und wiederholt. Ein gleichzeitiger `MutateGrant` läuft ohne gemeinsame Sperre.
- Abweichung: AGENTS.md „must not … weaken cancellation, permissions“.
- Behebung: Exec-Anteil der Rechte über `mutate_access_grant` (Journal) schreiben oder `confirm_connection_rights` im Daemon unter `exec_grant_lock` mit derselben Ausstehend-Sperre ausführen.

### W9 — Ältere Version liest das Profil mit Ordnern je Gerät als „alle Ordner“

- Stellen: neues Feld `device_folders` (`N/share/core/export_config.rs:114-118`), `pending_rights`, `code_rights` (`N/share/core/profiles.rs:69-77`); die Profilversion blieb gleich (`SHARE_PROFILE_VERSION`, Abgleich `N/share/core/profile_persistence.rs:66-82`), der Test `export_config.rs:266` sichert ausdrücklich, dass eine ältere Fassung die Datei liest.
- Fehlerfall: v0.5.189 (älterer Worker während der Übergabe bei einem Update, Rückkehr zu einer älteren Version) ignoriert `device_folders`; ein Gerät mit „Keine Rechte“ (`FolderChoice::None`) sieht dort alle Direkt-Ordner lesend. Speichert die ältere Fassung, fallen `device_folders`, `pending_rights`, `code_rights` weg; nach erneutem Update sieht das Gerät weiter alle Ordner.
- Abweichung: Prüfauftrag „gebrochene Abwärtsverträglichkeit … gespeicherter Profile“; die erste Review (B2) behandelte denselben Fall für Aufträge.
- Behebung: die Profilversion erhöhen (die vorhandene Prüfung lässt eine ältere Fassung die neuere ablehnen), oder ein Feld wählen, dessen Fehlen in der älteren Fassung weniger Rechte bedeutet.

### W10 — Sommerzeit-Regel nimmt eine echte Änderung um genau eine Stunde als „gleich“, auch wenn die FAT-Seite unverändert ist

- Stellen: `N/bisync/core/compare.rs:30-37` (`across`: Regel, sobald eine Seite FAT ist), `:85-97`; `N/bisync/core/plan_decide.rs:110`, `:244`, `:262`; Zusammenführen `N/transfer/core/update_rule.rs:84-92` (neu in diesem Stapel).
- Fehlerfall Sync: A ist FAT, B NTFS, Basis gleich. B wird genau eine Stunde (± 2 s) später bei gleicher Größe gespeichert (Containerdatei fester Größe). `a_changed = false`, `b_changed = true`; `same_entry(a, b, across)` ergibt „gleich“ → `Decision::Record`, B wird nie nach A kopiert, die Basis hält beide Stände. Eine echte Zeitumstellung verschiebt aber die FAT-Seite, nicht die NTFS-Seite: ist die FAT-Seite gegenüber ihrer eigenen Basis unverändert, kann die Stunde keine Umstellung sein. Für FAT gegen feinere Seiten bestand der Fall schon mit `cross()`; `across` dehnt ihn auf FAT gegen gröbere Seiten aus.
- Fehlerfall Zusammenführen: eine Quelldatei, die genau eine Stunde nach dem Ziel gespeichert wurde, gleiche Größe, wird als „gleich übersprungen“ gezählt statt ersetzt.
- Abweichung: N1 „neuere … ersetzen ältere“; Prüfauftrag „falsches Vergleichsergebnis durch Sommerzeit-Regel“.
- Behebung: die Regel im Sync nur anwenden, wenn die FAT-Seite selbst gegenüber ihrer Basis um die Stunde abweicht oder keine Basis besteht; im Zusammenführen den Fall als Konflikt (beide behalten) statt „gleich“ werten.

## Gering

### G1 — Dateigrößenregel: `N/transfer/os/shared/engine/mod.rs` hat 501 Zeilen
Vorher 499, `+2` in diesem Stapel (Modulzeilen für `update_keys`, `update_listing`). AGENTS.md: neue oder wesentlich geänderte Dateien unter 500 Zeilen; ohne Herauslösen keine neuen Zeilen in übergroßen Dateien. Übrige geänderte Rust-Dateien liegen unter 500 Zeilen und 50 KiB außer `N/share/mod.rs` (587, `+9`, nur Moduldeklarationen, Plan E begründet).

### G2 — Android sagt beim „Ablauf“ nicht, dass Dateien nicht ablaufen
`A/ui/share/RightsSheet.kt:154-176` zeigt nur „Ablauf“; der Desktop ergänzt „Gilt für Befehle, Terminal und Desktop, nicht für Dateien.“ (`N/app/core/share_rights_rows_ui.rs:159-163`). Wer auf Android „1 Stunde“ wählt, behält Lesen und Schreiben ohne Ende (`N/share/core/connection_rights.rs:197-200`).

### G3 — MCP-Anleitung nennt die neuen Rechte nicht als Vertrauensänderung
`N/cli/mcp/catalog.rs:39-41` zählt „access requests, Exec grants and their program lists, device and room settings, discoverability“ auf; `se share grants confirm`, `se share identity --preset`, `se share request policy auto-accept --preset` fehlen in der Aufzählung, auf die `docs/oberflaechen/share-exec.md:260` sich beruft.

### G4 — Bridge übergeht falsch typisierte Rechtefelder stillschweigend
`N/mobile/os/shared/domains/share_rights.rs:127` (`write` nur als Bool), `:75-81` (Nicht-Text-Pfade fallen weg), `:151` (`account` nur als Text). Beispiel: `{"preset":"read-write","rights":{"write":"false"}}` gibt Schreibrecht. Die Kotlin-Seite sendet korrekte Typen; ein anderer Aufrufer der Bridge erhält mehr Rechte als verlangt. Erwartet: `invalid` wie bei `commands` (`:59-66`).

### G5 — „Alle Ordner“ heißt bei Räumen „Kopie von heute“, bei Direkt „auch künftige“
`N/share/core/connection_rights_rooms.rs:55-63` kopiert die Direkt-Ordner beim Bestätigen; `N/share/core/connection_rights.rs:168-170` (Direkt) schließt später hinzugefügte ein. Beide Oberflächen zeigen dieselbe Voreinstellung „Nur lesen: Alle Direkt-Ordner lesen“ (`connection_rights.rs:96-98`).

### G6 — Hash-Cache der App landet unter Umständen im Agent-Ordner
`N/agent_proto/os/shared/hash_cache.rs:106-112` legt beim ersten Zugriff ohne vorheriges `configure_once` den Agent-Ordner `~/.cache/se-agent` fest; in der App ruft der lokale Schlüssel-Durchlauf (`N/agent_proto/os/shared/walk_hashed.rs:452`, über `provide_local`) den Cache auf, bevor `N/bisync/os/shared/snapshot_hash.rs:46` konfiguriert; `configure_once` ändert danach nichts mehr. Plan F4: „App im App-Datenordner“.

### G7 — Neue Ereignisvariante `OfferRights` erreicht eine ältere GUI über IPC
`N/share/core/discovery_signal_types.rs:279-285` wird mit `ui_events` an GUI-Clients weitergereicht (`N/daemon/os/shared/ipc_host_events.rs:56`); eine ältere GUI während der Übergabe kennt die Variante nicht und kann den Ereignisabruf nicht dekodieren, bis sie neu startet. Das IPC-Protokoll hat keine Versionsprüfung.

### G8 — Testspeicher legt beigetretene Räume mit allen Direkt-Ordnern an
`N/share/core/discovery_relation_store.rs:416` (`InMemoryRelationStore::persist_room`: `exports: default_direct_exports.clone()`), der Produktivweg legt sie leer an (`N/share/os/shared/profile_operations.rs:139`). Tests über diesen Speicher prüfen ein anderes Verhalten als das ausgelieferte.

## Geprüft ohne Befund

- Rechte vor Bestätigung: jede Erzeugung eines `DirectGrant` außerhalb von Tests ruft `settle_new_direct_grant` (`N/share/core/direct_ledger_projection.rs:129-144`, `N/share/core/legacy_direct_request_decision.rs:218-231`, Schlüsselwechsel `:204-212`, `N/share/core/direct_relation.rs:381-397`, `N/share/core/direct_reciprocal.rs:405-421`, `N/share/core/profiles.rs:150-167`); der Zugriff auf Dateien nimmt die Ordner je Gerät in der Stream-Prüfung (`N/share/core/session_authorize.rs:137`, `N/share/core/relation_rights.rs:51-56`); Räume entstehen produktiv ohne Freigaben und mit „Rechte offen“ (`N/share/os/shared/profile_operations.rs:131-148`, `N/share/core/profile_persistence.rs:271-291`); PIN-Angebots-Rechte gelten nur für die Austausche dieses Angebots (`N/share/core/discovery_offer_rights.rs`) und gehen bei einem Neustart des Workers verloren (Gerät wartet dann ohne Rechte).
- Ordnerschlüssel (Richtung „gleich ⇒ gleich“): Host und Client kodieren dieselben Felder (`N/agent_proto/os/shared/walk_keyed.rs:116-130`, `N/share/os/shared/host_hash_walk_keys.rs:98-112`, `N/bisync/os/shared/folder_key_cache.rs:65-111`); gesendete Einträge werden vor dem Vergleich auf die Schlüsselfelder verkürzt (`N/bisync/os/shared/snapshot_keyed.rs:166-171`); gespeicherte Datensätze werden beim Laden gegen ihren Schlüssel geprüft (`N/bisync/os/shared/folder_key_file.rs:52`); `system` und Erstellzeit gehen in keine Sync-Entscheidung ein (Systemordner nach Namen, `N/bisync/os/shared/snapshot_policy.rs:171-183`). Beim Zusammenführen bedeutet gleicher Schlüssel gleiche Größe und Zeitabstand 0 (`N/transfer/os/shared/engine/update_keys.rs:10-16`).
- Paarphasen: ein zwischen den Phasen geänderter Ordner ergibt keinen passenden Datensatz, die Seite fällt auf den Durchlauf mit bekannten Schlüsseln zurück (`N/bisync/os/shared/snapshot_pair_side.rs:65-79`, `:136-158`).
- Testnamen: alle neuen Rust-Tests beginnen mit `abgleich_task_` außer zwei verschobenen (`N/transfer/os/shared/engine/batches_backend_webdav_compatibility_task_tests.rs`, Namen aus `batches_backend_compatibility_task_tests.rs` übernommen); alle neuen Kotlin-Testklassen heißen `AbgleichTask…`, ihre Methoden beginnen mit `abgleichTask`.
- Oberflächen der Rechte: `docs/oberflaechen/share-exec.md:233-262` führt CLI, Bridge, IPC, Desktop, Android und MCP für Voreinstellungen, offene Rechte, Bestätigen, Abbrechen, Annahme mit Rechten, Code-, PIN- und Raumrechte und das Ändern bestehender Rechte; der Abgleich mit dem Code ergab keine fehlende Oberfläche außer G2, G3 und der Cache-Steuerung in W1.
