# Drive API v3 – Suchnamen, Paging und Objektidentität

Geprüft: 2026-10-04. Gesicherte verwendete Verträge:

- Google [`files.list`](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/list)
- Google [Suchbedingungen](https://developers.google.com/workspace/drive/api/guides/search-files)
- Google [Ordner und Root-Alias](https://developers.google.com/workspace/drive/api/guides/folder)
- Vergleichsimplementierung: [rclone Drive-Backend](https://raw.githubusercontent.com/rclone/rclone/master/backend/drive/drive.go)
  (`list`, clientseitiger exakter Namensvergleich).

## `GET /drive/v3/files`

Parameter: `q`, `fields`, `pageSize` (maximal 1000), optional `pageToken`.
Ein Parent-/Namensfilter lautet:

```text
'<parent-id>' in parents and name = '<literal-title>' and trashed = false
```

Backslash und einfaches Anführungszeichen werden innerhalb der Query-Literale
mit Backslash geschützt; danach wird die gesamte Query als URL-Parameter
kodiert. Feldprojektion für Identitätsprüfung:
`nextPageToken,incompleteSearch,files(id,name,mimeType,parents,trashed,size,md5Checksum,modifiedTime)`.

Die Google-Referenz garantiert für `=` keine bytegenaue Namensgleichheit.
rclone behandelt die Suche ausdrücklich als case-insensitive und filtert die
tatsächlichen `item.Name` danach. Entscheidung: Suchtreffer mit anderem
Literalnamen aussortieren. Das ist kein Hinweis auf doppelte Ordner. Echte
gleichnamige Objekte behalten ihre unterschiedlichen IDs.

Quellabgleich 2026-10-05: Der M1/M2-Provider benutzt diese exakte Titel- und
ID-Prüfung auch für Pfadauflösung und Mutation. `GDriveBackend` meldet deshalb
`case_sensitive_paths=true`: `Notebook` und `notebook` bleiben verschiedene
Literalnamen, auch nach Wiederöffnen. Die Kandidatensuche ist keine Aussage
über die Gleichheit von Providerpfaden. Der konservative VFS-Default und
die gemeinsame NFC-Normalisierung bleiben erhalten; sobald eine Gegenstelle
keine bewiesene Case-Sensitivität meldet, faltet die Pair-KeyPolicy weiterhin
Groß-/Kleinschreibung und schützt unvereinbare Zielnamen. Account-/Root-IDs,
gespeicherte Locators, Literalmarker und bestehende Ordnerbindungen ändern
sich durch diese Fähigkeitsangabe nicht. Der Policywechsel benötigt die
Erhaltung bisheriger Baseline- und seitenspezifischer Schreibweisenrecords;
alte gefaltete Schlüssel dürfen nicht als beschädigte Zustände verworfen
oder als neue Pfadauswahl interpretiert werden.

Leere/teilweise Seiten dürfen vor dem Ende auftreten. `nextPageToken` bestimmt
das Ende. Neue oder entfernte Objekte können Ergebnisse während der Pagination
verändern. Ein erneut auftauchendes gleiches Objekt ist keine zweite Identität;
eine Sammlung dedupliziert anhand `id`. Eine zyklische Tokenfolge beweist keine
vollständige Auflistung. Bei verworfenem Token verlangt Google einen Neustart
der Pagination; ein Retry verwirft den unvollständigen Versuch.

`incompleteSearch=true` bedeutet ausdrücklich unvollständige Ergebnisse. Solche
Ergebnisse dürfen keinen vollständigen Sync-Snapshot oder eine bewiesene
Abwesenheit erzeugen. Die Suche wird mit geeignetem Corpus wiederholt; bleibt
der Dienst vorübergehend unvollständig, bleibt der Lauf wiederanlaufbar.
Ein willkürliches Seitenbudget ist keine Providergrenze.

## `GET /drive/v3/files/<fileId>?fields=...`

`root` ist ein zulässiger Alias überall dort, wo ein `fileId` angegeben wird.
Der Root-Ordner hat trotzdem eine eigene tatsächliche ID. `files/root?fields=id`
liefert diese Identität. Ein `parents`-Feld enthält die tatsächliche Eltern-ID;
Vergleiche gegen einen angefragten Root-Alias müssen diesen auflösen.

Namen sind nicht eindeutig. Für ausgewählte Dateien/Ordner werden `id`,
`name`, `mimeType`, `parents` und `trashed` frisch gegen die erfasste Identität
geprüft. Elternzugehörigkeit und Objekttyp dürfen nicht allein wegen eines
Namenssuchtreffers angenommen werden. Ein Snapshot oder ein Update erhält die
vorherige gültige Ordnerbindung; ein anderes Konto erhält sie nicht.

## Lokale Syntax und Infrastruktur

HTTP-Agent-, Fehler- und Retry-Signaturen: [gdrive-ureq-throughput.md](gdrive-ureq-throughput.md).
Metadaten, mtime, Checksummen und Mutationen:
[sync-remote-metadata.md](sync-remote-metadata.md), Abschnitt Google Drive.
Kandidatengebundener Remote-Testhost und Parser:
[rv1-remote-suite.md](rv1-remote-suite.md).

## Private Bindungstransaktion (vorhandene Rust-Syntax)

[Rust std 1.99 `File`](https://doc.rust-lang.org/std/fs/struct.File.html#method.lock),
geprüft 2026-10-04: `lock(&self) -> io::Result<()>` erwirbt eine exklusive
Dateisperre und wartet. Die letzte Handle-Schließung gibt die Sperre frei.
Dasselbe bereits gesperrte Handle darf nicht rekursiv gesperrt werden;
das Verhalten ist dann plattformabhängig bis zum Deadlock.

Vorhandene Adapter: `support_dirs::ensure_private_dir(&Path) -> io::Result<()>`,
`open_private_lock(&Path) -> io::Result<File>`,
`open_private_file(&Path) -> io::Result<File>` und
`write_private_atomic(&Path, &[u8]) -> io::Result<()>`. Die Atomic-Hülle
prüft das vorhandene Ziel, erzeugt eine private exklusive Stage, schreibt
und `sync_all`-bestätigt sie, ersetzt über den bestehenden VFS-OS-Adapter
und bestätigt den Parent. Fehler bewahren die bisherige Datei. Alle
Hostaufrufe bleiben im neuen `gdrive/os/shared`-Persistenzadapter.

Die Transaktion benutzt einen Prozessmutex und eine separate private
Lockdatei; Zustand wird unter der gehaltenen Dateisperre neu gelesen,
zusammengeführt und dauerhaft gespeichert. Netzwerkprüfungen finden vor
der Sperre statt; keine rekursive Netzwerk-/Storetransaktion unter Lock.
