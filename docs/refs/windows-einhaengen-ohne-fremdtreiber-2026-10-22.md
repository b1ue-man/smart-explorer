# Windows-eigene Einhängewege ohne Fremdtreiber (Recherche, Stand 2026-10-22)

Abgelegt für P25 (`docs/plaene/2026-10-22-laufwerke-medien-agenten/machbarkeit.md`, Board `WINDOWS-OHNE-FREMDTREIBER`). Ergänzt die Recherche `docs/refs/windows-dateisystem-frontends-2026-10-17.md`; dort belegte Fakten zu Cloud Files API, ProjFS und WebDAV-Miniredirector sind hier nicht wiederholt.

Marken: Q = `[Quelle: URL, abgerufen 2026-10-22]`, S = `[laut Suchergebnis: URL]`, A = `[Annahme]`, L = `[Lücke: ...]`.
GitHub-Metadaten (SPDX, letzter Push, Sterne) stammen aus `https://api.github.com/repos/<Repo>`, abgerufen 2026-10-22.

## 1. WebClient / WebDAV-Redirector abgekündigt?

| Aussage | Marke |
|---|---|
| Seite "Deprecated features in the Windows client" (ms.date 2026-09-23) führt "Webclient (WebDAV) Service", "Deprecation announced: November 2023". Wortlaut: "The Webclient (WebDAV) service is deprecated. The Webclient service isn't started by default in Windows." | [Quelle: https://learn.microsoft.com/en-us/windows/whats-new/deprecated-features, abgerufen 2026-10-22] |
| Dieselbe Seite nennt für den WebClient keinen Entfernungstermin. Die Seite sagt allgemein: "The features in this article are no longer being actively developed, and might be removed in a future update." | [Quelle: https://learn.microsoft.com/en-us/windows/whats-new/deprecated-features, abgerufen 2026-10-22] |
| Die Seite "Features removed" (ms.date 2026-08-25) enthält keinen Eintrag zu WebClient, WebDAV-Redirector oder NFS-Client. Es ist also nicht entfernt. | [Quelle: https://learn.microsoft.com/en-us/windows/whats-new/removed-features, abgerufen 2026-10-22] |
| Windows Server 2025: "The WebDAV Redirector service is deprecated. The service isn't installed by default in Windows Server." | [Quelle: https://learn.microsoft.com/en-us/windows-server/get-started/removed-deprecated-features-windows-server-2025, abgerufen 2026-10-22] |
| Microsoft-Mitarbeiter (März 2024): Dienst "no longer started by default", "remain in Windows for the foreseeable future"; keine offizielle Frist. | [laut Suchergebnis: https://learn.microsoft.com/en-us/answers/a/2020009] |
| Starttyp "Manual (Trigger Start)" unter Windows 11 | [laut Suchergebnis: Forenbeitrag, über https://malwaretips.com/threads/hackers-abuse-windows-file-explorer-and-webdav-for-stealthy-malware-delivery.139967/post-1173162] |
| Ob Windows 11 24H2 und 25H2 den Dienst noch ausliefern: keine Microsoft-Aussage gefunden, die es für diese Builds bestätigt oder verneint. Dass er nicht in der Entfernt-Liste steht, spricht für "vorhanden". | [Lücke: Primärbeleg für das Vorhandensein in 24H2 und 25H2 sowie für den Starttyp fehlt; nur durch Test auf 24H2 und 25H2 zu schließen] |
| Entfernungsplan | [Lücke: Microsoft nennt weder Version noch Datum] |
| rclone-Anleitung: Basic-Auth nur über HTTPS; Ausnahme per Registry `HKLM\SYSTEM\CurrentControlSet\Services\WebClient\Parameters\BasicAuthLevel` = 2; Dateigrößenlimit `FileSizeLimitInBytes` erhöhen, danach WebClient neu starten. HKLM-Änderung und Dienstneustart brauchen Administratorrechte. | [Quelle: https://rclone.org/commands/rclone_serve_webdav/, abgerufen 2026-10-22] für Registry und Limit; Administratorbedarf: [Annahme] |

Folgerung: WebDAV-Redirector ist als Hauptweg riskant (abgekündigt, keine Frist), als Rückfall brauchbar (so macht es Cryptomator, siehe Frage 6). [Annahme]

## 2. SMB-Client auf anderem TCP-Port

| Aussage | Marke |
|---|---|
| Der SMB-Client kann sich mit alternativen TCP-, QUIC- und RDMA-Ports verbinden, wenn der Server dort lauscht. Voraussetzung: Windows 11 24H2 oder neuer, Windows Server 2025 oder neuer. | [Quelle: https://learn.microsoft.com/en-us/windows-server/storage/file-server/smb-ports, abgerufen 2026-10-22] |
| Befehle: `NET USE <Laufwerk>: \\server\share /TCPPORT:<Port>` und `New-SmbMapping -LocalPath <Laufwerk>: -RemotePath \\server\share -TcpPort <Port>`. Der Port ist nicht Teil des UNC-Pfads. | [Quelle: https://learn.microsoft.com/en-us/windows-server/storage/file-server/smb-ports, abgerufen 2026-10-22] |
| Laufwerksbuchstabe: ja, der Syntaxparameter heißt `<drive letter>`. | [Quelle: https://learn.microsoft.com/en-us/windows-server/storage/file-server/smb-ports, abgerufen 2026-10-22] |
| Rechte laut Microsoft: Prerequisites nennen "Admin permissions", Befehle in einer "elevated" Konsole. | [Quelle: https://learn.microsoft.com/en-us/windows-server/storage/file-server/smb-ports, abgerufen 2026-10-22] |
| Widerspruch zur Rechtefrage: Eine Sicherheitsanalyse (CVE-2026-24294, gepatcht März 2026) beschreibt, ein Nutzer mit niedrigen Rechten könne per `net use \\host\share /tcpport:<port>` eine Verbindung zu einem Server auf hohem Port aufbauen. | [laut Suchergebnis: https://hacktricks.wiki/en/windows-hardening/windows-local-privilege-escalation/local-ntlm-reflection-via-smb-arbitrary-port.html] |
| Ob ein Standardnutzer ohne Administratorrechte mit `/TCPPORT` einhängen darf (Microsoft sagt Admin, Sicherheitsanalyse sagt nein-Admin) | [Lücke: Widerspruch ungeklärt; Test mit Standardnutzer auf Windows 11 24H2 nötig] |
| Ausgaben: Windows 11 24H2 und neuer ohne Editionsangabe (kein Ausschluss von Home genannt), Windows Server 2025. Windows 10 ist ausgeschlossen. | [Quelle: https://learn.microsoft.com/en-us/windows-server/storage/file-server/smb-ports, abgerufen 2026-10-22]; Home-Edition: [Lücke: nicht ausdrücklich bestätigt] |
| Gruppenrichtlinie kann alternative Ports sperren oder auf bestimmte Server beschränken. | [Quelle: https://learn.microsoft.com/en-us/windows-server/storage/file-server/smb-ports, abgerufen 2026-10-22] |
| Loopback: `NET USE F: \\localhost\foo /TCPPORT:1234` wird als Beispiel genannt (Hacker-News-Kommentar, kein Beleg). Die Microsoft-Seite erwähnt Loopback nicht. | [laut Suchergebnis: https://news.ycombinator.com/item?id=38887069] |
| Loopback gegen einen Nutzerraum-Server auf 127.0.0.1 mit Laufwerksbuchstabe | [Lücke: kein Primärbeleg; Test nötig; Risiko: Signierung, NTLM-Anmeldung gegen lokalen Rechner (NTLM Local Authentication), Windows-Firewall, Härtung seit März 2026] |
| SMB-Signierung für ausgehende Verbindungen ist unter Windows 11 24H2 standardmäßig erzwungen. Ein eigener Server muss also signieren. | [laut Suchergebnis: https://hacktricks.wiki/en/windows-hardening/windows-local-privilege-escalation/local-ntlm-reflection-via-smb-arbitrary-port.html] |
| Geteilter Nachteil: nur 24H2 und neuer; Windows 10 und Windows 11 23H2 bleiben außen vor. | [Annahme], abgeleitet aus der Mindestversion oben |

### SMB-Server-Bibliotheken (Referenz)

| Projekt | Sprache | SPDX | Stand (letzter Push) | Hinweis | Marke |
|---|---|---|---|---|---|
| paltaio/rust-smb-server | Rust | MIT | 2026-10-21, 10 Sterne, angelegt 2026-04-27 | einbettbar, SMB 2/3, AES-128/256-CCM/GCM, NTLM, Beispiel lauscht auf 127.0.0.1:4445; keine Aussage zu getesteten Windows-Clients | [Quelle: https://github.com/paltaio/rust-smb-server, abgerufen 2026-10-22] |
| sonroyaalmerol/go-smb-server | Go | MIT | 2026-07-15, 7 Sterne, angelegt 2026-07-13 | "SMB2/3 server library for Go with pluggable auth and VFS" | [Quelle: https://api.github.com/repos/sonroyaalmerol/go-smb-server, abgerufen 2026-10-22] |
| fortra/impacket | Python | NOASSERTION (eigene Apache-Software-License-1.1-Variante) | 2026-10-21, 16191 Sterne | Test- und Angriffswerkzeug mit SMB-Server, keine Einbettungsbibliothek | [Quelle: https://api.github.com/repos/fortra/impacket, abgerufen 2026-10-22]; Lizenzwortlaut: [Lücke: nicht gelesen] |
| SMBLibrary (Fork XIEBBS) | C# | LGPL-3.0 | nicht erhoben | SMB 1/2/2.1, nur C#, hier nicht verwendbar; Hinweis, `\\127.0.0.1` und `\\localhost` funktionierten in Explorer unter Windows 7/8 nicht | [Quelle: https://github.com/XIEBBS/SMBLibrary, abgerufen 2026-10-22] |
| farazshaikh/smb-server-rs | Rust | nicht erhoben | nicht erhoben | Repo-API lieferte 404 | [laut Suchergebnis: https://github.com/farazshaikh/smb-server-rs]; Lizenz und Stand: [Lücke] |
| Samba | C | GPL-3.0-oder-später (nicht erneut geprüft) | nicht erhoben | als Referenz, nicht einbettbar in MIT-Code | [Annahme] |

Alle drei Rust- und Go-Treffer sind jung (angelegt 2026). Ob einer davon einen Windows-24H2-Client bedient, ist nicht belegt. [Lücke: kein Interoperabilitätsbeleg mit Windows-SMB-Client]

## 3. NFS-Client von Windows

| Aussage | Marke |
|---|---|
| Client for NFS: NFSv2 und NFSv3, "Windows client (all currently supported versions)". Server for NFS kann bis NFSv4.1, der Windows-Client nicht. | [Quelle: https://learn.microsoft.com/en-us/windows-server/storage/nfs/nfs-overview, abgerufen 2026-10-22] |
| `mount` ist "available only if Client for NFS is installed". Syntax: `mount -o anon \\ServerIP\ShareName Z:` hängt anonym auf Laufwerk Z: ein. Optionen: `nolock` (Sperren standardmäßig an), `casesensitive`, `fileaccess`, `mtype=soft/hard`, `lang`, `rsize/wsize` (1 bis 32 KB), `sec=sys/krb5/krb5i/krb5p`. Eine Option für einen Port gibt es in dieser Referenz nicht. | [Quelle: https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/mount, abgerufen 2026-10-22] |
| Ausgaben: nicht in Home; Pro, Enterprise, Education vorhanden. Ein Beitrag nennt Pro seit Windows 10 1607. | [laut Suchergebnis: https://techbloat.com/?p=1946520 und https://bandaancha.eu/foros/nfs-windows-10-pro-instala-pero-no-1724046]; Microsoft-Primärbeleg: [Lücke: keine Editionsliste auf learn.microsoft.com gefunden] |
| Optionale Komponente (Windows-Features "Services for NFS" / `ServicesForNFS-ClientOnly`), nicht standardmäßig aktiv; Aktivierung per `Enable-WindowsOptionalFeature` oder Features-Dialog. | [laut Suchergebnis: https://woshub.com/mount-nfs-share-in-windows]; Administratorbedarf: [Annahme] (Aktivieren optionaler Komponenten braucht Administrator) |
| Anonyme Identität: Standard `-2` ohne Active Directory; maschinenweit über Registry `HKLM\SOFTWARE\Microsoft\ClientForNFS\CurrentVersion\Default` (`AnonymousUid`, `AnonymousGid`); Schreiben scheitert, wenn der Server der anonymen Identität kein Schreiben erlaubt. Die Abbildung gilt für alle Nutzer des Rechners. | [laut Suchergebnis: https://www.hpe.com/psnow/resources/ebooks/a00110181en_us_v11/AdministratorGuide/MountingNFSonWindowsClient.html] |
| Groß-/Kleinschreibung: nur durch `-o casesensitive` erzwingbar. Standard ist ohne. | [Quelle: https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/mount, abgerufen 2026-10-22] |
| Sperren: Standard an (NLM), mit `nolock` aus. Weitere Grenzen (Byte-Range-Locks, NFSv4-ACLs: nicht unterstützt) | [laut Suchergebnis: HPE-Seite oben] für NFSv4-ACLs; Sperrverhalten im Detail: [Lücke] |
| Einhängen eines Nutzerraum-NFS-Servers auf 127.0.0.1 mit Laufwerk: Das README von `nfsserve` gibt `mount.exe -o anon,nolock,mtype=soft,fileaccess=6,casesensitive,lang=ansi,rsize=128,wsize=128,timeout=60,retry=2 \\127.0.0.1\ X:` an und sagt, es funktioniere "kinda", nur Windows 11 Pro, mit viel Rauschen wegen nicht umgesetzter Aufrufe. | [Quelle: https://github.com/xetdata/nfsserve, abgerufen 2026-10-22] (leitet auf huggingface/nfsserve um) |
| Ob mount.exe einen Server auf einem Nicht-Standardport (nicht 2049 und Portmapper 111) erreichen kann | [Lücke: Mount-Referenz kennt keine Portoption; wie der Windows-Client ein Beispiel mit Port 11111 erreicht, ist nicht belegt; ob ein Nutzerraum-Server die Ports 111 und 2049 auf 127.0.0.1 frei belegen kann, ist ungeprüft] |
| Rechte zum Einhängen eines Laufwerks mit `mount` | [Lücke: nicht belegt] |

### NFS-Server-Bibliotheken

| Projekt | Sprache | SPDX | Stand (letzter Push) | Hinweis | Marke |
|---|---|---|---|---|---|
| huggingface/nfsserve (früher xetdata/nfsserve) | Rust | BSD-3-Clause | 2026-10-20, 768 Sterne, angelegt 2023-06-05 | NFSv3; READDIR fehlt (Windows probiert zuerst READDIR), READDIRPLUS da; Hugging Face nutzt es in `hf-mount` für dessen Einhängefunktion. xet-core ist im README nur als Verweis genannt, nicht als Nutzer. | [Quelle: https://api.github.com/repos/xetdata/nfsserve, abgerufen 2026-10-22] und [Quelle: https://github.com/xetdata/nfsserve, abgerufen 2026-10-22] |
| Vaiz/nfs3 (Crate `nfs3_server` 0.11.1) | Rust | GitHub: Unlicense; docs.rs: BSD-3-Clause (Widerspruch) | 2026-10-21, 25 Sterne | Fork von nfsserve; Windows-Beispiel mit 127.0.0.1 auf X:; Beispiele nutzen Port 11111; Portmapper nur GETPORT | [Quelle: https://api.github.com/repos/Vaiz/nfs3, abgerufen 2026-10-22] und [Quelle: https://docs.rs/nfs3_server, abgerufen 2026-10-22]; maßgebliche Lizenz: [Lücke: Unlicense gegen BSD-3-Clause; Lizenzdatei prüfen] |
| willscott/go-nfs | Go | Apache-2.0 | 2026-09-26, 780 Sterne | NFSv3, "minimally tested"; README ohne Windows-Hinweise | [Quelle: https://api.github.com/repos/willscott/go-nfs, abgerufen 2026-10-22] und [Quelle: https://github.com/willscott/go-nfs, abgerufen 2026-10-22] |
| fernfs | Rust | nicht erhoben | nicht erhoben | "full NFSv3 protocol implementation" | [laut Suchergebnis: https://docs.rs/fernfs]; [Lücke: Lizenz, Stand] |

## 4. Cloud Files API plus Laufwerksbuchstabe (subst, DefineDosDevice)

| Aussage | Marke |
|---|---|
| `CfRegisterSyncRoot`: ab Windows 10 1709; keine Syntaxregel zu subst oder Laufwerken auf der Seite; Überlappung von Sync-Roots verboten; Zugriff WRITE_DATA oder WRITE_DAC auf den Sync-Root nötig. | [Quelle: https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfregistersyncroot, abgerufen 2026-10-22] |
| Laufwerkswurzel als Sync-Root scheitert: Nextcloud und ownCloud melden "The Virtual filesystem feature does not support a drive as sync root"; Abhilfe: Unterordner. Eine subst-Wurzel ist eine Laufwerkswurzel. | [laut Suchergebnis: https://github.com/nextcloud/desktop/issues/7658 und https://github.com/owncloud/client/issues/8949] |
| Zugeordnetes Netzlaufwerk als Sync-Root: Fehler "Couldn't convert to placeholder". | [laut Suchergebnis: https://help.nextcloud.com/t/error-synchronization-windows-desktop-client-with-a-network-drive/184034] |
| Ob eine registrierte Sync-Root-Ordnerstruktur zusätzlich über `subst X: C:\Pfad\SyncRoot` als Laufwerksbuchstabe sichtbar bleibt (Platzhalter, Hydration) | [Lücke: weder Microsoft noch OneDrive, Nextcloud, ownCloud dokumentieren subst mit Platzhaltern; Treffer ohne subst-Erwähnung] |
| Technische Vermutung: Der Minifilter `cldflt.sys` hängt am NTFS-Volume, `subst` und `DefineDosDevice` sind nur Namensumleitungen; ein Zugriff über `X:\` löst auf dasselbe Volume auf. Platzhalter sollten dort sichtbar sein. Risiko: Programme, die den Pfad umwandeln oder `GetFinalPathName` nutzen, sehen den Zielpfad. | [Annahme] |

Folgerung: Laufwerksbuchstabe für Cloud-Files-Platzhalter ist unbelegt. Sicher ist nur: Sync-Root in einem Unterordner eines NTFS-Volumes. [Annahme]

## 5. Saubere Referenzen je Weg

| Weg | Projekt (Repository) | SPDX | Letzter Stand | Hinweis | Marke |
|---|---|---|---|---|---|
| Cloud Files | microsoft/Windows-classic-samples, Ordner `Samples/CloudMirror` | MIT (LICENSE-Datei; API meldet NOASSERTION, "Portions ... SIL Open Font License") | Push 2026-09-03, 5687 Sterne | lokaler Ordner als "Cloud", Sync-Root, Platzhalter, Hydration; README: "early preview"; Windows 10 1809, SDK 10.0.22598 | [Quelle: https://raw.githubusercontent.com/microsoft/Windows-classic-samples/main/LICENSE, abgerufen 2026-10-22] und [Quelle: https://github.com/microsoft/Windows-classic-samples/tree/main/Samples/CloudMirror, abgerufen 2026-10-22] |
| Cloud Files (Rust) | ho-229/cloud-filter-rs (Crate `cloud-filter` 0.0.6) | MIT | Push 2025-11-02, 8 Sterne | Rust-Hülle, Crate-Update 2025-09-29, 27 300 Downloads | [Quelle: https://api.github.com/repos/ho-229/cloud-filter-rs, abgerufen 2026-10-22] und [Quelle: https://crates.io/api/v1/crates/cloud-filter, abgerufen 2026-10-22] |
| ProjFS | microsoft/Windows-classic-samples, Ordner `Samples/ProjectedFileSystem` = **RegFS** (C++, liest Registry schreibgeschützt; Windows 10 1809, Feature `Client-ProjFS` mit erhöhter PowerShell) | MIT | Push 2026-09-03 (Repository) | **RegFS liegt nicht in ProjFS-Managed-API**, sondern in den classic-samples | [Quelle: https://github.com/microsoft/Windows-classic-samples/tree/main/Samples/ProjectedFileSystem, abgerufen 2026-10-22] |
| ProjFS (C#) | microsoft/ProjFS-Managed-API | MIT (LICENSE-Datei; API: NOASSERTION) | Push 2026-03-31, 174 Sterne | NuGet `Microsoft.Windows.ProjFS` 2.1.0; Beispiel `simpleProviderManaged`; .NET 8/9/10 | [Quelle: https://github.com/microsoft/ProjFS-Managed-API, abgerufen 2026-10-22] und [Quelle: https://raw.githubusercontent.com/microsoft/ProjFS-Managed-API/main/LICENSE, abgerufen 2026-10-22] |
| ProjFS (Praxis) | microsoft/VFSForGit | MIT | Push 2026-10-20, 6146 Sterne | großer produktiver ProjFS-Einsatz | [Quelle: https://api.github.com/repos/microsoft/VFSForGit, abgerufen 2026-10-22] |
| ProjFS (Rust) | clouds56/projfs-rs (`projfs` 0.1.2, MIT, zuletzt 2020-10-03); WolverinDEV/windows-projfs (`windows-projfs` 0.1.8, GPL-2.0, zuletzt 2026-06-08) | MIT / GPL-2.0 | siehe links | `projfs` veraltet, `windows-projfs` nicht MIT-kompatibel | [Quelle: https://crates.io/api/v1/crates/projfs, abgerufen 2026-10-22] und [Quelle: https://crates.io/api/v1/crates/windows-projfs, abgerufen 2026-10-22] |
| WebDAV-Server | messense/dav-server-rs (Crate `dav-server` 0.11.0) | Apache-2.0 laut crates.io; GitHub-API: kein Lizenzfeld | Push 2026-10-12, 243 Sterne, Crate-Update 2026-02-21, 672 392 Downloads | Fork von webdav-handler | [Quelle: https://crates.io/api/v1/crates/dav-server, abgerufen 2026-10-22] und [Quelle: https://api.github.com/repos/messense/dav-server-rs, abgerufen 2026-10-22]; Lizenzdatei im Repository: [Lücke: nicht gelesen] |
| WebDAV-Server mit Windows-Anleitung | rclone/rclone, `rclone serve webdav` | MIT | Push 2026-10-22, 60 471 Sterne | Anleitung nennt Basic-Auth nur über HTTPS, `BasicAuthLevel`, `FileSizeLimitInBytes`, WebClient-Neustart; Standard: nur localhost, ohne Anmeldung | [Quelle: https://rclone.org/commands/rclone_serve_webdav/, abgerufen 2026-10-22] und [Quelle: https://api.github.com/repos/rclone/rclone, abgerufen 2026-10-22] |
| NFS | huggingface/nfsserve (BSD-3-Clause), Vaiz/nfs3, willscott/go-nfs (Apache-2.0) | siehe Frage 3 | siehe Frage 3 | | siehe Frage 3 |
| SMB | paltaio/rust-smb-server (MIT), sonroyaalmerol/go-smb-server (MIT) | siehe Frage 2 | siehe Frage 2 | | siehe Frage 2 |

## 6. Cryptomators Prinzip der Einhängewege

| Aussage | Marke |
|---|---|
| `integrations-api` (SPDX laut GitHub-API: AGPL-3.0, Push 2026-10-13, 16 Sterne) lädt Implementierungen zur Laufzeit per Java-ServiceLoader: Klasse registrieren, Name in `META-INF/services/` eintragen, Jar auf den Klassenpfad legen. | [Quelle: https://github.com/cryptomator/integrations-api, abgerufen 2026-10-22] und [Quelle: https://api.github.com/repos/cryptomator/integrations-api, abgerufen 2026-10-22]; Lizenz-Wortlaut der Datei: [Lücke: LICENSE.txt lieferte 404] |
| Schnittstelle `MountService`: statisches `get()` lädt alle Anbieter und behält die, deren `isSupported()` wahr ist (schnell, konstante Zeit); `displayName()`; `capabilities()` mit `hasCapability()`; `forFileSystem(Path)` liefert einen `MountBuilder`; `getDefaultMountFlags()` nur mit `MOUNT_FLAGS`, `getDefaultLoopbackPort()` nur mit `LOOPBACK_PORT`, sonst `UnsupportedOperationException`. | [Quelle: https://raw.githubusercontent.com/cryptomator/integrations-api/develop/src/main/java/org/cryptomator/integrations/mount/MountService.java, abgerufen 2026-10-22] |
| Fähigkeiten (`MountCapability`, 13 Werte): FILE_SYSTEM_NAME, LOOPBACK_HOST_NAME, LOOPBACK_PORT, MOUNT_FLAGS, MOUNT_TO_EXISTING_DIR, MOUNT_WITHIN_EXISTING_PARENT (beide gegenseitig ausschließend), MOUNT_AS_DRIVE_LETTER, MOUNT_TO_SYSTEM_CHOSEN_PATH, READ_ONLY, UNMOUNT_FORCED, VOLUME_ID, VOLUME_NAME, WHEN_TERMINATED. | [Quelle: https://raw.githubusercontent.com/cryptomator/integrations-api/develop/src/main/java/org/cryptomator/integrations/mount/MountCapability.java, abgerufen 2026-10-22] |
| Auswahl in der Anwendung (`Mounter`): zuerst der pro Tresor bevorzugte Dienst nach Name; sonst der anwendungsweite Standarddienst; unverträgliche Dienste (z. B. zwei macOS-FUSE-Varianten) werden abgelehnt. Der Einhängepunkt richtet sich nach den Fähigkeiten: ohne Nutzerwahl systemgewählter Pfad, erster freier Laufwerksbuchstabe, gemeinsames Elternverzeichnis, Unterordner je Tresor; Fehler werden auf konkrete Ursachen abgebildet. | [Quelle: https://raw.githubusercontent.com/cryptomator/cryptomator/develop/src/main/java/org/cryptomator/common/mount/Mounter.java, abgerufen 2026-10-22] |
| Nutzerwahl in der Oberfläche: Einstellungen, Reiter "Virtual Drive". "The availability of volume types depends on your operating system and installed drivers." Die Dokumentation sagt: ein Volumetyp für alle Tresore. Das widerspricht dem Quelltext (Dienst pro Tresor möglich). Ob der Quelltext oder die Dokumentation den aktuellen Stand der Oberfläche trifft: | [Quelle: https://docs.cryptomator.org/desktop/volume-type/, abgerufen 2026-10-22]; Widerspruch: [Lücke: Oberfläche pro Tresor nicht bestätigt] |
| Rückfall: "WebDAV (HTTP Address)" ist immer vorhanden; lokaler WebDAV-Server, manuell oder mit Drittanbieter-Anwendung einzubinden, "when all other volume types fail to mount". Unter Windows zusätzlich "WebDAV (Windows Explorer)" über `net use` auf zufälligen Buchstaben, mit Grenzen: keine Dateien über 4 GB, keine Pfade über 260 Zeichen, Platzangabe zeigt C:. | [Quelle: https://docs.cryptomator.org/desktop/volume-type/, abgerufen 2026-10-22] |
| Der `FallbackMounter` mountet selbst nichts, sondern meldet, keine passende Strategie gefunden zu haben, und verweist auf manuelles Einbinden der WebDAV-Adresse. | [laut Suchergebnis: https://github.com/cryptomator/cryptomator/issues/2920] |

Prinzip für Smart Explorer (Folgerung, nicht Code): ein schmaler Mount-Dienst je Weg mit Eignungsprüfung (`isSupported`), deklarierten Fähigkeiten (Laufwerksbuchstabe, Ordner, nur lesen, erzwungenes Aushängen, Loopback-Port), einer Auswahl "pro Verbindung bevorzugt, sonst global", und einem immer verfügbaren Rückfall (lokale WebDAV-Adresse). Dokany bleibt als einer der Dienste, bis ein anderer trägt. [Annahme]

## Lücken (vollständige Liste)

1. Primärbeleg, dass WebClient in Windows 11 24H2 und 25H2 ausgeliefert wird; sein Starttyp dort; Entfernungsplan (Microsoft nennt keinen).
2. Ob `net use /TCPPORT` für Standardnutzer ohne Administratorrechte funktioniert (Microsoft: Admin; Sicherheitsanalyse: Nicht-Admin).
3. Ob der SMB-Client auf 24H2 einen Nutzerraum-Server auf 127.0.0.1 mit Laufwerksbuchstaben bedient; Home-Edition ausdrücklich nicht bestätigt.
4. Interoperabilität von paltaio/rust-smb-server und sonroyaalmerol/go-smb-server mit dem Windows-SMB-Client; Lizenzwortlaut Impacket; Lizenz und Stand von farazshaikh/smb-server-rs.
5. Microsoft-Editionsliste für Client for NFS; Administratorbedarf beim Aktivieren und beim `mount`; Sperrverhalten im Detail; Portbeschränkung von mount.exe (Port 2049/111) gegenüber den Beispielen mit Port 11111; widersprüchliche Lizenzangabe von Vaiz/nfs3 (Unlicense gegen BSD-3-Clause); Lizenz und Stand von fernfs.
6. Cloud Files API unter subst/DefineDosDevice mit Platzhaltern: keine Dokumentation, keine Fehlerberichte mit subst.
7. dav-server-rs: Lizenzdatei im Repository nicht gelesen (nur crates.io: Apache-2.0).
8. Cryptomator: Lizenzdatei von integrations-api (404); Oberfläche pro Tresor (Dokumentation und Quelltext widersprechen sich); Prioritätsmechanismus der Anbieter im Lader nicht gelesen.
9. Nicht lesbar waren `https://4sysops.com/archives/change-smb-ports-in-windows-11-and-server-2025/` (HTTP 403) und `https://techcommunity.microsoft.com/blog/filecab/smb-alternative-ports-now-supported-in-windows-insider/3974509` (Artikeltext leer).
