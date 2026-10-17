# Exec unter einem abweichenden Windows-Konto: Faktenrecherche mit Primärquellen

**Stand und Abrufdatum 2026-10-17**

- Auftragsbezug: `docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`, Abschnitt A, Ergebnis E2.
- Art des Dokuments: Faktensammlung ohne Empfehlung und ohne Entscheidung. Bewertung und Auswahl trifft der Hauptagent.
- Jede URL dieses Dokuments wurde am 2026-10-17 abgerufen; Ausnahmen (nur als Suchzusammenfassung gesehen, nicht einzeln gelesen) sind in Quelle S56 ausdrücklich vermerkt. URLs, die HTTP 404 lieferten, sind als solche gekennzeichnet. Der Seitenstand (Metadatum `updated_at` bzw. Seitenangabe) steht im Quellenverzeichnis (Abschnitt 6), soweit die Seite ihn lieferte.
- Es wurde nichts gebaut, kompiliert, installiert, gestartet oder getestet. Lokal gelesen wurden nur der Plan und die vier Dateien `native/src/share/os/windows/exec_conpty.rs`, `exec_supervisor.rs`, `exec.rs`, `exec_handles.rs`.

## 0. Lesehilfe

| Kennzeichen | Bedeutung |
|---|---|
| `[Snn]` | Quelle im Quellenverzeichnis (Abschnitt 6) |
| Belegart P | Microsoft-Dokumentation (Microsoft Learn, Microsoft Support) in der am 2026-10-17 abrufbaren Fassung |
| Belegart M | Microsoft-Archivseite oder archivierter Microsoft-Blog |
| Belegart Q | Quellcode oder Issue eines öffentlichen Projekts (Win32-OpenSSH, Tailscale, microsoft/terminal, PowerShell) |
| Belegart C | Community-Beitrag (Microsoft Q&A, Kommentar in einem Issue); kein Herstellerbeleg |
| Belegart D | README oder Dokumentation eines Drittprojekts |
| Annahme: | Schlussfolgerung des Verfassers; nicht wörtlich belegt |
| Lücke Lnn | fehlender Fakt; genau beschrieben in Abschnitt 5 |
| Widerspruch Wnn | widersprüchliche Angaben; beschrieben in Abschnitt 4 |

Abrufart: Microsoft-Learn-Seiten, Microsoft-Support-Seiten, die Seite des Sysinternals-Tools PsExec und die Seiten des Archivs lieferte das Abrufwerkzeug im Volltext. Microsoft-Q&A-Threads, GitHub-Issues, GitHub-Wiki-Seiten und Quelldateien lieferte das Abrufwerkzeug als Zusammenfassung mit kurzen Zitaten; dort gilt nur das als wörtlich, was in Anführungszeichen steht und als Zitat des Abrufwerkzeugs ausgewiesen ist. Zitate sind englisch und unverändert; API-Namen sind unverändert.

## 1. Ziel und Fragen

### 1.1 Ziel

Der Plan (Ergebnis E2) gibt den Nutzerwunsch wörtlich an: „i would prefer support of setting a user, a different user then se is running as when wished, for the incoming terminal connections so permissions can be handled on the actual operating system“. Gesucht sind die Fakten, mit denen ein Windows-Worker (Prozess im Konto des angemeldeten Benutzers, im Bestand ohne Administratorrechte) eingehende Terminalverbindungen (ConPTY) unter einem Zielkonto ausführen könnte, das vom Konto des Workers abweicht.

### 1.2 Die sieben Fragen

| Nr. | Frage (Kurzfassung des Auftrags) |
|---|---|
| 1 | `LogonUserW` + `CreateProcessAsUserW`: Privilegien `SE_ASSIGNPRIMARYTOKEN_NAME`, `SE_INCREASE_QUOTA_NAME`; Verhalten aus nicht erhöhtem Nutzerprozess, erhöhtem Administratorprozess und LocalSystem-Dienst; `LoadUserProfileW`, `CreateEnvironmentBlock`, Fensterstation-/Desktop-ACLs |
| 2 | `CreateProcessWithLogonW` und `CreateProcessWithTokenW` mit `EXTENDED_STARTUPINFO_PRESENT`/`STARTUPINFOEXW` und `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`; Muster von Projekten, die ConPTY unter einem abweichenden Konto betreiben |
| 3 | Anmeldung ohne Passwort: S4U über `LsaLogonUser` (`MSV1_0_S4U_LOGON`, `KERB_S4U_LOGON`), Bedingungen; Win32-OpenSSH (sshd): Sitzungsstart mit Passwort und mit Schlüssel, Dienstkonto von sshd |
| 4 | Ablage eines Kontopassworts (`CredWriteW`, DPAPI, `LsaStorePrivateData`), Folgen einer Passwortänderung; Microsoft-Konten und Windows-Hello-PIN bei `LogonUserW` |
| 5 | Eigenes lokales Konto anlegen und entfernen (`NetUserAdd`, `NetLocalGroupAddMembers`, `NetUserDel`): Rechte, UAC, Unterschiede Windows Home/Pro |
| 6 | Einschränkung ohne zweites Konto: `CreateRestrictedToken`, AppContainer/LPAC, Integritätsstufe Low, Job-Objekte; Wirkung auf den Dateizugriff einer Shell; Konsolenprogramme im AppContainer |
| 7 | PsExec (`-u`/`-p`), `runas` (`/savecred`), Aufgabenplanung (Task mit gespeichertem Kennwort, `schtasks /RU /RP`), WinRM/PowerShell Remoting: Voraussetzungen |

### 1.3 Bestand (aus den gelesenen Dateien; nur Tatsachen)

- `exec.rs`, Funktion `launch_supervisor`: Der Worker startet mit `CreateProcessW` die eigene Programmdatei (`std::env::current_exe()`, `lpApplicationName`) mit der Befehlszeile `se --share-exec-supervisor`. Parameter: `bInheritHandles = 1`, `dwCreationFlags = EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED | CREATE_NO_WINDOW`, `lpEnvironment = NULL`, `lpCurrentDirectory = NULL`. Attribute: `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (Pipe-Enden für stdin und stdout) und `PROC_THREAD_ATTRIBUTE_JOB_LIST` (Job-Objekt mit `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`). Ein Token wird nicht übergeben; der Supervisor läuft deshalb im Konto des Workers.
- `exec.rs`: `run_as_supported()` liefert `false`; `resolve_account` liefert `io::ErrorKind::Unsupported`. Der Kommentar nennt zwei Voraussetzungen: das Passwort des Zielkontos (`CreateProcessWithLogonW`) oder einen SYSTEM-Dienst mit `SeTcbPrivilege` (S4U-Anmeldung). `process_is_elevated()` liest `TokenElevation`.
- `exec_supervisor.rs`: Bei einer Terminal-Anforderung läuft `run_terminal` → `launch_terminal` → `exec_conpty::start`. Programm: `<Systemverzeichnis>\WindowsPowerShell\v1.0\powershell.exe` mit Befehlszeile `powershell.exe -NoLogo`. Arbeitsverzeichnis: `request.cwd`, sonst `USERPROFILE` aus `start.environment`. Eingabe und Ausgabe des Terminals laufen über zwei Threads und `SupervisorCommand`/`SupervisorEvent` über stdin/stdout des Supervisors.
- `exec_conpty.rs`, Funktion `start`: lädt `CreatePseudoConsole`, `ResizePseudoConsole`, `ClosePseudoConsole` zur Laufzeit aus `kernel32.dll`; erzeugt zwei Pipes ohne `SECURITY_ATTRIBUTES`; ruft `CreatePseudoConsole(size, input_read, output_write, 0, &hpc)`; startet die Shell mit `CreateProcessW` (`bInheritHandles = 0`, `EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT`, `STARTUPINFOEXW` mit `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` = Wert des `HPCON`, `STARTF_USESTDHANDLES` mit `INVALID_HANDLE_VALUE` für alle drei Standardhandles). Die Umgebung ist ein vollständiger UTF-16-Block aus einer `BTreeMap`.
- `exec_handles.rs`: `AttributeList` kapselt `InitializeProcThreadAttributeList`/`UpdateProcThreadAttribute`/`DeleteProcThreadAttributeList`; `OwnedHandle` kapselt `CloseHandle`.
- Folge (Abbildung, keine Empfehlung): Der Prozess, der `CreatePseudoConsole` aufruft und die Shell startet, ist im Bestand der Supervisor-Prozess. Er ist bereits ein eigener Prozess, der die Pseudokonsole öffnet und per stdin/stdout-Pipe mit dem Worker verbunden ist.

## 2. Befunde je Frage

### 2.1 Frage 1: `LogonUserW` + `CreateProcessAsUserW`

#### 2.1.1 Kurzbefund

1. `CreateProcessAsUserW` verlangt laut Seite „typically“ das Privileg `SE_INCREASE_QUOTA_NAME` und „may require“ `SE_ASSIGNPRIMARYTOKEN_NAME`, „if the token is not assignable“. Bei Fehlschlag mit `ERROR_PRIVILEGE_NOT_HELD` (1314) verweist die Seite auf `CreateProcessWithLogonW` [S02].
2. Standardzuweisung der Rechte auf Client-Computern: „Replace a process-level token“ (`SeAssignPrimaryTokenPrivilege`) = Network Service, Local Service (nicht Administrators); „Adjust memory quotas for a process“ (`SeIncreaseQuotaPrivilege`) = Administrators, Local Service, Network Service [S08, S09]. Ein Archivblog von 2013 sagt, `CreateProcessAsUser()` verlange das Recht „which even Administrators do not have by default“ [S15].
3. Das Token eines LocalSystem-Dienstes enthält `SE_ASSIGNPRIMARYTOKEN_NAME` (disabled), `SE_INCREASE_QUOTA_NAME` (disabled), `SE_TCB_NAME` (enabled), `SE_IMPERSONATE_NAME` (enabled), `SE_BACKUP_NAME` (disabled), `SE_RESTORE_NAME` (disabled) [S10]. „If the necessary privileges are not already enabled, CreateProcessAsUser enables them for the duration of the call.“ [S02].
4. Das gefilterte Token eines nicht erhöhten Prozesses enthält laut MSDN Magazine (Januar 2007) genau diese fünf Privilegien: `SeShutdownPrivilege`, `SeChangeNotifyPrivilege`, `SeUndockPrivilege`, `SeIncreaseWorkingSetPrivilege`, `SeTimeZonePrivilege` [S47]. Keines davon ist ein `CreateProcessAsUser`-Privileg.
5. Ausnahme: „If hToken is a restricted version of the caller's primary token, the SE_ASSIGNPRIMARYTOKEN_NAME privilege is not required.“ [S02]; die Seite „Restricted Tokens“ sagt dazu: „This enables ordinary applications to create restricted processes.“ [S46].
6. `LoadUserProfileW` ist nur für Administratoren oder LocalSystem aufrufbar, setzt `SE_RESTORE_NAME` und `SE_BACKUP_NAME` voraus; Impersonation genügt nicht [S05].
7. `CreateProcessAsUserW` lädt weder das Profil noch passt es die Umgebung an [S02]. Benutzerspezifische Variablen entstehen nur bei geladenem Profil; die Seite nennt als Variable `%USERPROFILE%` und gibt keine vollständige Liste der Variablen an [S06].
8. Standardmäßig entsteht der Prozess auf einer nicht interaktiven Fensterstation; interaktiv wird er nur mit `winsta0\default` und passend geänderten DACLs [S02].

#### 2.1.2 `LogonUserW` (Advapi32.dll; Windows XP, Windows Server 2003 und neuer) [S01]

- Pflichtparameter: `lpszUsername`, `dwLogonType`, `dwLogonProvider`, `phToken`. Optional laut Signatur: `lpszDomain`, `lpszPassword`. Signatur siehe Anhang A.
- `lpszDomain`: bei UPN-Form `User@DNSDomainName` im Benutzernamen muss `lpszDomain` `NULL` sein; „If this parameter is NULL, the user name must be specified in UPN format. If this parameter is ".", the function validates the account by using only the local account database.“
- `lpszPassword`: „the plaintext password“; die Seite fordert, es danach mit `SecureZeroMemory` zu löschen.
- `dwLogonType` (alle sieben Werte der Seite):
  - `LOGON32_LOGON_BATCH`: Stapelserver, „processes may be executing on behalf of a user without their direct intervention“; Anmeldeinformationen werden nicht zwischengespeichert (laut der verwandten Seite zu `LogonUserExExW` [S57]).
  - `LOGON32_LOGON_INTERACTIVE`: „has the additional expense of caching logon information for disconnected operations“.
  - `LOGON32_LOGON_NETWORK`: „does not cache credentials“; liefert ein Impersonation-Token, kein Primärtoken; nach `DuplicateTokenEx` in ein Primärtoken kann der neue Prozess „other network resources, such as remote servers or printers, through the redirector“ nicht erreichen, ausgenommen nicht zugriffsgeschützte Ressourcen.
  - `LOGON32_LOGON_NETWORK_CLEARTEXT`: „preserves the name and password in the authentication package“.
  - `LOGON32_LOGON_NEW_CREDENTIALS`: „clone its current token and specify new credentials for outbound connections“; nur mit `LOGON32_PROVIDER_WINNT50`; seit Januar 2023 nicht mit einem Group Managed Service Account nutzbar.
  - `LOGON32_LOGON_SERVICE`: „The account provided must have the service privilege enabled.“
  - `LOGON32_LOGON_UNLOCK`: „GINAs are no longer supported.“
- `dwLogonProvider`: `LOGON32_PROVIDER_DEFAULT` (standardmäßig Negotiate; NTLM, wenn die Domäne `NULL` ist und der Name nicht in UPN-Form steht), `LOGON32_PROVIDER_WINNT50` (Negotiate), `LOGON32_PROVIDER_WINNT40` (NTLM).
- Tokenart: „In most cases, the returned handle is a primary token that you can use in calls to the CreateProcessAsUser function.“ Ausnahme: `LOGON32_LOGON_NETWORK` liefert ein Impersonation-Token, das nur nach `DuplicateTokenEx` in `CreateProcessAsUser` verwendbar ist.
- Rechte des anzumeldenden Kontos: „The account specified by lpszUsername, must have the necessary account rights. For example, to log on a user with the LOGON32_LOGON_INTERACTIVE flag, the user (or a group to which the user belongs) must have the SE_INTERACTIVE_LOGON_NAME account right.“ Die zehn Kontorecht-Konstanten und ihre Bedeutung stehen in [S45] (Abschnitt 2.5.5). `LogonUser` und `LsaLogonUser` „fail if the account being logged on does not have the account rights required for the type of logon being performed“ [S45].
- Privileg des Aufrufers: „The SE_TCB_NAME privilege is not required for this function unless you are logging onto a Passport account.“ (Bedeutung von „Passport account“: Lücke L11.)
- Anmeldestatus: „A user is considered logged on if at least one token exists. If you call CreateProcessAsUser and then close the token, the system considers the user as still logged on until the process (and all child processes) have ended.“
- Fehlercodes: Die Seite nennt keine Fehlercodeliste; sie verweist nur auf `GetLastError`.
- Verwandte Funktion `LogonUserExExW`: nicht in einem öffentlichen Header deklariert, ohne Importbibliothek, Zugriff über `LoadLibrary`/`GetProcAddress` auf `Advapi32.dll`; zusätzlicher Parameter `pTokenGroups` (dafür muss der Aufrufer `SE_TCB_PRIVILEGE` besitzen und aktiviert haben); Windows Vista/Server 2008 und neuer [S57].

#### 2.1.3 `CreateProcessAsUserW` (Advapi32.dll; Windows XP, Windows Server 2003 und neuer) [S02]

- Pflichtparameter laut Signatur: `lpStartupInfo`, `lpProcessInformation`, `bInheritHandles`, `dwCreationFlags`. `hToken` ist in der Signatur `[in, optional]`; die Beschreibung verlangt „the primary token that represents a user“ mit den Zugriffsrechten `TOKEN_QUERY`, `TOKEN_DUPLICATE`, `TOKEN_ASSIGN_PRIMARY`.
- Zugriff auf die Programmdatei: „The user represented by the token must have read and execute access to the application specified by the lpApplicationName or the lpCommandLine parameter.“ Außerdem: „By default, CreateProcessAsUser accesses the directory and executable image in the security context of the caller“; für den Zugriff im Kontext des Zielkontos gilt `ImpersonateLoggedOnUser` vor dem Aufruf.
- Aufruferrecht am eigenen Token: „CreateProcessAsUser must be able to open the primary token of the calling process with the TOKEN_DUPLICATE and TOKEN_IMPERSONATE access rights.“
- Sitzung: „The process is run in the session specified in the token. By default, this is the same session that called LogonUser. To change the session, use the SetTokenInformation function.“ `bInheritHandles`: „You cannot inherit handles across sessions. Additionally, if this parameter is TRUE, you must create the process in the same session as the caller.“
- Umgebung: `lpEnvironment = NULL` bedeutet die Umgebung des Aufrufers; „the USERNAME and USERDOMAIN variables are inherited from the calling process“. Ein Unicode-Block verlangt `CREATE_UNICODE_ENVIRONMENT`.
- Startstruktur: `STARTUPINFO` oder `STARTUPINFOEX`; „To set extended attributes, use a STARTUPINFOEX structure and specify EXTENDED_STARTUPINFO_PRESENT in the dwCreationFlags parameter.“
- Standardsicherheit von Prozess und Thread: „The default security descriptor is that of the user referenced in the hToken parameter. This security descriptor may not allow access for the caller, in which case the process may not be opened again after it is run. The process handle is valid and will continue to have full access rights.“
- Rückgabe: „the function returns before the process has finished initialization. If a required DLL cannot be located or fails to initialize, the process is terminated. To get the termination status of a process, call GetExitCodeProcess.“

#### 2.1.4 Privilegien und Standardzuweisungen

| Privileg (Konstante) | Zeichenfolge | Anzeigename des Rechts | Standardzuweisung auf Client-Computern | Quelle |
|---|---|---|---|---|
| `SE_ASSIGNPRIMARYTOKEN_NAME` | `SeAssignPrimaryTokenPrivilege` | Replace a process-level token | Network Service, Local Service | [S07], [S08] |
| `SE_INCREASE_QUOTA_NAME` | `SeIncreaseQuotaPrivilege` | Adjust memory quotas for a process | Administrators, Local Service, Network Service | [S07], [S09] |
| `SE_TCB_NAME` | `SeTcbPrivilege` | Act as part of the operating system | „Not defined“ für alle Richtlinienvarianten; „If a service requires this user right, configure the service to sign in by using the local System account“ | [S07], [S55] |
| `SE_IMPERSONATE_NAME` | `SeImpersonatePrivilege` | Impersonate a client after authentication | Administrators, Local Service, Network Service, Service | [S07], [S55] |
| `SE_BACKUP_NAME` | `SeBackupPrivilege` | Back up files and directories | nicht gelesen (Lücke L03) | [S07] |
| `SE_RESTORE_NAME` | `SeRestorePrivilege` | Restore files and directories | nicht gelesen (Lücke L03) | [S07] |

Zusatz zur Richtlinie „Replace a process level token“: „Specifically, the Replace a process level token setting determines which user accounts can call the CreateProcessAsUser() application programming interface (API) so that one service can start another. An example of a process that uses this user right is Task Scheduler“ [S08].

#### 2.1.5 Verhalten je Aufrufer

| Aufrufer | Tokeninhalt (belegt) | Folge für `CreateProcessAsUserW` mit dem Token eines fremden Kontos aus `LogonUserW` | Quelle |
|---|---|---|---|
| Nicht erhöhter Nutzerprozess (gefiltertes Token) | `SeShutdownPrivilege`, `SeChangeNotifyPrivilege`, `SeUndockPrivilege`, `SeIncreaseWorkingSetPrivilege`, `SeTimeZonePrivilege` | Weder `SE_INCREASE_QUOTA_NAME` noch `SE_ASSIGNPRIMARYTOKEN_NAME` im Token. Die Seite beschreibt `ERROR_PRIVILEGE_NOT_HELD` (1314) als Fehlerfall und nennt `CreateProcessWithLogonW` als Ausweg. Annahme: Der Aufruf scheitert mit 1314, weil beide Privilegien fehlen. | [S47], [S02] |
| Erhöhter Administratorprozess (volles Token) | Rechtezuweisung „Adjust memory quotas“ enthält Administrators; Rechtezuweisung „Replace a process-level token“ enthält Administrators nicht. Die vollständige Tokenliste des erhöhten Tokens wurde nicht gelesen (Lücke L03). | `SE_ASSIGNPRIMARYTOKEN_NAME` fehlt nach der Standardzuweisung. Annahme: Fehler 1314, sofern das Token nicht „assignable“ ist (Definition: Lücke L01). | [S08], [S09], [S15] |
| LocalSystem-Dienst | siehe [S10]: beide Privilegien vorhanden (disabled), zusätzlich `SE_TCB_NAME`, `SE_IMPERSONATE_NAME` (enabled), `SE_BACKUP_NAME`, `SE_RESTORE_NAME` (disabled) | Privilegien werden für den Aufruf aktiviert; Prozess läuft in der Sitzung des Tokens (Standard: Sitzung des `LogonUser`-Aufrufers); Dienste laufen in Sitzung 0; nicht interaktive Fensterstation als Standard | [S10], [S02], [S58] |

Zusätzlich belegte Eigenschaften des LocalSystem-Kontos (drei Aussagen der Seite) [S10]: „This account does not have a password.“; „HKEY_CURRENT_USER is associated with the default user, not the current user.“; „The service presents the computer's credentials to remote servers.“ `CreateProcessWithLogonW` ist aus diesem Konto nicht aufrufbar (Frage 2, Abschnitt 2.2.2).

UAC und `LogonUserW`: Die aktuelle Seite „How User Account Control works“ (Seitenstand 2026-10-08) sagt, bei der Anmeldung eines Administrators entstünden zwei Tokens; das Standardbenutzer-Token „Contains the same user-specific information as the administrator access token, but the administrative Windows privileges and SIDs are removed“ [S47]. MSDN Magazine (2007) beschreibt, dass UAC bei der interaktiven Anmeldung wirkt (gefiltertes Token plus verknüpftes volles Token) und dass „UAC does not affect service, network, or batch logons“ [S47]. Dass `LogonUserW(LOGON32_LOGON_INTERACTIVE)` für ein Administratorkonto das gefilterte Token liefert, steht auf der `LogonUserW`-Seite nicht (Lücke L04). Annahme: Es folgt aus dem Artikel.

#### 2.1.6 `LoadUserProfileW` (Userenv.dll; Windows 2000 und neuer) [S05]

- Parameter: `hToken` (Token mit `TOKEN_QUERY`, `TOKEN_IMPERSONATE`, `TOKEN_DUPLICATE`, geliefert von `LogonUser`, `CreateRestrictedToken`, `DuplicateToken`, `OpenProcessToken` oder `OpenThreadToken`), `lpProfileInfo` (`PROFILEINFO`, `dwSize` = `sizeof(PROFILEINFO)`, `lpUserName` nicht `NULL`; sonst `ERROR_INVALID_PARAMETER`).
- Aufrufer: „The calling process must have the SE_RESTORE_NAME and SE_BACKUP_NAME privileges.“ „Starting with Windows XP Service Pack 2 (SP2) and Windows Server 2003, the caller must be an administrator or the LocalSystem account. It is not sufficient for the caller to merely impersonate the administrator or LocalSystem account.“
- Wirkung: „When a user logs on interactively, the system automatically loads the user's profile. If a service or an application impersonates a user, the system does not load the user's profile.“ Rückgabe `hProfile` = Registrierungsschlüssel auf die Wurzel des Benutzerhives mit `KEY_ALL_ACCESS`; „Do not close the hProfile handle. Instead, pass it to the UnloadUserProfile function.“ Offene Handles auf Schlüssel des Hives verhindern das Entladen.
- Roaming-Profile: Dienste und Anwendungen sollen den Pfad (`NetUserGetInfo`, Informationsstufe 3 oder 4) in `lpProfilePath` angeben.
- Verknüpfung zu `CreateProcessAsUserW`: Es ist Aufgabe des Aufrufers, den Hive vor dem Aufruf zu laden; `CreateProcessAsUserW` lädt ihn nicht [S02], [S05].

#### 2.1.7 `CreateEnvironmentBlock` (Userenv.dll; Windows 2000 und neuer) [S06]

- Parameter: `lpEnvironment` (Ausgabe: Zeiger auf Block aus null-terminierten Unicode-Zeichenfolgen, Ende mit zwei Nullzeichen), `hToken` (optional; Primärtoken: `TOKEN_QUERY` und `TOKEN_DUPLICATE`; Impersonationstoken: `TOKEN_QUERY`), `bInherit` (Umgebung des Aufrufers erben).
- `hToken = NULL`: „the returned environment block contains system variables only.“
- Weitergabe an `CreateProcessAsUser`: `CREATE_UNICODE_ENVIRONMENT` ist Pflicht; danach `DestroyEnvironmentBlock`.
- „User-specific environment variables such as %USERPROFILE% are set only when the user's profile is loaded. To load a user's profile, call the LoadUserProfile function.“

#### 2.1.8 Fensterstation und Desktop

- Standard bei `CreateProcessAsUserW`: „By default, CreateProcessAsUser creates the new process on a noninteractive window station with a desktop that is not visible and cannot receive user input.“ Für Interaktion: `lpDesktop = "winsta0\default"` und DACL-Änderung beider Objekte; „The DACLs for the window station and desktop must grant access to the user or the logon session represented by the hToken parameter.“ Außerdem: „The user must have full access to both the specified window station and desktop.“ Ist `lpDesktop` `NULL`, erbt der Prozess Fensterstation und Desktop des Elternprozesses [S02].
- Zugriffsrechte der Fensterstation (alle zehn objektspezifischen Werte aus [S11]): `WINSTA_ALL_ACCESS` (0x37F), `WINSTA_ACCESSCLIPBOARD` (0x0004), `WINSTA_ACCESSGLOBALATOMS` (0x0020), `WINSTA_CREATEDESKTOP` (0x0008), `WINSTA_ENUMDESKTOPS` (0x0001), `WINSTA_ENUMERATE` (0x0100), `WINSTA_EXITWINDOWS` (0x0040), `WINSTA_READATTRIBUTES` (0x0002), `WINSTA_READSCREEN` (0x0200), `WINSTA_WRITEATTRIBUTES` (0x0010); dazu die fünf Standardrechte `DELETE`, `READ_CONTROL`, `SYNCHRONIZE` (für Fensterstationen nicht unterstützt), `WRITE_DAC`, `WRITE_OWNER`. „The system assigns noninteractive window stations to all logon sessions other than that of the interactive user.“
- Zugriffsrechte des Desktops (alle neun Werte aus [S13]): `DESKTOP_CREATEMENU` (0x0004), `DESKTOP_CREATEWINDOW` (0x0002), `DESKTOP_ENUMERATE` (0x0040), `DESKTOP_HOOKCONTROL` (0x0008), `DESKTOP_JOURNALPLAYBACK` (0x0020), `DESKTOP_JOURNALRECORD` (0x0010), `DESKTOP_READOBJECTS` (0x0001), `DESKTOP_SWITCHDESKTOP` (0x0100), `DESKTOP_WRITEOBJECTS` (0x0080). „The ACLs in the default security descriptor for a desktop come from its parent window station.“
- Verbindung eines Prozesses mit einer Fensterstation [S12]: erst `SetProcessWindowStation`, dann die vom Elternprozess geerbte, dann der Name in `STARTUPINFO.lpDesktop`, dann die interaktive Fensterstation (nur im Anmeldekontext des interaktiven Benutzers), sonst eine aus der Anmeldesitzungs-ID gebildete Fensterstation; „If the open operation fails because this window station does not exist, the system tries to create the window station and a default desktop.“ Die Verbindung entsteht beim ersten Aufruf einer USER32- oder GDI32-Funktion.
- Archivbeispiel von Microsoft [S14]: `LogonUser(LOGON32_LOGON_INTERACTIVE)`, `GetLogonSID`, `AddAceToWindowStation` (ACE mit `GENERIC_ACCESS`, zweite ACE mit `WINSTA_ALL`) und `AddAceToDesktop` (`DESKTOP_ALL`) für die Logon-SID, `ImpersonateLoggedOnUser`, `CreateProcessAsUser` mit `lpDesktop = winsta0\default` und `CREATE_NEW_CONSOLE`. Der Text sagt: „your process must have the SE_ASSIGNPRIMARYTOKEN_NAME and SE_INCREASE_QUOTA_NAME privileges for successful execution of CreateProcessAsUser“.
- Archivblog von Microsoft (WinSDK, `ms.date` 2013-12-27) [S15]: Die DACL-Änderung der interaktiven Fensterstation/des Desktops hat die Schwäche, dass der Aufrufer für diese DACLs verantwortlich ist: „If the SYSTEM or another process makes a change to the DACLs and is unaware of your changes, your user will not have any more access to the INTERACTIVE desktop.“ Als Technik beschreibt der Blog: einen temporären Prozess mit `CreateProcessWithLogonW` angehalten starten, mit `OpenProcessToken` und `DuplicateTokenEx` das Token übernehmen und später `CreateProcessWithTokenW` aufrufen; der Blog nennt als Begründung gegen `CreateProcessAsUser`: erfordert das Recht „Replace a Process Level Token“.
- Ob ein ConPTY-Kindprozess überhaupt Zugriff auf eine Fensterstation/einen Desktop des Zielkontos benötigt, steht in keiner gelesenen Primärquelle (Lücke L05). Belegt ist nur: Win32-OpenSSH setzt `lpDesktop` nicht (bleibt `NULL`) und startet Konsolen-Shells (Abschnitt 2.3.6) [S33]; in den dort gelesenen Dateien kommt keine Änderung von Fensterstations-/Desktop-ACLs vor (Annahme: Konsolenshells laufen dort ohne ACL-Änderung).

### 2.2 Frage 2: `CreateProcessWithLogonW`, `CreateProcessWithTokenW` und ConPTY

#### 2.2.1 Kurzbefund

1. Dokumentierte Startstruktur: `CreateProcessAsUserW` und `CreateProcessWithTokenW`: „STARTUPINFO or STARTUPINFOEX“; `CreateProcessWithLogonW`: „A pointer to a STARTUPINFO structure.“ [S02], [S03], [S04].
2. Die Seite „Process Creation Flags“ (2025-07-14) führt alle vier Funktionen als Nutzer der Flags und beschreibt `EXTENDED_STARTUPINFO_PRESENT` allgemein (Widerspruch W01) [S22].
3. Ein Microsoft-Q&A-Thread (2022-10) berichtet: `CreateProcessAsUser` mit `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` aus einem SYSTEM-Prozess startet, aber der Kindprozess erhält „access denied“ bei `WriteConsoleInput`; als Ursache wird ein höheres Integritätsniveau der Pseudokonsole gegenüber dem Kind genannt; `CreateProcessWithLogonW` nehme `EXTENDED_STARTUPINFO_PRESENT` nicht an; gelöst wurde es mit einem Broker-Prozess [S20] (Belegart C).
4. Microsofts Konsolen-Dokumentation beschreibt „Wrong-Way Verbs“: Konsolen-Clients mit niedrigerem Integritätsniveau als die angehängte Konsole (und UWP-Konsolen-Apps) dürfen `ReadConsoleOutput`, `ReadConsoleOutputCharacter`, `ReadConsoleOutputAttribute`, `WriteConsoleInput` nicht aufrufen [S19].
5. Zwei Projekte betreiben ConPTY unter einem abweichenden Konto über einen Prozess im Zielkonto: Tailscale (Relay-Prozess) und Win32-OpenSSH (Worker-Prozess im Benutzerkonto, startet `conhost.exe --headless`); der Q&A-Thread beschreibt dieselbe Struktur als Broker-Prozess [S37], [S33], [S20].
6. Die quelloffene `conpty.dll` von microsoft/terminal enthält `ConptyCreatePseudoConsoleAsUser(HANDLE hToken, COORD size, HANDLE hInput, HANDLE hOutput, DWORD dwFlags, HPCON* phPC)`; die In-Box-Funktion `CreatePseudoConsole` in `kernel32.dll` hat keinen Token-Parameter [S16], [S38].
7. Suche nach Issues im Repository microsoft/terminal zu `CreateProcessWithLogonW`/`CreateProcessWithTokenW` oder `CreateProcessAsUser` zusammen mit Pseudokonsole: vier Websuchen ohne passenden Treffer; die Issue-Liste von GitHub selbst wurde nicht durchsucht (die GitHub-API lieferte für Verzeichnislisten HTTP 403, die Suchfunktion von GitHub wurde nicht abgerufen). Gefunden und gelesen wurden stattdessen der Quelltext der `conpty.dll` und die Spezifikation „Process Model 2.0“ (Abschnitte 2.2.10, 2.2.11) (Lücke L06).

#### 2.2.2 `CreateProcessWithLogonW` (Advapi32.dll; Windows XP, Windows Server 2003 und neuer) [S03]

- Pflichtparameter: `lpUsername`, `lpPassword`, `dwLogonFlags`, `dwCreationFlags`, `lpStartupInfo`, `lpProcessInformation`. Optional: `lpDomain`, `lpApplicationName`, `lpCommandLine` (in/out), `lpEnvironment`, `lpCurrentDirectory`. Signatur: Anhang A.
- Privileg des Aufrufers: keines („requires no special privileges“, laut [S02]); das Zielkonto braucht „the Log On Locally permission on the local computer. This permission is granted to all users on workstations and servers, but only to administrators on domain controllers.“
- Aufruf aus LocalSystem: „Windows XP with SP2, Windows Server 2003, or later: You cannot call CreateProcessWithLogonW from a process that is running under the "LocalSystem" account, because the function uses the logon SID in the caller token, and the token for the "LocalSystem" account does not contain this SID. As an alternative, use the CreateProcessAsUser and LogonUser functions.“ Eine verwandte Aussage für `runas` steht in Microsoft Support (KB 325859, `ms.date` 2026-02-12): „The runas command does not function correctly when you run it by using the LocalSystem account.“ [S49].
- `dwLogonFlags`: `LOGON_WITH_PROFILE` (0x00000001; lädt das Profil nach `HKEY_USERS`; „Loading the profile can be time-consuming“), `LOGON_NETCREDENTIALS_ONLY` (0x00000002; gleicher Token wie der Aufrufer, neue Anmeldesitzung nur für ausgehende Verbindungen; Zugangsdaten werden nicht geprüft).
- `dwCreationFlags`: „The CREATE_DEFAULT_ERROR_MODE, CREATE_NEW_CONSOLE, and CREATE_NEW_PROCESS_GROUP flags are enabled by default.“; bei Wert 0: „The process gets the default error mode, creates a new console and creates a new process group.“ Befehlszeile: höchstens 1024 Zeichen (bei `CreateProcessAsUserW`: 32K Zeichen [S02]).
- `lpEnvironment = NULL`: „the new process uses an environment created from the profile of the user specified by lpUsername.“ Fehlen `HOMEDRIVE`/`HOMEPATH`, ergänzt die Funktion sie aus dem Arbeitsverzeichnis des Benutzers.
- Fensterstation und Desktop: „The application must add permission for the specified user account to the specified window station and desktop, even for WinSta0\Default.“ Bei leerem/`NULL` `lpDesktop` gilt dasselbe für die geerbten Objekte; „Windows XP: CreateProcessWithLogonW adds permission for the specified user account to the inherited window station and desktop.“ (Widerspruch W04.)
- Zugriff auf Programmdatei und Arbeitsverzeichnis im Kontext des Zielkontos; Netzlaufwerksbuchstaben sind für das Zielkonto nicht verfügbar, UNC-Pfade schon.
- Dienst: `runas` nennt als Fehlerursache einen nicht laufenden Dienst Secondary Logon [S49]; die Dienstbeschreibung lautet „Enables starting processes under alternate credentials. If you stop this service, this method of signing in becomes unavailable. If you disable this service, any services that explicitly depend on it become unable to start.“, Dienstname `seclogon`, „Installation: Always installed“, Starttyp „Manual“ (Seite für Windows Server 2016, Seitenstand 2026-02-16) [S49].
- Fehlercode für Entra-ID-Konten: `ERROR_LOGON_FAILURE` (1326) auf Windows 10 und Windows 11 (die Seite nennt zusätzlich „or another error code“, ohne den Code zu benennen), wenn `lpUsername` ein Microsoft-Entra-Konto in UPN-Form und `lpDomain` `NULL` ist („Microsoft has confirmed this is a problem in Windows 10 and Windows 11“); Umgehung laut Seite: erst mit `lpDomain = NULL`, bei Fehlschlag erneut mit `lpDomain = "AzureAD"` [S21].
- Zur Startstruktur: `lpStartupInfo` ist als `LPSTARTUPINFOW` (nicht `STARTUPINFOEX`) beschrieben; eine ausdrückliche Aussage über `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` fehlt (Lücke L06).

#### 2.2.3 `CreateProcessWithTokenW` (Advapi32.dll; Windows Vista, Windows Server 2003 und neuer) [S04]

- Aufrufer: „The process that calls CreateProcessWithTokenW must have the SE_IMPERSONATE_NAME privilege.“ Bei `ERROR_PRIVILEGE_NOT_HELD` (1314) verweist die Seite auf `CreateProcessAsUser` oder `CreateProcessWithLogonW`. Standardzuweisung von `SeImpersonatePrivilege` siehe Abschnitt 2.1.4.
- `hToken`: Primärtoken mit `TOKEN_QUERY`, `TOKEN_DUPLICATE`, `TOKEN_ASSIGN_PRIMARY`; Ersatz für Impersonationstoken über `DuplicateTokenEx`.
- Sitzung: „The caller's process always runs in the caller's session, not in the session specified in the token. To run a process in the session specified in the token, use the CreateProcessAsUser function.“
- Startstruktur: „A pointer to a STARTUPINFO or STARTUPINFOEX structure.“ Fensterstation/Desktop: bei leerem/`NULL` `lpDesktop` „The function adds permission for the specified user account to the inherited window station and desktop.“; bei angegebenem Desktop ist die Anwendung verantwortlich.
- Profil/Umgebung: lädt das Profil nur mit `LOGON_WITH_PROFILE` (oder vorherigem `LoadUserProfile`); bei `lpEnvironment = NULL` „an environment block created from the profile of the user specified by lpUsername“ (die Seite verweist auf einen Parameter `lpUsername`, den diese Funktion nicht hat; Widerspruch W02).
- Zuordnung zum Dienst `seclogon`: nur in einem Blog (Pavel Yosifovich, TrainSec, 2024-12-17: „Relies on the Secondary Logon (seclogon) service“; Belegart C) [S59]; Microsoft-Seiten nennen sie nicht ausdrücklich (Lücke L27).

#### 2.2.4 ConPTY-Grundlagen (Microsoft)

- `CreatePseudoConsole(COORD size, HANDLE hInput, HANDLE hOutput, DWORD dwFlags, HPCON* phPC)` liefert `HRESULT`; „Creates a new pseudoconsole object for the calling process.“; Flags: `0` oder `PSEUDOCONSOLE_INHERIT_CURSOR` (1); Handles nur für synchrone E/A; Mindestversion „Windows 10 October 2018 Update (version 1809)“ und „Windows Server 2019“; `Kernel32.dll` [S16].
- Kindstart: „The child process will still be created using the CreateProcess function, but with some additional information“; `STARTUPINFOEX` mit `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` (Wert `hpc`, Größe `sizeof(hpc)`) und `EXTENDED_STARTUPINFO_PRESENT`; nach dem Start sind die an `CreatePseudoConsole` übergebenen Handles im Hostprozess freizugeben; das Beispiel der Seite benutzt `CreateProcessW` mit `bInheritHandles = FALSE` [S17].
- Fehler `0xc0000142`: tritt laut Seite auf, wenn die Sitzung geschlossen wird, während der gehostete Prozess startet, oder wenn dem gehosteten Prozess ein ungültiges Pseudokonsolen-Handle übergeben wird; „To the hosted process initialization code, the two circumstances are identical.“ [S17].
- Schließen: `ClosePseudoConsole` sendet `CTRL_CLOSE_EVENT`; ab Windows 11 24H2 (Build 26100) kehrt die Funktion sofort zurück, frühere Versionen warten, bis die Pseudokonsole beendet ist; nicht im Thread aufrufen, der die Ausgabe liest [S18].
- Rolle von conhost: Das Windows-Blog (Rich Turner, 2018-08-02) beschreibt, dass der Aufruf der API eine ConHost-Instanz startet und Pipes „created by the caller“ verwendet; Aussagen zu einem abweichenden Konto oder zur Erhöhung enthält der Artikel nicht [S60].

#### 2.2.5 Fakten zu Sicherheitsstufen der Konsole

- Microsoft-Seite „Console Buffer Security and Access Rights“ (Seitenstand 2025-08-05) [S19]: „Universal Windows Platform console apps and those with a lower integrity level than the attached console will be prohibited from both reading the output buffer and writing to the input buffer even if the security descriptors above would normally permit it.“ Und: „If either of these scenarios is detected, the console will apply the "wrong-way verbs" flag to the command-line application connection and reject calls to the following APIs“: `ReadConsoleOutput`, `ReadConsoleOutputCharacter`, `ReadConsoleOutputAttribute`, `WriteConsoleInput`. „Rejected calls will receive an access denied error code“.
- Standardsicherheit eines Konsolenpuffers: „The ACLs in the default security descriptor for a console buffer come from the primary or impersonation token of the creator.“ Für `CreatePseudoConsole` existiert kein Parameter für einen Security Descriptor [S16]; der Q&A-Thread nennt das als Mangel [S20].

#### 2.2.6 Q&A-Thread „CreatePseudoConsole with reduced integrity level“ [S20] (Belegart C; Inhalt als Zusammenfassung des Abrufwerkzeugs)

- Fragesteller Corey Minyard (2022-10-09): Programm im SYSTEM-Konto; Schritte: Impersonationstoken des Benutzers erzeugen, `SetThreadToken(NULL, token)`, Pipes anlegen, `CreatePseudoConsole()`, `DuplicateTokenEx()` zum Primärtoken, Attributliste mit `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`, `CreateProcessAsUser` mit dem Primärtoken (übrige Argumente nicht wiedergegeben), `RevertToSelf()`. Ergebnis: alles funktioniert bis auf `access denied` bei `WriteConsoleInput()` im Kindprozess.
- Eigene Deutung des Fragestellers: Die Pseudokonsole entstehe mit höherem Integritätsniveau als das Kind („wrong-way verb“-Problem).
- Antwortgeber RLWA32 (2022-10-09): schlug einen Test mit `CreateProcessWithLogonW` unter den Anmeldedaten eines Standardbenutzers vor, um die Integritätsstufe als Ursache zu prüfen. Fragesteller (2022-10-10): `CreateProcessWithLogonW` akzeptiere `EXTENDED_STARTUPINFO_PRESENT` nicht, eine Pseudokonsole könne also nicht übergeben werden. RLWA32 (2022-10-10): nur `CreateProcessAsUserW` habe ein Argument für Handle-Vererbung; „Only Microsoft knows for sure“, ob `CreateProcessWithTokenW` und `CreateProcessWithLogonW` einen Pseudokonsolen-Handle in der Attributliste bewusst ablehnen; Vorschlag: Broker-Prozess mit mittlerer Integrität, der die Pseudokonsole erstellt, das Ziel startet und die Konsolen-E/A per IPC an den SYSTEM-Prozess weiterleitet; `HPCON` sei kein von `DuplicateHandle` akzeptiertes Handle.
- Ergebnis (2022-10-11): Der Fragesteller baute den Broker-Prozess; „it solved the problem“. Eine Microsoft-Stellungnahme oder Korrektur enthält der Thread nicht.

#### 2.2.7 Muster 1: Tailscale (Relay-Prozess im Zielkonto) [S37] (Belegart Q; Datei `util/winutil/s4u/s4u_windows.go`, Branch `main`)

Kommentar im Quelltext (aus der Abrufausgabe, in Segmenten):

> A note on pseudoconsoles: The conpty API currently does not provide a way to create a pseudoconsole for a different user than the current process. The way we deal with this is to first create a "relay" process running with the desired user token, and then create the actual requested process as a child of the relay, at which time we create the pseudoconsole. The relay simply copies the PTY's I/O into/out of its own stdin and stdout, which are piped to the parent still running as LocalSystem. We also relay pseudoconsole resize requests.

Ablauf laut Quelltext (Zusammenfassung des Abrufwerkzeugs):

1. `Login(logf, srcName, u, capLevel)`: „logs user u into Windows on behalf of service srcName, loads the user's profile, and returns a Session.“ Voraussetzung: „The current OS thread's access token must have SeTcbPrivilege.“ Profil: `winutil.LoadUserProfile`; für `CapCreateProcess` wird das Impersonationstoken in ein Primärtoken dupliziert.
2. `StartProcessWithPTY` setzt `useRelay := ptySizeValid && useToken`. Der Dienst legt Pipes für stdin, stdout, stderr und eine Resize-Pipe an (Leseende vererbbar über `sib.InheritHandles(resizeRead)`), ersetzt die Befehlszeile durch die eigene Programmdatei mit den Argumenten `be-child s4u --resize 0x<Handle-Wert> --x N --y N --cmd <Original-Befehl>`, weist den Prozess einem Job-Objekt mit `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` und `JOB_OBJECT_LIMIT_BREAKAWAY_OK` zu und startet den Relay mit `windows.CreateProcessAsUser` und `opts.token` als Token (Flags `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`; übrige Argumente in der Abrufzusammenfassung nicht wiedergegeben). Umgebung: `token.Environ(false)` (aus dem Token); Arbeitsverzeichnis: Profilordner (`FOLDERID_Profile`), bei erhöhtem Token `FOLDERID_System`.
3. Der Relay (`beRelay`) läuft im Zielkonto, prüft das Resize-Handle (`FILE_TYPE_PIPE`) und die Größe, ruft `conpty.NewPseudoConsole` auf (öffentliche Windows-API `CreatePseudoConsole`; „requires Windows 10 1809 or newer“ [S37]), startet das Zielprogramm als Kind mit Pseudokonsole, kopiert die E/A zwischen Pseudokonsole und eigener stdin/stdout und liest Größenänderungen (`windows.Coord`) aus der Resize-Pipe.
4. Der Dienst verwendet Pipe-Enden des Relay als `Stdin()`/`Stdout()`; `PTYResizer()` schreibt eine `windows.Coord` in die Resize-Pipe; Relay-stderr wird protokolliert.
5. Der Quelltext enthält keinen Hinweis auf den Grund für die Relay-Lösung außer dem oben zitierten Kommentar.

#### 2.2.8 Muster 2: Win32-OpenSSH (Worker im Zielkonto startet `conhost.exe --headless`) [S33], [S34], [S36]

- Architektur laut Wiki „About Win32 OpenSSH and Design Details“ (Zitate aus der Abrufausgabe): „SSHD listener runs as System with TCB privileges (equivalent to root on Linux).“ „The worker acts as a privileged monitor, further spawning an unprivileged child (running as "sshd")“; „unprivileged child exits, and the privileged monitor spawns one more worker“ „running in the context of the authenticated user.“ „launched using CreateProcess(user_token)“.
- Quelltext (`sshd-session.c`, Branch `latestw_all`): `privsep_preauth()` ruft `__posix_spawn_asuser(&pid, options.sshd_auth_path, &actions, NULL, saved_argv, NULL, SSH_PRIVSEP_USER)`; `privsep_postauth()` ruft `__posix_spawn_asuser(&pmonitor->m_pid, argv[0], &actions, NULL, argv, NULL, authctxt->pw->pw_name)`; die Zusammenfassung des Abrufwerkzeugs beschreibt: Der Monitor läuft als SYSTEM, der Nach-Authentifizierungs-Kindprozess läuft im Benutzerkontext und ruft `do_authenticated()`.
- `spawn-ext.c`, `__posix_spawn_asuser`: Token aus `password_auth_token`, sonst `sspi_auth_user`, sonst `get_user_token(user, 1)`; für Benutzer außer `sshd` `load_user_profile(user_token, user)`; Aufruf `posix_spawn_internal(&pid, path, file_actions, attrp, argv, envp, user_token, TRUE, !is_privsep)`.
- `w32fd.c`, `posix_spawn_internal` → `spawn_child_internal`: bei Token `CreateProcessAsUserW(as_user, NULL, t, NULL, NULL, TRUE, flags | CREATE_UNICODE_ENVIRONMENT, lpEnvironment, lpCurrentDirectory, lpStartupInfo, lpProcessInformation)` (die Werte der letzten vier Argumente gibt die Abrufzusammenfassung nicht wieder), sonst `CreateProcessW(NULL, t, NULL, NULL, TRUE, flags, NULL, NULL, &si_ex->StartupInfo, &pi)`; `STARTUPINFOEXW` mit `STARTF_USESTDHANDLES` und `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (`build_inherit_handle_attr_list()`); `lpDesktop` nicht gesetzt (`NULL`); `CreateEnvironmentBlock(&lpEnvironment, as_user, TRUE)`, wenn `load_user_env` gesetzt ist; `DETACHED_PROCESS` für `sshd-session.exe` und `sshd-auth.exe`; `EXTENDED_STARTUPINFO_PRESENT` nur bei vorhandener Handle-Liste.
- `win32_pty.c`: `is_conpty_supported()` prüft, ob `kernel32.dll` den Export `CreatePseudoConsole` hat (Ergebnis in `isConpty` zwischengespeichert). Ist er vorhanden, baut `exec_command_with_pty` die Befehlszeile `%ls\conhost.exe --headless --width %d --height %d --signal 0x%x -- %ls` (Systemverzeichnis aus `GetSystemDirectoryW`; `--signal` = Tty-Handle `ttyh`, vererbbar gemacht mit `SetHandleInformation(ttyh, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT)`) und startet sie mit `CreateProcessW` (`bInheritHandles = TRUE`, Flag `CREATE_NO_WINDOW`), ohne Token. Annahme (Schluss aus `CreateProcessW` ohne Token und dem Nach-Authentifizierungsprozess im Benutzerkontext): Konsolenhost und Shell laufen im Benutzerkonto. Fehlt der Export, startet er `ssh-shellhost.exe ---pty <Befehl>`.
- `shell-host.c` (Rückfall `ssh-shellhost.exe`): kein ConPTY-Aufruf (die Zusammenfassung meldet, dass `CreatePseudoConsole`, `HPCON`, `PROC_THREAD_ATTRIBUTE`, `STARTUPINFOEX` nicht vorkommen); Bildschirmabgriff über `FreeConsole()`, `AttachConsole(pi.dwProcessId)`, `SetWinEventHook`, `ReadConsoleOutput`, `WriteConsoleInputW`; Kindstart mit `CreateProcess` (`bInheritHandles = TRUE`, `dwCreationFlags = CREATE_NEW_CONSOLE`); Größenänderung im Quelltext deaktiviert („For now we disable this logic.“).
- Wiki „TTY PTY support in Windows OpenSSH“ (zuletzt bearbeitet 2021-04-12): „On Windows 10+ and Windows server 2019+, OpenSSH uses the features provided by console.“; „Windows versions before Windows 10 1809 did not have built-in support for a pseudoterminal.“; Rückfall `ssh-shellhost.exe`.
- Issue #1339 (2019-02): Log „This windows OS supports conpty“ gefolgt von der Befehlszeile `C:\Windows\system32\conhost.exe --headless --width 80 --height 24 --signal 0x214 -- <Shell-Befehl>`; Fehlermeldung `exec_command_with_pty - failed to execute <Befehlszeile> error:2` (Mittelteil in der Abrufzusammenfassung nicht wiedergegeben) bei 32-Bit-Build auf 64-Bit-Windows (WOW64-Umleitung); Microsoft-Entwickler Dustin L. Howett erklärt (2019-02-12), das Team habe die Pseudokonsolen-Unterstützung in Win32-OpenSSH aktiviert, bevor die öffentlichen APIs in RS5 bereit waren [S36] (Zusammenfassung des Abrufwerkzeugs).
- Issue #2186 (2023-12): Log „couldn't find CreatePseudoConsole() in kernel32.dll“ → „This windows OS doesn't support conpty“ → `ssh-shellhost.exe ---pty`; Antwort von Tess Gauthier: „Starting with Windows 10 Build 18309, ConPTY should be supported so ssh-shellhost is not needed.“ (Widerspruch W06) [S36].

#### 2.2.9 Muster 3: Broker-Prozess

Beschrieben im Q&A-Thread [S20]: Der SYSTEM-Prozess startet einen Broker mit mittlerer Integrität im Zielkontext; der Broker erzeugt die Pseudokonsole, startet das Ziel und leitet die Konsolen-E/A per IPC weiter; Aufwand laut Fragesteller: viele Daten (Umgebung, Befehlszeile) müssen an den Broker übergeben werden. Zu Muster 1 und 2 besteht strukturelle Übereinstimmung.

#### 2.2.10 Muster 4: `ConptyCreatePseudoConsoleAsUser` der quelloffenen `conpty.dll` [S38]

- microsoft/terminal, `src/winconpty/winconpty.h`: `HRESULT WINAPI ConptyCreatePseudoConsoleAsUser(_In_ HANDLE hToken, _In_ COORD size, _In_ HANDLE hInput, _In_ HANDLE hOutput, _In_ DWORD dwFlags, _Out_ HPCON* phPC);` (ohne Kommentar im Header).
- `src/winconpty/winconpty.cpp`: `_CreatePseudoConsole` ruft `CreateProcessAsUserW` (nicht `CreateProcessW`) mit dem übergebenen `hToken` (`INVALID_HANDLE_VALUE` wird zu `nullptr`; Kommentar: der Aufruf „expects the token to be either valid or null“). Die Befehlszeile des Konsolenhosts enthält `--headless`, `--width`, `--height`, `--signal 0x<Handle-Wert>` und `--server 0x<Handle-Wert>`; `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` begrenzt die Vererbung auf vier Handles (Server-Handle, Eingabe, Ausgabe, Signal-Pipe-Ende des Hosts); `ConptyCreatePseudoConsole` ruft `ConptyCreatePseudoConsoleAsUser(INVALID_HANDLE_VALUE, size, hInput, hOutput, dwFlags, phPC)`.
- `src/winconpty/dll/winconpty.def` exportiert neun `Conpty*`-Funktionen (`ConptyCreatePseudoConsole`, `ConptyCreatePseudoConsoleAsUser`, `ConptyResizePseudoConsole`, `ConptyClosePseudoConsole`, `ConptyClearPseudoConsole`, `ConptyShowHidePseudoConsole`, `ConptyReparentPseudoConsole`, `ConptyReleasePseudoConsole`, `ConptyPackPseudoConsole`) und fünf unpräfixierte Aliase (`CreatePseudoConsole`, `ResizePseudoConsole`, `ClosePseudoConsole`, `ClearPseudoConsole`, `ReleasePseudoConsole`); „New consumers are expected to use the Conpty* versions.“ (Zusammenfassung des Abrufwerkzeugs).
- Commit `cc8faaf` (microsoft/terminal, Autor DHowett, „Force the use of the static pseudoconsole functions in TConn (#3582)“) enthält bereits `ConptyCreatePseudoConsoleAsUser(INVALID_HANDLE_VALUE, size, hInput, hOutput, dwFlags, phPC)`; kein erklärender Kommentar zur `AsUser`-Variante.
- Verteilung: NuGet-Paket `Microsoft.Windows.Console.ConPTY`, Beschreibung „This package allows applications to host Windows Console sessions.“; neueste Version 1.25.260930003 (Stand 2026-10-02); Lizenz MIT; kompatibel „native“ und „uap10.0“; „should work on Windows 10.0.17763.0 and above“ [S38].
- Für die `AsUser`-Variante gibt es keine Microsoft-Learn-Seite; eine durchgängige Erprobung für ein abweichendes Konto ist in den gelesenen Quellen nicht belegt (Lücke L07).

#### 2.2.11 Zusatzbefund: Windows Terminal, Erhöhungsstufen

Die Spezifikation „Process Model 2.0“ von Windows Terminal hält fest: Gemischte Erhöhung in einem Fenster „is not possible to do safely after all“; deshalb „one window for unelevated processes, and another for elevated windows“; je Erhöhungsstufe ein eigener Monarch; zu einem abweichenden Konto trifft die Spezifikation keine Aussage [S53]. Microsofts „sudo für Windows“ (Windows 11 24H2 oder neuer) erhöht nur denselben Benutzer: „At this point in time, the sudo command on Windows does not support running programs as other users.“ [S52].

### 2.3 Frage 3: Anmeldung ohne Passwort (S4U), Win32-OpenSSH

#### 2.3.1 Kurzbefund

1. `LsaLogonUser` verlangt `SeTcbPrivilege` nur, wenn eine Subauthentifizierung benutzt wird, `KERB_S4U_LOGON` mit Anforderung eines Impersonationstokens verwendet wird oder `LocalGroups` nicht `NULL` ist [S23]. `LsaRegisterLogonProcess` verlangt `SeTcbPrivilege` immer (Fehler `STATUS_PORT_CONNECTION_REFUSED`) [S27]; `LsaConnectUntrusted` prüft nichts über den Aufrufer [S28].
2. Ohne `SeTcbPrivilege` ist das S4U-Token laut Microsoft-Blog auf „SecurityIdentification level“ begrenzt: „You can't actually access a secured resource with this token.“ [S29].
3. S4U-Token haben keine Anmeldedaten für entfernte Server: „the token has no credentials to present to a remote server for authentication.“ [S29]; Task Scheduler: „no password is stored by the system and there is no access to either the network or to encrypted files.“ [S31].
4. Kerberos-S4U erfordert laut `LsaLogonUser`-Seite, dass der Aufrufer ein Domänenkonto ist (einschließlich LocalSystem bei Domänenmitgliedschaft) [S23]. `MsV1_0S4ULogon` (Wert 12) ist die Variante des Authentifizierungspakets MSV1_0; sie ist auf Windows XP, Windows Server 2003, Windows Server 2003 SP2 und Windows Vista „not supported“ [S25]. Eine Learn-Seite zur Struktur `MSV1_0_S4U_LOGON` existiert nicht (HTTP 404, Lücke L08).
5. Win32-OpenSSH: sshd läuft als LocalSystem mit TCB-Privileg; Passwortanmeldung über `LogonUserExExW` mit `dwLogonType = LOGON32_LOGON_NETWORK_CLEARTEXT` und `dwLogonProvider = LOGON32_PROVIDER_DEFAULT`; Schlüsselanmeldung ohne Passwort über S4U (`generate_s4u_user_token`); die Sitzung läuft als `CreateProcessAsUserW` mit dem Benutzertoken [S33], [S34].
6. Sitzungen nach Schlüsselanmeldung haben keine Anmeldedaten („A remote session opened via key based authentication does not have associated user credentials“), nach Passwortanmeldung schon [S34].

#### 2.3.2 `LsaLogonUser` (Secur32.dll; Windows XP, Windows Server 2003 und neuer) [S23]

- Pflichtparameter: alle vierzehn Parameter der Signatur (Anhang A) bis auf `LocalGroups` (`[in, optional]`).
- `LsaHandle` kommt aus `LsaRegisterLogonProcess`; „If SeTcbPrivilege is not required, call LsaConnectUntrusted to obtain the handle.“ Bedingungen für `SeTcbPrivilege`: (a) Subauthentifizierungspaket, (b) `KERB_S4U_LOGON` mit Anforderung eines Impersonationstokens, (c) `LocalGroups` ungleich `NULL`.
- Tokenart nach `LogonType`: „If LogonType is Interactive or Batch, a primary token is generated to represent the new user. If LogonType is Network, an impersonation token is generated.“
- `KERB_S4U_LOGON`-Bedingungen (wörtlich): „The caller must be a domain account (this includes LOCAL_SYSTEM if the computer is a domain member). If using a service account, the account must have SeTcbPrivilege set on the local computer to get an impersonation token. Otherwise, the identity token is used. The caller must be a member of the Pre-Windows 2000 Compatible Access or have read access to the group memberships of the client.“ Die Puffer `ClientUpn` und `ClientRealm` müssen unmittelbar hinter der Struktur im Speicher liegen.
- Rückgabewerte (NTSTATUS): `STATUS_QUOTA_EXCEEDED`, `STATUS_ACCOUNT_RESTRICTION`, `STATUS_BAD_VALIDATION_CLASS`, `STATUS_LOGON_FAILURE`, `STATUS_NO_LOGON_SERVERS`, `STATUS_NO_SUCH_PACKAGE`, `STATUS_PKINIT_FAILURE`, `STATUS_PKINIT_CLIENT_FAILURE`; `SubStatus` (MSV1_0): `STATUS_INVALID_LOGON_HOURS`, `STATUS_INVALID_WORKSTATION`, `STATUS_PASSWORD_EXPIRED`, `STATUS_ACCOUNT_DISABLED`.
- Die Tabelle der Authentifizierungsstrukturen nennt `KERB_S4U_LOGON` (Kerberos), aber keine MSV1_0-S4U-Struktur (Lücke L08).

#### 2.3.3 S4U-Strukturen und Konstanten

- `KERB_S4U_LOGON` [S24] (Windows Server 2003 als Mindestversion; Spalte „Minimum supported client“: „None supported“): Felder `MessageType` (muss `KerbS4ULogon` sein), `Flags`, `ClientUpn` (nicht `NULL`), `ClientRealm` (kann `NULL` sein). `Flags`: `KERB_S4U_LOGON_FLAG_CHECK_LOGONHOURS` (0x2; „Requests the hours that the user has been logged on.“), `KERB_S4U_LOGON_FLAG_IDENTIFY` (0x8; „Requests the identity token.“).
- `MSV1_0_LOGON_SUBMIT_TYPE` [S25]: `MsV1_0S4ULogon = 12`: „Requests a service for user (S4U) logon.“; nicht unterstützt auf Windows Server 2003 mit SP2, Windows Vista, Windows Server 2003, Windows XP (Aussage zu Windows 7 und neuer ergibt sich nur durch Auslassung; Lücke L25).
- `MSV1_0_S4U_LOGON` [S26] (von Microsoft erzeugte Bindings-Dokumentation für Rust, keine Learn-Seite): Felder `MessageType: MSV1_0_LOGON_SUBMIT_TYPE`, `Flags: u32`, `UserPrincipalName: LSA_UNICODE_STRING`, `DomainName: LSA_UNICODE_STRING`. Flag-Werte: nicht gelesen (Lücke L08). Tailscale nennt für beide Strukturen `CHECK_LOGONHOURS` = `0x2` [S37].
- Task Scheduler: `TASK_LOGON_S4U` (Wert 2): „The service will log the user on using Service For User (S4U), and the task will run in a non-interactive desktop. When an S4U logon is used, no password is stored by the system and there is no access to either the network or to encrypted files.“ [S31] (Windows Vista, Windows Server 2008 und neuer).

#### 2.3.4 `LsaRegisterLogonProcess` und `LsaConnectUntrusted`

- `NTSTATUS LsaRegisterLogonProcess(PLSA_STRING LogonProcessName, PHANDLE LsaHandle, PLSA_OPERATIONAL_MODE SecurityMode)` [S27]: Name höchstens 127 Bytes (`STATUS_NAME_TOO_LONG`); Rückgabe `STATUS_PORT_CONNECTION_REFUSED`: „The caller does not have the SeTcbPrivilege privilege, which is required to call this function.“ Die Seite verweist auf `LsaAddAccountRights` zum Setzen des Privilegs. „It also opens the application's process for PROCESS_DUP_HANDLE access in anticipation of future LSA authentication calls.“
- `NTSTATUS LsaConnectUntrusted(PHANDLE LsaHandle)` [S28]: „does not verify any information about the caller“; das Handle ist laut Seite in Aufrufen von `LsaCallAuthenticationPackage` und `LsaLookupAuthenticationPackage` verwendbar („If your application simply needs to query information from authentication packages“); „Applications with the SeTcbPrivilege privilege may create a trusted connection by calling LsaRegisterLogonProcess.“
- Standardzuweisung von `SeTcbPrivilege`: keinem Konto zugewiesen („Not defined“); „it shouldn't even be assigned to the Administrators group under typical circumstances“ [S55].

#### 2.3.5 Identifikations- gegenüber Impersonationstoken, Netzwerkzugriff

- Archivblog „Logon as a user without a password“ (WinSDK, 2015-08-28) [S29]: „The caller needs to have the SeTcbPrivilege to generate an SecurityImpersonation level token. If the caller doesn't have this privilege, the resultant token will be at a SecurityIdentification level. An Identification level token only allows you to get the identity and privileges of the user to Access Check against a secured resource. You can't actually access a secured resource with this token.“ Und: „The token can only be used locally.“ Auch nach Weitergabe an `CreateProcessAsUser` „the process can only access secured resources locally“. Der Blog beschreibt Kerberos-S4U für Domänenbenutzer, nicht die MSV1_0-Variante.
- .NET-Weg des Blogs: `new WindowsIdentity("user@domain")`; ohne `SeTcbPrivilege` ist das innere Token ein Identification-Token.
- Drittprojekt `rgl/s-4-u-for-windows` (README; Belegart D) [S32]: Demo startet `cmd.exe` im Kontext lokaler oder Domänenkonten ohne Passwort über die S4U-Erweiterung von MSV1_0. Geforderte Rechte „invoking user“: `SeTcbPrivilege`, `SeIncreaseQuotaPrivilege`, `SeAssignPrimaryTokenPrivilege`; „invoked user“: `SeBatchLogonRight`.

#### 2.3.6 Win32-OpenSSH: Sitzungsstart und Konto

Quelle: Quelltext Branch `latestw_all` (Zusammenfassungen des Abrufwerkzeugs) und Wiki [S33], [S34].

- Dienstkonto: Wiki „About Win32 OpenSSH and Design Details“: „SSHD listener runs as System with TCB privileges (equivalent to root on Linux).“ Datum der letzten Bearbeitung dieser Wikiseite: nicht erfasst.
- `win32_usertoken_utils.c` (29516 Bytes), Funktionen und aufgerufene APIs (laut Zusammenfassung):
  - `generate_s4u_user_token`: `LsaRegisterLogonProcess` (Modus „trusted mode - used for impersonation“) oder `LsaConnectUntrusted` („untrusted mode - used for information lookup“), `LsaLookupAuthenticationPackage`, `AllocateLocallyUniqueId`, `LsaLogonUser`, `LsaDeregisterLogonProcess`, `LsaFreeReturnBuffer`; `use_kerberos = domain_user && is_domain_joined_machine()` bestimmt `MICROSOFT_KERBEROS_NAME_A` (Struktur `KERB_S4U_LOGON`) gegenüber `MSV1_0_PACKAGE_NAME` (Struktur `MSV1_0_S4U_LOGON`, Domänenpräfix entfernt); Flags `0x0` für Impersonation, `0x8` sonst.
  - `windows_password_auth`: `pLogonUserExExW` mit `LOGON32_LOGON_NETWORK_CLEARTEXT` und `LOGON32_PROVIDER_DEFAULT`; Rückfall `process_custom_lsa_auth` (benutzerdefiniertes LSA-Paket, Name aus Registrierungswert `LSAAuthenticationPackage`); `lookup_principal_name` wandelt `DOMAIN\sam` in UPN.
  - `get_user_token`: Verteiler; ruft `generate_s4u_user_token`, Rückfall `pLogonUserExExW`.
  - `load_user_profile` / `unload_user_profile`: `LoadUserProfileW` mit `PI_NOUI`, `SeBackupPrivilege` und `SeRestorePrivilege` werden um den Aufruf aktiviert (`EnablePrivilege`).
  - `generate_sshd_virtual_token`: virtuelles Konto für den Vor-Authentifizierungsprozess (SIDs unter `S-1-5-111`), `LsaAddAccountRights` für `SeServiceLogonRight`, `LogonUserExExW` mit `LOGON32_LOGON_SERVICE` und `LOGON32_PROVIDER_VIRTUAL`, `CreateRestrictedToken` mit `ExistingTokenHandle = va_token` und `Flags = DISABLE_MAX_PRIVILEGE`.
  - `SeTcbPrivilege` wird in dieser Datei nicht aktiviert; es erscheint nur in der Fehlermeldung „ensure sshd service has TCB privileges“.
  - Für Microsoft-/AAD-Konten gibt es in dieser Datei keine Sonderbehandlung.
- Sitzungsstart (Abschnitt 2.2.8): Monitor (SYSTEM) startet den Nach-Authentifizierungsprozess mit `CreateProcessAsUserW` und `hToken = user_token`; dieser Prozess startet die Shell (mit PTY: `conhost.exe --headless` oder `ssh-shellhost.exe`; ohne PTY: `posix_spawn` ohne zusätzlichen Token-Wechsel).
- Wiki „SSH remote sessions on Windows“ (zuletzt bearbeitet 2017-05-16): „A remote session opened via password authentication has the user credentials attached to it“; „A remote session opened via key based authentication does not have associated user credentials“; Letzteres sei „by design and goes by the rules of standard Windows security“.
- Wiki „Design Details“: „End result of authentication in Windows is a Windows user token (if authentication succeeds).“; „If user profile does not exist, it gets created upon first logon via ssh.“
- Konfiguration (Microsoft Learn, OpenSSH-Serverkonfiguration, Seitenstand 2025-08-05) [S35]: „For Windows OpenSSH, the only available authentication methods are password and publickey.“; „Authentication via a Microsoft Entra account isn't currently supported.“; „Work group users, groups, and internet-connected accounts are always resolved to their local account name (no domain part, similar to standard Unix names).“; Administratoren verwenden `%programdata%/ssh/administrators_authorized_keys` (nur `NT Authority\SYSTEM` und `BUILTIN\Administrators` dürfen Einträge in der ACL haben, SYSTEM mit Vollzugriff); Standardshell über `HKLM\SOFTWARE\OpenSSH\DefaultShell`; nicht verfügbare Optionen (vollständig): `AcceptEnv`, `AllowStreamLocalForwarding`, `AuthorizedKeysCommand`, `AuthorizedKeysCommandUser`, `AuthorizedPrincipalsCommand`, `AuthorizedPrincipalsCommandUser`, `ExposeAuthInfo`, `GSSAPICleanupCredentials`, `GSSAPIStrictAcceptorCheck`, `HostbasedAcceptedKeyTypes`, `HostbasedAuthentication`, `HostbasedUsesNameFromPacketOnly`, `IgnoreRhosts`, `IgnoreUserKnownHosts`, `KbdInteractiveAuthentication`, `KerberosAuthentication`, `KerberosGetAFSToken`, `KerberosOrLocalPasswd`, `KerberosTicketCleanup`, `PermitTunnel`, `PermitUserEnvironment`, `PermitUserRC`, `PidFile`, `PrintLastLog`, `PrintMotd`, `RDomain`, `StreamLocalBindMask`, `StreamLocalBindUnlink`, `StrictModes`, `X11DisplayOffset`, `X11Forwarding`, `X11UseLocalhost`, `XAuthLocation`.
- Verfügbarkeit (Übersichtsseite, `ms.date` 2025-02-20) [S35]: „Beginning with Windows 10 build 1809 and Windows Server 2019, OpenSSH is available as a feature on demand.“; Installationsstand: Windows Server 2019 und 2022 „Not installed“, Windows Server 2025 „Installed but not enabled“, Windows 10 ab 1809 „Not installed, install and enable using optional features“.

#### 2.3.7 Tailscale als zweite Referenz für S4U unter LocalSystem [S37]

- Paketkommentar `s4u`: „Package s4u is an API for accessing Service-For-User (S4U) functionality on Windows.“ Aufrufer: LocalSystem mit `SeTcbPrivilege`.
- `lsa_windows.go`: `newLSASessionForQuery` → `lsaConnectUntrusted`; `newLSASessionForLogon` → `lsaRegisterLogonProcess`; `getAuthPkgID` → `lsaLookupAuthenticationPackage` (Kerberos oder MSV1_0, zwischengespeichert); `logonAs` → `lsaLogonUser`. `_MSV1_0_S4U_LOGON` für lokale und Nicht-Domänenkonten, `_KERB_S4U_LOGON` für Domänenkonten („domain logons fail with `os.ErrInvalid` unless the machine is domain-joined“). Logon-Typ fest `_Network`. `CHECK_LOGONHOURS` (0x2) nur bei `capLevel == CapCreateProcess`; `_KERB_S4U_LOGON_FLAG_IDENTIFY` (0x8) definiert, nicht gesetzt. Herkunftsname (`srcName`) höchstens 8 ASCII-Zeichen.
- `s4u_windows.go`: Fähigkeitsstufen `CapImpersonateOnly` (nur `Do`) und `CapCreateProcess` (`Do` und `StartProcess*`); `Do(fn)` führt `fn` unter Impersonation aus; Prozesse erhalten Job-Objekt mit `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` und `JOB_OBJECT_LIMIT_BREAKAWAY_OK`.

### 2.4 Frage 4: Ablage eines Kontopassworts, Microsoft-Konten, PIN

#### 2.4.1 Kurzbefund

1. Credential Manager (`CredWriteW`): der Eintrag gehört zur Anmeldesitzung des aktuellen Tokens; Blobgröße höchstens `CRED_MAX_CREDENTIAL_BLOB_SIZE` (5*512 = 2560 Bytes); `CRED_TYPE_GENERIC`-Blobs sind anwendungsdefiniert [S39].
2. DPAPI (`CryptProtectData`): Entschlüsselung im Normalfall nur durch Benutzer mit denselben Anmeldedaten auf demselben Computer; Flag `CRYPTPROTECT_LOCAL_MACHINE` erlaubt jedem Benutzer des Computers die Entschlüsselung [S40].
3. LSA-Secrets (`LsaStorePrivateData`): Zugriffsrecht `POLICY_CREATE_SECRET` am Policy-Handle; Lesen nur durch Ersteller und Administratoren; Microsoft rät von der Nutzung für allgemeine Verschlüsselung ab [S41].
4. Passwortänderung: Siehe Tabelle 2.4.5.
5. Microsoft-Konto, PIN und `LogonUserW`: kein Primärbeleg gefunden (Lücken L11, L12); belegt sind nur die Randfakten in 2.4.6.

#### 2.4.2 `CredWriteW` und `CREDENTIALW` (Advapi32.dll; Windows XP, Windows Server 2003 und neuer) [S39]

- Signatur: `BOOL CredWriteW(PCREDENTIALW Credential, DWORD Flags)`; Flag `CRED_PRESERVE_CREDENTIAL_BLOB` (Blob des vorhandenen Eintrags mit gleichem Namen und Typ bleibt erhalten, `CredentialBlobSize` muss 0 sein).
- Zuordnung: „The new credential is associated with the logon session of the current token. The token must not have the user's security identifier (SID) disabled.“
- Fehlercodes: `ERROR_NO_SUCH_LOGON_SESSION` („Network logon sessions do not have an associated credential set.“), `ERROR_INVALID_PARAMETER` (Feld eines vorhandenen Eintrags, das nicht geändert werden darf), `ERROR_INVALID_FLAGS`, `ERROR_BAD_USERNAME`, `ERROR_NOT_FOUND` (nur mit `CRED_PRESERVE_CREDENTIAL_BLOB`), `SCARD_E_NO_READERS_AVAILABLE`, `SCARD_E_NO_SMARTCARD`, `SCARD_W_REMOVED_CARD`, `SCARD_W_WRONG_CHV` (die vier Chipkartencodes nur für `CRED_TYPE_CERTIFICATE`).
- Typen: `CRED_TYPE_GENERIC` (1): „The credential will be stored securely but has no other significant characteristics.“; `CRED_TYPE_DOMAIN_PASSWORD` (2): wird von NTLM, Kerberos und Negotiate beim Verbinden mit dem Ziel automatisch verwendet; „this member can only be read by the authentication packages.“
- Persistenz (`Persist`): `CRED_PERSIST_SESSION` (1; nur für die Dauer der Anmeldesitzung, nicht sichtbar für Anmeldesitzungen desselben Benutzers, die nicht die schreibende Sitzung sind), `CRED_PERSIST_LOCAL_MACHINE` (2; für alle späteren Anmeldesitzungen auf diesem Computer; „Windows Vista Home Basic, Windows Vista Home Premium, Windows Vista Starter and Windows XP Home Edition: This value is not supported.“), `CRED_PERSIST_ENTERPRISE` (3; zusätzlich sichtbar für Anmeldesitzungen dieses Benutzers auf Computern, die nicht dieser Computer sind; „can be implemented as locally persisted credential if the administrator or user configures the user account to not have roam-able state“; dieselbe Editionsaussage).
- Größen: `CRED_MAX_CREDENTIAL_BLOB_SIZE` (5*512 Bytes), `CRED_MAX_GENERIC_TARGET_NAME_LENGTH` (32767 Zeichen), `CRED_MAX_USERNAME_LENGTH` (513), `CRED_MAX_ATTRIBUTES` (64).
- Credential Manager laut Windows-Authentifizierungsdokumentation: „Credentials are saved in special encrypted folders on the computer under the user's profile.“ [S42].
- Zugriff aus einem LocalSystem-Dienst auf den Credential Manager: nicht dokumentiert (Lücke L13).

#### 2.4.3 DPAPI (`CryptProtectData`, Crypt32.dll; Windows XP, Windows Server 2003 und neuer) [S40]

- Signatur: `BOOL CryptProtectData(DATA_BLOB* pDataIn, LPCWSTR szDataDescr, DATA_BLOB* pOptionalEntropy, PVOID pvReserved, CRYPTPROTECT_PROMPTSTRUCT* pPromptStruct, DWORD dwFlags, DATA_BLOB* pDataOut)`; Flags `CRYPTPROTECT_LOCAL_MACHINE`, `CRYPTPROTECT_UI_FORBIDDEN`, `CRYPTPROTECT_AUDIT`. Der Prompt-gesteuerte Ablauf (`pPromptStruct`) ist veraltet und wird im Februar 2027 entfernt; „operations on data originally protected with the PromptStruct flow will fail.“ (Seitenstand 2026-05-15).
- Wirkung: „Typically, only a user with logon credentials that match those of the user who encrypted the data can decrypt the data. In addition, decryption usually can only be done on the computer where the data was encrypted. However, a user with a roaming profile can decrypt the data from another computer on the network.“ Bei `CRYPTPROTECT_LOCAL_MACHINE` „any user on the computer where the encryption was done can decrypt the data.“
- Technischer Hintergrund (Archivartikel „Windows Data Protection“, NAI Labs, Oktober 2001, für Windows XP) [S40]: Der Schlüssel (MasterKey) wird mit einem aus dem Anmeldepasswort abgeleiteten Schlüssel geschützt (PBKDF2, 4000 Iterationen) und im Benutzerprofil gespeichert; MasterKeys laufen nach drei Monaten ab, bleiben aber erhalten. „A small drawback to using the logon password is that all applications running under the same user can access any protected data that they know about.“; zusätzliche Entropie (`pOptionalEntropy`) erschwert das nur.
- DPAPI läuft über einen lokalen RPC-Aufruf in die LSA [S40].

#### 2.4.4 LSA-Secrets (`LsaStorePrivateData`, Advapi32.dll; Windows XP, Windows Server 2003 und neuer) [S41]

- Signatur: `NTSTATUS LsaStorePrivateData(LSA_HANDLE PolicyHandle, PLSA_UNICODE_STRING KeyName, PLSA_UNICODE_STRING PrivateData)`.
- Die Seite beginnt: „Do not use the LSA private data functions or generic data encryption and decryption. Instead, use the CryptProtectData and CryptUnprotectData functions. Only use the LSA private data functions when it is necessary to manipulate LSA secrets“.
- Zugriff: Der Policy-Handle braucht `POLICY_CREATE_SECRET`, wenn unter `KeyName` erstmals Daten abgelegt werden. „The function encrypts this data before storing it.“ `PrivateData = NULL` löscht Daten und Schlüssel. „the key has a DACL that allows only the creator and administrators to read the data.“
- Namenspräfixe: `L$` (lokal, nicht aus der Ferne zugreifbar), `G$` (global), `M$` (Computer, „can be accessed only by the operating system“).
- „The LsaStorePrivateData function can be used by server applications to store client and machine passwords.“
- Windows-Authentifizierungsdokumentation [S42]: „Credentials stored as LSA secrets might include: Account password for the computer's Active Directory Domain Services (AD DS) account. Account passwords for Windows services that are configured on the computer. Account passwords for configured scheduled tasks. Account passwords for IIS application pools and websites. Passwords for Microsoft accounts.“ LSA-Secrets seien „accessible only to SYSTEM account processes“.

#### 2.4.5 Folgen einer Passwortänderung

| Speicher | Folge bei Änderung des Passworts des Kontos | Quelle |
|---|---|---|
| Geplante Aufgabe mit gespeichertem Kennwort (`schtasks /RP`, `TASK_LOGON_PASSWORD`) | „if the password for an account changes or expires, and you don't change the password saved in the task, then the task won't run.“ Das Task-Passwort liegt als LSA-Secret („Account passwords for configured scheduled tasks“). | [S50], [S42] |
| `TASK_LOGON_S4U` / `schtasks /NP` | Es wird kein Passwort gespeichert („No password is stored.“) | [S31], [S50] |
| DPAPI im Benutzerumfang, Passwortänderung durch den Benutzer | Archivartikel 2001: DPAPI hängt sich in das Passwortänderungsmodul ein, „all MasterKeys are re-encrypted under the new password“; zusätzlich Datei „Credential History“ mit alten Passwörtern. Die Passwort-Reset-Diskette verschlüsselt die MasterKeys bei einem Reset neu. | [S40] |
| DPAPI im Benutzerumfang, Passwort administrativ zurückgesetzt oder von einem fremden Gerät aus zurückgesetzt („reset from another device“) | „the previously encrypted data can no longer be decrypted using the new keys derived from the user's new password“ (Seite zu den DPAPI-Sicherungsschlüsseln der Active-Directory-Domänencontroller, ms.date 2023-06-02, Seitenstand 2025-04-15; Wiederherstellung dort über die Domänensicherungsschlüssel; ein Äquivalent für lokale Konten nennt die Seite nicht) | [S40] |
| DPAPI mit `CRYPTPROTECT_LOCAL_MACHINE` | unabhängig vom Benutzerkonto („any user on the computer where the encryption was done can decrypt“) | [S40] |
| Credential-Manager-Eintrag (`CredWriteW`) und LSA-Secret (`LsaStorePrivateData`) | In den gelesenen Seiten steht keine Aussage zur automatischen Anpassung eines gespeicherten Passwortwertes. Annahme: Der gespeicherte Wert bleibt das alte Passwort, weil die Funktionen nur einen vom Aufrufer gelieferten Blob ablegen (Lücke L14). | [S39], [S41] |
| Windows-Hello-PIN | Kein Passwort; „the PIN never leaves the device“ | [S43] |

#### 2.4.6 Microsoft-Konten und Windows-Hello-PIN

- `LogonUserW`-Seite: einziger Bezug: „The SE_TCB_NAME privilege is not required for this function unless you are logging onto a Passport account.“ Die Seite erläutert den Begriff nicht (Lücke L11) [S01].
- Microsoft Support „Change or reset your Windows password“: Die Seite unterscheidet „If you sign in to Windows with a Microsoft account password“ und „If you sign in to Windows with a PIN“ und nennt für das Ändern eines Microsoft-Konto-Passworts „You will need to enter your old password to create a new one“ sowie das Zurücksetzen über einen Browser [S43].
- Microsoft Support „Go passwordless in Windows“ (Windows 10 und Windows 11): Option „For improved security, only allow Windows Hello sign-in for Microsoft accounts on this device“ (Windows 10: „Require Windows Hello sign-in for Microsoft accounts“), nur verfügbar bei Anmeldung mit Microsoft-Konto; Wirkung: „The next time you sign in to your device, you won't have the option to use your password.“ „Only the Windows Hello face, fingerprint, or PIN gestures will be available.“ [S43]. Wirkung auf API-Aufrufe wie `LogonUserW`: nicht beschrieben (Lücke L11).
- Windows-Hello-Übersicht (Seitenstand 2026-10-08): „Windows Hello can also be used with local accounts for convenient sign-ins, instead of entering a password.“; „the PIN never leaves the device“; Windows Hello for Business: unterstützt für Windows Pro, Enterprise, Pro Education/SE und Education (Home nicht aufgeführt) [S43]. Dass eine PIN von `LogonUserW` angenommen wird, ist nicht belegt (Lücke L12).
- Entra-Konten: `CreateProcessWithLogonW` kann mit `ERROR_LOGON_FAILURE` (1326) scheitern (Abschnitt 2.2.2) [S21]; OpenSSH für Windows: „Authentication via a Microsoft Entra account isn't currently supported.“ [S35].
- Community-Aussagen zu SSH und Microsoft-Konten (Belegart C): Ein Microsoft-Q&A-Thread (2024-02-08) sagt, der OpenSSH-Server „is not intended to work with Microsoft accounts“ und unterstütze nur lokale oder Active-Directory-Konten; Gegenaussagen siehe Widerspruch W09 [S56].

### 2.5 Frage 5: Eigenes lokales Konto anlegen und entfernen

#### 2.5.1 Kurzbefund

1. `NetUserAdd`, `NetLocalGroupAddMembers` und `NetUserDel` können auf Servern und Arbeitsstationen standardmäßig nur von Administrators und Power Users aufgerufen werden [S44].
2. Die Seiten nennen keine UAC-Erhöhung (Lücke L16). Das gefilterte Token enthält die Gruppe Administrators nur als „Group used for deny only“ [S47] (Annahme: Der Aufruf braucht einen erhöhten Prozess).
3. Die Seiten nennen keine Editionsbeschränkung; Mindestversion: Windows 2000 Professional/Server [S44]. Aussagen zu Windows Home beruhen auf Community-Beiträgen (Lücke L15).
4. Das Entfernen mit `NetUserDel` löscht das Konto; die Seite sagt nichts über den Profilordner (Lücke L24); `DeleteProfileW` löscht Profil und Einstellungen, Aufrufer braucht Administratorrechte [S44], [S45].

#### 2.5.2 `NetUserAdd` (Netapi32.dll; Windows 2000 Professional/Server und neuer) [S44]

- Signatur: `NET_API_STATUS NetUserAdd(LPCWSTR servername, DWORD level, LPBYTE buf, LPDWORD parm_err)`; `servername = NULL` heißt lokaler Computer; `level`: 1 (`USER_INFO_1`), 2 (`USER_INFO_2`), 3 und 4 (nur Server; Stufe 4 nicht unter Windows 2000).
- Aufrufer: „On a member server or workstation, only Administrators and Power Users can call this function.“ (Bei Domänencontrollern gilt die ACL; Standard: Domain Admins und Account Operators.) „The security descriptor of the user container is used to perform the access check for this function. The caller must be able to create child objects of the user class.“
- Rückgabewerte: `NERR_Success`, `ERROR_ACCESS_DENIED`, `NERR_InvalidComputer`, `NERR_NotPrimary`, `NERR_GroupExists`, `NERR_UserExists`, `NERR_PasswordTooShort` („The password could also be too long, be too recent in its change history, not have enough unique characters, or not meet another password policy requirement.“).
- `USER_INFO_1`: `usri1_name`; `usri1_password` (Länge höchstens `PWLEN` Bytes; Konvention `LM20_PWLEN` Zeichen); `usri1_priv` (bei `NetUserAdd` muss `USER_PRIV_USER` sein); `usri1_home_dir`; `usri1_comment`; `usri1_flags` (`UF_SCRIPT` „This value must be set.“; alle übrigen Werte der Seite: `UF_ACCOUNTDISABLE`, `UF_PASSWD_NOTREQD`, `UF_PASSWD_CANT_CHANGE`, `UF_DONT_EXPIRE_PASSWD`, `UF_LOCKOUT`, `UF_ENCRYPTED_TEXT_PASSWORD_ALLOWED`, `UF_NOT_DELEGATED`, `UF_SMARTCARD_REQUIRED`, `UF_USE_DES_KEY_ONLY`, `UF_DONT_REQUIRE_PREAUTH`, `UF_TRUSTED_FOR_DELEGATION`, `UF_PASSWORD_EXPIRED`, `UF_TRUSTED_TO_AUTHENTICATE_FOR_DELEGATION`, `UF_HOMEDIR_REQUIRED`; Kontotyp `UF_NORMAL_ACCOUNT`); `usri1_script_path`. Das Setzen von Kontosteuer-Flags „may require certain privileges and control access rights“ [S44].
- Standardwerte bei Stufe 1 für die Felder der Strukturen `USER_INFO_2`, `USER_INFO_3`, `USER_INFO_4` (Präfix `usriX_`): `auth_flags` 0, `full_name` leer, `usr_comment` leer, `parms` leer, `workstations` alle, `acct_expires` nie (`TIMEQ_FOREVER`), `max_storage` unbegrenzt, `logon_hours` jederzeit, `logon_server` jeder Domänencontroller, `country_code` 0, `code_page` 0.
- Namensregeln: „User account names are limited to 20 characters and group names are limited to 256 characters. In addition, account names cannot be terminated by a period and they cannot include commas or any of the following printable characters: ", /, , [, ], :, |, <, >, +, =, ;, ?, *.“ (Die Abrufausgabe zeigt zwischen `/` und `[` eine Lücke; Annahme: dort steht das Zeichen `\`.) Zeichen mit Codes 1 bis 31 sind ausgeschlossen.
- Beispiel der Seite: `usri1_priv = USER_PRIV_USER`, `usri1_flags = UF_SCRIPT`, `usri1_password` gleich dem Kontonamen.

#### 2.5.3 `NetLocalGroupAddMembers` [S44]

- Signatur: `NET_API_STATUS NetLocalGroupAddMembers(LPCWSTR servername, LPCWSTR groupname, DWORD level, LPBYTE buf, DWORD totalentries)`; `level` 0 (SID, `LOCALGROUP_MEMBERS_INFO_0`) oder 3 (Domäne und Name, `LOCALGROUP_MEMBERS_INFO_3`).
- Aufrufer wie bei `NetUserAdd` („only Administrators and Power Users“); Sicherheitsbeschreibung des Objekts `LocalGroup` entscheidet.
- Rückgabewerte: `NERR_GroupNotFound`, `ERROR_ACCESS_DENIED`, `ERROR_NO_SUCH_MEMBER`, `ERROR_MEMBER_IN_ALIAS`, `ERROR_INVALID_MEMBER`.
- Hinweis der Seite „Requirements for Network Management Functions on Servers and Workstations“ (Seitenstand 2025-03-11): „By default, only Administrators and Power Users can write information.“ Lesen (Abfragen) dürfen standardmäßig alle authentifizierten Benutzer [S44].

#### 2.5.4 `NetUserDel` [S44]

- Signatur: `NET_API_STATUS NetUserDel(LPCWSTR servername, LPCWSTR username)`.
- Rückgabewerte: `NERR_Success`, `ERROR_ACCESS_DENIED`, `NERR_InvalidComputer`, `NERR_NotPrimary`, `NERR_UserNotFound`.
- „An account cannot be deleted while a user or application is accessing a server resource. If the user was added to the system with a call to the NetUserAdd function, deleting the user also deletes the user's system account.“

#### 2.5.5 Konten-Lebenszyklus: Profil, Rechte, leere Passwörter

- `CreateProfile(LPCWSTR pszUserSid, LPCWSTR pszUserName, LPWSTR pszProfilePath, DWORD cchProfilePath)` → `HRESULT` (Userenv.dll; Windows Vista/Server 2008 und neuer): „The caller must have administrator privileges to call this function.“; `E_ACCESSDENIED`, `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)` [S45].
- `DeleteProfileW(LPCWSTR lpSidString, LPCWSTR lpProfilePath, LPCWSTR lpComputerName)` → `BOOL` (Userenv.dll; Windows 2000 und neuer): „The caller must have administrative privileges to delete a user's profile.“; ab Windows Vista muss `lpComputerName` `NULL` sein (sonst `ERROR_INVALID_PARAMETER`); „DeleteProfile might fail when passed the security identifier (SID) of the local system account (S-1-5-18).“ [S45].
- Kontorechte (`LsaAddAccountRights(LSA_HANDLE PolicyHandle, PSID AccountSid, PLSA_UNICODE_STRING UserRights, ULONG CountOfRights)`; Policy-Handle mit `POLICY_LOOKUP_NAMES`, bei nicht vorhandenem Konto zusätzlich `POLICY_CREATE_ACCOUNT`) und die zehn Kontorecht-Konstanten [S45]: `SE_BATCH_LOGON_NAME` („SeBatchLogonRight“, Stapelanmeldung), `SE_DENY_BATCH_LOGON_NAME`, `SE_DENY_INTERACTIVE_LOGON_NAME`, `SE_DENY_NETWORK_LOGON_NAME`, `SE_DENY_REMOTE_INTERACTIVE_LOGON_NAME`, `SE_DENY_SERVICE_LOGON_NAME`, `SE_INTERACTIVE_LOGON_NAME`, `SE_NETWORK_LOGON_NAME`, `SE_REMOTE_INTERACTIVE_LOGON_NAME`, `SE_SERVICE_LOGON_NAME`. „The SE_DENY rights override the corresponding account rights.“ `LogonUser` und `LsaLogonUser` scheitern, wenn das Konto das zum Anmeldetyp passende Recht nicht hat.
- Richtlinie „Accounts: Limit local account use of blank passwords to console logon only“ (Windows 10/11; Standard auf Client-Computern: Enabled): „a local account must have a nonblank password to be used to perform an interactive or network logon from a remote client.“; sie wirkt nicht auf Anmeldungen am Konsolenbildschirm und nicht auf Domänenkonten [S45]. Wirkung auf lokale `LogonUserW`-Aufrufe: nicht ausdrücklich beschrieben (Lücke L23).
- Standardzuweisung „Log on as a batch job“ auf Client-Computern: Administrators; „Task Scheduler automatically grants this right when a user schedules a task.“ [S55].

#### 2.5.6 Windows Home und Pro

- Die Seiten zu `NetUserAdd`, `NetUserDel`, `NetLocalGroupAddMembers`, `CreateProfile`, `DeleteProfileW`, `LsaAddAccountRights` enthalten keine Editionsangabe; nur Mindestversionen (siehe oben) [S44], [S45].
- Community (Belegart C): Im gelesenen Microsoft-Q&A-Thread „Setting local password for an account linked with Microsoft account“ (Antwort vom 2025-03-18) steht, `lusrmgr.msc` erfordere eine Edition außer Home [S56]. Die Suchzusammenfassung nicht einzeln gelesener Microsoft-Q&A-Threads nennt dasselbe und als Ersatz `net user`, `net localgroup`, das PowerShell-Modul `Microsoft.PowerShell.LocalAccounts` und `netplwiz` (diese Threads wurden nicht einzeln gelesen). Eine Primärquelle dafür wurde nicht gefunden (Lücke L15).
- Das Ausblenden eines Kontos auf dem Anmeldebildschirm (Registrierung `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon\SpecialAccounts\UserList`, DWORD-Wert 0 mit dem Kontonamen) ist nur in Community-Beiträgen beschrieben (Lücke L17) [S56].

### 2.6 Frage 6: Einschränkung ohne zweites Konto

#### 2.6.1 Kurzbefund

| Mechanismus | Wirkung auf den Dateizugriff einer Shell (belegt) | Quelle |
|---|---|---|
| `CreateRestrictedToken` (Flags `DISABLE_MAX_PRIVILEGE`, `SANDBOX_INERT`, `LUA_TOKEN`, `WRITE_RESTRICTED`; Deny-only-SIDs; Privilegien löschen; Restricting SIDs) | Zwei Zugriffsprüfungen; Zugriff nur, wenn beide Prüfungen den angeforderten Zugriff erlauben. Ohne Restricting SIDs bleibt der Prozess dasselbe Konto (Russinovich 2007). | [S46], [S47] |
| Integritätsstufe Low | Schreibzugriff auf Objekte mit mittlerer Stufe und auf Objekte ohne Label ist gesperrt; Lesen ist durch die Standardrichtlinie nicht gesperrt | [S46], [S47] |
| AppContainer / LPAC | Zugriff nur über die DACL-Einträge für Paket-SID/Capability-SIDs; erlaubt ist die Schnittmenge aus Benutzer- und AppContainer-SIDs | [S46] |
| Job-Objekt | Begrenzt Arbeitssatzgröße, Prozesspriorität, Endzeit, CPU-Rate und UI-Funktionen (Seite „Job Objects“ und verlinkte Strukturen); die Seite nennt keinen Mechanismus für den Dateizugriff | [S46] |

#### 2.6.2 `CreateRestrictedToken` (Advapi32.dll; Windows XP, Windows Server 2003 und neuer) [S46]

- Signatur: Anhang A. Pflicht: `ExistingTokenHandle` mit `TOKEN_DUPLICATE`, `Flags`, die Zähler und `NewTokenHandle`.
- Flags: `DISABLE_MAX_PRIVILEGE` (0x1; „Disables all privileges in the new token except the SeChangeNotifyPrivilege privilege.“), `SANDBOX_INERT` (0x2; AppLocker und Software Restriction Policies werden nicht geprüft; auf Windows Server 2008 R2, Windows 7, Windows Server 2008, Windows Vista, Windows Server 2003 und Windows XP mit installiertem KB2532445 nur wirksam, wenn der Aufrufer LocalSystem oder TrustedInstaller ist), `LUA_TOKEN` (0x4), `WRITE_RESTRICTED` (0x8; „The new token contains restricting SIDs that are considered only when evaluating write access.“).
- Wirkung: Deny-only-SIDs (`SE_GROUP_USE_FOR_DENY_ONLY`), gelöschte Privilegien und Restricting SIDs. „The system performs two access checks: one using the token's enabled SIDs, and another using the list of restricting SIDs. Access is granted only if both access checks allow the requested access rights.“ Das neue Token hat dieselbe Art (primär oder Impersonation) wie das vorhandene; „Restricting SIDs are always enabled for access checks.“
- Verwendung: „If a process calls CreateProcessAsUser using a restricted version of its own token, the calling process does not need to have the SE_ASSIGNPRIMARYTOKEN_NAME privilege.“ Warnung: „Applications that use restricted tokens should run the restricted application on desktops other than the default desktop.“
- Russinovich (Archivblog 2007-02-12) [S47]: `PsExec -l` verwendet `CreateRestrictedToken` ohne Mitgliedschaft in der lokalen Administratorengruppe und ohne Administratorprivilegien; „processes running in the sandbox are running as you, and so can read and write any files, Registry keys, and even other processes to which your account has access.“

#### 2.6.3 Integritätsstufen [S46], [S47]

- „Windows defines four integrity levels: low, medium, high, and system. Standard users receive medium, elevated users receive high.“ „Objects that lack an integrity label are treated as medium by the operating system; this prevents low-integrity code from modifying unlabeled objects.“
- „By default, the system creates every object with an access mask of SYSTEM_MANDATORY_LABEL_NO_WRITE_UP.“ (Annahme: Daraus folgt, dass Lesen und Ausführen durch Standard-Labels nicht gesperrt sind; die Seite sagt es nicht ausdrücklich.) Bestätigung durch Russinovich: „with the exception of processes and threads, the wall doesn't block reads“; Schreiben ist nur auf Objekte mit Low-Label möglich; geprüft hat Russinovich das temporäre Verzeichnis des Protected-Mode-Internet-Explorers (Low-Label: Datei anlegen gelang) und das temporäre Verzeichnis seines Profils (Medium-Label: Zugriff verweigert).
- Prozesserzeugung: „the new process is created with the minimum of the user integrity level and the file integrity level.“ `PsExec -l`: „Run process as limited user (strips the Administrators group and allows only privileges assigned to the Users group). On Windows Vista the process runs with Low Integrity.“ [S48].
- Microsoft-Aussage zu Sicherheitsgrenzen (Russinovich, 2007): „neither UAC elevations nor Protected Mode IE define new Windows security boundaries“; „User accounts running in separate sessions are separated by a Windows security boundary“ [S47].
- Konsole: Clients mit niedrigerer Integrität als die angehängte Konsole unterliegen den „Wrong-Way Verbs“ (Abschnitt 2.2.5) [S19].

#### 2.6.4 AppContainer und LPAC [S46]

- Seite „Launch an AppContainer“ (ms.date 2023-07-20, Seitenstand 2025-09-03): „AppContainers run with a Low Integrity Level (IL)“; „the permitted access is the intersection of that granted by the user/group SIDs and AppContainer SIDs“; „AppContainers are isolated from accessing processes and windows belonging to other applications as well as from devices, files/directories, registry keys, the network, and credentials.“; „Regular AppContainers are granted access to certain system files/directories, common registry keys and COM objects“; LPAC: „LPAC cannot open any keys in the registry unless it has the registryRead capability and cannot use COM unless it has the lpacCom capability.“
- Start: `CreateAppContainerProfile(PCWSTR pszAppContainerName, PCWSTR pszDisplayName, PCWSTR pszDescription, PSID_AND_ATTRIBUTES pCapabilities, DWORD dwCapabilityCount, PSID* ppSidAppContainerSid)` (Userenv.dll; Windows 8/Server 2012 und neuer): Name höchstens 64 Zeichen, Muster `[-_. A-Za-z0-9]+`; Anzeigename höchstens 512, Beschreibung höchstens 2048 Zeichen; Rückgabe `S_OK`, `E_ACCESSDENIED`, `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`, `E_INVALIDARG`; „The function creates a profile for the current user. To create a profile on behalf of another user, you must impersonate that user.“ Ablauf: `STARTUPINFOEX` mit `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES` (`SECURITY_CAPABILITIES` mit `AppContainerSid`, `Capabilities`, `CapabilityCount`, `Reserved = 0`); LPAC zusätzlich `PROC_THREAD_ATTRIBUTE_ALL_APPLICATION_PACKAGES_POLICY` mit `PROCESS_CREATION_ALL_APPLICATION_PACKAGES_OPT_OUT`; `CreateProcess` mit `EXTENDED_STARTUPINFO_PRESENT`. Die Umgebungsvariablen `LOCALAPPDATA`, `TEMP`, `TMP` werden in das AppContainer-Profil umgeleitet (`C:\Users\<Benutzer>\AppData\Local\Packages\<Name>\AC` bzw. `C:\Users\<Benutzer>\AppData\Local\Packages\<Name>\AC\Temp`).
- Seite „AppContainer isolation“ (Seitenstand 2025-07-10): „Read-write access can be granted to specific persistent files and registry keys. Read-only access is less restricted.“
- Seite „Mandatory Integrity Control“: „Windows ensures that processes running with a low integrity level cannot obtain access to a process which is associated with an app container.“
- Win32 app isolation (Seitenstand 2025-05-15): „This feature is in preview“; Mindestanforderung „Windows 11, version 24H2 (build 26100) or later“ und Visual Studio 17.10.2 oder neuer für die Paketierung; Zielanwendungen: Win32, Desktop Bridge (Centennial), Desktop-Apps „packaged with external location“ [S46].
- Konsolenprogramme: UWP-Konsolen-Apps (ab Windows 10, Version 1803; nur C++/WinRT und C++/CX; die Projektvorlage trägt eine `AppExecutionAlias`-Erweiterung in `Package.appxmanifest` ein) laufen im Konsolenfenster, dürfen kein Fenster erzeugen und „can access the file-system from the directory it is run from, and below“; breiterer Dateizugriff über die eingeschränkte Capability `broadFileSystemAccess` [S46].
- Bericht zu `pwsh.exe` im AppContainer (PowerShell-Issue #27253, geöffnet 2026-04-10, geschlossen 2026-04-20 über PR #27266; Belegart Q/C; Zusammenfassung des Abrufwerkzeugs): `pwsh.exe` 7.5.5 auf Windows 10.0.26593 wurde mit `CreateAppContainerProfile` und `CreateProcessW` mit `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES` gestartet; Ergebnis: keine Dateisystem-PSDrives („No filesystem PSDrives are created“), `Write-Output` nicht gefunden; Ursache nach Angabe des Berichterstatters: `GetFileAttributesEx` auf `C:\` liefert `ERROR_ACCESS_DENIED` (5), weil dem Volumestamm ein `ALL APPLICATION PACKAGES`-Eintrag fehle, während Unterverzeichnisse wie `C:\Windows` zugänglich seien [S54]. Ob PR #27266 in einer veröffentlichten Version steckt, ist nicht erfasst (Lücke L18).
- `cmd.exe` und `powershell.exe` (Windows PowerShell) im AppContainer/LPAC, mit oder ohne ConPTY: keine Primärquelle (Lücke L18). Ob diese Shells die vier Wrong-Way-Verb-Funktionen aufrufen: nicht belegt (Lücke L19).

#### 2.6.5 Job-Objekte [S46]

- Zweck: „A job object allows groups of processes to be managed as a unit.“ Grenzwerte: Arbeitssatzgröße, Prozesspriorität, Endzeit je Prozess (`JOBOBJECT_BASIC_LIMIT_INFORMATION`), UI-Beschränkungen (`JOBOBJECT_BASIC_UI_RESTRICTIONS`), CPU-Rate, erweiterte Limits (`JOBOBJECT_EXTENDED_LIMIT_INFORMATION`, darunter `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), Benachrichtigungen, Abrechnung. „Security limits must be set individually for each process associated with a job object.“ (seit Windows Vista).
- `JOB_OBJECT_UILIMIT_*` (alle acht): `DESKTOP` (0x40), `DISPLAYSETTINGS` (0x10), `EXITWINDOWS` (0x80), `GLOBALATOMS` (0x20), `HANDLES` (0x1), `READCLIPBOARD` (0x2), `SYSTEMPARAMETERS` (0x8), `WRITECLIPBOARD` (0x4).
- Verschachtelte Jobs ab Windows 8 und Windows Server 2012; vorher nur ein Job je Prozess.
- Dateizugriffsregeln für Prozesse eines Jobs nennen die gelesenen Seiten nicht.

### 2.7 Frage 7: PsExec, runas, Aufgabenplanung, WinRM/PowerShell Remoting

#### 2.7.0 Kurzbefund

1. PsExec mit `-u`/`-p`: Passwort nötig; nach dem Archivartikel von 2007 setzt PsExec Administratorrechte auf dem Ziel, die Freigabe `admin$` und die Ports der Datei- und Druckerfreigabe voraus und startet einen Dienst, der die Anmeldedaten verwendet; die aktuelle Seite nennt als Plattform Windows 8.1 und neuer sowie Windows Server 2012 und neuer [S48].
2. `runas`: nutzt den Dienst Secondary Logon (`seclogon`); Passwort wird abgefragt; `/savecred` ist auf Windows Vista Home und Windows Vista Starter wirkungslos; unter LocalSystem „does not function correctly“ [S49].
3. Aufgabenplanung: gespeichertes Kennwort liegt als LSA-Secret; S4U-Aufgaben speichern kein Kennwort und haben keinen Zugriff auf das Netzwerk und keinen Zugriff auf verschlüsselte Dateien („encrypted files“); Zielkonto braucht `SeBatchLogonRight`; nach einer Passwortänderung läuft die Aufgabe nicht mehr, bis das gespeicherte Kennwort geändert wird [S30], [S31], [S42], [S50], [S55].
4. WinRM/PowerShell Remoting: Verbindung nur für Mitglieder von Administrators oder Remote Management Users auf dem Ziel; Loopback nur aus erhöhter PowerShell; Anmeldung über das Netzwerk (Kerberos oder NTLM) ohne delegierbare Anmeldedaten; `winrs` lädt das Profil standardmäßig und verlangt für Nicht-Administratoren `/noprofile` [S51].
5. Zu ConPTY enthält keine der gelesenen Seiten für PsExec, `runas`, Aufgabenplanung, WinRM oder `winrs` eine Aussage (Lücken L21, L22).

#### 2.7.1 PsExec (`-u`/`-p`) [S48]

- Seite: Autor Mark Russinovich, veröffentlicht 2023-04-11; „Runs on: Client: Windows 8.1 and higher. Server: Windows Server 2012 and higher.“
- Optionen (Wortlaut der Seite): `-u` „Specifies optional user name for login to remote computer.“; `-p` „Specifies optional password for user name. If you omit this you will be prompted to enter a hidden password.“; `-s` „Run the remote process in the System account.“; `-h` „If the target system is Vista or higher, has the process run with the account's elevated token, if available.“; `-e` „Does not load the specified account's profile.“ (Widerspruch W07); `-l` „Run process as limited user (strips the Administrators group and allows only privileges assigned to the Users group). On Windows Vista the process runs with Low Integrity.“; `-i` „Run the program so that it interacts with the desktop of the specified session on the remote system. If no session is specified the process runs in the console session. This flag is required when attempting to run console applications interactively (with redirected standard IO).“; `-x` „Display the UI on the Winlogon secure desktop (local system only).“; `-d` nicht auf Beendigung warten; `-w` Arbeitsverzeichnis; `-c`/`-f`/`-v` Kopieren der Programmdatei.
- Ohne `\\computer`: „If you omit the computer name, PsExec runs the application on the local system“.
- Ohne `-u`: „the process will run in the context of your account on the remote system, but will not have access to network resources (because it is impersonating).“ „Note that the password and command are encrypted in transit to the remote system.“ „Input is only passed to the remote system when you press the Enter key. Typing Ctrl-C terminates the remote process.“
- Mechanismus laut TechNet Magazine „PsTools Primer“ (Wes Miller, 2007-03; Archiv): Das Werkzeug kopiert sich in die administrative Freigabe (`admin$`), installiert einen Windows-Dienst, der „performs the task you requested, using the credentials you provided at launch time“, und deinstalliert ihn danach; Voraussetzungen: „the default administrative network share enabled on the remote system you are running the tools on (admin$); and the ports for File and Printer Sharing opened on the remote system“. Das Skript `Run.vbs` des Artikels startet lokal ohne `\\computer` mit `-d -i -e -u COMPUTERNAME\USER -p PASSWORD` und gilt als „the ability for a limited user to provide more privileged credentials to launch PsExec locally“.
- Die ITProToday-Seite, auf die die Learn-Seite für die technische Funktionsweise verweist, war nicht lesbar (Weiterleitung auf eine allgemeine Seite; Lücke L21).
- Mindestrechte lokal (ohne `\\computer`) für Windows 10/11: nicht belegt (Lücke L21).

#### 2.7.2 `runas` (`/savecred`) [S49]

- Quelle: archivierte Seite „Runas“ (TechNet, Windows Server 2012 R2, ms.date 2016-08-31, „Applies To: Windows Server 2003, Windows Vista, Windows XP, Windows HPC Server 2008 R2, Windows Server 2008, Windows 7, Windows Server 2003 R2, Windows Server 2000, Windows Server 2012, Windows Server 2003 with SP1, Windows 8“). Die aktuelle Learn-Seite (`https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/runas`, auch ohne Sprachpräfix versucht) lieferte am 2026-10-17 HTTP 404 (Lücke L20).
- Syntax: `runas [{/profile | /noprofile}] [/env] [{/netonly | /savecred}] [/smartcard] [/showtrustlevels] [/trustlevel] /user:<UserAccountName> "<ProgramName> <PathToProgramFile>"`.
- Optionen: `/profile` (Standard: lädt das Profil, nicht mit `/netonly`), `/noprofile`, `/env` („the current network environment be used instead of the user's local environment“), `/netonly` („user information specified is for remote access only“), `/savecred` („Indicates if the credentials have been previously saved by this user. This parameter is not available and will be ignored on Windows Vista Home or Windows Vista Starter Editions. This parameter cannot be used with the /smartcard parameter.“), `/smartcard`, `/showtrustlevels`, `/trustlevel`, `/user` (`<User>@<Domain>` oder `<Domain>\<UserAccountName>`).
- Bemerkungen der Seite: „Enter the user's password only when prompted.“; „The use of runas is not restricted to administrator accounts“; „If the runas command fails, the Secondary Logon service might not be running or the user account you are using might not be valid.“; Programme von Netzwerkpfaden können scheitern, weil die Anmeldeinformationen abweichen; „Group Policy is not processed for the user whose credentials are supplied to the runas command.“
- Microsoft Support (KB 325859, für Windows Server 2003; Seitenstand 2026-02-19): „To use the Run as feature, the Secondary Logon service must be running.“; „The runas command does not function correctly when you run it by using the LocalSystem account.“
- Ablageort der mit `/savecred` gespeicherten Anmeldedaten: nicht in den gelesenen Seiten (Lücke L20).
- Community-Hinweis (Microsoft Q&A, Belegart C; nur aus der Suchzusammenfassung des Threads „Error while opening software in administrator mode using runas command“, Thread nicht einzeln gelesen): Ein Moderator warne, dass nach dem Speichern des Passworts jedes Standardkonto auf diesem Computer beliebige Programme als das gespeicherte Konto ohne Passwort starten könne [S56].

#### 2.7.3 Aufgabenplanung (Task mit gespeichertem Kennwort, `schtasks /RU /RP`) [S30], [S31], [S50]

- `schtasks /create` (Seitenstand 2025-10-22): `/ru {[<domain>\]<user> | system}`: „Runs the task with permissions of the specified user account.“; `/rp <password>`: „Specifies the password for the existing user account, or the user account specified by the /ru parameter. If you don't use this parameter when specifying a user account, SchTasks.exe will prompt you for the password next time you sign in. Don't use the /rp parameter for tasks that run with System account credentials (/ru System).“; `/it`: Aufgabe läuft nur, wenn der Ausführungsbenutzer angemeldet ist; `/np`: „No password is stored. The task runs non-interactively as the given user. Only local resources are available.“; `/rl LIMITED|HIGHEST`; `/xml`.
- „Schtasks always prompts for a password unless you provide one, even when you schedule a task on the local computer using the current user account.“ „Schtasks doesn't verify program file locations or user account passwords. If you don't enter the correct file location or the correct password for the user account, the task is created, but it won't run.“
- System-Konto: „The System account doesn't have interactive logon rights. Users can't see or interact with programs or tasks run with system permissions.“; „Only Administrators can schedule tasks, regardless of the value of the /ru parameter.“ (Widerspruch W08). Der Hinweis steht im Abschnitt über Aufgaben mit Systemrechten.
- Win32-Seite „Security Contexts for Tasks“ (Seitenstand 2025-05-12): „Tasks registered with the TASK_LOGON_PASSWORD or TASK_LOGON_S4U flag will only launch if the specified user has the Logon as Batch privilege enabled. Administrators and Backup Operators group users have this privilege enabled by default.“ Registrierung aus einem Administratorkonto: Passwort nötig bei `TASK_LOGON_PASSWORD` (eigenes oder fremdes Konto) und bei `TASK_LOGON_S4U` für ein fremdes Konto. Registrierung aus einem Nicht-Administratorkonto: kein Passwort, wenn die Aufgabe im eigenen Konto mit S4U oder interaktivem Anmeldetyp läuft; sonst Passwort nötig; Registrierung unter Local Service oder für eine Gruppe nicht möglich. Standardmäßig dürfen Benutzer nur selbst erstellte Aufgaben lesen, ändern, löschen und ausführen; die DACL der Aufgabendatei kann das ändern (`IRegisteredTask.SetSecurityDescriptor`); Ausführen erfordert Ausführungsrecht an der Aufgabendatei (`IRegisteredTask::Run`, `RunEx`).
- `TASK_LOGON_TYPE` [S31] (alle sieben Werte): `TASK_LOGON_NONE` (0), `TASK_LOGON_PASSWORD` (1; „The password must be supplied at registration time.“), `TASK_LOGON_S4U` (2), `TASK_LOGON_INTERACTIVE_TOKEN` (3; „User must already be logged on. The task will be run only in an existing interactive session.“), `TASK_LOGON_GROUP` (4), `TASK_LOGON_SERVICE_ACCOUNT` (5), `TASK_LOGON_INTERACTIVE_TOKEN_OR_PASSWORD` (6; „currently identical to TASK_LOGON_PASSWORD“; für Windows 10 1507/1511 und älter: erst interaktives Token, sonst Passwort). Mindestversion: Windows Vista / Windows Server 2008.
- Passwortspeicherung: Passwörter geplanter Aufgaben liegen als LSA-Secrets [S42]. Das Anmeldeverhalten bei Aufgaben (Stapelanmeldung statt interaktiv) beschreibt die Richtlinienseite „Log on as a batch job“: „the Task Scheduler service logs on the user as a batch job instead of as an interactive user, and the task runs in the user's security context.“ [S55].
- UAC-Stufe der Aufgabe: Aufgaben laufen standardmäßig mit niedrigen Rechten („By default, a task runs with low level privileges when UAC is turned on.“); Registrierung mit `TASK_RUNLEVEL_HIGHEST` aus einem Prozess mit niedrigen Rechten ist nicht möglich [S30].
- ConPTY oder Konsole für Aufgabenprozesse (Sitzung, Fensterstation): keine Quelle (Lücke L22).

#### 2.7.4 WinRM / PowerShell Remoting / `winrs` [S51]

- Voraussetzungen (`about_Remote_Requirements`, Seitenstand 2025-11-07): „To create remote sessions and run remote commands connect with a user account that's a member of the Administrators or Remote Management Users group on the remote computer.“; die Sicherheitsdeskriptoren der Standard-Sitzungskonfigurationen `Microsoft.PowerShell` und `Microsoft.PowerShell32` erlauben nur Mitgliedern der Gruppe Administrators; Administratorrechte sind nötig für: Verbindung zum lokalen Computer („loopback“), Verwaltung von Sitzungskonfigurationen, Anzeige und Änderung der WS-Management-Einstellungen des lokalen Computers; PowerShell muss auch für Mitglieder der Administratorengruppe mit „Run as administrator“ gestartet werden. Windows Server 2012 und neuere Server sind standardmäßig für Remoting aktiviert; auf Clients ist `Enable-PSRemoting` (erhöht) nötig; auf Clients im Netzwerkprofil „public“ ist das normalerweise nicht erlaubt (Parameter `SkipNetworkProfileCheck`); Systemanforderungen: Windows PowerShell 3.0 oder neuer, .NET Framework 4 oder neuer, WinRM 3.0 oder neuer (für Windows PowerShell 2.0: .NET 2.0, WinRM 2.0).
- `Enable-PSRemoting` [S51]: startet den WinRM-Dienst, setzt den Starttyp auf Automatic, erstellt einen Listener, aktiviert die Firewall-Ausnahme, erstellt die Sitzungskonfigurationen, aktiviert alle Sitzungskonfigurationen und ändert deren Sicherheitsdeskriptor; „To run this cmdlet on the Windows platform, start PowerShell by using the Run as administrator option.“
- Sicherheit (`WinRM-Security`, Seitenstand 2025-12-09): Ports HTTP 5985, HTTPS 5986; „By default, PowerShell Remoting only allows connections from members of the Administrators group. Sessions are launched under the user's context, so all operating system access controls applied to individual users and groups continue to apply to them while connected over PowerShell Remoting.“ „WinRM runs as a service under the Network Service account, and spawns isolated processes running as user accounts to host PowerShell instances.“ Authentifizierung: Kerberos (Domänenserver über den Computernamen), sonst NTLM; „NTLM-based authentication is disabled by default. You can enable NTLM by either configuring SSL on the target server, or by configuring the WinRM TrustedHosts setting on the client.“ Beide Protokolle senden keine delegierbaren Anmeldedaten; daraus folgt das Problem des zweiten Hops.
- FAQ (Seitenstand 2025-12-09): Fernstart von Programmen mit Benutzeroberfläche: „the program process starts, but the user interface doesn't appear“; `Enter-PSSession` bietet eine interaktive Sitzung; „It isn't the same as using Telnet, but it provides a similar experience.“; PowerShell-Profile werden in Remotesitzungen nicht automatisch ausgeführt; Test auf einem Einzelrechner ohne Domäne ist möglich; für Arbeitsgruppen unter Windows Vista, 7, 8 ist der Registrierungseintrag `LocalAccountTokenFilterPolicy` = 1 in `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Policies\System` genannt (mit Warnung, dass die Einstellung alle Benutzer betrifft); Anmeldedaten über den Parameter `Credential`; Remoting über SSH existiert (PowerShell 7).
- `winrs` (Seitenstand 2024-11-01): Syntax `winrs [/<parameter>[:<value>]] <command>`; `/remote:<endpoint>` (ohne Angabe: `/r:localhost`); `/username`, `/password` (nur zusammen); `/directory` (Standard: `%USERPROFILE%`); `/noprofile`: „By default, the server attempts to load the user profile. If the remote user isn't a local administrator on the target system, then this option is required (the default results in error.)“; `/allowdelegate`; `/usessl`; Nachrichten standardmäßig mit Kerberos- oder NTLM-Schlüsseln verschlüsselt.
- Ob WinRM/WinRS ein Pseudokonsolen-Terminal (PTY) liefert: keine Aussage der gelesenen Seiten (Lücke L22).

### 2.8 Fehlercodes und Statuswerte (nur belegte; Zusammenstellung)

| Code | Funktion / Situation | Quelle |
|---|---|---|
| `ERROR_PRIVILEGE_NOT_HELD` (1314) | `CreateProcessAsUserW`, `CreateProcessWithTokenW`: erforderliches Privileg fehlt | [S02], [S04] |
| `ERROR_LOGON_FAILURE` (1326) | `CreateProcessWithLogonW` mit Entra-Konto in UPN-Form und `lpDomain = NULL` | [S21] |
| `ERROR_INVALID_PARAMETER` | `LoadUserProfileW` (`dwSize` falsch oder `lpUserName` `NULL`); `DeleteProfileW` (`lpComputerName` nicht `NULL`); `CreateRestrictedToken` (zu viele Privilegien); `CredWriteW` (nicht änderbares Feld) | [S05], [S45], [S46], [S39] |
| `ERROR_ACCESS_DENIED` | Net*-Funktionen ohne Berechtigung; Wrong-Way-Verbs bei `ReadConsoleOutput*`/`WriteConsoleInput` | [S44], [S19] |
| `STATUS_PORT_CONNECTION_REFUSED` | `LsaRegisterLogonProcess` ohne `SeTcbPrivilege` | [S27] |
| `STATUS_QUOTA_EXCEEDED`, `STATUS_ACCOUNT_RESTRICTION`, `STATUS_BAD_VALIDATION_CLASS`, `STATUS_LOGON_FAILURE`, `STATUS_NO_LOGON_SERVERS`, `STATUS_NO_SUCH_PACKAGE`, `STATUS_PKINIT_FAILURE`, `STATUS_PKINIT_CLIENT_FAILURE` | `LsaLogonUser` | [S23] |
| `STATUS_INVALID_LOGON_HOURS`, `STATUS_INVALID_WORKSTATION`, `STATUS_PASSWORD_EXPIRED`, `STATUS_ACCOUNT_DISABLED` | `LsaLogonUser`, `SubStatus` (MSV1_0) | [S23] |
| `STATUS_NAME_TOO_LONG` | `LsaRegisterLogonProcess`, Name länger als 127 Bytes | [S27] |
| `STATUS_NO_SUCH_PRIVILEGE` | `LsaAddAccountRights`, ungültiger Privilegname | [S45] |
| `0xc0000142` | gehosteter Prozess, Pseudokonsole während des Starts geschlossen oder ungültiges Handle | [S17] |
| `NERR_UserExists`, `NERR_GroupExists`, `NERR_PasswordTooShort`, `NERR_InvalidComputer`, `NERR_NotPrimary`, `NERR_UserNotFound`, `NERR_GroupNotFound`, `ERROR_NO_SUCH_MEMBER`, `ERROR_MEMBER_IN_ALIAS`, `ERROR_INVALID_MEMBER` | Net*-Funktionen | [S44] |
| `E_ACCESSDENIED`, `HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)`, `E_INVALIDARG` | `CreateProfile`, `CreateAppContainerProfile` | [S45], [S46] |
| `ERROR_NO_SUCH_LOGON_SESSION`, `ERROR_INVALID_FLAGS`, `ERROR_BAD_USERNAME`, `ERROR_NOT_FOUND` | `CredWriteW` | [S39] |

## 3. Vergleichstabelle der Wege (nur Fakten, keine Empfehlung)

Zellen mit „unbelegt“ bedeuten: Die gelesenen Quellen sagen es weder zu noch ab. Quellenkennungen siehe Abschnitt 6.

| Weg | Benötigtes Konto/Privileg des Aufrufers | Passwort nötig | ConPTY möglich | Profil/Umgebung | Windows-Editionen/-Versionen | Quelle |
|---|---|---|---|---|---|---|
| A. `LogonUserW` + `CreateProcessAsUserW` | Token des Aufrufers: `SE_INCREASE_QUOTA_NAME` und, wenn das Token nicht zuweisbar ist, `SE_ASSIGNPRIMARYTOKEN_NAME`. LocalSystem-Token enthält beide. Nicht erhöhter Nutzer: keines; erhöhter Administrator: nur `SE_INCREASE_QUOTA_NAME` per Standardzuweisung. Ausnahme: eingeschränktes Token des eigenen Prozesses. | ja (Klartext `lpszPassword`) | ja für den Start (`STARTUPINFOEX` erlaubt); Konsolenhost entsteht im Konto des Aufrufers (Wrong-Way-Verbs-Bericht [S20]); belegte Lösungen: Zeilen E, F | Profil nicht geladen; `LoadUserProfileW` nur Administrator/LocalSystem mit `SE_BACKUP_NAME`+`SE_RESTORE_NAME`; Umgebung: `CreateEnvironmentBlock(hToken)`; Fensterstation: nicht interaktiv als Standard | Windows XP / Server 2003 und neuer; keine Editionsbeschränkung auf den Seiten | [S01], [S02], [S05], [S06], [S08], [S09], [S10], [S47] |
| B. `CreateProcessWithLogonW` | kein Privileg; Zielkonto: „Log On Locally“; Aufrufer nicht LocalSystem; Dienst `seclogon` muss laufen | ja (Klartext `lpPassword`) | unbelegt (Seite: `STARTUPINFO`; Community-Bericht: `EXTENDED_STARTUPINFO_PRESENT` nicht akzeptiert) | `LOGON_WITH_PROFILE` lädt das Profil; `lpEnvironment = NULL` → Umgebung aus Profil des Zielkontos; Befehlszeile höchstens 1024 Zeichen; `CREATE_NEW_CONSOLE` als Standard | Windows XP / Server 2003 und neuer | [S03], [S20], [S22], [S49] |
| C. `CreateProcessWithTokenW` | `SE_IMPERSONATE_NAME` (Standard: Administrators, Local Service, Network Service, Service); Token mit `TOKEN_QUERY`, `TOKEN_DUPLICATE`, `TOKEN_ASSIGN_PRIMARY` | nein bei vorhandenem Token (Token aus Weg A, D oder aus Archivblog-Technik [S15]) | unbelegt (Seite: `STARTUPINFO` oder `STARTUPINFOEX`; Community-Zweifel [S20]) | `LOGON_WITH_PROFILE` lädt Profil; Prozess läuft in der Sitzung des Aufrufers | Windows Vista / Server 2003 und neuer | [S04], [S15], [S20], [S55] |
| D. S4U (`LsaLogonUser` mit `MSV1_0_S4U_LOGON`/`KERB_S4U_LOGON`) + `CreateProcessAsUserW` | `SeTcbPrivilege` (für `LsaRegisterLogonProcess` und Impersonationstoken); für `CreateProcessAsUserW` zusätzlich Privilegien wie Weg A; LocalSystem hat alle; Kerberos-Variante: Domänenkonto/Domänenmitgliedschaft | nein | über Helferprozess im Zielkonto (Zeile F) belegt; direkt wie Zeile A | Profil durch Aufrufer (`LoadUserProfileW`); Umgebung aus Token; Token ohne Anmeldedaten für entfernte Server | `MsV1_0S4ULogon` nicht auf Windows XP / Server 2003 / Vista; Windows 7 und neuer nur durch Auslassung belegt; `KERB_S4U_LOGON` ab Server 2003 | [S23]–[S29], [S33], [S37] |
| E. Win32-OpenSSH (sshd) | sshd-Dienst als LocalSystem mit TCB-Privileg; Authentifizierungsmethoden `password`, `publickey` | Passwortanmeldung: ja; Schlüsselanmeldung: nein (S4U) | ja: Worker im Benutzerkonto startet `conhost.exe --headless --width <Spalten> --height <Zeilen> --signal <Handle> -- <Befehl>`; Rückfall `ssh-shellhost.exe` ohne ConPTY-API | `load_user_profile`; `CreateEnvironmentBlock`; Sitzung nach Schlüsselanmeldung ohne Anmeldedaten | Windows 10 ab 1809, Windows Server 2019 und neuer (Feature on Demand); Microsoft Entra nicht unterstützt | [S33]–[S36] |
| F. Helferprozess im Zielkonto, der `CreatePseudoConsole` selbst aufruft | Starter braucht eine der Zeilen A, C, D; der Helfer braucht kein zusätzliches Privileg (läuft im Zielkonto) | wie Starter | ja (belegt in Tailscale-Relay [S37], Win32-OpenSSH-Worker [S33], Broker im Q&A-Thread [S20]) | Profil und Umgebung wie Starter; Tailscale: Umgebung aus Token, Arbeitsverzeichnis Profilordner | `CreatePseudoConsole`: Windows 10 1809, Windows Server 2019 und neuer | [S16], [S20], [S33], [S37] |
| G. `ConptyCreatePseudoConsoleAsUser(hToken, size, hInput, hOutput, dwFlags, phPC)` der quelloffenen `conpty.dll` | `CreateProcessAsUserW`-Voraussetzungen (Weg A) für den Aufruf mit Token | wie Token-Beschaffung | Funktion vorhanden und exportiert; durchgängiger Einsatz für ein abweichendes Konto unbelegt | wie Weg A | NuGet-Paket: „should work on Windows 10.0.17763.0 and above“; MIT-Lizenz | [S38] |
| H. Aufgabenplanung (`TASK_LOGON_PASSWORD`, `TASK_LOGON_S4U`; `schtasks /RU /RP`, `/NP`, `/IT`) | Registrierung für fremdes Konto: laut `schtasks`-Seite Administrators, laut Win32-Seite Passwort (Widerspruch W08); Zielkonto: `SeBatchLogonRight` | `TASK_LOGON_PASSWORD`: ja (als LSA-Secret gespeichert); `TASK_LOGON_S4U`: nein, aber Registrierung für fremdes Konto durch Administrator verlangt Passwort | unbelegt | S4U-Aufgaben: nicht interaktiver Desktop, kein Zugriff auf das Netzwerk und kein Zugriff auf verschlüsselte Dateien („encrypted files“); Umgebung: nicht beschrieben | Windows Vista / Server 2008 und neuer | [S30], [S31], [S42], [S50], [S55] |
| I. `runas /user` (`/savecred`) | kein Privileg; Dienst `seclogon`; nicht unter LocalSystem | ja (Eingabe); mit `/savecred` gespeichert (Ablageort unbelegt) | unbelegt | `/profile` (Standard) lädt Profil; `/noprofile`; `/env`; `/netonly` | `/savecred` nicht auf Vista Home/Starter (archivierte Seite); aktuelle Seite nicht abrufbar | [S49], [S03] |
| J. PsExec `-u`/`-p` (auch `-s`, `-h`, `-e`, `-l`, `-i`) | Administratorrechte auf dem Ziel; `admin$` und Ports für Datei- und Druckerfreigabe (Stand 2007); lokal: „limited user“ kann privilegierte Anmeldedaten angeben | ja (`-p` oder Abfrage) | unbelegt (Werkzeug bietet eigene Interaktivität) | `-e`: Profil laden bzw. nicht laden (Widerspruch W07) | Client Windows 8.1 und neuer, Server 2012 und neuer | [S48] |
| K. WinRM/PowerShell Remoting, `winrs` | Mitglied von Administrators oder Remote Management Users auf dem Ziel; Loopback nur erhöht; `Enable-PSRemoting` (erhöht) auf Clients | ja (`Credential`) oder aktuelle Identität; Netzwerkanmeldung (Kerberos/NTLM) | unbelegt | PowerShell-Profile nicht automatisch; `winrs` lädt Profil standardmäßig, für Nicht-Administratoren ist `/noprofile` nötig | Windows PowerShell 3.0+, WinRM 3.0+; Server ab 2012 standardmäßig aktiv | [S51] |
| L. `CreateRestrictedToken` + `CreateProcessAsUserW` (gleiches Konto) | kein `SE_ASSIGNPRIMARYTOKEN_NAME`, wenn eingeschränkte Version des eigenen Primärtokens | nein | unbelegt | Konto und Profil bleiben die des Aufrufers | Windows XP / Server 2003 und neuer | [S02], [S46], [S47] |
| M. Integritätsstufe Low (Token) | Aufrufer mit Berechtigung zum Setzen der Stufe (die gelesenen Seiten benennen das Recht nicht); Werkzeug, das ein Low-Integrity-Token erzeugt: `PsExec -l` | nein | unbelegt; Konsolen-Clients mit niedrigerer Stufe als die Konsole: Wrong-Way-Verbs | gleiches Konto | ab Windows Vista | [S19], [S46], [S47], [S48] |
| N. AppContainer / LPAC | normaler Benutzer; Profil je Benutzer (`CreateAppContainerProfile`) | nein | unbelegt; UWP-Konsolen-Apps/niedrigere Stufe: Wrong-Way-Verbs; `pwsh.exe`-Bericht ohne ConPTY | Umgebung `LOCALAPPDATA`, `TEMP`, `TMP` in Profil umgeleitet | Windows 8 / Server 2012 und neuer; Win32 app isolation: Windows 11 24H2 oder neuer, Preview | [S19], [S46], [S54] |
| O. Job-Objekt | keine besondere Berechtigung genannt | nein | nicht betroffen | gleiches Konto | `JOBOBJECT_BASIC_UI_RESTRICTIONS`: Windows XP / Server 2003 und neuer; verschachtelte Jobs ab Windows 8 / Server 2012 | [S46] |
| P. Eigenes lokales Konto anlegen (`NetUserAdd`, `NetLocalGroupAddMembers`, optional `CreateProfile`, `LsaAddAccountRights`) als Voraussetzung für die Zeilen A–F | Administrators oder Power Users; erhöhter Prozess (Annahme); `CreateProfile`/`DeleteProfileW`: Administrator | Passwort wird beim Anlegen vergeben (`usri1_password`) | nicht Gegenstand | Profil entsteht bei Anmeldung oder durch `CreateProfile` (Windows Vista und neuer) | `NetUserAdd` ab Windows 2000; Home/Pro: unbelegt | [S44], [S45] |
| Q. `sudo` für Windows | Erhöhung desselben Benutzers per UAC | nein | nicht Gegenstand | gleicher Benutzer | Windows 11 24H2 oder neuer; „does not support running programs as other users“ | [S52] |

## 4. Widersprüchliche Angaben

| Nr. | Angabe 1 | Angabe 2 | Einordnung |
|---|---|---|---|
| W01 | „Process Creation Flags“ (2025-07-14): Die Flags (darunter `EXTENDED_STARTUPINFO_PRESENT`) werden von `CreateProcess`, `CreateProcessAsUser`, `CreateProcessWithLogonW` und `CreateProcessWithTokenW` verwendet [S22] | Seite zu `CreateProcessWithLogonW`: `lpStartupInfo` ist „a STARTUPINFO structure“; Seitenmetadaten der Funktion führen `EXTENDED_STARTUPINFO_PRESENT` nicht, die von `CreateProcessWithTokenW` schon; Community: wird nicht akzeptiert [S03], [S04], [S20] | Microsoft-Texte uneinheitlich; Verhalten nicht belegt (L06) |
| W02 | Seite `CreateProcessWithTokenW` verweist bei `lpEnvironment` und `lpEnvironment = NULL` auf `lpUsername`/`lpUserName` | Die Funktion hat keinen solchen Parameter (Signatur) [S04] | Dokumentationsfehler (aus `CreateProcessWithLogonW` übernommen) |
| W03 | `CreateProcessAsUserW`: Aufrufer muss „typically“ `SE_INCREASE_QUOTA_NAME` haben [S02] | „Restricted Tokens“: „This enables ordinary applications to create restricted processes.“ [S46]; das gefilterte Token enthält `SeIncreaseQuotaPrivilege` nicht [S47] | Klärung offen (L02) |
| W04 | `CreateProcessWithLogonW`-Seite: „The application must add permission for the specified user account to the specified window station and desktop“; nur unter Windows XP füge die Funktion die Berechtigung hinzu [S03] | WinSDK-Blog: „CreateProcessWithLogonW() internally grants access to the user by obtaining the LOGON SID of the current INTERACTIVELY logged on user and adding this LOGON SID“ [S15] | Aktuelles Verhalten nicht belegt (L05) |
| W05 | Tailscale-Kommentar: „The conpty API currently does not provide a way to create a pseudoconsole for a different user than the current process.“ [S37] | `ConptyCreatePseudoConsoleAsUser(hToken, size, hInput, hOutput, dwFlags, phPC)` in der quelloffenen `conpty.dll` [S38] | Beide Aussagen stimmen für das In-Box-`kernel32.dll`-API überein; die Redistributable-Variante hat einen Token-Parameter |
| W06 | Wiki „TTY PTY support“: OpenSSH nutze ab Windows 10/Server 2019 die Funktionen der Konsole („conpty support provided by the Microsoft console team“) [S34]; Maintainer-Kommentar: „Starting with Windows 10 Build 18309, ConPTY should be supported“ [S36] | Quelltext nutzt `conhost.exe --headless --signal` statt `CreatePseudoConsole` [S33]; Howett: Unterstützung vor den öffentlichen APIs in RS5 aktiviert [S36]; ConPTY-Dokumentation: Version 1809 (Build 17763) [S16], [S38] | „ConPTY“ in Win32-OpenSSH meint die interne Konsolenhost-Schnittstelle; Buildnummer 18309 weicht von 17763 ab |
| W07 | PsExec-Seite (Learn): `-e` „Does not load the specified account's profile.“ [S48] | TechNet-Primer 2007: `-e` „Loads using the specified account's profile.“ [S48] | Beschreibung geändert oder fehlerhaft; aktuelle Seite steht auf Microsoft Learn |
| W08 | `schtasks create`: „Only Administrators can schedule tasks, regardless of the value of the /ru parameter.“ [S50] | Win32-Seite: Nicht-Administratoren können Aufgaben im eigenen Konto (S4U/interaktiv) ohne Passwort und sonst mit Passwort registrieren [S30] | Geltungsbereich der `schtasks`-Aussage unklar (Abschnitt über Systemrechte) |
| W09 | Q&A-Thread (2024-02-08): SSH zu einem Microsoft-Konto-Rechner sei nicht möglich; OpenSSH-Server unterstütze nur lokale oder Active-Directory-Konten [S56] | Learn: „internet-connected accounts are always resolved to their local account name“ [S35]; Blog (laut Suchzusammenfassung): Anmeldung mit dem Kurznamen des Profils und dem Microsoft-Konto-Passwort sei möglich [S56] | Belege nur Community; kein Primärbeleg (L11) |
| W10 | `MSV1_0_LOGON_SUBMIT_TYPE`: `MsV1_0S4ULogon` ab Windows 7 (nur ältere Versionen als nicht unterstützt genannt) [S25] | `LsaLogonUser`-Seite beschreibt S4U nur für Kerberos und Domänenkonten; keine Strukturseite für `MSV1_0_S4U_LOGON` [S23], [S26] | MSV1_0-S4U auf Learn nur über die Enumeration belegt (L08) |
| W11 | `CreateProcessAsUserW` verweist bei Fehler 1314 auf `CreateProcessWithLogonW` [S02] | `CreateProcessWithLogonW` ist aus LocalSystem nicht aufrufbar [S03]; das LocalSystem-Token enthält die Privilegien von `CreateProcessAsUserW` [S10] | kein Widerspruch; Reichweite der Hinweise je Aufrufer unterschiedlich |

## 5. Offene Lücken

| Nr. | Fehlender Fakt |
|---|---|
| L01 | Definition von „assignable“ in der Aussage „may require the SE_ASSIGNPRIMARYTOKEN_NAME privilege if the token is not assignable“ (`CreateProcessAsUserW`). |
| L02 | Ob `SE_INCREASE_QUOTA_NAME` für ein eingeschränktes Token des eigenen Prozesses entfällt (die Seite sagt „typically“; die Seite „Restricted Tokens“ nennt nur `SE_ASSIGNPRIMARYTOKEN_NAME`). |
| L03 | Vollständige Privilegienliste des erhöhten (vollen) Administrator-Tokens; Standardzuweisung der Rechte `SeBackupPrivilege` und `SeRestorePrivilege` (nicht gelesen). |
| L04 | Ob `LogonUserW(LOGON32_LOGON_INTERACTIVE)` für ein Administratorkonto das gefilterte Token liefert (die `LogonUserW`-Seite sagt nichts zu UAC). |
| L05 | Ob ein ConPTY-Kindprozess Zugriff auf Fensterstation/Desktop des Zielkontos braucht; Ursachen von `0xc0000142` außer den zwei in der ConPTY-Dokumentation genannten Fällen; aktuelles Verhalten von `CreateProcessWithLogonW` bei Fensterstation/Desktop-Berechtigungen. |
| L06 | Verhalten von `CreateProcessWithLogonW` und `CreateProcessWithTokenW` mit `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE`: kein Microsoft-Text und kein Issue im Repository `microsoft/terminal` gefunden, das es ausdrücklich beschreibt; nur der Community-Thread [S20]. |
| L07 | Ob `ConptyCreatePseudoConsoleAsUser(hToken, size, hInput, hOutput, dwFlags, phPC)` zusammen mit `CreateProcessAsUserW` und `PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE` für ein abweichendes Konto durchgängig funktioniert; Dokumentation der Funktion; Mindestversion der `AsUser`-Variante. |
| L08 | `MSV1_0_S4U_LOGON`: keine Strukturseite auf Microsoft Learn (HTTP 404 am 2026-10-17); Flag-Werte der Struktur nicht gelesen; `LsaLogonUser`-Seite führt nur `KERB_S4U_LOGON`. |
| L09 | Rechte des Zielkontos für die S4U-Anmeldung je Anmeldetyp: die `LsaLogonUser`-Seite nennt keine; nur die Task-Scheduler-Seite (`SeBatchLogonRight`) und das Drittprojekt [S32]. |
| L10 | Ob die MSV1_0-S4U-Anmeldung ohne `SeTcbPrivilege` ein Identification-Token liefert (auf Learn und im Blog nur für Kerberos beschrieben). |
| L11 | `LogonUserW` mit Microsoft-Konto: akzeptiertes Passwort und Benutzername (lokaler Kurzname, E-Mail-Adresse, UPN); Bedeutung von „Passport account“ auf der `LogonUserW`-Seite. |
| L12 | `LogonUserW` mit Windows-Hello-PIN. |
| L13 | Credential Manager aus einem LocalSystem-Dienst (`CredWriteW` im SYSTEM-Anmeldekontext); Lesezugriff durch Prozesse desselben Kontos, die den Eintrag nicht geschrieben haben, auf einen `CRED_TYPE_GENERIC`-Eintrag. |
| L14 | Auswirkung einer Passwortänderung auf `CredWriteW`- und LSA-Secret-Einträge (nicht ausdrücklich dokumentiert; nur abgeleitet). |
| L15 | Editionsunterschiede Windows Home/Pro für `NetUserAdd` & Co., `CredWriteW`-Persistenz (die Home-Aussage betrifft Windows Vista und Windows XP), `CreateRestrictedToken`, AppContainer, ConPTY; die Aussage zu `lusrmgr.msc` stammt nur aus Community-Beiträgen. |
| L16 | Ob UAC-Erhöhung Bedingung für `NetUserAdd`/`NetLocalGroupAddMembers` ist (die Seiten sagen „Administrators and Power Users“ ohne UAC-Aussage). |
| L17 | Ausblenden eines Kontos auf dem Anmeldebildschirm: nur Community-Beiträge. |
| L18 | Verhalten von `cmd.exe` und `powershell.exe` im AppContainer/LPAC (mit und ohne ConPTY); Veröffentlichungsstand von PowerShell-PR #27266. |
| L19 | Ob `cmd.exe` und `powershell.exe` `ReadConsoleOutput`, `ReadConsoleOutputCharacter`, `ReadConsoleOutputAttribute` oder `WriteConsoleInput` aufrufen. |
| L20 | Aktuelle `runas`-Seite (HTTP 404); Ablageort der `/savecred`-Anmeldedaten; Editionsaussage für Windows 10/11. |
| L21 | PsExec lokal mit `-u`: Mechanismus und Mindestrechte unter Windows 10/11 (nur TechNet-Primer 2007); Erklärartikel von Mark Russinovich nicht lesbar. |
| L22 | Aufgabenplanung, WinRM und `winrs` in Verbindung mit ConPTY/Pseudokonsole; Sitzung und Fensterstation von Aufgabenprozessen. |
| L23 | Wirkung der Richtlinie „Accounts: Limit local account use of blank passwords to console logon only“ auf lokale `LogonUserW`-Aufrufe (die Seite spricht von „remote client“). |
| L24 | Verbleib des Profilordners nach `NetUserDel` (die Seite sagt nichts). |
| L25 | Unterstützung von `MsV1_0S4ULogon` unter Windows 10 und Windows 11 (nur ältere Versionen als nicht unterstützt genannt). |
| L26 | Die Wiki-Seite „About Win32 OpenSSH and Design Details“: Datum der letzten Bearbeitung nicht erfasst. |
| L27 | Zuordnung von `CreateProcessWithLogonW`/`CreateProcessWithTokenW` zum Dienst `seclogon` in Microsoft-Seiten (nur indirekt über `runas`-Fehlerhinweis und Dienstbeschreibung; ausdrücklich nur im Blog [S59]). |

## 6. Quellenverzeichnis

Alle Abrufe: 2026-10-17. „Seitenstand“ ist das Seitenmetadatum `updated_at` bzw. die auf der Seite genannte Angabe.

| Kennung | Titel / Gegenstand | URL | Belegart | Seitenstand |
|---|---|---|---|---|
| S01 | LogonUserW | https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-logonuserw | P | 2024-11-20 |
| S02 | CreateProcessAsUserW | https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessasuserw | P | 2025-07-01 |
| S03 | CreateProcessWithLogonW | https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createprocesswithlogonw | P | 2024-02-22 |
| S04 | CreateProcessWithTokenW | https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createprocesswithtokenw | P | 2024-02-22 |
| S05 | LoadUserProfileW | https://learn.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-loaduserprofilew | P | 2024-11-20 |
| S06 | CreateEnvironmentBlock | https://learn.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-createenvironmentblock | P | 2024-02-22 |
| S07 | Privilege Constants | https://learn.microsoft.com/en-us/windows/win32/secauthz/privilege-constants | P | 2024-03-04 |
| S08 | Richtlinie „Replace a process level token“ | https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-10/security/threat-protection/security-policy-settings/replace-a-process-level-token | P (gilt für Windows 10/11) | 2026-04-27 |
| S09 | Richtlinie „Adjust memory quotas for a process“ | https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-10/security/threat-protection/security-policy-settings/adjust-memory-quotas-for-a-process | P | 2026-04-27 |
| S10 | LocalSystem Account | https://learn.microsoft.com/en-us/windows/win32/services/localsystem-account | P | 2025-04-15 |
| S11 | Window Station Security and Access Rights | https://learn.microsoft.com/en-us/windows/win32/winstation/window-station-security-and-access-rights | P | 2025-03-11 |
| S12 | Process Connection to a Window Station | https://learn.microsoft.com/en-us/windows/win32/winstation/process-connection-to-a-window-station | P | 2025-03-11 |
| S13 | Desktop Security and Access Rights | https://learn.microsoft.com/en-us/windows/win32/winstation/desktop-security-and-access-rights | P | 2025-03-11 |
| S14 | Starting an Interactive Client Process in C++ | https://learn.microsoft.com/en-us/previous-versions/aa379608(v=vs.85) | M (Build 2013-11-16) | 2021-12-02 |
| S15 | WinSDK-Blog: How to launch a process as a different user without storing their credentials? | https://learn.microsoft.com/en-us/archive/blogs/winsdk/how-to-launch-a-process-as-a-different-user-without-storing-their-credentials | M | ms.date 2013-12-27 |
| S16 | CreatePseudoConsole | https://learn.microsoft.com/en-us/windows/console/createpseudoconsole | P | 2024-02-23 |
| S17 | Creating a Pseudoconsole session | https://learn.microsoft.com/en-us/windows/console/creating-a-pseudoconsole-session | P | 2025-08-05 |
| S18 | ClosePseudoConsole | https://learn.microsoft.com/en-us/windows/console/closepseudoconsole | P | 2025-07-09 |
| S19 | Console Buffer Security and Access Rights | https://learn.microsoft.com/en-us/windows/console/console-buffer-security-and-access-rights | P | 2025-08-05 |
| S20 | Microsoft Q&A: CreatePseudoConsole with reduced integrity level | https://learn.microsoft.com/en-us/answers/questions/1040676/createpseudoconsole-with-reduced-integrity-level | C | Frage 2022-10-09 |
| S21 | CreateProcessWithLogonW fails when called on a Microsoft Entra account | https://learn.microsoft.com/en-us/troubleshoot/windows/win32/createprocesswithlogonw-fails-microsoft-entra-id-account | P | 2025-06-11 |
| S22 | Process Creation Flags | https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags | P | 2025-07-14 |
| S23 | LsaLogonUser | https://learn.microsoft.com/en-us/windows/win32/api/ntsecapi/nf-ntsecapi-lsalogonuser | P | 2024-02-22 |
| S24 | KERB_S4U_LOGON | https://learn.microsoft.com/en-us/windows/win32/api/ntsecapi/ns-ntsecapi-kerb_s4u_logon | P | 2024-02-22 |
| S25 | MSV1_0_LOGON_SUBMIT_TYPE | https://learn.microsoft.com/en-us/windows/win32/api/ntsecapi/ne-ntsecapi-msv1_0_logon_submit_type | P | 2024-02-22 |
| S26 | MSV1_0_S4U_LOGON (windows-docs-rs-Bindings; Learn-Seite `https://learn.microsoft.com/en-us/windows/win32/api/ntsecapi/ns-ntsecapi-msv1_0_s4u_logon` lieferte HTTP 404) | https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/Security/Authentication/Identity/struct.MSV1_0_S4U_LOGON.html | P (generierte Bindings-Doku) | nicht erfasst |
| S27 | LsaRegisterLogonProcess | https://learn.microsoft.com/en-us/windows/win32/api/ntsecapi/nf-ntsecapi-lsaregisterlogonprocess | P | 2024-02-22 |
| S28 | LsaConnectUntrusted | https://learn.microsoft.com/en-us/windows/win32/api/ntsecapi/nf-ntsecapi-lsaconnectuntrusted | P | 2024-02-22 |
| S29 | WinSDK-Blog: Logon as a user without a password | https://learn.microsoft.com/en-us/archive/blogs/winsdk/logon-as-a-user-without-a-password | M | ms.date 2015-08-28 |
| S30 | Security Contexts for Tasks | https://learn.microsoft.com/en-us/windows/win32/taskschd/security-contexts-for-running-tasks | P | 2025-05-12 |
| S31 | TASK_LOGON_TYPE | https://learn.microsoft.com/en-us/windows/win32/api/taskschd/ne-taskschd-task_logon_type | P | 2024-02-22 |
| S32 | rgl/s-4-u-for-windows (README) | https://github.com/rgl/s-4-u-for-windows | D | nicht erfasst |
| S33 | Win32-OpenSSH-Quelltext, Repository PowerShell/openssh-portable, Branch `latestw_all`: `contrib/win32/win32compat/w32fd.c`, `spawn-ext.c`, `spawn.c`, `win32_usertoken_utils.c`, `win32_pty.c`, `shell-host.c`, `w32-doexec.c`, `wmain_sshd-session.c`; `sshd-session.c` | https://raw.githubusercontent.com/PowerShell/openssh-portable/latestw_all/ (Dateipfade wie angegeben) | Q | Branch-Stand 2026-10-17 |
| S34 | Win32-OpenSSH-Wiki: About Win32 OpenSSH and Design Details; TTY PTY support in Windows OpenSSH (zuletzt 2021-04-12); SSH remote sessions on Windows (zuletzt 2017-05-16) | https://github.com/PowerShell/Win32-OpenSSH/wiki | Q | siehe Titel |
| S35 | OpenSSH Server Configuration for Windows; OpenSSH for Windows overview | https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh-server-configuration ; https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh-overview | P | 2025-08-05 ; 2025-05-01 (ms.date 2025-02-20) |
| S36 | Win32-OpenSSH-Issues #1339 und #2186 | https://github.com/PowerShell/Win32-OpenSSH/issues/1339 ; https://github.com/PowerShell/Win32-OpenSSH/issues/2186 | Q/C | #1339: 2019-02; #2186: 2023-12 |
| S37 | Tailscale: `util/winutil/s4u/s4u_windows.go`, `lsa_windows.go`, `util/winutil/conpty/conpty_windows.go`; Paketseiten pkg.go.dev (v1.104.1, 2026-10-07) | https://github.com/tailscale/tailscale/tree/main/util/winutil/s4u ; https://github.com/tailscale/tailscale/tree/main/util/winutil/conpty ; https://raw.githubusercontent.com/tailscale/tailscale/main/util/winutil/s4u/s4u_windows.go ; https://raw.githubusercontent.com/tailscale/tailscale/main/util/winutil/s4u/lsa_windows.go ; https://raw.githubusercontent.com/tailscale/tailscale/main/util/winutil/conpty/conpty_windows.go ; https://pkg.go.dev/tailscale.com/util/winutil/conpty | Q | Branch `main` am 2026-10-17 |
| S38 | microsoft/terminal: `src/winconpty/winconpty.cpp`, `winconpty.h`, `dll/winconpty.def`, Commit `cc8faaf`; NuGet `Microsoft.Windows.Console.ConPTY` | https://raw.githubusercontent.com/microsoft/terminal/main/src/winconpty/winconpty.cpp ; https://github.com/microsoft/terminal/commit/cc8faaf04f71fa4fe61ff97333672d2194a34af7 ; https://www.nuget.org/packages/Microsoft.Windows.Console.ConPTY | Q | Branch `main` am 2026-10-17; NuGet 2026-10-02 |
| S39 | CredWriteW; CREDENTIALW | https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew ; https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentialw | P | 2024-11-20 ; 2025-11-25 |
| S40 | CryptProtectData; Windows Data Protection (NAI Labs, Oktober 2001); DPAPI backup keys on Active Directory domain controllers | https://learn.microsoft.com/en-us/windows/win32/api/dpapi/nf-dpapi-cryptprotectdata ; https://learn.microsoft.com/en-us/previous-versions/ms995355(v=msdn.10) ; https://learn.microsoft.com/en-us/windows/win32/seccng/cng-dpapi-backup-keys-on-ad-domain-controllers | P ; M ; P | 2026-05-15 ; 2021-12-02 ; 2025-04-15 |
| S41 | LsaStorePrivateData | https://learn.microsoft.com/en-us/windows/win32/api/ntsecapi/nf-ntsecapi-lsastoreprivatedata | P | 2025-07-01 |
| S42 | Credentials Processes in Windows Authentication | https://learn.microsoft.com/en-us/windows-server/security/windows-authentication/credentials-processes-in-windows-authentication | P | 2026-02-16 |
| S43 | Windows Hello for Business overview; Go passwordless in Windows; Change or reset your Windows password | https://learn.microsoft.com/en-us/windows/security/identity-protection/hello-for-business/ ; https://support.microsoft.com/en-us/topic/585a71d7-2295-4878-aeac-a014984df856 ; https://support.microsoft.com/en-us/windows/change-or-reset-your-windows-password-8271d17c-9f9e-443f-835a-8318c8f68b9c | P | 2026-10-08 ; ohne Datum ; ohne Datum |
| S44 | NetUserAdd; USER_INFO_1; NetUserDel; NetLocalGroupAddMembers; Requirements for Network Management Functions on Servers and Workstations | https://learn.microsoft.com/en-us/windows/win32/api/lmaccess/nf-lmaccess-netuseradd ; https://learn.microsoft.com/en-us/windows/win32/api/lmaccess/ns-lmaccess-user_info_1 ; https://learn.microsoft.com/en-us/windows/win32/api/lmaccess/nf-lmaccess-netuserdel ; https://learn.microsoft.com/en-us/windows/win32/api/lmaccess/nf-lmaccess-netlocalgroupaddmembers ; https://learn.microsoft.com/en-us/windows/win32/netmgmt/requirements-for-network-management-functions-on-servers-and-workstations | P | 2024-02-22 (Funktionsseiten); 2025-03-11 |
| S45 | CreateProfile; DeleteProfileW; LsaAddAccountRights; Account Rights Constants; Richtlinie „Accounts: Limit local account use of blank passwords to console logon only“ | https://learn.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-createprofile ; https://learn.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-deleteprofilew ; https://learn.microsoft.com/en-us/windows/win32/api/ntsecapi/nf-ntsecapi-lsaaddaccountrights ; https://learn.microsoft.com/en-us/windows/win32/secauthz/account-rights-constants ; https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-10/security/threat-protection/security-policy-settings/accounts-limit-local-account-use-of-blank-passwords-to-console-logon-only | P | 2024-02-22 ; 2024-11-20 ; 2025-07-01 ; 2021-01-07 ; 2026-04-27 |
| S46 | CreateRestrictedToken; Restricted Tokens; Mandatory Integrity Control; AppContainer isolation; Launch an AppContainer; CreateAppContainerProfile; Job Objects; JOBOBJECT_BASIC_UI_RESTRICTIONS; UWP console app; Win32 app isolation overview | https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-createrestrictedtoken ; https://learn.microsoft.com/en-us/windows/win32/secauthz/restricted-tokens ; https://learn.microsoft.com/en-us/windows/win32/secauthz/mandatory-integrity-control ; https://learn.microsoft.com/en-us/windows/win32/secauthz/appcontainer-isolation ; https://learn.microsoft.com/en-us/windows/win32/secauthz/implementing-an-appcontainer ; https://learn.microsoft.com/en-us/windows/win32/api/userenv/nf-userenv-createappcontainerprofile ; https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects ; https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_basic_ui_restrictions ; https://learn.microsoft.com/en-us/windows/uwp/launch-resume/console-uwp ; https://learn.microsoft.com/en-us/windows/win32/secauthz/app-isolation-overview | P | 2025-07-01 ; 2025-03-12 ; 2025-07-10 ; 2025-07-10 ; 2025-09-03 ; 2024-11-27 ; 2025-07-16 ; 2024-02-22 ; 2025-06-10 ; 2025-05-15 |
| S47 | Mark Russinovich, PsExec, User Account Control and Security Boundaries (2007-02-12); Chris Corio, Teach Your Apps To Work With Windows Vista User Account Control (MSDN Magazine, Januar 2007); How User Account Control works | https://learn.microsoft.com/en-us/archive/blogs/markrussinovich/psexec-user-account-control-and-security-boundaries ; https://learn.microsoft.com/en-us/archive/msdn-magazine/2007/january/teach-your-apps-to-work-with-windows-vista-user-account-control ; https://learn.microsoft.com/en-us/windows/security/application-security/application-control/user-account-control/how-it-works | M ; M ; P | 2024-11-06 ; 2022-01-21 ; 2026-10-08 |
| S48 | PsExec (Sysinternals); The Desktop Files: PsTools Primer (Wes Miller, TechNet Magazine, 2007-03) | https://learn.microsoft.com/en-us/sysinternals/downloads/psexec ; https://learn.microsoft.com/en-us/previous-versions/technet-magazine/cc162490(v=msdn.10) | P ; M | 2023-10-18 (Veröffentlicht 2023-04-11) ; 2022-01-21 |
| S49 | Runas (Windows Server 2012 R2, archiviert); Enable and use the Run As feature (KB 325859); Security guidelines for system services in Windows Server 2016 (Eintrag seclogon) | https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-server-2012-r2-and-2012/cc771525(v=ws.11) ; https://learn.microsoft.com/en-us/troubleshoot/windows-server/shell-experience/use-run-as-feature ; https://learn.microsoft.com/en-us/windows-server/security/windows-services/security-guidelines-for-disabling-system-services-in-windows-server | M ; P ; P | 2021-11-12 (ms.date 2016-08-31) ; 2026-02-19 ; 2026-02-16 |
| S50 | schtasks create | https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/schtasks-create | P | 2025-10-22 |
| S51 | about_Remote_Requirements; Enable-PSRemoting; Security considerations for PowerShell Remoting using WinRM; PowerShell Remoting FAQ; winrs | https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_remote_requirements ; https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/enable-psremoting ; https://learn.microsoft.com/en-us/powershell/scripting/security/remoting/winrm-security ; https://learn.microsoft.com/en-us/powershell/scripting/learn/remoting/powershell-remoting-faq ; https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/winrs | P | 2025-11-07 ; 2025-02-24 ; 2025-12-09 ; 2025-12-09 ; 2024-11-01 |
| S52 | Sudo for Windows | https://learn.microsoft.com/en-us/windows/advanced-settings/sudo/ | P | 2026-02-26 |
| S53 | Windows Terminal Spec #5000 Process Model 2.0 | https://raw.githubusercontent.com/microsoft/terminal/main/doc/specs/%235000%20-%20Process%20Model%202.0/%235000%20-%20Process%20Model%202.0.md | Q | Branch `main` am 2026-10-17 |
| S54 | PowerShell-Issue #27253: FileSystem PSDrives not mounted when PowerShell runs inside an AppContainer | https://github.com/PowerShell/PowerShell/issues/27253 | Q/C | geöffnet 2026-04-10, geschlossen 2026-04-20 |
| S55 | Richtlinien „Impersonate a client after authentication“, „Act as part of the operating system“, „Log on as a batch job“ | https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-10/security/threat-protection/security-policy-settings/impersonate-a-client-after-authentication ; https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-10/security/threat-protection/security-policy-settings/act-as-part-of-the-operating-system ; https://learn.microsoft.com/en-us/previous-versions/windows/it-pro/windows-10/security/threat-protection/security-policy-settings/log-on-as-a-batch-job | P | 2026-04-27 |
| S56 | Community-Beiträge. Gelesen: Microsoft Q&A „ssh login to Microsoft account linked Windows impossible“ (Frage 2024-02-08); Microsoft Q&A „Setting local password for an account linked with Microsoft account“ (Antwort zu `lusrmgr.msc` vom 2025-03-18). Nur als Suchzusammenfassung gesehen (nicht einzeln gelesen): Microsoft-Q&A-Thread zu `SpecialAccounts\UserList`, Microsoft-Q&A-Thread zu `runas /savecred`, Blog „John Siu“ zu SSH mit Microsoft-Konto | https://learn.microsoft.com/en-us/answers/questions/4063067/ssh-login-to-microsoft-account-linked-windows-impo ; https://learn.microsoft.com/en-us/answers/questions/3889182/setting-local-password-for-an-account-linked-with ; https://learn.microsoft.com/en-us/answers/questions/1281830/how-to-remove-user-account-from-windows-logon-scre ; https://learn.microsoft.com/en-us/answers/questions/3756027/error-while-opening-software-in-administrator-mode ; https://johnsiu.com/blog/win-ssh/ | C | 2024-02-08 ; 2025-03-17/18 ; übrige nicht erfasst |
| S57 | LogonUserExExW | https://learn.microsoft.com/en-us/windows/win32/secauthn/logonuserexexw | P | 2021-01-07 |
| S58 | Interactive Services | https://learn.microsoft.com/en-us/windows/win32/services/interactive-services | P | 2025-04-15 |
| S59 | TrainSec-Blog (Pavel Yosifovich): Understanding the Differences Between CreateProcessAsUser and CreateProcessWithTokenW in Windows (2024-12-17) | https://trainsec.net/library/windows-internals/understanding-the-differences-between-createprocessasuser-and-createprocesswithtokenw-in-windows/ | C (Blog) | 2024-12-17 |
| S60 | Windows Command-Line: Introducing the Windows Pseudo Console (ConPTY) (Rich Turner) | https://devblogs.microsoft.com/commandline/windows-command-line-introducing-the-windows-pseudo-console-conpty/ | M (Blog) | 2018-08-02 |

## Anhang A: Signaturen (aus den Quellen)

```c
BOOL LogonUserW(
  [in]           LPCWSTR lpszUsername,
  [in, optional] LPCWSTR lpszDomain,
  [in, optional] LPCWSTR lpszPassword,
  [in]           DWORD   dwLogonType,
  [in]           DWORD   dwLogonProvider,
  [out]          PHANDLE phToken);                       // [S01]

BOOL LogonUserExExW(
  _In_      LPTSTR        lpszUsername,
  _In_opt_  LPTSTR        lpszDomain,
  _In_opt_  LPTSTR        lpszPassword,
  _In_      DWORD         dwLogonType,
  _In_      DWORD         dwLogonProvider,
  _In_opt_  PTOKEN_GROUPS pTokenGroups,
  _Out_opt_ PHANDLE       phToken,
  _Out_opt_ PSID          *ppLogonSid,
  _Out_opt_ PVOID         *ppProfileBuffer,
  _Out_opt_ LPDWORD       pdwProfileLength,
  _Out_opt_ PQUOTA_LIMITS pQuotaLimits);                 // [S57]

BOOL CreateProcessAsUserW(
  [in, optional]      HANDLE                hToken,
  [in, optional]      LPCWSTR               lpApplicationName,
  [in, out, optional] LPWSTR                lpCommandLine,
  [in, optional]      LPSECURITY_ATTRIBUTES lpProcessAttributes,
  [in, optional]      LPSECURITY_ATTRIBUTES lpThreadAttributes,
  [in]                BOOL                  bInheritHandles,
  [in]                DWORD                 dwCreationFlags,
  [in, optional]      LPVOID                lpEnvironment,
  [in, optional]      LPCWSTR               lpCurrentDirectory,
  [in]                LPSTARTUPINFOW        lpStartupInfo,
  [out]               LPPROCESS_INFORMATION lpProcessInformation);   // [S02]

BOOL CreateProcessWithLogonW(
  [in]                LPCWSTR               lpUsername,
  [in, optional]      LPCWSTR               lpDomain,
  [in]                LPCWSTR               lpPassword,
  [in]                DWORD                 dwLogonFlags,
  [in, optional]      LPCWSTR               lpApplicationName,
  [in, out, optional] LPWSTR                lpCommandLine,
  [in]                DWORD                 dwCreationFlags,
  [in, optional]      LPVOID                lpEnvironment,
  [in, optional]      LPCWSTR               lpCurrentDirectory,
  [in]                LPSTARTUPINFOW        lpStartupInfo,
  [out]               LPPROCESS_INFORMATION lpProcessInformation);   // [S03]

BOOL CreateProcessWithTokenW(
  [in]                HANDLE                hToken,
  [in]                DWORD                 dwLogonFlags,
  [in, optional]      LPCWSTR               lpApplicationName,
  [in, out, optional] LPWSTR                lpCommandLine,
  [in]                DWORD                 dwCreationFlags,
  [in, optional]      LPVOID                lpEnvironment,
  [in, optional]      LPCWSTR               lpCurrentDirectory,
  [in]                LPSTARTUPINFOW        lpStartupInfo,
  [out]               LPPROCESS_INFORMATION lpProcessInformation);   // [S04]

USERENVAPI BOOL LoadUserProfileW(
  [in]      HANDLE         hToken,
  [in, out] LPPROFILEINFOW lpProfileInfo);               // [S05]

BOOL CreateEnvironmentBlock(
  [out]          LPVOID *lpEnvironment,
  [in, optional] HANDLE hToken,
  [in]           BOOL   bInherit);                       // [S06]

HRESULT WINAPI CreatePseudoConsole(
    _In_ COORD size, _In_ HANDLE hInput, _In_ HANDLE hOutput,
    _In_ DWORD dwFlags, _Out_ HPCON* phPC);              // [S16]

HRESULT WINAPI ConptyCreatePseudoConsoleAsUser(
    _In_ HANDLE hToken, _In_ COORD size, _In_ HANDLE hInput,
    _In_ HANDLE hOutput, _In_ DWORD dwFlags, _Out_ HPCON* phPC);   // [S38], winconpty.h

NTSTATUS LsaLogonUser(
  [in]           HANDLE              LsaHandle,
  [in]           PLSA_STRING         OriginName,
  [in]           SECURITY_LOGON_TYPE LogonType,
  [in]           ULONG               AuthenticationPackage,
  [in]           PVOID               AuthenticationInformation,
  [in]           ULONG               AuthenticationInformationLength,
  [in, optional] PTOKEN_GROUPS       LocalGroups,
  [in]           PTOKEN_SOURCE       SourceContext,
  [out]          PVOID               *ProfileBuffer,
  [out]          PULONG              ProfileBufferLength,
  [out]          PLUID               LogonId,
  [out]          PHANDLE             Token,
  [out]          PQUOTA_LIMITS       Quotas,
  [out]          PNTSTATUS           SubStatus);         // [S23]

typedef struct _KERB_S4U_LOGON {
  KERB_LOGON_SUBMIT_TYPE MessageType;   // KerbS4ULogon
  ULONG                  Flags;         // 0x2 CHECK_LOGONHOURS, 0x8 IDENTIFY
  UNICODE_STRING         ClientUpn;
  UNICODE_STRING         ClientRealm;
} KERB_S4U_LOGON, *PKERB_S4U_LOGON;                      // [S24]

// MSV1_0_S4U_LOGON (Rust-Bindings, [S26]):
// MessageType: MSV1_0_LOGON_SUBMIT_TYPE (MsV1_0S4ULogon = 12), Flags: u32,
// UserPrincipalName: LSA_UNICODE_STRING, DomainName: LSA_UNICODE_STRING

NTSTATUS LsaRegisterLogonProcess(
  [in]  PLSA_STRING           LogonProcessName,
  [out] PHANDLE               LsaHandle,
  [out] PLSA_OPERATIONAL_MODE SecurityMode);             // [S27]

NTSTATUS LsaConnectUntrusted([out] PHANDLE LsaHandle);   // [S28]

BOOL CredWriteW([in] PCREDENTIALW Credential, [in] DWORD Flags);   // [S39]

DPAPI_IMP BOOL CryptProtectData(
  [in]           DATA_BLOB                 *pDataIn,
  [in, optional] LPCWSTR                   szDataDescr,
  [in, optional] DATA_BLOB                 *pOptionalEntropy,
  [in]           PVOID                     pvReserved,
  [in, optional] CRYPTPROTECT_PROMPTSTRUCT *pPromptStruct,
  [in]           DWORD                     dwFlags,
  [out]          DATA_BLOB                 *pDataOut);   // [S40]

NTSTATUS LsaStorePrivateData(
  [in] LSA_HANDLE          PolicyHandle,
  [in] PLSA_UNICODE_STRING KeyName,
  [in] PLSA_UNICODE_STRING PrivateData);                 // [S41]

NET_API_STATUS NET_API_FUNCTION NetUserAdd(
  [in] LPCWSTR servername, [in] DWORD level, [in] LPBYTE buf, [out] LPDWORD parm_err);   // [S44]
NET_API_STATUS NET_API_FUNCTION NetUserDel([in] LPCWSTR servername, [in] LPCWSTR username);   // [S44]
NET_API_STATUS NET_API_FUNCTION NetLocalGroupAddMembers(
  [in] LPCWSTR servername, [in] LPCWSTR groupname, [in] DWORD level,
  [in] LPBYTE buf, [in] DWORD totalentries);             // [S44]

USERENVAPI HRESULT CreateProfile(
  [in]  LPCWSTR pszUserSid, [in] LPCWSTR pszUserName,
  [out] LPWSTR pszProfilePath, [in] DWORD cchProfilePath);   // [S45]
USERENVAPI BOOL DeleteProfileW(
  [in] LPCWSTR lpSidString, [in, optional] LPCWSTR lpProfilePath,
  [in, optional] LPCWSTR lpComputerName);                // [S45]
NTSTATUS LsaAddAccountRights(
  [in] LSA_HANDLE PolicyHandle, [in] PSID AccountSid,
  [in] PLSA_UNICODE_STRING UserRights, [in] ULONG CountOfRights);   // [S45]

BOOL CreateRestrictedToken(
  [in]           HANDLE               ExistingTokenHandle,
  [in]           DWORD                Flags,
  [in]           DWORD                DisableSidCount,
  [in, optional] PSID_AND_ATTRIBUTES  SidsToDisable,
  [in]           DWORD                DeletePrivilegeCount,
  [in, optional] PLUID_AND_ATTRIBUTES PrivilegesToDelete,
  [in]           DWORD                RestrictedSidCount,
  [in, optional] PSID_AND_ATTRIBUTES  SidsToRestrict,
  [out]          PHANDLE              NewTokenHandle);   // [S46]

USERENVAPI HRESULT CreateAppContainerProfile(
  [in]  PCWSTR              pszAppContainerName,
  [in]  PCWSTR              pszDisplayName,
  [in]  PCWSTR              pszDescription,
  [in]  PSID_AND_ATTRIBUTES pCapabilities,
  [in]  DWORD               dwCapabilityCount,
  [out] PSID                *ppSidAppContainerSid);      // [S46]
```
