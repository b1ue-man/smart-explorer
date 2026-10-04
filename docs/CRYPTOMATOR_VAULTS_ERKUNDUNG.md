# Cryptomator-Vaults in Smart Explorer: Erkundungsbericht

Stand und Primärquellenprüfung: **2026-10-04**. Untersuchte Smart-Explorer-Basis:
`2d2dcf36`. Auftrag: Machbarkeit der Vault-Verwaltung, Schwerpunkt Windows,
ergänzend andere Plattformen. Der parallel erarbeitete
[Bericht zur Laufwerksanbindung](refs/cryptomator-mounting-2026-10-04.md)
untersucht das Mounting eigenständig.

## Ergebnis

**Ja: Smart Explorer kann Cryptomator-kompatible Tresore verwalten.** Technisch
lassen sich vorhandene Tresore entsperren, ihre Dateien bearbeiten, neue Tresore
erstellen und Funktionen für Passwortwechsel und Wiederherstellung anbieten.
Welche dieser Funktionen eine Integration tatsächlich mitbringt, hängt vom
gewählten Baustein ab. Die offizielle CLI allein liefert keine vollständige
Tresorverwaltung.

Für Smart Explorer sind drei Wege wesentlich: ein extern geöffnetes
Cryptomator-Laufwerk verwenden; die offiziellen Java-Komponenten über einen
verwalteten Hilfsprozess anbinden; oder eine kompatible Rust-Schicht integrieren.
Die vorhandenen VFS-, Daemon- und Windows-Mountgrenzen bieten Anschlussstellen.
**Die schreibende Vault-Integration ist aber kein fertiger Backend-Steckplatz:**
Lizenz, Klartextablage, stabile Ortsidentität und sichere Schreibtransaktionen
müssen zusammenpassen. Das ist eine Schlussfolgerung aus dem unten belegten
Architekturabgleich, kein Laufzeitnachweis.

Als belastbare Referenz für Format und Verhalten dienen die offiziellen
Komponenten. Als Rust-Kandidat verdient insbesondere OxCrypt eine vertiefte
Bewertung; die hier gefundenen Probleme verbieten eine ungeprüfte Übernahme.
Ein Wechsel des bestehenden Dokany-Mounts zu WinFsp ist für die
Vault-Kompatibilität technisch nicht erforderlich.

## Untersuchungsumfang und Beleggrenzen

Die Untersuchung folgte `arbeitsweise`: lokale Orientierung und Graphabfrage,
erste Erkundung der Integrationswege, breite Primärquellenrecherche,
anschließende zweite Prüfung der Lücken bei Lizenz, Releases, Schlüsselpfaden,
Schreibgarantien und Plattformübertragung. Das Ergebnis sind Befunde und eine
Bewertung; eine Feature-Spezifikation oder Umsetzungsplanung wurde nicht erstellt.

Es wurden keine Anwendungen installiert, Vaults entsperrt, Benchmarks ausgeführt,
Builds oder Tests gestartet. Aussagen über Quelltext sind statische Befunde;
Kompatibilität, Geschwindigkeit und Zuverlässigkeit in Smart Explorer wurden
nicht praktisch nachgewiesen. Insbesondere wurde kein vollständiges
Sicherheitsaudit der Drittbibliotheken vorgenommen.

| Untersuchte Komponente | Referenzstand |
|---|---|
| Cryptomator Desktop | Release **1.19.3**, veröffentlicht 2026-06-29; `fc76f5f83f6f4d3356f3bd161e27719372e1fc74` |
| CryptoFS | **2.10.0**; `488b201f09a8124cbda64f6b68060493ee447de8` |
| CryptoLib | **2.2.2**; `8fabeee72910cd65f73daa9cfc8dc44ffd82605a` |
| Cryptomator CLI | Release **0.6.2**, veröffentlicht 2025-04-10; `65813f5d5b6b416707ec0e9e474ec9287ecc43f2` |
| OxCrypt | `main` am Abrufdatum: `916864f66184dae6f97b0e97e02b879144fcfed6`; `oxcrypt-core` deklariert **0.3.0**, kein hier geprüfter Release-Binary |
| Dar9586/cryptomator-rs | `main`: `c0c96cf45ecf17f18d6ece1e68adf51b7d18446d`; Crypto-Crate deklariert **0.1.3**, kein hier geprüfter Release-Binary |

Die vier offiziellen Release-Stände wurden über GitHubs `releases/latest`
und Tag-Auflösung geprüft. Desktop 1.19.3 bindet genau die genannten
CryptoFS-/CryptoLib-Versionen ein ([Release](https://github.com/cryptomator/cryptomator/releases/tag/1.19.3),
[POM](https://github.com/cryptomator/cryptomator/blob/fc76f5f83f6f4d3356f3bd161e27719372e1fc74/pom.xml)).
Entwicklungszweige sind davon getrennt: beispielsweise zeigt die aktuelle
CLI-README einen 0.7.0-Download und andere Signalhinweise als Release 0.6.2.

## Was ein Cryptomator-Vault technisch ist

Ein Vault ist ein verschlüsselter Verzeichnisbaum. `vault.cryptomator` enthält
eine signierte Konfiguration mit Format, Cipher-Kombination, Vault-ID und
Schlüsselverweis. Bei Passwort-Vaults schützt scrypt einen Key-Encryption-Key;
AES Key Wrap schützt damit die beiden Masterkey-Hälften. Die Signaturprüfung
erfolgt nach Schlüsselgewinnung. Vorher gelesene Schlüsselverweise sind deshalb
unvertrauenswürdige Eingabe. `hub+…` bezeichnet eine weitere Schlüsselquelle mit
zusätzlicher Authentifizierung und Serverkommunikation. Eine Passwortintegration
belegt folglich keine Hub-Unterstützung.
([Sicherheitsarchitektur](https://docs.cryptomator.org/security/architecture/))

Beim dokumentierten **SIV_GCM** bestehen Dateien aus einem 68-Byte-Header und
authentifizierten Inhaltsblöcken mit bis zu 32 KiB Nutzdaten sowie 28 Byte
Zusatzdaten. Namen werden mit AES-SIV und der Elternverzeichnis-ID verarbeitet;
NFC-Normalisierung gehört zum Format. Verzeichnis-IDs verschleiern die Hierarchie,
`.c9s` behandelt lange verschlüsselte Namen, `dirid.c9r` unterstützt die
Wiederherstellung. Das ist wesentlich mehr als AES auf den Dateiinhalt anzuwenden.
([Vault-Kryptografie](https://docs.cryptomator.org/security/vault/))

**Ableitung für Leistung:** Blockweises Lesen erlaubt gezielte Zugriffe statt
vollständiger Entschlüsselung großer Dateien. Bei Fernspeichern bestimmen
zusätzlich Range-Reads, Metadatenabfragen, RTT und Publikationsmöglichkeiten den
Durchsatz. Aus der Blockgröße folgt keine allgemeine Netzwerkgeschwindigkeit.
SIV_CTRMAC ist eine gesonderte Cipher-Kombination; Unterstützung für SIV_GCM
darf nicht als vollständige Format-8-Unterstützung ausgegeben werden.

Cryptomator schützt Cloud-Inhalte und Namen, verbirgt aber nicht sämtliche
Metadaten wie Dateigrößen und Zeitstempel. Entsperrte Inhalte sind für berechtigte
Programme lesbar; deren temporäre Dateien und Backups liegen außerhalb seiner
Kontrolle. Authentifizierte Blöcke bedeuten zudem keine vollständige Absicherung
gegen jede historische Wiederholung oder Vertauschung ganzer Dateien.
([Schutzziel](https://docs.cryptomator.org/security/security-target/),
[Advisory zur Dateivertauschung](https://github.com/cryptomator/cryptomator/security/advisories/GHSA-qwfw-w5qf-7wcj))

## Wie weit die einzelnen Integrationswege tragen

| Weg | Was er ermöglicht | Wesentliche Grenze | Bewertung für Smart Explorer |
|---|---|---|---|
| Extern geöffnetes Laufwerk | Entschlüsselte Dateien wie einen normalen lokalen Ort benutzen | Cryptomator besitzt Entsperren/Sperren; keine eigene Vault-Verwaltung dadurch | Geringste Integrationshürde; sauber als externe Sitzung kenntlich machen |
| Offizielle CLI als Prozess | Einen Passwort-Vault entsperren und mounten; Mounter auflisten | Keine Befehle für Anlegen, Passwortwechsel, Health Check oder vollständige Verwaltung; Prozess-/Unmountsteuerung erforderlich | Geeigneter Einstieg für begrenzte Verwaltung bestehender lokaler Vaults |
| Offizielles CryptoFS/CryptoLib in einem Java-Helfer | Vollständige Dateioperationen; Anlegen, Schlüsselverwaltung und weitere Verwaltungsfunktionen über passende APIs | Java-Laufzeit und eigene Prozessschnittstelle; AGPL/kommerzielle Lizenz; zusätzliche Remote-I/O-Anpassung | Referenznaher Weg mit klarer Runtime- und Lizenzentscheidung |
| Kompatible Rust-Schicht | Native In-App-Verwaltung, potenziell auf allen unterstützten Smart-Explorer-Plattformen | Eigenverantwortliche Format-, Sicherheits-, Provider- und Transaktionskompatibilität | Architektonisch passend; Bibliotheksauswahl noch nicht als produktionsreif belegt |

Die Java-Dateisystembibliothek verwendet `java.nio.file.FileSystem` und bietet
u. a. `initialize`, `newFileSystem` und einen Read-only-Modus. Sie übernimmt
selbst keine native Rust-Backend-Anbindung. CryptoLib stellt Schlüsseldatei-Laden,
Speichern und Passwortwechsel bereit. Die lokal gesicherten
[API-Befunde](refs/cryptomator-vault-api-2026-10-04.md) nennen die gegen
Release-Quellcode geprüften Signaturen und die Grenzen der Schreibgarantien.

Die CLI 0.6.2 besitzt `unlock`, `list-mounters` und Hilfe. Ihr Unlock-Aufruf
liest das Passwort unter anderem über `--password:stdin`, öffnet das
Dateisystem, mountet und bleibt als laufender Prozess bestehen. Es existiert in
dieser Befehlsoberfläche kein separater Lock-Befehl; der Shutdown-Hook schließt
den Mount. Offene Ressourcen können das saubere Auswerfen verhindern.
Für eine Windows-Anbindung wäre deshalb ein nachgewiesener Prozessabschluss
mit überprüftem Mountzustand nötig; ein Unix-Signalhinweis aus der README reicht
als Windows-Vertrag nicht aus.
([CLI 0.6.2](https://github.com/cryptomator/cli/blob/65813f5d5b6b416707ec0e9e474ec9287ecc43f2/README.md),
[Befehle](https://github.com/cryptomator/cli/blob/65813f5d5b6b416707ec0e9e474ec9287ecc43f2/src/main/java/org/cryptomator/cli/CryptomatorCli.java),
[Unlock-Lebenszyklus](https://github.com/cryptomator/cli/blob/65813f5d5b6b416707ec0e9e474ec9287ecc43f2/src/main/java/org/cryptomator/cli/Unlock.java))

Für vollständige Verwaltung gehören auch Passwortwechsel, Recovery Key,
fehlende Schlüsseldateien, beschädigte Vaults und bewusst angestoßene Migration
zum fachlichen Umfang. Passwortwechsel umhüllt dieselben Schlüssel neu; er
verschlüsselt nicht alle Dateien neu und hebt alte Cloud-Versionen der
Schlüsseldatei nicht auf. Das Produkt darf ihn nicht als Schlüsselrotation oder
nachträglichen Schutz früher exponierter Daten darstellen.
([Passwort und Recovery Key](https://docs.cryptomator.org/desktop/password-and-recovery-key/))

## Lizenzbefund

Smart Explorer trägt eine MIT-Lizenz (`LICENSE`). Cryptomators Desktop-App
nennt GPLv3 beziehungsweise eine kommerzielle Lizenz. CryptoFS, CryptoLib und
CLI nennen **AGPLv3 beziehungsweise kommerzielle Lizenzbedingungen**. Direkte
Einbindung kann deshalb nicht pauschal als MIT-only-Auslieferung behandelt
werden. MIT-Code kann Teil eines entsprechend lizenzierten Gesamtwerks sein;
das ist aber eine andere Distributionsentscheidung. Eine separate Prozess- oder
IPC-Grenze beweist für sich keine rechtliche Unabhängigkeit. Dieser Bericht
stellt die veröffentlichten Lizenzangebote fest, keine abschließende Bewertung
eines noch nicht definierten Distributionsmodells.
([Desktop](https://github.com/cryptomator/cryptomator/blob/fc76f5f83f6f4d3356f3bd161e27719372e1fc74/README.md),
[CryptoFS](https://github.com/cryptomator/cryptofs/blob/488b201f09a8124cbda64f6b68060493ee447de8/README.md),
[CryptoLib](https://github.com/cryptomator/cryptolib/blob/8fabeee72910cd65f73daa9cfc8dc44ffd82605a/README.md))

## Rust-Kandidaten: interessante Bausteine, konkrete Vorbehalte

**OxCrypt:** `oxcrypt-core` deklariert 0.3.0 und MPL-2.0. Der gelesene Stand
hat separate Kryptografie-, Vault- und Mountmodule; sein I/O-Fassadenmodul
verwendet ausdrücklich `std::fs`. Das passt zunächst zu lokalen Vaults, nicht
unverändert zu Smart Explorers SFTP-/Drive-/Share-Backends. Die Kern-API prüft
JWT-Claims und unterstützt SIV_GCM sowie SIV_CTRMAC. Rust-Version 1.95 und
Abhängigkeiten wie `ring`/`memsafe` sind Teil der Plattformbewertung; eine
Android-Lauffähigkeit wurde hier nicht belegt.
([Cargo-Metadaten](https://github.com/agucova/oxcrypt/blob/916864f66184dae6f97b0e97e02b879144fcfed6/crates/oxcrypt-core/Cargo.toml),
[Vault-I/O](https://github.com/agucova/oxcrypt/blob/916864f66184dae6f97b0e97e02b879144fcfed6/crates/oxcrypt-core/src/vault/operations.rs),
[Konfigurationsprüfung](https://github.com/agucova/oxcrypt/blob/916864f66184dae6f97b0e97e02b879144fcfed6/crates/oxcrypt-core/src/vault/config.rs))

Dabei zeigt dieselbe Quelllesung zwei erhebliche Integrationshindernisse:

- `extract_master_key` liest `kid` vor Signaturprüfung und bildet den
  Schlüsselpfad mit `vault_path.join(masterkey_uri.path())`. Im gelesenen Ablauf
  fehlt vor Existenzprüfung und Lesen eine Bindung an die autorisierte
  Vaultwurzel. Absolute Pfade und `..` brauchen daher eine eigene Abwehr.
  Das ist ein statischer Befund am genannten SHA, kein ausgeführter Exploit.
- `rename_file` prüft den Zielnamen durch Listing, liest den gesamten
  verschlüsselten Dateiinhalt, schreibt das Ziel und entfernt die Quelle.
  Dieser Ablauf liefert allein keine atomare NoReplace-Garantie gegenüber
  externen Schreibern und kann bei großen Dateien erheblichen Speicher und I/O
  beanspruchen. Sein Kommentar „crash-safe“ ersetzt keine Prüfung von
  Publikation, Flush und Wiederanlauf.

Der Autor nennt Sicherheitsprüfungen und Benchmarks, erklärt aber selbst die
Grenzen seiner Kryptografieexpertise. Ein unabhängiges Gesamtaudit wurde in den
gelesenen Quellen nicht nachgewiesen. Die Dokumentation nennt Memory-Locking
sowohl als vorhanden als auch als geplant; insbesondere daraus folgt keine
verifizierte Windows-Garantie. Werbeaussagen zur Geschwindigkeit wurden nicht
als Leistung von Smart Explorer übernommen.
([README](https://github.com/agucova/oxcrypt/blob/916864f66184dae6f97b0e97e02b879144fcfed6/README.md),
[Sicherheitsmodell](https://github.com/agucova/oxcrypt/blob/916864f66184dae6f97b0e97e02b879144fcfed6/docs/SECURITY.md))

**Dar9586/cryptomator-rs:** Apache-2.0; im gelesenen Stand ein kleinerer
Rust-Kandidat mit lokalem Dateisystemzugriff. `open` akzeptiert nur Format 8,
SIV_GCM und zunächst HS256. Auch hier wird der unverifizierte Schlüsselpfad
vor JWT-Prüfung verwendet. In `rename` wird als vermeintliches Ziel erneut
`old_dir.lookup(old_name)` abgefragt. Für eine existierende Quelle endet
`no_replace=true` damit bereits in `EEXIST`; andere Pfade löschen das Ziel vor
Abschluss der Umbenennung. **Dieser Stand ist keine geeignete unveränderte
Grundlage für sichere schreibende Integration.**
([Quellcode am SHA](https://github.com/Dar9586/cryptomator-rs/blob/c0c96cf45ecf17f18d6ece1e68adf51b7d18446d/cryptomator-rs-crypto/src/cryptomator.rs),
[Crate-Metadaten](https://github.com/Dar9586/cryptomator-rs/blob/c0c96cf45ecf17f18d6ece1e68adf51b7d18446d/cryptomator-rs-crypto/Cargo.toml))

Die Auswahl ist eine gezielte Stichprobe, keine vollständige Marktübersicht.

## Anschluss an die bestehende Smart-Explorer-Architektur

| Bestehende Grenze | Geprüfte Stelle | Bedeutung für Vaults |
|---|---|---|
| Dateioperationen und stabile Identität | `native/src/vfs/core/core.rs`, `Backend` | Eine entschlüsselte Sicht kann dieselbe Dateioberfläche anbieten; die Identität muss Vault **und** zugrunde liegenden Speicherort binden |
| Gespeicherte Orte und Wiederöffnung | `connect/core/location.rs`, `connect/os/shared/resolution.rs` | Aktuell kein Vault-Endpunkttyp; eine wechselnde Laufwerkszuordnung reicht nicht als persistierte Vault-Identität |
| Schreib-/Rootfähigkeiten | `vfs/core/capabilities.rs`, `vfs/core/extensions.rs` | Kryptografie verleiht dem darunterliegenden Provider keine atomaren Schreib- oder Dauerhaftigkeitsgarantien |
| Windows-Mount und Prozessgrenze | `mount/mod.rs`, `mount/core/types.rs` | Dokany ist vorhanden, MountSource umfasst SavedRemote/Drive/Peer; Vaults sind noch keine eigene Quelle |
| Lokale Arbeitsdateien und Inhaltcache | `mount/core/spool.rs`, `materialization.rs`, `file_io.rs`, `clean_cache.rs` | Ein entschlüsseltes Backend an dieser Stelle könnte Klartext auf Disk speichern |
| Android-/Linux-Mountfähigkeit | `mount/mod.rs::drive_mount_supported` | Betriebssystemlaufwerke werden aktuell nur auf Windows unterstützt; In-App-Vaultzugriff ist eine separate Möglichkeit |

Graphify wurde mit `VFS Backend mount Dokany encrypted vault filesystem endpoint
resolution` abgefragt; der Graph diente zur Navigation, die Verträge wurden am
aktuellen Quelltext geprüft. Die bestehenden Test-/Dateinamen mit „vault“ belegen
keine Cryptomator-Unterstützung: sie betreffen unter anderem Obsidian-/Metadaten-
und Wire-Szenarien. Die gezielte Suche in `connect`, `vfs`, `mount` und `creds`
fand keine Cryptomator-Ent-/Verschlüsselung.

Der wichtigste konkrete Befund ist die **Klartextgrenze**: Materialization kopiert
den Stream aus `backend.open_read_id` unverändert in eine normale `.spool`-Datei
und bestätigt sie mit `sync_data`. `file_io::write` schreibt Eingabebytes in
dieselbe lokale Arbeitsablage. Bei einem entschlüsselten Backend wären diese
Bytes Klartext. Den optionalen Cache zu deaktivieren entfernt nicht die aktive
Arbeitsdatei und ihre Wiederanlaufanforderung.

**Ableitung:** Für Vault-Inhalte braucht es entweder einen Mount-Datenpfad mit
blockweiser Verschlüsselung vor dauerhafter Ablage oder eine verschlüsselte
Arbeits-/Recoveryablage. Auch Namen in Journalen, Sync-Baselines, Suche,
Vorschau und Fehlermeldungen gehören zur Betrachtung. Recovery nach einem
Absturz und Schlüsselvernichtung beim Sperren müssen dabei gemeinsam gelöst
werden; ungespeicherte Arbeit still zu löschen wäre keine zulässige Abkürzung.
Anders ist die Lage, wenn der bestehende Mount ausschließlich **verschlüsselte**
Vaultdateien als Speicher für einen darüberliegenden Cryptomator-Prozess
bereitstellt: sein Spool enthält dann Ciphertext. Dieser mehrschichtige Weg
wurde hier nicht praktisch qualifiziert.

## Entfernte Vaults und Synchronisation

Cryptomators Desktop arbeitet üblicherweise auf lokalen beziehungsweise vom
System zugänglichen Vaultpfaden. Direkter Vaultzugriff auf Fernspeichern ist
dennoch möglich: Cyberduck/Mountain Duck unterstützen interoperable Vaults auf
Servern und Cloudspeichern, derzeit die Formate 7/8. Das ist ein vergleichbarer
Produktweg, kein Beweis, dass deren Code als Rust-Bibliothek übernommen werden
kann. ([Cyberduck-Dokumentation](https://docs.cyberduck.io/cryptomator/))

Für Smart Explorer wäre eine entschlüsselnde Sicht **über dem bestehenden
Speicherbackend** der passende gemeinsame Anschluss. Diese Bewertung folgt aus
dessen Backend- und Ortsverträgen. Sie müsste lokale/UNC-Pfade, SFTP mit
SSH-Agent, FTP/FTPS, WebDAV, Google Drive und Direct-/Room-Share samt jeweiliger
Verbindungsidentität erhalten. Zwei Vaults unter `/vault` auf verschiedenen
Remotes bleiben verschiedene Orte; Laufwerksbuchstaben sind nur Präsentation.

Dabei sind zwei unterschiedliche Vorgänge sichtbar zu unterscheiden:

- **Verschlüsselten Vault sichern/übertragen:** Inhalt und Strukturdaten bleiben
  verschlüsselt. Ein laufend veränderter Baum ist kein automatisch konsistenter
  Snapshot; unvollständige Übertragung von Verzeichnis-IDs kann Inhalte
  unerreichbar machen.
- **Dateien aus einem entsperrten Vault synchronisieren:** Der Sync arbeitet auf
  Klartext. Ein unverschlüsseltes Ziel erhält Klartext; Vault-zu-Vault bedeutet
  Entschlüsselung an der Quelle und Verschlüsselung am Ziel, mit getrennten
  Identitäten, Baselines und Schlüsseln.

Sperren oder Verlust des Speichers müssen „nicht verfügbar“ ergeben und dürfen
keine scheinbar leere Quelle mit nachfolgenden Löschungen erzeugen. Links und
Junctions bleiben geschützte Auslassungen; Read-only-Rechte, Backups, Konflikte,
Cancel und Retry bleiben erhalten. Neue Fähigkeiten dürfen nicht großzügiger
deklariert werden als der tatsächliche Speichervertrag. Diese Erwartungen folgen
aus den bestehenden Repository-Verträgen; sie wurden in diesem Auftrag nicht
dynamisch überprüft.

Cryptomator behandelt Synchronisationskonflikte sichtbar, etwa über abgeleitete
Konfliktnamen. Daraus folgt keine globale Sperre zwischen mehreren Rechnern oder
eine über den Cloudprovider atomare Transaktion.
([Konfliktdokumentation](https://docs.cryptomator.org/desktop/sync-conflicts/))

## Was die Sicherheitsfunde für eine Integration lehren

Die 2026 veröffentlichte Advisory **GHSA-5phc-5pfx-hr52** beschreibt Zugriffe auf
lokale beziehungsweise UNC-Pfade aus unverifizierten Schlüsselverweisen.
Auf Windows kann bereits eine Pfadprüfung unerwünschten SMB-Verkehr auslösen.
Im geprüften Desktop-Release 1.19.3 lädt die Standardstrategie dagegen den
festen Namen `masterkey.cryptomator` und übernimmt abweichende Namen nicht direkt.
Die Advisory enthält weiterhin einen älteren „Patched versions: None“-Stand;
deshalb wurde ihr Status mit dem Release-Quellcode abgeglichen. Das ist keine
Behauptung einer vollständigen Sicherheit von 1.19.3.
([Advisory](https://github.com/cryptomator/cryptomator/security/advisories/GHSA-5phc-5pfx-hr52),
[Release-Keyloader](https://github.com/cryptomator/cryptomator/blob/fc76f5f83f6f4d3356f3bd161e27719372e1fc74/src/main/java/org/cryptomator/ui/keyloading/masterkeyfile/MasterkeyFileLoadingStrategy.java))

Bei Hub beschreibt **GHSA-9q8x-whrw-x44p** einen HTTPS-/HTTP-Verwechslungsfehler
in 1.19.1, behoben in 1.19.2. Die Folgerung für Smart Explorer ist präzise:
Vaultmetadaten dürfen keine unbestätigten neuen Netzwerkautoritäten bestimmen;
Schemas, Hosts, Ports und Weiterleitungen brauchen echte Vertrauensgrenzen.
Hub, Geräteauthentifizierung und Rechteentzug sind ein eigener fachlicher Umfang.
([Hub-Advisory](https://github.com/cryptomator/cryptomator/security/advisories/GHSA-9q8x-whrw-x44p))

## Windows und weitere Plattformen

Windows ist der am besten passende Ausgangspunkt: Smart Explorer besitzt bereits
Dokany, Cryptomator verwendet WinFsp. Beide liefern Dateisystemfrontends;
Verschlüsselungskompatibilität hängt vom Vaultformat ab. Der zweite Bericht
erläutert Laufwerksbuchstaben, Ordner-Mounts, Netzwerk-/Local-Drive-Unterschiede,
Lifecycle und WebDAV-Grenzen samt konkreter Java/native-Kette.

Unter Linux ist In-App-Verwaltung in der Rust-/VFS-Architektur plausibel; ein
systemweiter Mount bräuchte einen zusätzlichen FUSE-Adapter. Android kann
entschlüsselte Dateien in der App anbieten, ohne ein systemweites Laufwerk zu
erzeugen. Zugriff anderer Apps folgt dort Content-URI-/Provider-Verträgen.
Das ist eine andere Plattformaufgabe mit Berechtigungs- und Hintergrundregeln.
macOS hat mehrere Mountwege bei Cryptomator, ist aber kein derzeit in
`docs/ARCHITEKTUR.md` ausgewiesenes Smart-Explorer-Ziel. Eine übertragbare
Kryptografieschicht ersetzt daher keine macOS-Portierung der Anwendung.

## Abschließende Bewertung und verbleibende Unsicherheit

**Für das gewünschte Produktbild ist die Integration sinnvoll und machbar.**
Einheitliche Vaultverwaltung lässt sich mit dem vorhandenen Dateimanager,
seinen Transfers und Windows-Laufwerken verbinden. Die zentrale technische
Idee ist die Trennung von Vaultformat/Kryptografie, Speicherbackend und
Betriebssystem-Mount.

Meine Empfehlung aus diesem Auftrag: Die offiziellen Komponenten als
Kompatibilitätsreferenz verwenden. Für einen begrenzten Einstieg eignet sich
die Verwaltung externer, lokal zugänglicher Vaults; für vollständige native
Verwaltung verdient OxCrypt eine genauere Bewertung gegen die Smart-Explorer-
Verträge. Die unveränderte Übernahme eines Rust-Kandidaten oder das direkte
Einstecken eines entschlüsselten Backends in den vorhandenen Spool-Mount ist
durch diese Untersuchung nicht gerechtfertigt.

Offen bleiben die konkrete Lizenz-/Distributionsentscheidung, praktische
Windows-Prozesssteuerung bei der CLI, sichere verschlüsselte Recovery,
beidseitige Interoperabilität inklusive langer/Unicode-Namen und beschädigter
Dateien, tatsächliche Provider-Transaktionen, Hub-Umfang und nachgewiesene
Android-Portabilität eines gewählten Bausteins. Diese Punkte sind Grenzen der
Erkundung, keine beauftragten Implementierungsaufgaben und keine Release-Gates
für den unveränderten Smart Explorer.

Die angekündigte Arbeit am **Unified Vault Format** ist ein Zukunftssignal,
kein Ersatz für die im untersuchten Release vorhandene Format-8-Kompatibilität.
Der offizielle Roadmapbeitrag bezeichnete das Format als in Arbeit; ein aktueller
verbindlicher Supportvertrag wurde in diesem Auftrag nicht nachgewiesen.
([Offizieller Roadmapbeitrag](https://cryptomator.org/blog/2025/07/24/post-quantum-roadmap/))
