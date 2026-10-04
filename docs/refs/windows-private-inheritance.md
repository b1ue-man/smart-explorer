# Windows: Owner-Rechte bei bestehenden privaten Appdaten

Geprüft am 2026-10-04 für die gemeldeten 0.5.170-Start-/OAuth-Regressionen.

[SetSecurityInfo](https://learn.microsoft.com/en-us/windows/win32/api/aclapi/nf-aclapi-setsecurityinfo)
wendet Änderungen der vererbbaren ACEs auf vorhandene Kinder an. Der Rückgabewert
ist ein Win32-Fehlercode; nur Null bedeutet Erfolg. Die vorhandene Ownerprüfung
und die DACL-only-Mutation ohne Owner-/SACL-Änderung bleiben erforderlich.

[Automatische Vererbung](https://learn.microsoft.com/en-us/windows/win32/secauthz/automatic-propagation-of-inheritable-aces)
entfernt Kinderkopien einer entfernten vererbbaren ACE. Bleibt keine ACE übrig,
entsteht eine leere DACL, die jeden gewöhnlichen Datenzugriff verweigert. Eine neue
vererbbare ACE kann auch einer vorhandenen leeren Kinder-DACL wieder Rechte geben.
Geschützte Kind-DACLs müssen separat am jeweiligen Verzeichnis aktualisiert werden.

[AddAccessAllowedAceEx](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-addaccessallowedaceex):
`BOOL AddAccessAllowedAceEx(PACL, DWORD revision, DWORD flags, DWORD mask, PSID)`.
`OBJECT_INHERIT_ACE` gilt für Dateien, `CONTAINER_INHERIT_ACE` für Verzeichnisse.
Ohne `INHERIT_ONLY_ACE` gilt dieselbe ACE auch auf dem Parent. Gepinnter lokaler
`windows-sys 0.59.0`-Quelltext bestätigt Funktion und u32-Flags 1/2; `ACE_HEADER`
speichert sie als u8. Die ACL-Puffergröße bleibt bei einer ACE unverändert.

Entscheidung nach zweiter Lückenprüfung: nur die bereits geprüfte aktuelle
Owner-SID erhält `FILE_ALL_ACCESS`. Verzeichnisse setzen Flags 1|2 und bleiben
gegen übergeordnete Vererbung geschützt; private Dateien behalten Flags 0.
Die Verifikation erkennt ältere nicht vererbende Verzeichnis-DACLs als
migrationsbedürftig. Microsoft empfiehlt für Datei-/Verzeichnisobjekte
`SetSecurityInfo`; die untersuchte Alternative
[SetKernelObjectSecurity](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-setkernelobjectsecurity)
wird nicht verwendet. Bestehende NoFollow-/Owner-/Hardlink-Prüfungen bleiben.

Der erste Remote-Lauf bestätigte zusätzlich einen Fehler der 0.5.170-
Ownerprüfung: gewöhnlich erstellte Windows-Objekte können dem Standardbesitzer
des effektiven Tokens gehören, der bei administrativen Tokens eine Gruppe ist.
[TOKEN_OWNER](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-token_owner)
und [Owner of a New Object](https://learn.microsoft.com/en-us/windows/win32/secauthz/owner-of-a-new-object)
beschreiben diese vom System gültig bestimmte Benutzer-/Gruppen-SID.
[GetTokenInformation](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-gettokeninformation)
liest `TokenUser` und `TokenOwner` mit `TOKEN_QUERY` am selben effektiven Token.
Der Fix akzeptiert genau diese beiden Owner-SIDs. DACL-Zugriff erhält weiterhin
nur `TokenUser`; Owner-/SACL-Änderungen, Takeover und beliebige Gruppenmitgliedschaft
sind kein Teil des Pfads. Die vorhandene Hardlink-Grenze bleibt vor der Mutation.
Windows-Suite prüft den unveränderten tatsächlichen Owner und lehnt World-SID
und Null-SID ab. API-/Binding-Lücken erneut am 2026-10-04 geprüft.

Die API-Aussagen sind Quellenbelege. Den Laufzeitbeweis lieferte am 2026-10-04 die
[fokussierte Remote-Windows-Suite](https://github.com/b1ue-man/smart-explorer/actions/runs/37220814819)
für Source `e388ea36`: echte vorhandene Dateien und DACLs reproduzierten den
Zugriffsverlust sowie `os error 183`; der reparierte Zugriff erhält Jobs,
Konfiguration und tatsächlichen Owner. Gewöhnliche neue Kinder bleiben zugänglich,
fremde Owner und private Hardlinks bleiben abgelehnt. Ausgeliefert in
[v0.5.171](https://github.com/b1ue-man/smart-explorer/releases/tag/v0.5.171).
