# Cryptomator: gesicherte Vault-API-Befunde

Primärquellen gelesen: **2026-10-04**. Reine Quell-/Dokumentationsprüfung;
keine API wurde ausgeführt. Fachliche Bewertung und Smart-Explorer-Anschluss:
[Erkundungsbericht](../CRYPTOMATOR_VAULTS_ERKUNDUNG.md).

## CryptoFS 2.10.0

Referenz-SHA: `488b201f09a8124cbda64f6b68060493ee447de8`.
Die Methoden gehören zu `org.cryptomator.cryptofs.CryptoFileSystemProvider`.

| Statische Methode | Rückgabe | Deklarierte Fehler |
|---|---|---|
| `newFileSystem(Path pathToVault, CryptoFileSystemProperties properties)` | `CryptoFileSystem` | `FileSystemNeedsMigrationException`, `IOException`, `MasterkeyLoadingFailedException` |
| `initialize(Path pathToVault, CryptoFileSystemProperties properties, URI keyId)` | `void` | `NotDirectoryException`, `IOException`, `MasterkeyLoadingFailedException` |

`initialize` setzt ein bestehendes Verzeichnis voraus, lädt den Masterkey über
den übergebenen Keyloader, erzeugt eine signierte Vaultkonfiguration mit
exklusiver Datei-Erstellung und legt Ciphertextwurzel sowie Directory-ID-Backup
an. Der Aufruf erzeugt **nicht selbst die Passwort-Schlüsseldatei**; diese ist
Teil der vorgelagerten Schlüsselverwaltung. `newFileSystem` öffnet eine
Java-NIO-Dateisystemsicht und mountet kein Windows-Laufwerk.
([Provider-Quellcode](https://github.com/cryptomator/cryptofs/blob/488b201f09a8124cbda64f6b68060493ee447de8/src/main/java/org/cryptomator/cryptofs/CryptoFileSystemProvider.java))

Der Property-Builder bietet `withKeyLoader(MasterkeyLoader)`,
`withFlags(FileSystemFlags...)`, `withCipherCombo(CryptorProvider.Scheme)`,
`withShorteningThreshold(int)`, `withMaxCleartextNameLength(int)` und
`withFilesystemEventConsumer(Consumer<FilesystemEvent>)`.
In diesem Release enthält `FileSystemFlags` den Wert `READONLY`.
Das aktuell dokumentierte Vaultformat ist 8 (`common/Constants.java`).
([Properties](https://github.com/cryptomator/cryptofs/blob/488b201f09a8124cbda64f6b68060493ee447de8/src/main/java/org/cryptomator/cryptofs/CryptoFileSystemProperties.java),
[Formatkonstante](https://github.com/cryptomator/cryptofs/blob/488b201f09a8124cbda64f6b68060493ee447de8/src/main/java/org/cryptomator/cryptofs/common/Constants.java))

Eine ältere beziehungsweise bewegliche README ist keine präzise Syntaxquelle:
die gesichtete develop-README übergibt bei `initialize` einen String, während
der Release-Provider hier `URI` verlangt. Beispiele wurden deshalb nicht
ungeprüft als compilierbarer Integrationscode übernommen.

## CryptoLib 2.2.2

Referenz-SHA: `8fabeee72910cd65f73daa9cfc8dc44ffd82605a`.
`org.cryptomator.cryptolib.common.MasterkeyFileAccess` wird mit `byte[] pepper`
und `SecureRandom csprng` konstruiert.

| Instanzmethode | Rückgabe | Bedeutung |
|---|---|---|
| `load(Path filePath, CharSequence passphrase)` | `Masterkey` | Schlüsseldatei laden und Passwort-KEK ableiten; Fehler `MasterkeyLoadingFailedException` |
| `persist(Masterkey masterkey, Path filePath, CharSequence passphrase)` | `void` | Passwortgeschützte Schlüsseldatei schreiben; Fehler `IOException` |
| `changePassphrase(byte[] masterkey, CharSequence oldPassphrase, CharSequence newPassphrase)` | `byte[]` | Neue JSON-Schlüsseldatei zurückgeben; Fehler `IOException`, `InvalidPassphraseException` |

Die Bytearray-Variante des Passwortwechsels publiziert selbst keine Datei.
`persist` schreibt exklusiv eine benachbarte `.tmp`-Datei und verwendet danach
`Files.move(..., REPLACE_EXISTING)`. An dieser Stelle wird weder `ATOMIC_MOVE`
verlangt noch ein File-/Directory-fsync ausgeführt. Daraus darf eine Integration
keine allgemeine Stromausfall-Dauerhaftigkeit oder Remote-NoReplace-Garantie
ableiten. Geladene Masterkeys sollen über ihre Close-/Destroy-Grenze beendet
werden.
([MasterkeyFileAccess](https://github.com/cryptomator/cryptolib/blob/8fabeee72910cd65f73daa9cfc8dc44ffd82605a/src/main/java/org/cryptomator/cryptolib/common/MasterkeyFileAccess.java))

Die `FileContentCryptor`-Schnittstelle bietet blockweises Ent-/Verschlüsseln,
Größenumrechnung und einen Authentifizierungsparameter bei der Entschlüsselung.
Die Dokumentation verlangt Authentifizierung als Standard; die konkrete
Cipher-Implementierung kann das Abschalten ablehnen. Eine Integration darf
Authentifizierungsfehler nicht als leere Datei oder End-of-File übersetzen.
([FileContentCryptor](https://github.com/cryptomator/cryptolib/blob/8fabeee72910cd65f73daa9cfc8dc44ffd82605a/src/main/java/org/cryptomator/cryptolib/api/FileContentCryptor.java))

## CLI 0.6.2

Referenz-SHA: `65813f5d5b6b416707ec0e9e474ec9287ecc43f2`.
Belegt sind `unlock`, `list-mounters`, Hilfe und die Passwortquellen
`--password:stdin`, `--password:env`, `--password:file`.
Für eine GUI-Anbindung ist eine private Eingabepipe die passende zu prüfende
Option; Passwortdatei und Umgebungsvariable schaffen zusätzliche Geheimniskopien.
Die Zeilen-/Encoding-/Prompt-Interaktion wurde nicht praktisch geprüft.
([Befehle](https://github.com/cryptomator/cli/blob/65813f5d5b6b416707ec0e9e474ec9287ecc43f2/src/main/java/org/cryptomator/cli/CryptomatorCli.java),
[PasswordSource](https://github.com/cryptomator/cli/blob/65813f5d5b6b416707ec0e9e474ec9287ecc43f2/src/main/java/org/cryptomator/cli/PasswordSource.java))

`Unlock.loadMasterkey` verwendet fest `masterkey.cryptomator`. Es wertet
`hub+…` nicht als Hub-Keyloader aus. Vaultöffnung und Mount bleiben in einem
laufenden Prozess; der Shutdown-Hook schließt den Mount. Erfolgreicher
Prozessstart allein belegt daher weder erfolgreiche Entsperrung noch einen
fertigen Mount. Die CLI ist hier keine zugesicherte strukturierte Management-API.
([Unlock](https://github.com/cryptomator/cli/blob/65813f5d5b6b416707ec0e9e474ec9287ecc43f2/src/main/java/org/cryptomator/cli/Unlock.java))
