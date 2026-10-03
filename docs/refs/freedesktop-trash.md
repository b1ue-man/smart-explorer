# Freedesktop Trash – RV1-Recycle-Integration

Geprüft: 2026-10-03. Primärquelle: [Trash Specification 1.0](https://specifications.freedesktop.org/trash/latest/).

Der Home-Papierkorb liegt unter `$XDG_DATA_HOME/Trash`. Auf anderen Volumes sind `.Trash/$uid`
und `.Trash-$uid` vorgesehen; `.Trash` braucht das Sticky-Bit und darf kein Link sein.
Bei fehlendem sicheren Papierkorb muss der Vorgang fehlschlagen; ein permanentes Löschen ist kein Ersatz.

Jeder Eintrag hat einen eindeutigen Namen in `files/` und eine gleichnamige Datei mit Suffix
`.trashinfo` in `info/`. Die Info wird vor dem Umzug exklusiv angelegt. Ihr Format ist:

```ini
[Trash Info]
Path=<URL-escaped original path>
DeletionDate=YYYY-MM-DDThh:mm:ss
```

Home-Einträge können absolute Ursprungspfade enthalten; Volume-Einträge verwenden Pfade relativ
zum Mountroot ohne `..`. `DeletionDate` verwendet die lokale Zeit. Existierende Einträge werden nie ersetzt.

RV1 ergänzt diese Speicherregeln um den vorhandenen DirectoryHandle-Vertrag: geprüfte reguläre
Dateien handlegebunden reversibel einfangen, Identität am eingefangenen Handle bestätigen und den
bestätigten Eintrag ausschließlich über einen no-replace-Umzug in einen geöffneten Zielordner
veröffentlichen. Keine erneute pfadbasierte Löschung nach der Hashprüfung. Fehler beim Umzug stellen
die Datei no-replace zurück oder erhalten eine explizit gemeldete Quarantäne zur Wiederherstellung.
