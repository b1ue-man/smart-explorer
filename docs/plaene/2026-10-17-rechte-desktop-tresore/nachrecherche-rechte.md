# Nachrecherche S1 — Antworten N1–N24 und Revalidierung des Strangplans „Exec-Rechte“

Stand und Abrufdatum 2026-10-17. Lesestand des Repositorys: `da2646d2` (Arbeitsbaum), Release-Tag `v0.5.182` für N12. Bezug: `strang-rechte.md` Abschnitt G (N1–N24, sechs Aussagen) und Abschnitt C. Diese Datei bewertet nicht neu und entscheidet nicht; Folgerungen sind als „Folgerung“ gekennzeichnet, Annahmen als „Annahme“. Nichts wurde gebaut, kompiliert, getestet oder ausgeführt außer `curl`/`grep`/`sed` auf Quelltexte und Webseiten.

Abkürzungen wie im Strangplan: `N/` = `native/src/`, `A/` = `android/app/src/main/java/app/smartexplorer/android/`. Systemd-Quelltext wird als `systemd@<Tag>:<Datei>:<Zeile>` zitiert (abgerufen über `raw.githubusercontent.com/systemd/systemd/<Tag>/…`), Linux-PAM als `linux-pam@<Tag>:…`, Tailscale als `tailscale@main:<Datei>:<Zeile>` (Zweig `main` am 2026-10-17), Win32-OpenSSH als `openssh-portable@latestw_all:contrib/win32/win32compat/win32_usertoken_utils.c:<Zeile>`, Kernel-Dokumentation als `landlock.rst:<Zeile>` (torvalds/linux `master`, Kopf „:Date: August 2026“).

Belegarten: **A** = Hersteller- oder Projektdokumentation (Microsoft Learn, man-pages, Kernel-Doku, developer.android.com); **Q** = Quelltext an genanntem Stand; **P** = Distributions-Paketquelle; **C** = Issue-/Forenbeitrag (auch von Maintainern); **R** = Repository dieses Projekts.

---

## 1 Antworten N1–N24

### N1 — S4U über `MsV1_0S4ULogon` auf Windows 10 22H2 / 11 24H2, Anmeldetyp, `SeBatchLogonRight`

**Antwort.**
- (a) Lokale Konten: Die Funktionsfähigkeit ist **indirekt belegt**, nicht für die beiden Builds einzeln. Microsoft dokumentiert für OpenSSH für Windows: „Key-based authentication in OpenSSH for Windows works with local Windows accounts and Active Directory (domain) accounts.“ Die Schlüsselanmeldung von Win32-OpenSSH erzeugt das Token über S4U mit `MSV1_0_S4U_LOGON` für Nicht-Domänenkonten (`win32_usertoken_utils.c:195-203`) und `LsaLogonUser(…, Network, …)` (`:230`, `:311`). Die Enumerationsseite nennt `MsV1_0S4ULogon` (Wert 12) nur für Windows XP, Windows Server 2003, Windows Server 2003 SP2 und Windows Vista als „not supported“; eine Aussage für Windows 10 22H2 und Windows 11 24H2 im Einzelnen gibt es nicht. Die Strukturseite `MSV1_0_S4U_LOGON` liefert weiterhin HTTP 404.
- (b) Lokale Konten mit verknüpftem Microsoft-Konto: **nicht belegbar**. Weder Microsoft Learn noch der Win32-OpenSSH- oder Tailscale-Quelltext behandelt diesen Fall (Tailscale und OpenSSH unterscheiden nur „Domänenkonto“/„lokales Konto“; `tailscale@main:util/winutil/s4u/lsa_windows.go:228-250`).
- Anmeldetyp: Beide Referenzimplementierungen verwenden **`Network`** (`lsa_windows.go:342`: `lsaLogonUser(ls.handle, originName, _Network, …)`; OpenSSH `:230`, `:311`). Laut `LsaLogonUser`-Seite erzeugt `Network` ein **Impersonationstoken** („If LogonType is Network, an impersonation token is generated.“). Tailscale wandelt es vor `CreateProcessAsUser` um: `DuplicateTokenEx(token, 0, &sa, SecurityImpersonation, TokenPrimary, &dupToken)` (`tailscale@main:util/winutil/s4u/s4u_windows.go:157-158`), Prozessstart `:521`. `Batch` würde laut derselben Seite direkt ein Primärtoken liefern.
- `SeBatchLogonRight`: Für den Typ `Network` verwendet keine der beiden Referenzen dieses Recht. Das für Netzwerkanmeldungen maßgebliche Recht `SeNetworkLogonRight` („Access this computer from the network“) hat auf Client-Computern die wirksame Vorgabe „Everyone, Administrators, Users, Backup Operators“ (Microsoft Learn, Seite „Access this computer from the network“, `ms.date` 2021-06-11, `updated_at` 2026-04-27). **Annahme:** LSA prüft bei einer S4U-Anmeldung vom Typ `Network` dieses Recht wie bei anderen Netzwerkanmeldungen; keine Seite sagt das für S4U ausdrücklich. Die Vorgabe von „Deny access to this computer from the network“ auf Clients wurde nicht gelesen.

**Belege.** https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh_keymanagement (`ms.date` 2025-10-03); https://learn.microsoft.com/en-us/windows/win32/api/ntsecapi/ne-ntsecapi-msv1_0_logon_submit_type; https://learn.microsoft.com/en-us/windows/win32/api/ntsecapi/ns-ntsecapi-msv1_0_s4u_logon (404); https://learn.microsoft.com/en-us/windows/win32/api/ntsecapi/nf-ntsecapi-lsalogonuser; https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-10/security/threat-protection/security-policy-settings/access-this-computer-from-the-network; Quelltexte wie oben, alle abgerufen 2026-10-17.

**Folgerung für den Plan.** Der Kontodienst muss den Anmeldetyp `Network` und die Umwandlung in ein Primärtoken (`DuplicateTokenEx(…, TokenPrimary, …)`) ausdrücklich vorsehen; C2 nennt beides nicht.

**Verbleibende Lücke.** Lauf auf Windows 10 22H2 und Windows 11 24H2 (lokales Konto, Konto mit verknüpftem Microsoft-Konto) mit `whoami /all` im Relais; Bestätigung, dass LSA `SeNetworkLogonRight`/`SeDenyNetworkLogonRight` bei S4U prüft.

### N2 — Erzeugt `LoadUserProfileW` das Profil eines nie angemeldeten Kontos?

**Antwort.** **Indirekt ja, ohne Microsoft-Text.** Die Seite `LoadUserProfileW` (`ms.date` 2018-12-05) sagt nichts über das Anlegen eines fehlenden Profils. Sie verlangt: „The calling process must have the SE_RESTORE_NAME and SE_BACKUP_NAME privileges.“ und „the caller must be an administrator or the LocalSystem account. It is not sufficient for the caller to merely impersonate the administrator or LocalSystem account.“ sowie Token-Rechte `TOKEN_QUERY`, `TOKEN_IMPERSONATE`, `TOKEN_DUPLICATE`. `CreateProfile` („Creates a new user profile.“) verlangt Administratorrechte und meldet einen Fehler, wenn das Profil existiert. Win32-OpenSSH ruft nur `LoadUserProfileW` (mit `PI_NOUI`) und aktiviert dafür `SeBackupPrivilege` und `SeRestorePrivilege` (`win32_usertoken_utils.c:447-463`); die Datei enthält keinen Aufruf von `CreateProfile`. Das OpenSSH-Wiki „Design Details“ sagt: „If user profile does not exist, it gets created upon first logon via ssh.“ Tailscale ruft ebenfalls nur `LoadUserProfile` mit `PI_NOUI` (`tailscale@main:util/winutil/userprofile_windows.go:75-91`).

**Belege.** https://learn.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-loaduserprofilew; https://learn.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-createprofile; Quelltexte wie oben (2026-10-17).

**Folgerung.** Der Kontodienst aktiviert `SeBackupPrivilege` und `SeRestorePrivilege` um den Aufruf; ein eigener `CreateProfile`-Schritt ist nach den beiden Referenzimplementierungen nicht nötig.

**Lücke.** Microsoft-Aussage, dass `LoadUserProfileW` ein fehlendes Profil anlegt; Lauf mit frisch angelegtem `se-remote`.

### N3 — Liefert S4U für ein Mitglied von Administrators ein ungefiltertes Token?

**Antwort.** **Für lokale Konten nach der Microsoft-Dokumentation nein (Standard), für Domänenkonten ja.** Microsoft Learn „User Account Control and remote restrictions“ (KB 951016, `ms.date` 2026-02-12): Lokale Konten (SAM), die Mitglied der lokalen Administratorengruppe sind, verbinden sich bei einer Netzwerkverbindung „won't connect as a full administrator. The user has no elevation potential“; `LocalAccountTokenFilterPolicy` = 0 „builds a filtered token. It's the default value.“, = 1 „builds an elevated token“. Domänenkonten: „will run with a full administrator access token on the remote computer, and UAC won't be in effect.“ Win32-OpenSSH-Maintainer `manojampalam` (Issue PowerShell/Win32-OpenSSH#962, Kommentar 2017-11-26): „IF you are using an admin account, you should have an elevated session. Make sure you set the registry entry …“ mit Verweis auf `LocalAccountTokenFilterPolicy` (Belegart C). Die im Strangplan zitierte Aussage „UAC does not affect service, network, or batch logons“ (MSDN Magazine 2007, Ref Win §2.1.5) steht dazu im Widerspruch, soweit es lokale Konten betrifft.

**Belege.** https://learn.microsoft.com/en-us/troubleshoot/windows-server/windows-security/user-account-control-and-remote-restriction; https://github.com/PowerShell/Win32-OpenSSH/issues/962 (2026-10-17).

**Folgerung.** Die Annahme in C2 „Das Ziel erhält damit ein volles Administrator-Token“ trifft für lokale Konten nicht zu, solange `LocalAccountTokenFilterPolicy` nicht 1 ist. Die Warnung „REMOTE ADMINISTRATOR SHELL“ beschreibt dann ein Token ohne Administratorrechte.

**Lücke.** (1) Ob die Filterung der „remote restrictions“ auch für eine lokal von LocalSystem ausgelöste S4U-Anmeldung vom Typ `Network` gilt: nur durch den Maintainer-Kommentar gestützt, kein Microsoft-Text. (2) Verhalten für das eingebaute Konto mit RID 500 (`FilterAdministratorToken`): nicht gelesen. (3) Prüflauf `whoami /groups` (Integritätsstufe, Gruppe `BUILTIN\Administrators` „Deny“ oder „Enabled“).

### N4 — Low-Integrity-Supervisor, ConPTY-Host, `powershell.exe`/`cmd.exe` auf Low

**Antwort.**
- Start mit Integritätsstufe Low: Microsoft beschreibt genau den Weg „Duplicate the handle of the current process … Use SetTokenInformation to set the integrity level in the access token to Low … Use CreateProcessAsUser“ mit Beispielcode `OpenProcessToken` → `DuplicateTokenEx(…, TokenPrimary, …)` → `SetTokenInformation(TokenIntegrityLevel)` → `CreateProcessAsUser` aus einem Medium-Prozess („Designing Applications to Run at a Low Integrity Level“, `ms.date` 2007-07-05, archiviert). Der Beispielcode setzt die SID-Zeichenfolge `L"S-1-16-1024"`; ob das der Low-SID entspricht, wurde gegen die SID-Liste nicht geprüft (Annahme: Low ist `S-1-16-4096`; Seite „Well-known SIDs“ lieferte im Abruf keine Werte). **Widerspruch ungeklärt:** Die aktuelle Seite `CreateProcessAsUserW` sagt „must have the SE_INCREASE_QUOTA_NAME privilege and may require the SE_ASSIGNPRIMARYTOKEN_NAME privilege if the token is not assignable“ und „If hToken is a restricted version of the caller's primary token, the SE_ASSIGNPRIMARYTOKEN_NAME privilege is not required.“; das gefilterte Token eines Standardnutzers enthält `SeIncreaseQuotaPrivilege` nicht (Ref Win §2.1.5). Ob eine Kopie mit gesenkter Integritätsstufe ohne dieses Privileg akzeptiert wird, sagt keine Seite. Derselbe Microsoft-Artikel nennt einen zweiten Weg ohne Token-API: die ausführbare Datei mit `icacls <Datei> /setintegritylevel Low` kennzeichnen; der Prozess startet dann mit Low (Ref Win §2.6.3: „minimum of the user integrity level and the file integrity level“).
- Konsolenhost: In der quelloffenen Implementierung startet `_CreatePseudoConsole` den Host mit `CreateProcessAsUserW(hToken, conhostPath, …)`, wobei `hToken` für `CreatePseudoConsole` `nullptr` ist („CreateProcessAsUserW expects the token to be either valid or null“), also mit dem Token des Aufrufers (`microsoft/terminal@main:src/winconpty/winconpty.cpp:119-138`, `:259-261`, `:468`). **Annahme:** Die In-Box-Implementierung in Windows verhält sich gleich.
- `powershell.exe -NoLogo` und `cmd.exe` auf Low: **nicht belegbar**. Belegt ist nur: Low-Prozesse haben keinen Schreibzugriff „to most areas under the user's local profile area of the file system or the registry under HKCU“; schreibbar sind `%USERPROFILE%\AppData\LocalLow` und `HKCU\Software\AppDataLow`.
- **Zusatzbefund (Sicherheit von „Nur lesen“):** Derselbe Artikel nennt für Low-Prozesse erlaubte Kommunikation: „Clipboard“, „Remote procedure call (RPC)“, „Sockets“ sowie Named Pipes, deren Label gesenkt wurde. Der se-Daemon spricht seine IPC unter Windows über TCP auf Loopback (`N/daemon/os/windows/platform.rs:80`) mit dem Token aus `daemon.token` (`N/daemon/os/windows/ipc_storage.rs:8`, geschrieben mit `crate::support_dirs::write_private_atomic`). Ein Low-Prozess desselben Kontos darf Medium-Dateien lesen (Ref Win §2.6.3). **Folgerung:** Eine Low-Shell im eigenen Konto kann das Token lesen und den Worker über die IPC steuern (Freigaben ändern, Dateiaufträge starten), sofern `write_private_atomic` kein „No-Read-Up“-Label setzt (Annahme; die Funktion wurde nicht gelesen).

**Belege.** https://learn.microsoft.com/en-us/previous-versions/dotnet/articles/bb625960(v=msdn.10); https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessasuserw; https://raw.githubusercontent.com/microsoft/terminal/main/src/winconpty/winconpty.cpp (2026-10-17); R wie genannt.

**Lücke.** Lauf als Standardnutzer: `CreateProcessAsUserW` mit Low-Kopie des eigenen Tokens (Fehler 1314 ja/nein); Verhalten von `powershell.exe -NoLogo` (PSReadLine-Verlauf unter `%APPDATA%`) und `cmd.exe` auf Low; In-Box-`CreatePseudoConsole`.

### N5 — `SERVICE_START` für einen Nutzer-SID; Aufruferprüfung an der Pipe

**Antwort.**
- Start ohne Erhöhung: **ja nach Dokumentation.** „Service Security and Access Rights“ (`ms.date` 2018-05-31): Local authenticated users erhalten am SCM `SC_MANAGER_CONNECT`, `SC_MANAGER_ENUMERATE_SERVICE`, `SC_MANAGER_QUERY_LOCK_STATUS`, `STANDARD_RIGHTS_READ`; „Only processes with Administrator privileges are able to open handles to the SCM that can be used by the CreateService and LockServiceDatabase functions.“; `SERVICE_START`: „Required to call the StartService function“; die Dienst-DACL wird mit `SetServiceObjectSecurity` gesetzt. Einschränkung: „To perform any operations, the user must be logged on interactively or the service must use one of the service accounts.“ (Worker über `HKCU\…\Run` läuft interaktiv, B7.)
- `GetNamedPipeClientProcessId` + Prozess-Token: **nicht die empfohlene Prüfung.** James Forshaw (Project Zero, 2019-09) zeigt drei Wege, die vom Server gelesene Client-PID zu verfälschen (lokales SMB mit Mount-Point ab Windows 10 1709; SMB mit fester PID 65279; Weitergabe des Pipe-Handles an einen anderen Prozess mit PID-Wiederverwendung) und schließt, die PID „should not be relied upon to make security decisions“. `ImpersonateNamedPipeClient` („impersonates a named-pipe client application“) braucht beim Aufrufer `SeImpersonatePrivilege` (LocalSystem hat es) und wirkt für „the security context of the last message read from the pipe“; danach liefert `OpenThreadToken` die Nutzer-SID. Standard-DACL einer Pipe ohne eigenen Deskriptor: „full control to the LocalSystem account, administrators, and the creator owner“ und „read access to members of the Everyone group and the anonymous account“.

**Belege.** https://learn.microsoft.com/en-us/windows/win32/services/service-security-and-access-rights; https://projectzero.google/2019/09/windows-exploitation-tricks-spoofing.html; https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-impersonatenamedpipeclient; https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights (2026-10-17).

**Folgerung.** Die in C2 gewählte Prüfung „`GetNamedPipeClientProcessId` → Prozess-Token → Nutzer-SID“ beruht auf einem fälschbaren Wert; der belegte Weg ist Impersonation des Clients nach dem ersten gelesenen Rahmen.

**Lücke.** Wirkung von `PIPE_REJECT_REMOTE_CLIENTS` nicht gelesen.

### N6 — Relais aus Sitzung 0 in den Kill-on-close-Job des Workers (Sitzung 1)

**Antwort.** **Nein nach Dokumentation.** `AssignProcessToJobObject`: „Terminal Services: All processes within a job must run within the same session as the job.“; das Prozesshandle braucht `PROCESS_SET_QUOTA` und `PROCESS_TERMINATE`; verschachtelte Jobs: „If the process is already associated with a job, the job specified by hJob must be empty or it must be in the hierarchy of nested jobs to which the process already belongs, and it cannot have UI limits set“. Ein Relais mit dem S4U-Token läuft in Sitzung 0 (Token-Sitzung, Ref Win §2.1.5), der Job des Workers in dessen interaktiver Sitzung. Die Sitzung eines Tokens lässt sich ändern: `TokenSessionId` „If TokenSessionId is set with SetTokenInformation, the application must have the Act As Part Of the Operating System privilege“ (`SeTcbPrivilege`, LocalSystem hat es).

**Belege.** https://learn.microsoft.com/en-us/windows/win32/api/jobapi2/nf-jobapi2-assignprocesstojobobject; https://learn.microsoft.com/en-us/windows/win32/api/winnt/ne-winnt-token_information_class (2026-10-17).

**Folgerung.** Der Ablauf „Relais in Sitzung 0 angehalten erzeugen → Worker ordnet es seinem Job zu“ scheitert an der Sitzungsregel. Belegte Auswege (zur Entscheidung beim Planverfasser): Token-Sitzung vor `CreateProcessAsUserW` auf die Sitzung des Workers setzen, oder Job im Dienst anlegen und halten.

**Lücke.** Annahme, dass „session of the job“ die Sitzung des erzeugenden Prozesses ist (die Seite definiert es nicht); Lauf mit beiden Auswegen.

### N7 — Eigenschaften von `StartTransientUnit` in systemd 249–259

**Antwort.** **Ja für alle fünf an v249, v252, v255, v257, v259.**
- `User` und `PAMName`: Typ `s` (`systemd@v249:src/core/dbus-execute.c:1575-1576`, `:1819-1820`; v259 `:1877`, `:2342`).
- `WorkingDirectory`: Typ `s`; führendes `-` = „missing_ok“, Wert `~` = Heimatverzeichnis (`systemd@v249:src/core/dbus-execute.c:2638-2672`; v259 ab `:3243`).
- `StandardInputFileDescriptor`, `StandardOutputFileDescriptor` (und `StandardErrorFileDescriptor`): Typ **`h`**, gesetzt in `dbus-service.c` (`systemd@v249:src/core/dbus-service.c:526-532`, Setter `:346-376` liest `"h"` und dupliziert mit `F_DUPFD_CLOEXEC`; v252 `:526-532`, v255 `:683-689`, v257 `:688-694`, v259 `:700-706`).
- machined verwendet dieselbe Kombination seit v249: `StandardInputFileDescriptor` `h`, `PAMName` `login`, `WorkingDirectory` `-~`, `User` (`systemd@v249:src/machine/machine-dbus.c:693-703`; `systemd@v259:src/machine/machine.c:994-1009`).
- **Zusatzbefund:** Die Eigenschaft `ExitType` gibt es in v249 nicht (kein Treffer in `systemd@v249:src/core/dbus-service.c`; vorhanden ab v250). Eine unbekannte Eigenschaft beendet `StartTransientUnit` mit „Cannot set property %s, or unknown property.“ (`systemd@v249:src/core/dbus-unit.c:2472`). Der Bestand sendet `("ExitType", "main")` ohne Bedingung (`N/share/os/linux_os/exec_systemd.rs:54`). **Folgerung:** Auf Ubuntu 22.04 (systemd 249.11) schlägt jeder eingehegte Linux-Exec-Start schon heute fehl (abgeleitet, nicht ausgeführt); D.2.6 übernimmt `ExitType=main` „unverändert“.

**Belege.** Quelltexte an den Tags v249, v250, v252, v255, v257, v259 (2026-10-17).

**Lücke.** Lauf auf Ubuntu 22.04 zur Bestätigung des `ExitType`-Fehlers.

### N8 — Lehnt der Konto- oder Sitzungsteil von `/etc/pam.d/login` Sitzungen ohne TTY ab?

**Antwort.** **Nein in allen drei Zieldistributionen; `pam_securetty` steht in keiner aktiven Zeile.** Aktive Zeilen (Kommentare weggelassen):
- Debian 13 (`util-linux` 2.41.5-0+deb13u1, `debian/login.pam`): auth `pam_faildelay`, `pam_nologin` (requisite), `@include common-auth`, `pam_group`; session `pam_selinux close`, `pam_loginuid` (required), `pam_motd motd=/run/motd.dynamic`, `pam_motd noupdate`, `pam_selinux open`, `pam_env readenv=1` (zweimal), `pam_limits`, `pam_mail standard`, `pam_keyinit force revoke`; `@include common-account`, `common-session`, `common-password`. `pam_time` und `pam_access` sind auskommentiert (Zeilen 69, 74). `common-account` (pam 1.7.0-5): `pam_unix` mit `[success=end new_authtok_reqd=done default=ignore]`, `pam_deny`, `pam_permit`; `common-session`: `pam_deny`, `pam_permit`, `pam_umask`, dazu `pam_unix` (`debian/pam-configs/unix`) und `pam_systemd` (Paket systemd, Ref Linux §2.1.2).
- Ubuntu 24.04 (`shadow` 1:4.13+dfsg1-4ubuntu3.2, `debian/login.pam`): wie Debian 13, zusätzlich `session optional pam_lastlog.so`.
- Ubuntu 26.04 (`util-linux`, Zweig `ubuntu/resolute`): wie Debian 13.
- Fedora 44 (`util-linux-login.pamd`, Zweig f44): `account required pam_nologin.so`, `account include system-auth`; session `pam_selinux close`, `pam_loginuid`, `pam_selinux open`, `pam_namespace`, `pam_keyinit force revoke`, `include system-auth`, `include postlogin`, `-session optional pam_ck_connector.so`. authselect 1.7.1, Standardprofil Fedora „local with-silent-lastlog“ (`authselect.spec:23`): `system-auth` account `pam_unix` (dazu `pam_access`, `pam_faillock`, `pam_systemd_home` nur mit den Merkmalen `with-pamaccess`, `with-faillock`, `with-systemd-homed`); session `pam_keyinit revoke`, `pam_limits`, `-pam_systemd`, `pam_succeed_if service in crond`, `pam_unix`; `postlogin`: `pam_umask`, `pam_lastlog2` (mit Sprung `[default=1]`).

Weitere entscheidungsrelevante Wirkungen derselben Dateien:
1. `/etc/nologin`: Debian 13, Ubuntu 24.04 und 26.04 prüfen `pam_nologin` nur im auth-Teil; systemd ruft `pam_authenticate` nie (Ref Linux §2.1.2; `systemd@v259:src/core/exec-invoke.c:1376-1405` ruft `pam_start`, optional `PAM_TTY`, `pam_acct_mgmt`, `pam_setcred`, `pam_open_session`). **Folgerung:** `/etc/nologin` wirkt nur auf Fedora 44.
2. Debian und Ubuntu: Die erste `pam_motd`-Zeile ohne `noupdate` führt bei jedem Sitzungsstart `run-parts --lsbsysinit /etc/update-motd.d` als root aus (Debian-Patch `update-motd`, pam 1.7.0-5, Zeilen 73-81). **Folgerung:** Jeder Start mit `PAMName=login` führt diese Skripte aus; Laufzeit unbekannt.
3. `pam_systemd` steht in allen drei Sitzungsstapeln (siehe N20 und Revalidierung Z-Linux-1: Prozesse wandern in einen Sitzungs-Scope).
4. `pam_unix` im Kontoteil liefert `PAM_NEW_AUTHTOK_REQD` bei `sp_lstchg == 0` (N20); Debian bricht dann mit `done` ab, systemd behandelt jedes Ergebnis außer `PAM_SUCCESS` als Fehler (`exec-invoke.c:1400-1402`).

**Belege.** https://sources.debian.org/data/main/u/util-linux/2.41.5-0%2Bdeb13u1/debian/login.pam; https://sources.debian.org/data/main/p/pam/1.7.0-5/debian/local/common-account, …/common-session, …/debian/pam-configs/unix, …/debian/patches/update-motd; https://git.launchpad.net/ubuntu/+source/shadow/plain/debian/login.pam?h=ubuntu/noble-updates; https://git.launchpad.net/ubuntu/+source/util-linux/plain/debian/login.pam?h=ubuntu/resolute; https://src.fedoraproject.org/rpms/util-linux/raw/f44/f/util-linux-login.pamd; https://src.fedoraproject.org/rpms/authselect/raw/f44/f/authselect.spec; https://raw.githubusercontent.com/authselect/authselect/1.7.1/profiles/local/system-auth und …/postlogin (alle 2026-10-17).

**Lücke.** Lauf ohne TTY je Distribution; Laufzeit von `update-motd.d` auf Ubuntu Desktop.

### N9 — Landlock-Regeln für eine interaktive Shell „Nur lesen“; Verhalten bei ABI 1

**Antwort.**
- Nötige Regeln (aus der Kernel-Doku abgeleitet): alle Dateisystemrechte der erkannten ABI als „handled“; erlaubt `READ_FILE`, `READ_DIR`, `EXECUTE` unterhalb `/`; erlaubt `WRITE_FILE` auf `/dev/null`, `/dev/tty` und unterhalb `/dev/pts` (ab ABI 3 zusätzlich `TRUNCATE`, da „creat(2) … also requires the truncate right“ und die Doku empfiehlt, `WRITE_FILE` und `TRUNCATE` immer gemeinsam zu vergeben, `landlock.rst:449-469`; ab ABI 5 zusätzlich `IOCTL_DEV` für nach der Einschränkung neu geöffnete Terminals, `landlock.rst:681-701`). Vor der Einschränkung geöffnete Deskriptoren (PTY von stdin/stdout/stderr) bleiben unberührt (`landlock.rst:473-487`, `:684-686`).
- ABI-Grenzen (Kernel-Doku, Abschnitt „Previous limitations“, `landlock.rst:703-800`): ABI < 2: Umbenennen/Verlinken nur im selben Verzeichnis; **ABI < 3: „File truncation could not be denied … so it is always allowed“** — `truncate(2)` und `open(O_RDONLY|O_TRUNC)` können Dateien leeren; ABI < 4: TCP nicht einschränkbar; ABI < 5: `ioctl` immer erlaubt; ABI < 6: abstrakte UNIX-Sockets und Signale an Prozesse außerhalb nicht einschränkbar; **ABI < 9: Verbindungen zu Pfad-UNIX-Sockets nicht einschränkbar** (`LANDLOCK_ACCESS_FS_RESOLVE_UNIX`, `landlock.rst:790-795`). Nicht einschränkbar unabhängig von der ABI: `chdir`, `stat`, `flock`, `chmod`, `chown`, `setxattr`, `utime`, `fcntl`, `access` (Ref Linux §2.3.3).
- ABI der Zielsysteme (Kernel am 2026-10-17): Debian 13 Linux 6.12.107 → ABI 6; Ubuntu 24.04 GA 6.8.0-147 → ABI 4, HWE 7.0.0-38 → ABI 8; Ubuntu 26.04 7.0.0-38 → ABI 8; Fedora 44 7.2.11-200.fc44 → ABI 9. Kein Zielsystem hat ABI 1; Debian 12 (6.1.176) hat ABI 2.
- **Zusatzbefund (Umgehung):** Der se-Daemon spricht seine IPC auch unter Linux über TCP auf `127.0.0.1` (`N/daemon/os/linux_os/platform.rs:78`, `:301`); das Token liegt in `daemon.token` mit Modus `0600` des Worker-Kontos (`N/daemon/os/linux_os/ipc_storage.rs:13-15`). Eine Shell „Nur lesen“ im eigenen Konto darf das Token lesen und, da das Netzwerk erlaubt bleibt (C1), den Worker steuern. Landlock erlaubt nur Freigaben (Allow-Liste mit Vererbung nach unten); ein Unterpfad eines freigegebenen Lesebaums lässt sich nicht ausnehmen (Folgerung aus dem Regelmodell in `landlock.rst`). Zusätzlich: bis ABI 8 Verbindungen zu Pfad-UNIX-Sockets des Nutzers (Sitzungsbus `/run/user/<UID>/bus`, Benutzerdienstmanager), über die ein nicht eingeschränkter Prozess gestartet werden kann (Annahme: `systemd-run --user` über den Sitzungsbus wird nicht von Landlock erfasst, weil der Prozess vom Benutzerdienstmanager gestartet wird).

**Belege.** https://raw.githubusercontent.com/torvalds/linux/master/Documentation/userspace-api/landlock.rst (Kopf „August 2026“); Kernelversionen: https://bodhi.fedoraproject.org (kernel-7.2.11-200.fc44, stabil 2026-10-17), https://sources.debian.org/api/src/linux/ (Debian 13: Ref Linux §2.3.3), https://packages.ubuntu.com/noble-updates/linux-image-generic (6.8.0-147 und 7.0.0-38.38.1~24.04.1), https://packages.ubuntu.com/resolute/linux-image-generic (7.0.0-38.38); R wie genannt.

**Lücke.** Ubuntu-Kernelkonfiguration `CONFIG_LSM` (Launchpad-Git HTTP 403, Ref Linux L-3); Verhalten von `bash` (Verlauf, Here-Documents in `/tmp`) unter dem Regelsatz.

### N10 — Rust-Crate `landlock`: Version, Lizenz

**Antwort.** `landlock` **0.4.7** (veröffentlicht 2026-07-27), Lizenz **„MIT OR Apache-2.0“**, Repository `landlock-lsm/rust-landlock`, `rust-version` 1.71 (Projekt: MSRV 1.88, `native/Cargo.toml:242`). Abhängigkeiten (normal): `enumflags2 ^0.7`, `libc ^0.2.186`, `thiserror ^2.0` — alle drei stehen bereits in `native/Cargo.lock` (`enumflags2` 0.7.12 Zeilen 2064-2065, `libc` 0.2.186 Zeilen 3722-3723, `thiserror` 2.0.18 Zeilen 7075-7076). Unterstützt ABI V1–V9 (`v0.4.7:src/compat.rs:57-81`), darunter `AccessFs::Truncate`, `IoctlDev`, `ResolveUnix` (`v0.4.7:src/fs.rs:92-96`) und `Scope::AbstractUnixSocket`, `Scope::Signal` (`v0.4.7:src/scope.rs:41-43`). Eigene Hülle: `libc` 0.2.186 definiert `SYS_landlock_create_ruleset = 444` für x86_64-gnu (`libc-0.2.186/src/unix/linux_like/linux/gnu/b64/x86_64/not_x32.rs:427`, lokales Register).

**Belege.** https://crates.io/api/v1/crates/landlock und …/0.4.7/dependencies; https://raw.githubusercontent.com/landlock-lsm/rust-landlock/v0.4.7/… (2026-10-17).

**Lücke.** Syscall-Konstanten von `libc` für aarch64/Android nicht geprüft.

### N11 — Landlock im Android-App-Seccomp-Filter und in GKI-Kernen

**Antwort.**
- Seccomp: `platform/bionic@android-17.0.0_r1:libc/SYSCALLS.TXT:103-105` führt `landlock_add_rule`, `landlock_create_ruleset`, `landlock_restrict_self` („all“); keine der Block- oder Allowlisten (`SECCOMP_BLOCKLIST_APP.TXT`, `SECCOMP_BLOCKLIST_COMMON.TXT`, `SECCOMP_ALLOWLIST_APP.TXT`, `SECCOMP_ALLOWLIST_COMMON.TXT`) nennt sie; Regel: „The final seccomp allowlist is SYSCALLS.TXT - SECCOMP_BLOCKLIST.TXT + SECCOMP_ALLOWLIST.TXT“ (`SECCOMP_BLOCKLIST_APP.TXT:4`). **Also im App-Filter von Android 17 erlaubt.** In `SYSCALLS.TXT` an `android-14.0.0_r1`, `android-15.0.0_r1` und `android-16.0.0_r1` steht kein `landlock`-Aufruf (0 Treffer) — dort sind sie für Apps gesperrt.
- GKI: `kernel/common` Zweig `android16-6.12` `arch/arm64/configs/gki_defconfig:792` und `android17-6.18` `:773`: `CONFIG_SECURITY_LANDLOCK=y`; kein `CONFIG_LSM` im Defconfig, Kconfig-Vorgabe beginnt mit `landlock,` (`security/Kconfig` Zweige `android16-6.12:268-272`, `android17-6.18:273-277`). `android15-6.6`, `android14-6.1`, `android14-5.15`: kein `CONFIG_SECURITY_LANDLOCK=y`.
- **Folgerung:** Landlock ist für Apps nur auf Android 17 mit einem GKI ab `android16-6.12` nutzbar; ABI dort 6 (6.12) bzw. 7 (6.18), also ohne `RESOLVE_UNIX`.

**Belege.** https://android.googlesource.com/platform/bionic/+/refs/tags/android-17.0.0_r1/libc/ (…?format=TEXT); https://android.googlesource.com/kernel/common/+/refs/heads/android16-6.12/…, …/android17-6.18/…, …/android15-6.6/…, …/android14-6.1/…, …/android14-5.15/… (2026-10-17).

**Lücke.** Geräte, die mit älterem GKI auf Android 17 aktualisiert werden (Kernel bleibt alt): Anteil unbekannt; SELinux-Wirkung nicht geprüft.

### N12 — Bleibt ein Exec-Client 0.5.180–0.5.182 nach `START` ohne `STARTED` verbunden?

**Antwort.** **Ja, solange der Host dessen Heartbeats beantwortet; ohne Antwort bricht er nach 20 s ab.** Der Client betritt direkt nach dem Schreiben von `Start` die aktive Schleife (`N/share/core/exec_client.rs:260-284`); dort sendet er alle 5 s ein `Ping` und setzt `peer_deadline` = jetzt + 20 s, verlängert nur durch ein gültiges `Pong` oder einen Server-Rahmen (`N/share/core/exec_client_active.rs:40-45`, `:61-74`, `:101`; Werte `N/share/core/exec_heartbeat.rs:35-39`). `Pong` wird vor `Started` angenommen (Prüfung unabhängig vom Merker `started`, `exec_client_active.rs:61-74`; `ClientServerState::accept` `:308-318`). Ohne Antwort: Fehler `peer_unresponsive` „Exec peer stopped answering authenticated heartbeats“ (`:283-288`). Der Host 0.5.182 beantwortet `Ping` in seiner Auftragsschleife (`N/share/core/exec_server.rs:102-120`). Weitere Grenze nur für Terminals: `se shell` bricht ab, wenn `Started` nicht binnen `START_TIMEOUT` = 60 s kommt (`N/cli/shell_session/terminal_transports.rs:24`, `:144-155`). Dieselben Werte stehen am Tag `v0.5.180` (`exec_heartbeat.rs`: 5/20/10 s; `terminal_transports.rs:24`: 60 s; Pong-Behandlung `exec_client_active.rs:61`).

**Folgerung.** Ein alter Client übersteht eine Rückfrage beliebiger Dauer bei `se exec`/MCP, wenn der neue Host während der offenen Rückfrage `Ping` mit `Pong` beantwortet; ein alter `se shell` wartet höchstens 60 s.

**Lücke.** Android-Client (Bridge `ExecDialog`) auf eigene Startfristen nicht gelesen.

### N13 — Zählung „Desktop vorher 4“, „Android vorher 3“

**Antwort.** **Am Code bestätigt, nicht an der laufenden Oberfläche** (ein Lauf ist nach AGENTS.md hier nicht erlaubt). Desktop: Knopf „Remote-Ausfuehrung aktivieren …“ (`N/app/core/share_exec_ui.rs:281-289`), Kästchen „Ich bestaetige diese uneingeschraenkte Codeausfuehrung bewusst“ (`:334`, `:338`), Knopf „Exec jetzt aktivieren“ (`:345`) — 3 Handlungen plus Einstieg = 4. Android: „Erlauben…“ (`A/ui/share/ExecHostSection.kt:145`), Kästchen „Ich habe verstanden“ (`:198-202`), „Aktivieren“ (`:206`) = 3.

**Lücke.** Zählung am laufenden Programm (GUI-Harness, Compose-Test) bleibt Aufgabe der Remote-Suite.

### N14 — `pkexec` vorinstalliert; Verhalten ohne polkit-Agenten

**Antwort.**
- Ubuntu 24.04 Desktop: Live-Manifest `ubuntu-24.04.5.1-desktop-amd64.manifest` enthält `pkexec 124-2ubuntu1.24.04.3`, `polkitd`, `policykit-1`. Ubuntu 26.04 Desktop: `ubuntu-26.04.1-desktop-amd64.manifest` enthält `pkexec 127-2ubuntu1`, `polkitd`. (Annahme: das installierte System enthält die Pakete des Live-Manifests.)
- Fedora 44: Paket `polkit` 127 liefert `%attr(4755,root,root) %{_bindir}/pkexec` (`polkit.spec:151`, Zweig f44); also überall, wo `polkit` installiert ist.
- Debian 13: `pkexec` ist ein eigenes Paket (126-2), abhängig nur von `polkitd`; Changelog: „Split policykit-1 into polkitd and pkexec packages … only packages that rely on being able to run pkexec will have to depend on it“; das Übergangspaket `policykit-1` ist entfernt. Ob die Desktop-Tasks von Debian 13 `pkexec` mitbringen: **nicht belegbar** (Abhängigkeiten der Tasks nicht gelesen).
- Ohne Agenten: `pkexec` registriert einen eigenen Textagenten, wenn eine Rückfrage nötig ist und `--disable-internal-agent` fehlt; „this will fail if we can't find a controlling terminal“ → Meldung „Error creating textual authentication agent“ (`polkit-org/polkit@main:src/programs/pkexec.c:924-938`); mit abgeschaltetem internem Agenten: „No authentication agent found.“ (`:957`).

**Belege.** https://releases.ubuntu.com/24.04/…manifest, https://releases.ubuntu.com/26.04/…manifest; https://src.fedoraproject.org/rpms/polkit/raw/f44/f/polkit.spec; https://sources.debian.org/data/main/p/policykit-1/126-2/debian/changelog und …/control; https://raw.githubusercontent.com/polkit-org/polkit/main/src/programs/pkexec.c (2026-10-17).

**Lücke.** Debian 13 Desktop-Standardinstallation; Rückfallweg `sudo` für die Einrichtung ohne grafischen Agenten.

### N15 — SELinux-Typ unter `/usr/local/lib/smart-explorer/` gegenüber `/usr/local/libexec/`

**Antwort.** fedora-selinux/selinux-policy Zweig f44: `/usr/(.*/)?lib(/.*)?` → `lib_t` (`policy/modules/system/libraries.fc:117`), trifft `/usr/local/lib/smart-explorer/se-exec`. `/usr/local/libexec` hat keine eigene Regel in `corecommands.fc` (nur `/usr/libexec(/.*)?` → `bin_t`, `:271`, und `/opt/(.*/)?libexec(/.*)?`, `:141`) und fällt damit auf `/usr/.*` → `usr_t` (`policy/modules/kernel/files.fc:247`). `bin_t` erhalten `/usr/(.*/)?bin(/.*)?` (`corecommands.fc:165`) und `/usr/(.*/)?sbin(/.*)?` (`:183`), also `/usr/local/bin` und `/usr/local/sbin`. Der Typ `unconfined_service_t` hat als Einstiegstypen `bin_t` und `shell_exec_t` (`policy/modules/system/unconfined.te:20-21`).

**Folgerung.** `/usr/local/libexec/smart-explorer/` ist nicht „besser“ (usr_t statt lib_t); ein Ablageort mit `bin_t` ist `/usr/local/bin/` oder `/usr/local/sbin/`.

**Lücke.** In welcher Domäne systemd (`init_t`) eine Dienstdatei vom Typ `lib_t` oder `usr_t` ausführt, und ob das erlaubt ist: Die Übergangsregel `init_t` → `unconfined_service_t` wurde in `init.te`/`init.if` (f44) nicht gefunden.

### N16 — `StartTransientUnit` für UID 0 ohne Capabilities

**Antwort.** **Ja an v249 und v259 (aus dem Quelltext abgeleitet).** v259: `bus_verify_polkit_async_full` ruft `sd_bus_query_sender_privilege(call, /* capability= */ -1)` (`systemd@v259:src/shared/bus-polkit.c:578-584`); mit `capability < 0` entscheidet allein der UID-Vergleich „Sender has same UID as us, then let's grant access“ (`systemd@v259:src/libsystemd/sd-bus/bus-convenience.c:713-733`). v249: `bus_verify_manage_units_async` übergibt `CAP_SYS_ADMIN` (`systemd@v249:src/core/dbus.c:1169-1170`); die Fähigkeiten des Senders sind nur bekannt, wenn die Credentials sie ohne `/proc`-Ergänzung enthalten (`bus-convenience.c:673-690`; `bus_creds_add_more` ergänzt nur mit `SD_BUS_CREDS_AUGMENT`, `systemd@v249:src/libsystemd/sd-bus/bus-creds.c:761-762`; `sd_bus_creds_has_effective_cap` liefert sonst `-ENODATA`, `:675-676`); dann folgt derselbe UID-Vergleich (`bus-convenience.c:697-721`). **Annahme:** Über `dbus-daemon`/`dbus-broker` enthalten die Sender-Credentials keine Fähigkeiten. Unter dieser Annahme genügt euid 0.

**Lücke.** Lauf mit `CapabilityBoundingSet=` leer; Fall „Fähigkeiten bekannt und `CAP_SYS_ADMIN` fehlt“ unter v249 (dann `know_caps = true`, UID-Vergleich übersprungen, Weg über polkit; polkit autorisiert uid 0 — nicht gelesen).

### N17 — `NetUserAdd`/`NetLocalGroupAddMembers` auf Windows 11 Home

**Antwort.** **Nicht belegbar.** `NetUserAdd`: „On a member server or workstation, only Administrators and Power Users can call this function.“; keine Editionsangabe. Microsoft-Support „Create a local user or administrator account in Windows“ nennt nur den Weg über die Einstellungen und keine Editionsbeschränkung. Die Seite `New-LocalUser` enthält keine Editions- oder Erhöhungsangabe. Erhöhung: Das gefilterte Token eines Administrators hat die Administratorengruppe nur als „deny only“ (Ref Win §2.1.5, „How UAC works“); **Folgerung:** Der Aufruf braucht einen erhöhten Prozess.

**Belege.** https://learn.microsoft.com/en-us/windows/win32/api/lmaccess/nf-lmaccess-netuseradd; https://support.microsoft.com/en-us/windows/create-a-local-user-or-administrator-account-in-windows-20de74e0-ac7f-3502-a866-32915af2a34d; https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.localaccounts/new-localuser (2026-10-17).

**Lücke.** Lauf auf einer Windows-11-Home-VM.

### N18 — `se-remote` als Kachel; `SpecialAccounts\UserList` dokumentiert?

**Antwort.** **Nicht dokumentiert.** Die Suche auf learn.microsoft.com findet keine Seite zu `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\SpecialAccounts\UserList`; Treffer stammen von Erkennungsregeln (Sigma „Hiding User Account Via SpecialAccounts Registry Key“, ManageEngine), die das Setzen als Verschleierungstechnik melden. Ob ein Standardkonto ohne Anmeldung auf dem Anmeldebildschirm erscheint: keine Primärquelle.

**Belege.** WebSearch 2026-10-17 (Treffer: detection.fyi/sigmahq, manageengine.com).

**Folgerung.** Ein Ausblenden über diesen Schlüssel ist undokumentiert und löst in EDR-Produkten mit Sigma-Regeln eine Meldung aus.

**Lücke.** Lauf auf Windows 10/11 nach `NetUserAdd`.

### N19 — Entra-Konten auf Entra-verbundenen Geräten: `KERB_S4U_LOGON`?

**Antwort.** **Kein belegter Weg.** `LsaLogonUser` zu `KERB_S4U_LOGON`: „The caller must be a domain account (this includes LOCAL_SYSTEM if the computer is a domain member).“ (Ref Win §2.3.2); `KERB_S4U_LOGON` „Minimum supported client: None supported“. Tailscale lehnt Domänenkonten auf nicht domänenverbundenen Rechnern ab: „cannot logon as domain user without being joined to a domain“ (`lsa_windows.go:270-271`). Microsoft zu OpenSSH: „Authentication via a Microsoft Entra account isn't currently supported.“ (Ref Win §2.3.6) und „Microsoft Entra ID accounts don't support key-based authentication.“ (N1-Beleg).

**Lücke.** Verhalten von `MsV1_0S4ULogon` mit Entra-Konten (`AzureAD\…`) auf einem nur Entra-verbundenen Gerät: keine Quelle.

### N20 — `pam_unix` und gesperrtes Passwort; `pam_systemd` für System-UIDs

**Antwort.**
- `pam_unix` Kontoteil: **lässt `!` zu** an linux-pam v1.4.0, v1.5.2, v1.5.3, v1.7.0. Der Vergleich mit `*`/`!` steht nur in `verify_pwd_hash` (auth: `passverify.c:89` an v1.4.0, `:87` an v1.5.2). `check_shadow_expiry` liefert `PAM_ACCT_EXPIRED` bei abgelaufenem `sp_expire`, **`PAM_NEW_AUTHTOK_REQD` bei `sp_lstchg == 0`** (v1.4.0 `:281-299`, zusätzlich bei „legacy“/„too cheap“ Hash laut `crypt_checksalt`; v1.5.2 `:282-295`; v1.7.0 ab `:302`, dort zusätzlich `sp_lstchg < 0` → `PAM_SUCCESS` „password aging disabled“). `useradd -r` legt „no aging information“ an (Ref Linux §2.4.1). **Folgerung:** `se-remote` aus `useradd --system` besteht `pam_acct_mgmt`; ein Konto mit `passwd -e` (Feld 0) scheitert.
- `pam_systemd`: Ohne `PAM_TTY` ist der Typ „unspecified“ und die Klasse „background“ (`systemd@v249:src/login/pam_systemd.c:788`; v259 `:997`). Ab **v258** wird die Klasse für System-, Intrinsic- und dynamische Nutzer auf **„background-light“** gesetzt (`systemd@v258:src/login/pam_systemd.c:1022`, `:1030`; v259 `:1012-1036`); v256 und v257 tun das nicht (v256 `:1034-1036`). „background-light“ startet keinen Benutzerdienstmanager: `SESSION_CLASS_WANTS_SERVICE_MANAGER` = `user`, `user-early`, `greeter`, `lock-screen`, `background` (`systemd@v259:src/login/logind-session.h:45`). Also: Debian 13 (257), Ubuntu 22.04 (249), Ubuntu 24.04 (255) → Klasse „background“ (nach der v259-Definition mit `user@.service`; für v249–v257 nicht einzeln gelesen); Fedora 44 und Ubuntu 26.04 (259) → „background-light“, kein `user@.service`. `XDG_RUNTIME_DIR` setzt `pam_systemd` nur, wenn die Sitzungs-UID gleich der angemeldeten ist und das Verzeichnis geprüft wurde (`systemd@v249:src/login/pam_systemd.c:902-907`; v259 `:1324-1329`).
- Beide Klassen erhalten einen eigenen Sitzungs-Scope: `SESSION_CLASS_WANTS_SCOPE` umfasst `background` und `background-light` (`logind-session.h:40-42`); logind startet `session-<ID>.scope` mit dem Leitprozess (`systemd@v259:src/login/logind-session.c:705-735`).

**Belege.** Quelltexte an den genannten Tags (2026-10-17).

**Lücke.** Anlage von `/run/user/<UID>` für „background-light“ nicht gelesen.

### N21 — Android-Rückfrage ohne Vordergrund

**Antwort.** Das Posten einer Benachrichtigung unterliegt nicht den Startbeschränkungen für Aktivitäten. Aktivitäten aus dem Hintergrund sind seit Android 10 beschränkt; Ausnahmen sind unter anderem „The activity is started from a PendingIntent that was sent by the system (for example, from a notification tap)“ und „The app has a visible window“ (developer.android.com „Restrictions on starting activities from the background“, „Last updated 2026-05-18 UTC“). Ein Vordergrunddienst ist dort nicht als Ausnahme genannt. Vollbild-Benachrichtigungen: Für Apps mit Ziel-API ≥ 34 ist `USE_FULL_SCREEN_INTENT` auf Anruf- und Wecker-Apps beschränkt; Play entzieht es anderen Apps (Android-14-Verhaltensänderungen). Die App hat `targetSdk = 36`, `minSdk = 30` (`android/app/build.gradle.kts:89-90`). Bestehende Muster: Aktionen über einen nicht exportierten Receiver (`ExecHostStopReceiver`, `android/app/src/main/AndroidManifest.xml:116-118`) — eine Benachrichtigungsaktion mit Broadcast-`PendingIntent` startet keine Aktivität. Der Exec-Host lebt im App-Prozess; im Hintergrund dauerhaft erreichbar nur mit dem vom Nutzer gewählten `BackgroundService` (`foregroundServiceType="specialUse"`, Manifest).

**Belege.** https://developer.android.com/guide/components/activities/background-starts; https://developer.android.com/about/versions/14/behavior-changes-14 (2026-10-17); R wie genannt.

**Folgerung.** Rückfrage als Benachrichtigung mit drei Broadcast-Aktionen ist ohne Hintergrund-Ausnahme möglich; ein selbst aufspringendes Rückfragefenster nicht.

### N22 — egui-Fenster unter Wayland in den Vordergrund ohne Aktivierungstoken

**Antwort.** **Nicht zuverlässig.** `xdg-activation-v1`: „The compositor may know who requested this by checking the activation token and might decide not to follow through with the activation“; „Compositors can ignore unknown activation tokens when an invalid token is passed.“; Tokens können schon beim Erhalt unwirksam sein (Protokolldatei `staging/xdg-activation/xdg-activation-v1.xml`, wayland-protocols `main`). GNOME: Bei nicht nachweisbaren Fokusanfragen zeigt GNOME Shell „<App> is ready“ statt des Fensters (GNOME-Shell-Blog „Understanding GNOME Shell's focus stealing prevention“, 2024). Mutter-Issue #3486 (Belegart C): im Standardmodus „smart“ erhielten neue Fenster derzeit Fokus auch ohne Token. KDE KWin: keine Quelle gelesen.

**Lücke.** Verhalten von KWin; Verhalten eines neu erzeugten (nicht reaktivierten) egui-Fensters unter GNOME 47/48/49.

### N23 — Windows: Fenster eines Hintergrundprozesses in den Vordergrund

**Antwort.** **Nur unter den Bedingungen der Seite.** `SetForegroundWindow` (`ms.date` 2025-10-06): erlaubt nur, wenn (alle) Desktop-App, kein `LockSetForegroundWindow`, kein aktives Menü **und** (mindestens eine) Sperrfrist abgelaufen, Aufrufer ist Vordergrundprozess, wurde vom Vordergrundprozess gestartet, kein Vordergrundfenster vorhanden, Aufrufer erhielt das letzte Eingabeereignis, oder einer der beiden Prozesse wird debuggt. „An application cannot force a window to the foreground while the user is working with another window. Instead, Windows flashes the taskbar button“. `AllowSetForegroundWindow` kann nur ein Prozess vergeben, der selbst darf; das Recht erlischt bei der nächsten Nutzereingabe an einen anderen Prozess.

**Belege.** https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow (2026-10-17).

**Folgerung.** Ein vom Daemon gestartetes Rückfragefenster erscheint im Normalfall nicht vorn; die Taskleiste blinkt (R8 unverändert gültig).

### N24 — `POST_NOTIFICATIONS` und Meldung fehlender Freigabe

**Antwort.** Das Manifest deklariert `android.permission.POST_NOTIFICATIONS` (`android/app/src/main/AndroidManifest.xml:11`); die Laufzeitabfrage steht in `A/ui/onboarding/OnboardingScreen.kt:97` und `A/ui/settings/BackgroundSettings.kt:354`. `Notifications.notify` prüft `Permissions.canPostNotifications` (Laufzeitrecht ab API 33 und `areNotificationsEnabled()`, `A/system/Permissions.kt:29-40`) und gibt bei fehlender Freigabe still `false` zurück (`A/system/Notifications.kt:71-81`); `ExecHostNotifier` wertet den Rückgabewert nicht aus (`A/service/ExecHostNotifier.kt:103`). Der Kanal `exec` hat `IMPORTANCE_LOW` (`A/system/Notifications.kt:52`, `:61-65`). **Folgerung:** Eine Rückfrage über den bestehenden Kanal erschiene ohne Ton und ohne Einblendung; ohne Freigabe erschiene sie gar nicht und niemand erführe es.

---

## 2 Revalidierung

### 2.1 Die sechs genannten Aussagen

| Aussage im Plan (Stelle) | Ergebnis | Beleg | Richtige Angabe |
|---|---|---|---|
| (1) „Bestehender Windows-Supervisor kann unverändert als Relais im Zielkonto laufen“ (G, B10, C2 W-b) | widerlegt (für die Einhegung) | N6: `AssignProcessToJobObject` „All processes within a job must run within the same session as the job.“ | Der Supervisor kann als Relais laufen (öffnet die ConPTY selbst, N4 `winconpty.cpp`), aber in Sitzung 0 nicht dem Job des Workers zugeordnet werden; nötig ist Token-Sitzung = Worker-Sitzung (`TokenSessionId`, `SeTcbPrivilege`) oder ein vom Dienst gehaltener Job |
| (2) „`PAMName=login` ist auf allen Zieldistributionen nutzbar“ (G, C3 L-a) | bestätigt mit Auflagen | N8, N20; machined nutzt es seit v249 | Kein TTY-Zwang in account/session. Auflagen: Prozesse wandern in `session-<ID>.scope` (siehe 2.2 Z-L1); Debian/Ubuntu führen `update-motd.d` je Start aus; `/etc/nologin` wirkt nur auf Fedora; `sp_lstchg == 0` scheitert |
| (3) „Landlock genügt für ‚Nur lesen‘ einer Shell“ (G, C3 L-i) | widerlegt | N9 | Abkürzen bleibt bis ABI 2 erlaubt; Pfad-UNIX-Sockets bis ABI 8 nicht einschränkbar; im eigenen Konto ist die se-IPC (TCP 127.0.0.1, Token lesbar) erreichbar, solange Netzwerk erlaubt ist; `chmod`/`chown`/`utime`/`setxattr` nie einschränkbar |
| (4) „Integritätsstufe Low genügt für ‚Nur lesen‘ unter Windows“ (G, C2 W-g) | widerlegt (Umgehung) und teilweise nicht belegbar | N4 | Low erlaubt Sockets und RPC; die se-IPC unter Windows ist TCP-Loopback mit für das Konto lesbarem Token. Start ohne `SeIncreaseQuotaPrivilege` und Verhalten von `powershell.exe`/`cmd.exe` auf Low: nicht belegbar |
| (5) „S4U deckt die Konten eines typischen Heim-PCs ab“ (G, C2) | nicht belegbar | N1, N17, N19 | Belegt nur lokale Konten (indirekt). Lücke: Konten mit verknüpftem Microsoft-Konto (keine Quelle), Windows 11 Home (`NetUserAdd` ohne Editionsaussage); Entra-Konten auf Entra-Geräten: kein belegter Weg |
| (6) „Alte Clients überstehen eine Rückfrage“ (G, R9) | bestätigt mit Bedingung | N12 | Nur wenn der Host während der offenen Rückfrage `Ping` mit `Pong` beantwortet (sonst Abbruch nach 20 s); alter `se shell` bricht nach 60 s ohne `Started` ab |

### 2.2 Weitere handlungsbestimmende Tatsachenaussagen in Abschnitt C

| Aussage im Plan (Stelle) | Ergebnis | Beleg | Richtige Angabe |
|---|---|---|---|
| C2 W-b: S4U „Anmeldung (MSV1_0) ohne Passwort … `CreateProcessAsUserW` des Supervisors“ | bestätigt mit Ergänzung | N1 | Typ `Network` liefert ein Impersonationstoken; Umwandlung mit `DuplicateTokenEx(…, TokenPrimary, …)` nötig (Tailscale `s4u_windows.go:157-158`) |
| C2 „S4U braucht `SeTcbPrivilege` (LocalSystem)“ (B18) | bestätigt | Ref Win §2.3.1/§2.3.4 (`LsaRegisterLogonProcess`: „The caller does not have the SeTcbPrivilege privilege, which is required“) | — |
| C2 „S4U-Token ohne Netzanmeldedaten“; Hinweistext Netzlaufwerke/EFS | bestätigt | Learn OpenSSH Key Management: „A remote session opened via key-based authentication doesn't have associated user credentials.“; Task Scheduler `TASK_LOGON_S4U` (Ref Win §2.3.3) | — |
| C2 „`CreateProcessWithLogonW` nimmt laut Doku nur `STARTUPINFO`“ (W-a, B18) | bestätigt | Seite `CreateProcessWithLogonW`: Parameter „STARTUPINFO structure“; „You cannot call CreateProcessWithLogonW from a process that is running under the "LocalSystem" account“ | — |
| C2 „`CreateProcessAsUserW` … mit `HANDLE_LIST`“ | bestätigt | Seite `CreateProcessAsUserW`: „STARTUPINFO or STARTUPINFOEX structure“, „use a STARTUPINFOEX structure and specify EXTENDED_STARTUPINFO_PRESENT“ | — |
| C2 „Administratorkonto als Ziel: … filtert UAC Netzwerk-, Dienst- und Batch-Anmeldungen nicht. Das Ziel erhält damit ein volles Administrator-Token.“ (B18, Ref Win §2.1.5) | widerlegt für lokale Konten | N3 (KB 951016) | Lokale Administratorkonten erhalten bei Netzwerkanmeldung standardmäßig ein gefiltertes Token (`LocalAccountTokenFilterPolicy` = 0); Domänenkonten ein volles |
| C2 „Die Dienst-DACL gibt dem einrichtenden Nutzer-SID `SERVICE_START` und `SERVICE_QUERY_STATUS`. Der Worker startet den Dienst bei Bedarf“ | bestätigt | N5 | Bedingung „logged on interactively“ |
| C2 „Prüfung je Anfrage: `GetNamedPipeClientProcessId` → Prozess-Token → Nutzer-SID“ | widerlegt als Sicherheitsprüfung | N5 (Project Zero 2019) | Client-PID ist fälschbar; belegt ist `ImpersonateNamedPipeClient` + Thread-Token |
| C2 „`LoadUserProfileW` und `CreateEnvironmentBlock`; Profil wird nach Ende des Relais entladen“ | bestätigt mit Ergänzung | N2 | Aufrufer braucht aktivierte `SeBackupPrivilege`/`SeRestorePrivilege`; `UnloadUserProfile` scheitert, solange Registrierungshandles offen sind (Seite `LoadUserProfileW`, Remarks) |
| C2 „Sitzung des Relais: die des Tokens (Sitzung 0, wie bei Win32-OpenSSH)“ | bestätigt | Ref Win §2.1.5; N6 | Folge für den Job siehe 2.1 (1) |
| C2 W-c „`ConptyCreatePseudoConsoleAsUser` … undokumentiert“ | bestätigt | `winconpty.cpp:471` (`extern "C" … ConptyCreatePseudoConsoleAsUser`) ohne Learn-Seite (Ref Win L07) | — |
| C2 W-g „Integritätsstufe Low …: Schreiben auf Medium-Objekte gesperrt, Lesen erlaubt“ | bestätigt | N4 (Artikel 2007; Ref Win §2.6.3) | Zusätzlich erlaubt: Sockets, RPC, Zwischenablage (Umgehung über se-IPC, 2.1 (4)) |
| C2 W-g „Konsolenhost und Shell erben die Stufe“ | bestätigt für den Konsolenhost (quelloffene Implementierung), Shell: bestätigt (Kind erbt Token) | N4 `winconpty.cpp:119-138`, `:259-261`; Artikel: „child processes inherit the integrity level of their parent process“ | In-Box-Gleichheit ist Annahme |
| C2 W-h „`pwsh.exe` im AppContainer ohne Dateisystem-PSDrives“ | nicht neu geprüft | Ref Win §2.6.4 [S54] (Belegart C) | — |
| C3 L-a „transiente Einheit mit `User=`, `PAMName=login`, `WorkingDirectory=-~`, Supervisor-Verbindung als Deskriptor“ | bestätigt (Eigenschaften), Typ `h` | N7 | — |
| C3 L-a Z1 „Gruppen, `HOME`, Arbeitsverzeichnis gleich“; L-b Z1 „Einhegung über die Einheit“; D.2.6 „unverändert … `KillMode=control-group`“ | **widerlegt (Z-L1)** | systemd.exec(5) v249 und v259 zu `PAMName=`: „the main unit process will be migrated to its own session scope unit … Any child processes of that process will however be associated with the session scope unit only.“; N20: Scope auch für „background“/„background-light“ | Mit `pam_systemd` im Stapel (alle drei Distributionen, N8) liegen Supervisor-Kinder in `session-<ID>.scope`, nicht in `smart-explorer-exec-<id>.service`; `KillMode=control-group` und `cgroup_populated` (B9) der Einheit erfassen sie nicht. `KillUserProcesses=` steuert, ob der Scope beim Abmelden beendet oder „abandoned“ wird (logind.conf(5) v259) |
| D.2.6 „unverändert: … `ExitType=main`“ | widerlegt für systemd 249 | N7 | `ExitType` erst ab v250; Ubuntu 22.04 lehnt die Einheit ab |
| C3 L-a Sicherheit „`pam_acct_mgmt`, `pam_limits`, `pam_loginuid`, `pam_systemd`“ | bestätigt | N8 | Zusätzlich `pam_motd` mit `update-motd.d` (Debian/Ubuntu), `pam_namespace`, `pam_selinux` (Fedora) |
| C3 L-a' „`pam_limits` verlangt einen einthreadigen Prozess“ | bestätigt | `pam_limits(8)`: „The module must not be called by a multithreaded application.“ (Ref Linux §2.1.2) | — |
| C3 L-c „CVE-2025-32463“; „sudo-rs-Abdeckung unbelegt“ | nicht neu geprüft | Ref Linux §2.2.1 | — |
| C3 L-d „`pkexec`: keine Argumentprüfung, CVE-2021-4034, kein `pam_acct_mgmt`“ | bestätigt | Ref Linux §2.2.2 (pkexec.1 SECURITY NOTES; Tabelle `pam_acct_mgmt` nein) | — |
| C3 L-e „`systemd-run --uid` mit polkit-Regel kann `User=` nicht einschränken“ (B19) | bestätigt | `systemd@v259:src/core/dbus-util.c:161-181`: Details nur `unit`, `verb` (und Nachricht) | — |
| C3 L-f „`machinectl shell`: nur PTY, kein Exit-Code“ | bestätigt | machinectl(1) laut Ref Linux §2.2.6; `machine.c:994` nur ein PTY-Deskriptor | — |
| C3 L-i „Landlock unprivilegiert ab Kernel 5.13“, „`no_new_privs` + Regelsatz“ | bestätigt | Ref Linux §2.3.3; `landlock.rst` | — |
| C3 L-i „Schreiben nur `/dev/pts`, `/dev/tty`, `/dev/null`“ als Wirkung | widerlegt in der Vollständigkeit | N9 | Abkürzen bis ABI 2, Pfad-Sockets bis ABI 8, se-IPC über TCP, `chmod`/`chown`/`utime`/`setxattr` |
| C3 L-j „bubblewrap braucht unprivilegierte User-Namespaces (Ubuntu 24.04 AppArmor-Sperre)“; „firejail ist setuid-root“ | nicht neu geprüft | Ref Linux §2.3.1, §2.3.2, §2.3.5 | — |
| C3 „`/proc/<pid>/exe` eines root-Prozesses ist für das Zielkonto nicht lesbar“ | bestätigt | proc_pid_exe(5), man-pages 6.19: „governed by a ptrace access mode PTRACE_MODE_READ_FSCREDS check“ | — |
| C3 „Ablageort unter Vorbehalt N15“ | Grundlage geändert | N15 | `/usr/local/lib/…` = `lib_t`; `/usr/local/libexec/…` = `usr_t`; `bin_t` nur unter `…/bin`, `…/sbin` |
| C3 „Härtung … `CapabilityBoundingSet=` leer (Vorbehalt N16)“ | bestätigt (abgeleitet) | N16 | Unter der Annahme N16 |
| C3 „`SocketMode=0666`; Zulassung nur über `SO_PEERCRED`“ | bestätigt | Ref Linux §2.2.7 (systemd.socket(5) „Defaults to 0666“; unix(7)) | — |
| C3 Fernzugriffskonto „`pam_unix` prüft im Kontoteil `!` nicht“ | bestätigt mit Ergänzung | N20 | `sp_lstchg == 0` → `PAM_NEW_AUTHTOK_REQD` → Start scheitert |
| C3 „`useradd --system --create-home --home-dir …`“ | bestätigt | Ref Linux §2.4.1 (`-r` legt ohne `-m` kein Heimatverzeichnis an) | — |
| C4 A-1/B17 „`setuid`-Familie per Seccomp gesperrt“ | bestätigt | `SECCOMP_BLOCKLIST_APP.TXT@android-17.0.0_r1:16-17`, `:24` („setresuid is explicitly allowed“), `:29-30` | — |
| C4 A-2 „13.6.0 unter Android 17 defekt, Issue #2180 offen; kein Release seit 2025-05-25“ | bestätigt (Stand 2026-10-17) | GitHub: neuestes Release `v13.6.0`; Issue #2180 „Android 17“ Zustand OPEN | Datum des Releases nicht neu gelesen |
| C4 A-6 „Verfügbarkeit von `landlock_*` im Seccomp-Filter … unbelegt“ | Grundlage geändert | N11 | Erlaubt ab Android 17; Kernel nur ab GKI `android16-6.12` |
| C5 „bestehende Grenzen (2 je Gerät, 8 gesamt; `exec_registry.rs:96-97`)“ | bestätigt | `N/share/core/exec_registry.rs:96-97`: `global_active: 8`, `principal_active: 2` | — |
| C5 „Shell-Befehlstext nie protokolliert; Regel `exec_audit.rs:7-8`“ | bestätigt | `N/share/os/shared/exec_audit.rs:7-8` | — |
| C5 „Android: Benachrichtigung mit drei Aktionen“ | bestätigt mit Auflage | N21, N24 | Kanal `exec` hat `IMPORTANCE_LOW`; ohne `POST_NOTIFICATIONS` wird still nichts gezeigt |
| C5 „Desktop … startet der Daemon das Rückfragefenster“ | bestätigt mit Auflage (Sichtbarkeit) | N22, N23 | Windows: Taskleiste blinkt statt Vordergrund; Wayland: Kompositor entscheidet |
| C6 Textstellen `N/cli/share/grants.rs:44`, `N/cli/exec.rs:7-8`, `N/cli/share/grants_exec.rs:50-52`, `N/app/core/share_exec_ui.rs:173`, `:308`, `:334`, `:259`, `share_exec_programs_ui.rs:163`, `ExecProgramsDialog.kt:137`, `ExecHostSection.kt:186-188` | bestätigt | Zeilen am Lesestand gelesen | — |
| C6 „Grund bleibt absichtlich unbestimmt … `N/share/core/exec_auth.rs:365-371`“ | bestätigt, Stelle ist Testcode | `exec_auth.rs:257` beginnt `mod tests`; Produktionsstellen `exec_auth.rs:253`, `N/share/core/exec_admission.rs:81` („exec authentication failed“) | — |
| C7 I-b „der bisherige Nachprüfweg `grants_exec.rs:200-204` deckt nur Konto/Liste“ | bestätigt | `N/cli/share/grants_exec.rs:200-204` | — |
| C7 „Katalogregel `N/cli/mcp/catalog.rs:36-38`“, „Sperrliste in `N/cli/mcp/cli_tool.rs`“ | bestätigt (Datei vorhanden, Regeltext Zeilen 36-38) | R | — |
| C1 „Netzwerk bleibt erlaubt (Landlock-Netzregeln erst ab ABI 4)“ | bestätigt | `landlock.rst` „TCP bind and connect (ABI < 4)“ | Folge siehe 2.1 (3) |
| D.4 „Rückfrage am Host … Windows Fenster + Ballon (N23)“ | bestätigt mit Auflage | N23 | — |
| D.5 „Linux … Einrichtung über `pkexec`/`sudo`“ (C3 L-b K5) | teilweise nicht belegbar | N14 | Debian 13: `pkexec` nicht sicher vorhanden; ohne Agent und ohne Terminal scheitert `pkexec` |

---

## 3 Folgen für Entscheidungen

Nur Entscheidungen, deren Grundlage sich geändert hat. Die Wahl trifft der Planverfasser.

1. **C2 W-b (Windows-Kontodienst), Einhegung (Z1).** Grundlage „Worker ordnet das Relais seinem Kill-on-close-Job zu“ trägt nicht (N6). Belegte Varianten: Token-Sitzung auf die Worker-Sitzung setzen (`SetTokenInformation(TokenSessionId)`, `SeTcbPrivilege`) — dann laufen auch grafische Programme in der Sitzung des Workers (R10 entfiele, Annahme); oder Job im Dienst anlegen und den Abbruch über das Kontodienst-Protokoll (`stop`) führen.
2. **C2 W-b, Anmeldung.** `Network` + `DuplicateTokenEx(TokenPrimary)` als Pflichtschritt (N1); `SeBackupPrivilege`/`SeRestorePrivilege` für `LoadUserProfileW` (N2).
3. **C2, Aufruferprüfung der Pipe.** `GetNamedPipeClientProcessId` ist fälschbar (N5); belegt ist `ImpersonateNamedPipeClient`.
4. **C2, Administratorkonto als Ziel.** Lokale Administratorkonten erhalten standardmäßig ein gefiltertes Token (N3); der Warntext „REMOTE ADMINISTRATOR SHELL“ und die Annahme „volles Administrator-Token“ passen für lokale Konten nicht.
5. **C2 W-g und C3 L-i („Nur lesen“).** Beide Mechanismen lassen im eigenen Konto die se-IPC über TCP-Loopback mit lesbarem Token zu (N4, N9). Linux zusätzlich: Abkürzen bis ABI 2, Pfad-UNIX-Sockets bis ABI 8 (betrifft Debian 13, Ubuntu 24.04, Ubuntu 26.04). Windows zusätzlich: Start ohne `SeIncreaseQuotaPrivilege` und Shell-Verhalten nicht belegbar; der dokumentierte Weg über eine Low-gekennzeichnete Programmdatei braucht kein Privileg. Die Rückfallwahl aus C2 W-g („nicht angeboten“) und die Bezeichnung „Nur lesen (Dateien)“ stehen damit neu zur Entscheidung, ebenso eine Einschränkung auf Kontoprofile ≠ eigenes Konto.
6. **C3 L-a/L-b (Linux-Konto-Einheit), Einhegung (Z1).** Mit `PAMName=login` wandern Supervisor-Kinder in `session-<ID>.scope` (systemd.exec(5); N20). Die Annahme „Einhegung über die Einheit“ und der Bestand `KillMode=control-group`/`cgroup_populated` (B9) gelten für diese Kinder nicht mehr. Belegte Stellgrößen: PAM-Stapel ohne `pam_systemd` (eigener Dienstname wie `smart-explorer-exec`, Option in G N8), oder Abbruch über den Sitzungs-Scope.
7. **C3, PAM-Dienstname `login`.** Neue Gründe gegen bzw. für `login`: Debian/Ubuntu führen `update-motd.d` je Start aus; `/etc/nologin` wirkt nur auf Fedora (R4-Text „/etc/nologin lehnen jetzt ab“ trifft nur Fedora); `pam_systemd` erzwingt den Sitzungs-Scope (Punkt 6).
8. **D.2.6 / Bestand B9.** `ExitType=main` ist auf systemd 249 (Ubuntu 22.04) unbekannt (N7); betrifft den Bestand und die neue Konto-Einheit.
9. **C3, Ablageort der root-eigenen Kopie.** `/usr/local/lib/smart-explorer/` = `lib_t`, `/usr/local/libexec/` = `usr_t`; `bin_t` nur unter `…/bin`/`…/sbin` (N15).
10. **C3 L-b, Einrichtung über `pkexec`.** Debian 13 ohne gesichertes `pkexec`; ohne grafischen Agenten und ohne Terminal scheitert `pkexec` (N14).
11. **C4 A-6 (Android „Nur lesen“).** Nicht mehr „unbelegt“: Seccomp erlaubt Landlock ab Android 17, Kernel ab GKI `android16-6.12` (ABI 6/7); der TODO-Eintrag kann diese Bedingung nennen (N11).
12. **C5 (Rückfrage), alte Clients und Android.** Der Host muss während der offenen Rückfrage `Ping` beantworten; alter `se shell` endet nach 60 s (N12). Android braucht für die Rückfrage einen Kanal mit höherer Wichtigkeit als `exec` (`IMPORTANCE_LOW`) und eine sichtbare Meldung bei fehlendem `POST_NOTIFICATIONS` (N24); ein Vollbild-Fenster ist für diese App nicht vorgesehen (N21).
13. **K3/R3 (Kontoauswahl Windows).** Konten mit verknüpftem Microsoft-Konto und Entra-Konten bleiben unbelegt bzw. ohne Weg (N1, N19); Windows 11 Home für `NetUserAdd` unbelegt (N17).
