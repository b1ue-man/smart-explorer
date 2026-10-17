# Cryptomator: Tresorformat 8, Rust-Bausteine, Sync und Vergleich

Stand und Abrufdatum 2026-10-17. Rechercheauftrag für Strang S3 „Cryptomator-Tresore“ (`docs/plaene/2026-10-17-rechte-desktop-tresore/plan.md`, Ergebnisse E8 und E9). Reine Quellenrecherche: keine Builds, keine Tests, keine Laufzeitversuche, kein Entsperren echter Tresore. Folgerungen sind als **Folgerung**, Annahmen als **Annahme** gekennzeichnet; die Entscheidung trifft der Hauptagent.

Methode: Quelltext wurde als Rohdatei von `raw.githubusercontent.com` am jeweiligen Release-Tag gelesen (nur einzelne Zeilenbereiche), Release-Stände von den HTML-Seiten `github.com/<owner>/<repo>/tags` bzw. `/releases/tag/<tag>` (die GitHub-REST-API lieferte ohne Anmeldung keine Daten). Dokumentationsseiten über WebFetch mit gezielter Frage.

## 0. Aktualität der Vorbefunde

| Vorbefund | Stand dort | Stand 2026-10-17 | Folge |
|---|---|---|---|
| `docs/CRYPTOMATOR_VAULTS_ERKUNDUNG.md`, `docs/refs/cryptomator-vault-api-2026-10-04.md`: CryptoLib | 2.2.2 (`8fabeee7…`) | **neu: CryptoLib 2.3.0**, Tag-Commit `214dfe803eb85048a07872cd09f417f87ae9c321`, veröffentlicht 2026-10-12 21:36 UTC. Einzige fachliche Änderung: #134 „Add masterkey file validator to prevent OOM crashes“ (neue Klasse `MasterkeyFileValidator`, siehe A.2). Alle übrigen Einträge (#113–#139) sind Abhängigkeits- und CI-Aktualisierungen. | Formatbefunde des Vorbefunds bleiben gültig; Validierungsgrenzen für scrypt-Parameter sind neu. |
| dieselben: CryptoFS | 2.10.0 (`488b201f…`) | **neu: CryptoFS 2.11.0**, Tag-Commit `b82c17185bbd4fa3ce15fe089b8afe24155246e7`, 2026-10-14. CHANGELOG: neu `VaultConfig.UnverifiedVaultConfig.allegedVaultId()`; behoben Issue #355 „Automatic conflict resolution could create duplicate directory link with an already existing directory id“ (PR #356, siehe A.12); Abhängigkeiten `com.auth0:java-jwt` 4.5.1 → 4.6.1, `caffeine` 3.2.3 → 3.3.0, `slf4j-api` 2.0.17 → 2.0.20. | Konfliktbehandlung für Verzeichnisse geändert; für E9 relevant. |
| Cryptomator Desktop | 1.19.3 | unverändert 1.19.3 „Latest“ (Beleg: `docs/refs/cryptomator-einhaengen-revalidierung-2026-10-17.md`, B1). Desktop 1.19.3 bindet noch CryptoFS 2.10.0 / CryptoLib 2.2.2. | Desktop-Verhalten = Stand vor #355-Korrektur. |
| Vorbefund „dirid.c9r unterstützt die Wiederherstellung“, „68-Byte-Header, 32 KiB + 28 Byte“ | — | bestätigt, präzisiert in A.5–A.7 | gültig |
| Vorbefund „Signaturprüfung erfolgt nach Schlüsselgewinnung; `kid` ist unvertrauenswürdig“ | — | bestätigt durch die Öffnungsreihenfolge der Architekturseite (A.1) | gültig |
| Neu seit Vorbefund nicht erfasst: „Files-in-Use“ (`.c9u`) | — | in CryptoFS **2.10.0** eingeführt (#312, #338); laut Doku nur für Hub-Tresore aktiv (A.11) | Ein Sync auf Chiffretextebene trifft `.c9u`-Dateien an. |

## A. Tresorformat 8

Quellen (alle abgerufen 2026-10-17):
- Q-ARCH: `https://docs.cryptomator.org/security/architecture/` (Tresorkonfiguration, Masterkey-Datei).
- Q-VAULT: `https://docs.cryptomator.org/security/vault/` (Header, Blöcke, Namen, Verzeichnisse, Kürzung, `dirid.c9r`).
- Q-LIB: `github.com/cryptomator/cryptolib`, Tag `2.3.0` (`214dfe80…`), Dateien `src/main/java/org/cryptomator/cryptolib/{common,v1,v2,api}/…`. In CryptoLib heißt **`v2` = `SIV_GCM`** und **`v1` = `SIV_CTRMAC`**.
- Q-FS: `github.com/cryptomator/cryptofs`, Tag `2.11.0` (`b82c1718…`).
- Q-SIV: `github.com/cryptomator/siv-mode`, Zweig `develop` (neuester Tag `2.0.0-rc1`, 2026-01-23), `SivMode.java`, `SivEngine.java`.
- Q-DESK: `github.com/cryptomator/cryptomator`, Tag `1.19.3`.

### A.1 `vault.cryptomator` (JWT)

- Lage: Tresorwurzel. Format: JWT (JWS compact), Q-ARCH.
- Header: `kid` = Schlüsselort, Beispiel `"masterkeyfile:masterkey.cryptomator"`; Hub: `"hub+https://…"`. `typ` = `"JWT"`. `alg` = `"HS256"`; Q-ARCH: aktuelle Implementierungen akzeptieren auch `HS384` und `HS512`. Q-FS `VaultConfig.initAlgorithm`: `case "HS256"`, `"HS384"`, `"HS512"`; andere Werte → Fehler. Beim Schreiben signiert CryptoFS immer mit `Algorithm.HMAC256(rawKey)`.
- Payload: `format` (int, 8), `shorteningThreshold` (int, Standard 220), `jti` (zufällige UUID = Tresor-ID), `cipherCombo` (`"SIV_GCM"` oder `"SIV_CTRMAC"`). Q-FS `VaultConfig`: Konstanten `JSON_KEY_VAULTVERSION = "format"`, `JSON_KEY_CIPHERCONFIG = "cipherCombo"`, `JSON_KEY_SHORTENING_THRESHOLD = "shorteningThreshold"`; `.withKeyId(keyId).withJWTId(id)`.
- Signaturschlüssel: der rohe 512-Bit-Masterkey = 32 Byte Encryption-Masterkey ‖ 32 Byte MAC-Masterkey (Q-LIB `api/Masterkey.java`: `arraycopy(encKey…, 0)`, `arraycopy(macKey…, 32)`).
- Öffnungsreihenfolge (Q-ARCH): (1) JWT ohne Prüfung dekodieren, (2) `kid` lesen und Masterkey vom bezeichneten Ort holen, (3) Signatur mit dem verketteten Masterkey prüfen, (4) `format` und `cipherCombo` auf Unterstützung prüfen. Q-FS prüft beim Verifizieren `withClaim("format", expectedVaultVersion)`.
- Desktop: `DEFAULT_KEY_ID = "masterkeyfile:masterkey.cryptomator"` (Q-DESK `common/Constants.java`). Die Tresor-Anlage setzt `cipherCombo` fest auf `SIV_GCM` (Q-DESK `CreateNewVaultPasswordController`); `shorteningThreshold` ist in den Experteneinstellungen wählbar zwischen `MIN_SHORTENING_THRESHOLD = 36` und `MAX_SHORTENING_THRESHOLD = 220`, Standard 220 (Q-DESK `CreateNewVaultExpertSettingsController`).
- **Folgerung:** Neue Tresore aus dem Desktop sind immer SIV_GCM; SIV_CTRMAC tritt nur bei älteren, migrierten Tresoren oder anderen Clients auf. Eine eigene Implementierung muss `shorteningThreshold` aus der Konfiguration lesen und darf nicht 220 annehmen.

### A.2 `masterkey.cryptomator` und Schlüsselableitung

- JSON-Felder (Q-ARCH, Q-LIB `MasterkeyFile`, `@SerializedName`): `version` (veraltet; geschrieben wird `999`, Q-LIB `DEFAULT_MASTERKEY_FILE_VERSION = 999 // legacy field. dropped with vault format 8`), `scryptSalt` (Base64), `scryptCostParam` (N), `scryptBlockSize` (r), `primaryMasterKey` (gewickelter Encryption-Masterkey, Base64), `hmacMasterKey` (gewickelter MAC-Masterkey, Base64), `versionMac` (Base64).
- Standardwerte beim Schreiben (Q-LIB `MasterkeyFileAccess`): `DEFAULT_SCRYPT_SALT_LENGTH = 8` Byte, `DEFAULT_SCRYPT_COST_PARAM = 1 << 15` (32768), `DEFAULT_SCRYPT_BLOCK_SIZE = 8`; Parallelität `P = 1` (Q-LIB `Scrypt.java`); KEK-Länge `Masterkey.SUBKEY_LEN_BYTES = 32`.
- Passwort-Kodierung: UTF-8 (Q-LIB `Scrypt.scrypt(CharSequence…)`: `UTF_8.encode`); CryptoLib normalisiert nicht. Die Normalisierung geschieht in der Desktop-Oberfläche: Q-DESK `ui/controls/SecurePasswordField.java` Zeile 150–161 „The text will be normalized to NFC“ (`Normalizer.normalize(text, Form.NFC)`). OxCrypt normalisiert ebenfalls NFC (`vault/master_key.rs`: „We use NFC normalization on the passphrase (matching Java implementation)“). **Folgerung:** Eine eigene Umsetzung muss das Passwort vor scrypt nach NFC normalisieren, sonst scheitert ein Passwort, dessen Zeichen in zerlegter Form (NFD) ankommen. Annahme: Es gibt Eingabewege, die NFD liefern; welche das unter Windows, Linux und Android sind, wurde nicht ermittelt.
- Salz und Pfeffer: scrypt-Salz = `scryptSalt ‖ pepper`. Der Desktop verwendet `PEPPER = new byte[0]` (Q-DESK `common/Constants.java`), also effektiv nur `scryptSalt`.
- KEK: `kek := scrypt(password, scryptSalt, scryptCostParam, scryptBlockSize)` (Q-ARCH wörtlich), p = 1, 32 Byte.
- Schlüsselwicklung: AES Key Wrap nach RFC 3394 mit der KEK für beide 32-Byte-Masterkeys (Q-ARCH; Q-LIB `AesKeyWrap.wrap/unwrap`). Gewickelte Länge je Schlüssel: 40 Byte (32 + 8 Byte Integritätsblock). Falsches Passwort zeigt sich als `InvalidKeyException` beim Entwickeln → `InvalidPassphraseException`.
- `versionMac` = HMAC-SHA256 mit dem **MAC-Masterkey** über `version` als 4-Byte-Big-Endian-Integer (Q-LIB `lock`: `MacSupplier.HMAC_SHA256.keyed(macKey)`, `ByteBuffer.allocate(4).putInt(vaultVersion)`). Länge 32 Byte. Q-LIB `load/unlock` prüft `versionMac` **nicht** (nur Anwesenheit in `isValid()`); Format 8 schützt vor Herabstufung über die JWT-Signatur.
- Gültigkeitsprüfung (Q-LIB `MasterkeyFile.isValid`): `version`-Feld vorhanden, `scryptSalt != null`, `scryptCostParam > 1`, `scryptBlockSize > 0`, beide Schlüssel und `versionMac` nicht null.
- **Neu in 2.3.0** (`MasterkeyFileValidator.DEFAULT`): lehnt ab, wenn `scryptCostParam > 1 << 20`, `scryptBlockSize > 64` oder Arbeitsspeicher `> 1 GiB + 3072 Byte`. Javadoc-Beispiel nennt eine engere Grenze `128 * r * N > 256 MiB`. **Folgerung:** Eine eigene Umsetzung braucht dieselbe Obergrenze vor der scrypt-Ableitung, sonst kann eine präparierte Schlüsseldatei (zum Beispiel auf einem fremden Server) Speicher erschöpfen; auf Android ist die Grenze eher niedriger zu wählen (Annahme: Android-Prozesse haben deutlich weniger verfügbaren Speicher als Desktop-Prozesse).
- Passwortwechsel (Q-LIB `changePassphrase`): entwickelt mit altem Passwort, wickelt neu mit neuem Passwort, **neues Salz**, gleiche `scryptCostParam` wie vorher, gleiche `version`; die Masterkeys bleiben gleich, kein Neuverschlüsseln von Dateien. Desktop (Q-DESK `ChangePasswordController`, Zeilen 80–84): alte Datei lesen → neue Bytes berechnen → alte Datei per `Files.move(..., REPLACE_EXISTING, ATOMIC_MOVE)` nach `masterkey.cryptomator.<8 Hex-Zeichen>.bkup` verschieben (Suffix = `"." + hex(SHA-256(alte Bytes)[0..4])`, Q-FS `common/BackupHelper.generateFileIdSuffix`) → neue Datei mit `CREATE_NEW` schreiben. `vault.cryptomator` bleibt unverändert, weil der Masterkey gleich bleibt.
- **Folgerung:** Zwischen Verschieben und Neuschreiben existiert kurz keine `masterkey.cryptomator`; auf einem Remote-Backend ohne atomares Umbenennen ist diese Lücke länger. Die Sicherung `.bkup` mit dem alten Passwort bleibt im Tresor liegen und wird mit synchronisiert.

### A.3 Tresor anlegen (Desktop 1.19.3, Q-DESK `CreateNewVaultPasswordController.createVault`)

Reihenfolge wörtlich aus dem Code: „0. create directory“ (`Files.createDirectory`), „1. write masterkey“ (`Masterkey.generate(csprng)`, `masterkeyFileAccess.persist`), „2. initialize vault“ (`CryptoFileSystemProvider.initialize(path, fsProps, DEFAULT_KEY_ID)` mit `SIV_GCM` und gewähltem `shorteningThreshold`; legt laut Vorbefund `vault.cryptomator` exklusiv an sowie die Ciphertext-Wurzel `d/XX/YYYY…` und deren `dirid.c9r`), „3. write vault-internal readme file“ (RTF-Datei im Tresor, Name aus Ressourcenbündel), „4. write vault-external readme file“ (RTF neben `d/` im Speicherort). Masterkey-Erzeugung: 64 Byte aus CSPRNG (Q-ARCH: Java `SecureRandom` SHA1PRNG, mit 440 Bit aus `getInstanceStrong()` geimpft).

### A.4 Wiederherstellungsschlüssel (Q-DESK `ui/recoverykey/RecoveryKeyFactory.java`, `WordEncoder.java`)

- Eingabe: roher Masterkey, genau 64 Byte (`checkArgument(rawKey.length == 64)`).
- Prüfsumme: `paddedKey = copyOf(rawKey, 66)`; `Hashing.crc32().hashBytes(rawKey).writeBytesTo(paddedKey, 64, 2)`. Beim Dekodieren: `actualCrc16 = copyOf(crc32.asBytes(), 2)`. Code-Kommentar sagt „16 most significant bits of CRC32“; siehe Widerspruch W1.
- Wortkodierung: Wortliste `/i18n/4096words_en.txt`, `WORD_COUNT = 4096`, Trennzeichen Leerzeichen; je 3 Byte → 2 Wörter à 12 Bit: `first = (b1 << 4) | (b2 >> 4)`, `second = ((b2 & 0x0F) << 8) | b3`. 66 Byte → **44 Wörter**. Dekodieren verlangt eine gerade Wortzahl und ein Ergebnis von genau 66 Byte, sonst Fehler; danach CRC-Vergleich.
- Neues Passwort mit Wiederherstellungsschlüssel (`newMasterkeyFileWithPassphrase`): vorhandene `masterkey.cryptomator` per `ATOMIC_MOVE` nach `masterkey.cryptomator.<hex>.bkup`, dann `persist` mit neuem Passwort (neues Salz, N = 32768).
- Lizenz der Wortliste: Datei liegt im GPLv3-Desktop-Repository; ob sie separat lizenziert ist, wurde nicht ermittelt (Lücke L3).

### A.5 Dateinamen

- Normalisierung: Klartextnamen werden in NFC gebracht (Q-VAULT; Q-FS `CryptoPathFactory`: `Normalizer.normalize(str, Normalizer.Form.NFC)`).
- Verschlüsselung: `ciphertextName := base64url(aesSiv(cleartextName, parentDirId, encryptionMasterKey, macMasterKey)) + '.c9r'` (Q-VAULT wörtlich). Klartext UTF-8; Zusatzdaten = Verzeichnis-ID des Elternverzeichnisses als UTF-8-Bytes (Wurzel: leere Bytefolge).
- Schlüsselreihenfolge AES-SIV: Q-LIB `v2/FileNameCryptorImpl` ruft `siv.encrypt(ek, mk, …)` mit `ek` = Encryption-Masterkey, `mk` = MAC-Masterkey; Q-SIV `encrypt(ctrKey, macKey, …)` bildet den kombinierten RFC-5297-Schlüssel `macKey ‖ ctrKey` (`arraycopy(macKey, 0, combinedKey, 0, …)`, `// K1 = leftmost(K, len(K)/2)`). **Für RustCrypto `aes-siv::Aes256Siv` (64-Byte-Schlüssel) also: Schlüssel = MAC-Masterkey ‖ Encryption-Masterkey.** (Folgerung aus RFC 5297 §2.2 und Q-SIV; nicht durch einen ausgeführten Test belegt.)
- Base64url: Guava `BaseEncoding.base64Url()` **mit** `=`-Auffüllung (Q-FS `LongFileNameProvider`, Q-FS `Constants.BASE64_PATTERN` erlaubt `=` und `==` am Ende).
- Gültige Länge: `MIN_CIPHER_NAME_LENGTH = 28` (16 Byte SIV-Tag + leerer Name, Base64 + `.c9r`); `MAX_CIPHER_NAME_LENGTH = 220` (Q-FS `common/Constants.java`).
- Erkennen eines Namens: Q-FS `C9rDecryptor` sucht im Basisnamen per Regex `[a-zA-Z0-9-_]{20}(?:[a-zA-Z0-9-_]{4})*(?:[a-zA-Z0-9-_]{4}|[a-zA-Z0-9-_]{3}=|[a-zA-Z0-9-_]{2}==)` die Base64-Teilfolge, die sich entschlüsseln lässt; Rest = Konfliktsuffix (A.12).
- Prüfvektoren: Q-LIB `v1|v2/FileNameCryptorImplTest` prüfen nur Rundlauf und Determinismus, **keine festen erwarteten Chiffrenamen** (keine Literal-Zeichenketten ≥ 24 Zeichen in der Datei). Für AES-SIV selbst bringt Q-SIV RFC-5297-Vektoren mit (Annahme: Testdatei nicht gelesen).

### A.6 Verzeichnisse, `dir.c9r`, `dirid.c9r`

- Wurzel-ID = leere Zeichenkette (`ROOT_DIR_ID = ""`). Andere IDs: zufällige ASCII-Zeichenkette, höchstens 36 Zeichen, empfohlen UUID (Q-VAULT; Q-FS `MAX_DIR_ID_LENGTH = 36 // UUIDv4`).
- Ablagepfad: `dirIdHash := base32(sha1(aesSiv(dirId, null, encKey, macKey)))`; `dirPath := vaultRoot + '/d/' + substr(dirIdHash, 0, 2) + '/' + substr(dirIdHash, 2, 30)` (Q-VAULT). Q-LIB `hashDirectoryId`: SIV **ohne** Zusatzdaten, SHA-1 (20 Byte), Guava `BaseEncoding.base32()` (Großbuchstaben A–Z, 2–7; 20 Byte → genau 32 Zeichen, keine Auffüllung). Alle Inhaltsverzeichnisse liegen flach unter `d/`.
- Verzeichnisknoten: Ordner `<name>.c9r/` mit Datei `dir.c9r`, Inhalt = Verzeichnis-ID **unverschlüsselt** als UTF-8 (Q-FS `CryptoFileSystemImpl.createDirectory`: `channel.write(UTF_8.encode(ciphertextDir.dirId()))`).
- `dirid.c9r`: im Inhaltsverzeichnis `d/XX/YYYY…/`; enthält die **eigene** ID dieses Verzeichnisses, verschlüsselt wie eine normale Datei (Header + Block), Q-FS `DirectoryIdBackup.write`: `encryptingChannel.write(…dirId().getBytes(US_ASCII))` in `ciphertextDirectory.path().resolve("dirid.c9r")`. Lesegrenze 36 + 1 Byte. Zweck: Wiederherstellung bei fehlendem/beschädigtem `dir.c9r`; Q-VAULT: „adds no extra security“. Siehe Widerspruch W2 zur Formulierung der Doku.
- Reihenfolge beim Anlegen (Q-FS `createDirectory`, 2.11.0): (1) `Files.createDirectory(<name>.c9r)` als atomare Existenzprüfung, (2) `dir.c9r` mit `CREATE_NEW` schreiben, (3) `Files.createDirectories(d/XX/YYYY…)`, (4) `dirid.c9r` schreiben, (5) bei langen Namen `name.c9s` schreiben; bei Fehler in (3)–(5) wird `dir.c9r` wieder gelöscht.
- **Folgerung für Sync:** Ein Beobachter (Sync-Client, zweites Gerät) kann jeden Zwischenzustand sehen: `<name>.c9r/` ohne `dir.c9r`, `dir.c9r` ohne Inhaltsverzeichnis, Inhaltsverzeichnis ohne `dirid.c9r`. CryptoFS 2.11.0 behandelt solche Zustände in der Konfliktauflösung als „transient“ und verschiebt die Auflösung (A.12).

### A.7 Dateiinhalt: Header und Blöcke

**SIV_GCM** (Q-VAULT; Q-LIB `v2/Constants`, `v2/FileHeaderImpl`, `v2/FileContentCryptorImpl`):
- Header 68 Byte: Nonce 12 Byte (`NONCE_POS = 0`), verschlüsselte Nutzlast 40 Byte (`PAYLOAD_POS = 12`), GCM-Tag 16 Byte (`TAG_POS = 52`). Nutzlast-Klartext: 8 Byte reserviert (geschrieben als `-1L`, also `0xFFFFFFFFFFFFFFFF`, Big-Endian `putLong`) ‖ 32 Byte Inhaltsschlüssel. Verschlüsselung: AES-256-GCM mit Encryption-Masterkey und Header-Nonce, ohne Zusatzdaten. Nonce und Inhaltsschlüssel zufällig je Datei.
- Block: `PAYLOAD_SIZE = 32 * 1024`; Block = Nonce 12 Byte ‖ AES-256-GCM-Chiffretext (≤ 32768 Byte) ‖ Tag 16 Byte; `CHUNK_SIZE = 32796`. Schlüssel = Inhaltsschlüssel aus dem Header. Zusatzdaten: `aad := bigEndian(i) . headerNonce` (8 Byte Blocknummer Big-Endian, ab 0, dann 12 Byte Header-Nonce). Nonce je Block zufällig.
- Authentifizierung nicht abschaltbar: `canSkipAuthentication() = false`.

**SIV_CTRMAC** (Q-LIB `v1/*`):
- Header 88 Byte: Nonce 16 Byte, Nutzlast 40 Byte (gleicher Aufbau: 8 Byte reserviert `-1L` ‖ 32 Byte Inhaltsschlüssel) mit AES-256-CTR (Encryption-Masterkey, IV = Nonce), MAC 32 Byte = HMAC-SHA256(MAC-Masterkey, Nonce ‖ verschlüsselte Nutzlast) (`MAC_POS = 56`).
- Block: Nonce 16 Byte ‖ AES-256-CTR-Chiffretext (≤ 32768 Byte, Schlüssel = Inhaltsschlüssel, IV = Block-Nonce) ‖ MAC 32 Byte; `CHUNK_SIZE = 32816`. MAC = HMAC-SHA256(**MAC-Masterkey**, Header-Nonce ‖ Blocknummer 8 Byte Big-Endian ‖ Block-Nonce ‖ Chiffretext) (Q-LIB `v1/FileContentCryptorImpl.calcChunkMac`). MAC-Vergleich zeitkonstant (`// time-constant equality check`, `MessageDigest.isEqual`).
- `canSkipAuthentication() = true` (Lesen ohne Prüfung möglich, nur für Wiederherstellung sinnvoll).

**Größenberechnung** (Q-LIB `api/FileContentCryptor`, Standardmethoden; Header nicht enthalten):
- `ciphertextSize(n) = full * chunkSize + (rest == 0 ? 0 : rest + overhead)`, mit `full = n / 32768`, `rest = n % 32768`, `overhead = 28` (GCM) bzw. `48` (CTRMAC).
- `cleartextSize(c) = full * 32768 + (rest == 0 ? 0 : rest - overhead)`, mit `full = c / chunkSize`, `rest = c % chunkSize`; **Fehler**, wenn `0 < rest <= overhead`.
- Dateigröße auf dem Speicher = Headergröße + `ciphertextSize`. CryptoFS (`attr/CryptoBasicFileAttributes`) meldet bei ungültiger Größe Klartextgröße `0` und protokolliert eine Warnung.

**Leere Dateien:** CryptoFS-Größenformel ergibt für 0 Byte Klartext genau den Header (68 bzw. 88 Byte). CryptoLibs eigener `EncryptingWritableByteChannel` schreibt dagegen beim `close()` immer einen weiteren (ggf. leeren) Block: leere Datei = 68 + 28 = 96 Byte (Q-LIB `EncryptingWritableByteChannel.close`: `writeHeaderOnFirstWrite(); encryptAndFlushBuffer();`; der Test `testFileEncryption` erwartet für „hello world“ Header + 1 Block). Für 96 Byte wäre `cleartextSize(28)` ein Fehler. Siehe Widerspruch W3; eine eigene Umsetzung muss beide Formen lesen können (Folgerung).

### A.8 Lange Namen (`.c9s`)

- Auslöser: verschlüsselter Name einschließlich `.c9r` länger als `shorteningThreshold` (Q-FS `CryptoPathMapper`: `if (ciphertextName.length() > vaultConfig.getShorteningThreshold())`).
- Kurzname: `base64url(sha1(<voller Chiffrename inkl. .c9r als UTF-8>)) + ".c9s"` (Q-FS `LongFileNameProvider.deflate`) — 28 Zeichen + 4.
- Inhalt des Ordners `<kurz>.c9s/`: `name.c9s` (voller Chiffrename im Klartext UTF-8, Lesegrenze `MAX_FILENAME_BUFFER_SIZE = 10 * 1024`), und je Knotentyp `contents.c9r` (Datei), `dir.c9r` (Verzeichnis) oder `symlink.c9r` (Link) (Q-VAULT; Q-FS `Constants`).
- Q-VAULT: Grenze 220 lässt Platz für Konfliktsuffixe der Cloud-Anbieter; die Schicht dient der Kompatibilität, nicht der Sicherheit.

### A.9 Symbolische Links

Ordner `<name>.c9r/` mit `symlink.c9r`; Inhalt = Linkziel als UTF-8, verschlüsselt wie eine Datei (Header + Blöcke), höchstens `MAX_SYMLINK_LENGTH = 32767` Zeichen (Q-FS `Symlinks.java`).

### A.10 Weitere Dateien im Tresorbaum

| Name | Ort | Inhalt | Quelle |
|---|---|---|---|
| `vault.cryptomator` | Wurzel | JWT | A.1 |
| `masterkey.cryptomator` | Wurzel | JSON | A.2 |
| `masterkey.cryptomator.<8 hex>.bkup` | Wurzel | alte Schlüsseldatei nach Passwortwechsel | Q-DESK `ChangePasswordController` |
| `vault.cryptomator.<8 hex>.bkup`, `masterkey.cryptomator.<8 hex>.bkup` | Wurzel | Sicherungen beim Entsperren (`BackupHelper.attemptBackup`, Suffix `BACKUP_SUFFIX = ".bkup"`) | Q-FS `common/BackupHelper`; Aufrufstellen im Desktop nicht gelesen (Lücke L4) |
| `d/XX/Y…/` | `d/` | Inhaltsverzeichnis | A.6 |
| `<b64>.c9r` (Datei) | Inhaltsverzeichnis | Header + Blöcke | A.7 |
| `<b64>.c9r/dir.c9r`, `…/symlink.c9r` | Inhaltsverzeichnis | Verzeichnis-ID / Linkziel | A.6, A.9 |
| `<b64>.c9s/name.c9s`, `contents.c9r` | Inhaltsverzeichnis | langer Name / Inhalt | A.8 |
| `dirid.c9r` | Inhaltsverzeichnis | eigene Verzeichnis-ID, verschlüsselt | A.6 |
| `<b64>.c9u` | Inhaltsverzeichnis | Nutzungsinfo „in use“ (A.11) | Q-FS `inuse/*` |
| Konfliktkopie `<b64><Suffix>.c9r` | Inhaltsverzeichnis | von Sync-Clients erzeugt | A.12 |
| RTF-Lesehinweis | neben `d/` | unverschlüsselter Hinweistext | A.3 |

Wiederherstellungsverzeichnis: `RECOVERY_DIR_ID = "recovery"`, `RECOVERY_DIR_NAME = "LOST+FOUND"` (Q-FS `Constants`; Verwendung durch die Health-Prüfung, Details nicht gelesen).

### A.11 „Files in Use“ (`.c9u`)

- CryptoFS 2.10.0 (#312, #338): `.c9u`-Datei neben der Datei: `<Basisname ohne .c9r>.c9u` (Q-FS `RealInUseManager.computeInUseFilePath`). Inhalt: Java-`Properties` mit `owner` und `lastUpdated` (ISO-8601-Instant), verschlüsselt wie Dateiinhalt, Klartext höchstens `MAX_CLEARTEXT_SIZE_BYTES = 1000`. Erneuerung mit exponentiellem Rückzug, spätestens alle 300 s (`MAX_REFRESH_DELAY_SECONDS = 300`), erste Erzeugung nach `CREATION_DELAY_MILLIS = 5000`; veraltet nach `STALE_THRESHOLD_MINUTES = 10`.
- Doku `https://docs.cryptomator.org/desktop/files-in-use/`: „This feature is only available for Cryptomator Hub vaults.“; Synchronisierung „~10 seconds“; Status verfällt nach „around 10 min“; Aktion „Ignore Use Status“.
- **Folgerung:** Für Passwort-Tresore schreibt der Desktop keine `.c9u`; ein eigener Sync muss sie trotzdem als Tresorbestandteil durchreichen und darf sie nicht als Klartextdatei oder Konflikt werten.

### A.12 Konfliktnamen und automatische Auflösung in CryptoFS

Doku `https://docs.cryptomator.org/desktop/sync-conflicts/`: Cryptomator „detects sync conflicts and appends the suffix from your cloud provider to the decrypted filename“; Beispiel `businessPitch.odp` → `businessPitch (Created by Alice).odp`, belegt → `businessPitch (1).odp`; Empfehlung: „manually check both files and determine which one to keep“.

Ablauf im Code (Q-FS 2.11.0 `dir/C9rConflictResolver.java`), beim Auflisten eines Verzeichnisses:
1. Der Basisname wird mit `BASE64_PATTERN` nach einer entschlüsselbaren Teilfolge durchsucht (`C9rDecryptor`). Ist `vollerName == extrahiert + ".c9r"` → kein Konflikt. Beginnt der Name mit `.` → ignoriert.
2. Sonst „trivial“: `Files.move(conflicting, canonical)` (ohne Ersetzen). Gelingt das, war das Original nicht vorhanden → gelöst.
3. Existiert das Original und ist eines von beiden eine Datei → Umbenennen erforderlich.
4. Beide sind Ordner: `dir.c9r` beider vergleichen (volle Länge erforderlich); gleich → Konfliktkopie rekursiv löschen; verschieden → Umbenennen. Danach analog `symlink.c9r`. Ist der Typ einer Seite nicht bestimmbar (zum Beispiel `dir.c9r` noch nicht geschrieben) → **überspringen** (`TrivialResult.SKIP`, „Postponing conflict resolution … Not all required files exist (yet)“).
5. Umbenennen: Klartext-Basisname + ursprüngliches Konfliktsuffix (das Stück zwischen Base64-Teil und `.c9r`, höchstens 50 % der verfügbaren Klartextlänge) + Erweiterung; ist der Name belegt, `" (1)"` bis `" (9)"`; sind alle belegt → Warnung, Name bleibt. Danach `Files.move(…, ATOMIC_MOVE)` und Ereignis `ConflictResolvedEvent`; bei Fehler `ConflictResolutionFailedEvent`.
- Desktop 1.16.0 zeigt diese Ereignisse in der Ereignisansicht (Glockensymbol; Typen „Decryption Failed“, „Conflict Resolved“, „Conflict Resolution Failed“, „Broken File Node“, „Broken Directory File“; nur im Speicher; Kontextaktion „revealing affected files in your file manager“; **keine Vergleichsaktion**) — `https://docs.cryptomator.org/desktop/vault-events/`, Blog `https://cryptomator.org/blog/2025/04/29/desktop-1.16.0`.
- Issue #355 (CryptoFS, 2026-08-05 bis 2026-08-12): Legen zwei Nutzer denselben Ordner „foo“ an, erzeugt der Sync-Client eine Konfliktkopie des `.c9r`-Ordners; die Umbenennung verschob den ganzen Ordner samt `dir.c9r`, sodass zwei Klartextordner dieselbe Verzeichnis-ID teilten (Health-Check-Befund `DirIdCollision`), „conflict resolution by renaming is only sound for files“. 2.11.0 löst Verzeichniskonflikte nur noch bei vollständig gelesener ID.
- **Folgerung:** Die Auflösung in CryptoFS ist **automatisch und verändert den Chiffretext-Baum beim bloßen Auflisten** (Umbenennen, Löschen identischer Ordnerkopien). Das widerspricht E9 („does not resolve conflicts automatically“) nur dann, wenn se dieselbe Logik übernimmt; eine eigene Umsetzung kann Konfliktkopien stattdessen nur anzeigen und erst auf Nutzerbefehl umbenennen.

### A.13 Prüfvektoren aus den offiziellen Tests (Q-LIB 2.3.0, `src/test/java/org/cryptomator/cryptolib/…`)

| Test | Eingaben | Erwartet (Base64, Standardalphabet) |
|---|---|---|
| `v2/FileHeaderCryptorImplTest.testEncryption` | Masterkey 64 × `0x00`; Header-Nonce 12 × `0x00`; Nutzlast `-1L` ‖ 32 × `0x00` | `AAAAAAAAAAAAAAAAMVi/wrKflJEHTsXTuvOdGHJgA8o3pip00aL1jnUGNY7dSrEoTUrhey+maVG6P0F2RBmZR74SjU0=` (68 Byte) |
| `v2/FileContentCryptorImplTest.testChunkEncryption` | Inhaltsschlüssel 32 × `0x00`; Header-Nonce 12 × `0x00`; Block 0; Block-Nonce 12 × `0x33`; Klartext `hello world` | `MzMzMzMzMzMzMzMzbYvL7CusRmzk70Kn1QxFA5WQg/hgKeba4bln` |
| `v2/FileContentCryptorImplTest.testFileEncryption` | Masterkey 64 × `0x00`; Header-Nonce 12 × `0x55`, Inhaltsschlüssel 32 × `0x77`, Block-Nonce 12 × `0x55`; Klartext `hello world` über `EncryptingWritableByteChannel` (Header + 1 Block) | `VVVVVVVVVVVVVVVVC+/OFHHE8UvKYTOPlrMO5rCRLAI7/zk8Hjoisja03+yi9ugeeMz1evZhxDExrawl93vf9DKQPx5VVVVVVVVVVVVVVVVSxe6Nf7RO8orsVTzHAmXlNSy1oJpDrg9coV0=` (Annahme zur Zuordnung der drei Zufallsaufrufe: Reihenfolge Header-Nonce, Inhaltsschlüssel, Block-Nonce laut `create()`-Code) |
| `v2/FileContentCryptorImplTest.testChunkDecryption` | Inhaltsschlüssel 32 × `0x00`, Header-Nonce 12 × `0x00`, Block 0 | Eingabe-Chiffretext `VVVVVVVVVVVVVVVVnHVdh+EbedvPeiCwCdaTYpzn1CXQjhSh7PHv` (erwarteter Klartext im Test, nicht gelesen) |
| `v1/FileHeaderCryptorImplTest.testEncryption` | Masterkey 64 × `0x00`; Nonce 16 × `0x00`; Nutzlast `-1L` ‖ 32 × `0x00` | `AAAAAAAAAAAAAAAAAAAAACNqP4ddv3Z2rUiiFJKEIIdTD4r7x0U2ualjtPHEy3OLzqdAPU1ga24VjC86+zlHN49BfMdzvHF3f9EE0LSnRLSsu6ps3IRcJg==` (88 Byte) |
| `v1/FileContentCryptorImplTest.testChunkEncryption` | Masterkey 64 × `0x00`; Header-Nonce 16 × `0x00`; Inhaltsschlüssel 32 × `0x00`; Block-Nonce 16 × `0x00` (`SecureRandomMock.NULL_RANDOM`); Block 0; Klartext `hello world` | `AAAAAAAAAAAAAAAAAAAAALTwrBTNYP7m3yTGKlhka9WPvX1Lpn5EYfVxlyX1ISgRXtdRnivM7r6F3Og=` |
| `common/MasterkeyFileAccessTest` (Setup) | Passwort `asd`, Salz 8 × `0x00`, N = 2, r = 8, p = 1, Pfeffer leer, Masterkey 64 × `0x00`, `version = 3` | `primaryMasterKey` = `hmacMasterKey` = `mM+qoQ+o0qvPTiDAZYt+flaC3WbpNAx1sTXaUzxwpy0M9Ctj6Tih/Q==`; `versionMac` = `iUmRRHITuyJsJbVNqGNw+82YQ4A3Rma7j/y1v0DCVLA=` |

Nicht vorhanden: feste Vektoren für Dateinamen, Verzeichnis-ID-Hash, `.c9s`-Kürzung und einen vollständigen Beispieltresor in CryptoLib 2.3.0 und CryptoFS 2.11.0 (`src/test/resources` von CryptoFS enthält nur `mockito-extensions`). **Folgerung:** Für Namen und Verzeichnispfade braucht ein Interop-Test einen mit Cryptomator erzeugten Referenztresor (eigene Fixture) oder Vektoren aus einer anderen Implementierung (Lücke L1).

### A.14 Vergleichstabelle Formatfelder (Bytegrößen)

| Element | SIV_GCM | SIV_CTRMAC | Kodierung / Bemerkung |
|---|---|---|---|
| Encryption-Masterkey | 32 | 32 | roh; AES-256 |
| MAC-Masterkey | 32 | 32 | roh; HMAC-SHA256 und SIV-K1 |
| JWT-Signaturschlüssel | 64 | 64 | Enc ‖ MAC; HS256 (HS384/HS512 lesbar) |
| scrypt-Salz | 8 | 8 | Base64 in JSON |
| KEK | 32 | 32 | scrypt N = 32768, r = 8, p = 1 |
| gewickelter Schlüssel | 40 | 40 | RFC 3394, Base64 |
| `versionMac` | 32 | 32 | HMAC-SHA256(MAC-Key, int32 BE) |
| Header-Nonce | 12 | 16 | zufällig |
| Header-Nutzlast | 40 (8 reserviert + 32 Inhaltsschlüssel) | 40 | GCM bzw. CTR |
| Header-Tag/MAC | 16 | 32 | GCM-Tag bzw. HMAC |
| Header gesamt | 68 | 88 | — |
| Block-Nonce | 12 | 16 | zufällig je Block |
| Block-Nutzdaten | ≤ 32768 | ≤ 32768 | — |
| Block-Tag/MAC | 16 | 32 | GCM (AAD: Blocknr. 8 BE ‖ Header-Nonce) bzw. HMAC(Header-Nonce ‖ Blocknr. ‖ Nonce ‖ CT) |
| Blockgröße voll | 32796 | 32816 | Überhang 28 bzw. 48 |
| Name: SIV-Tag | 16 | 16 | AES-SIV (S2V-Schlüssel = MAC-Key) |
| Chiffrename | ≥ 28, ≤ `shorteningThreshold` Zeichen | gleich | Base64url mit `=`, Suffix `.c9r` |
| Kurzname | 32 Zeichen | gleich | Base64url(SHA-1) + `.c9s` |
| Verzeichnis-ID | ≤ 36 ASCII | gleich | UUID empfohlen; Wurzel leer |
| Verzeichnis-Hash | 20 → 32 Zeichen | gleich | Base32 (RFC 4648) von SHA-1(SIV(ID)) |
| Wiederherstellungsschlüssel | 66 Byte → 44 Wörter | gleich | 64 Byte Key + 2 Byte CRC32-Anteil, 4096-Wörter-Liste |
| `.c9u` Klartext | ≤ 1000 | ≤ 1000 | Properties, verschlüsselt wie Datei |

## B. Rust-Bausteine

Quellen: crates.io-API `https://crates.io/api/v1/crates/<name>` (Version, Datum, Lizenz, `rust_version`, Abhängigkeiten), RustSec `https://rustsec.org/packages/<name>.html` (eine 404-Antwort bedeutet: keine Paketseite, also kein Eintrag), READMEs der RustCrypto-Repositories (Zweig `master`), `docs.rs/aes/0.9.3`, `docs.rs/jsonwebtoken/11.1.0` — alle abgerufen 2026-10-17.

### B.1 Bestand in `native/Cargo.toml` (gelesen 2026-10-17, Paketversion 0.5.182)

- `edition = "2021"`; **kein** `rust-version`-Feld (MSRV im Manifest nicht festgelegt).
- Vorhanden und für Format 8 nutzbar: `sha1 = "0.10"`, `sha2 = "0.10"`, `hmac = "0.12"`, `zeroize = "1.8"`, `base64 = "0.22"`, `ring = "0.17"`, `getrandom = "0.2"`, `rand = "0.10"` (ohne Standardfunktionen, `thread_rng`), `serde_json = "1"`. Daneben `chacha20 = "=0.10.0"` und `chacha20poly1305 = "0.10.1"`, `argon2 = "=0.5.3"`, `hkdf = "0.12.4"`, `ml-kem = "0.3"`, `opaque-ke = "=4.0.1"`.
- Nicht vorhanden: `aes`, `aes-siv`, `aes-gcm`, `ctr`, `aes-kw`, `scrypt`, `cmac`, `data-encoding`/`base32`, `unicode-normalization`, `uuid`, `jsonwebtoken` (als direkte Abhängigkeiten; transitive Vorkommen in `Cargo.lock` wurden nicht geprüft, weil `Cargo.lock` nicht zur zugewiesenen Lesemenge gehört — Lücke L5).
- **Folgerung zur Generationsfrage:** Die vorhandenen `hmac 0.12`/`sha1 0.10`/`sha2 0.10` gehören zur RustCrypto-Generation `digest 0.10`/`cipher 0.4`/`aead 0.5`; die neuesten Versionen der fehlenden Crates (`aes-siv 0.8`, `aes-gcm 0.11`, `scrypt 0.12`, `aes-kw 0.3`) verlangen `digest 0.11`/`cipher 0.5`/`aead 0.6`. Entweder die ältere Generation nehmen (alle Crates verfügbar, siehe Tabelle) oder `hmac`/`sha1`/`sha2` gleichzeitig anheben. `chacha20 =0.10.0` im Bestand gehört bereits zur neuen `cipher 0.5`-Generation (Annahme aus der Versionsnummer; Abhängigkeiten nicht abgefragt).

### B.2 Vergleichstabelle Crates

| Primitiv (Format 8) | Crate neueste stabile | Datum | MSRV | Lizenz | Abhängigkeitsgeneration | ältere kompatible Version (digest 0.10) | Sicherheitslage |
|---|---|---|---|---|---|---|---|
| AES-SIV (Namen, Verzeichnis-IDs) | `aes-siv` 0.8.0 | 2026-08-11 | 1.85 | Apache-2.0 OR MIT | aead 0.6, aes 0.9, cipher 0.5, cmac 0.8, ctr 0.10, digest 0.11 | 0.7.0 (2022-07-30, MSRV 1.56; aead 0.5, aes 0.8, cmac 0.7, ctr 0.9) | README: „No security audits of this crate have ever been performed, and it has not been thoroughly assessed to ensure its operation is constant-time on common CPU architectures.“ RustSec: keine Einträge. |
| AES-GCM (SIV_GCM Header + Blöcke) | `aes-gcm` 0.11.1 | 2026-08-21 | 1.85 | Apache-2.0 OR MIT | aead 0.6, cipher 0.5, ctr 0.10, ghash 0.6, aes 0.9 (optional) | 0.10.3 (2023-09-21, MSRV 1.56) | README: eine Prüfung durch NCC Group „with no significant findings“; konstante Laufzeit über AES-NI/CLMUL oder Bitslicing. RustSec **RUSTSEC-2023-0096**: `decrypt_in_place_detached` gab Klartext trotz fehlgeschlagener Tag-Prüfung frei; behoben in 0.10.3. |
| AES-Blockchiffre | `aes` 0.9.3 | 2026-08-28 | **1.89** | MIT OR Apache-2.0 | cipher 0.5, cpufeatures 0.3 | 0.8.4 (2024-02-13, MSRV 1.56) | NCC-Prüfung laut README; docs.rs 0.9.3: Laufzeiterkennung von AES-NI/VAES (x86) und ARMv8-AES (aarch64; Doku nennt „Linux and macOS“, `cpufeatures` `aarch64.rs` erkennt per `getauxval` auch `target_os = "android"`), sonst konstantzeitige „fixslicing“-Software. |
| AES-CTR (SIV_CTRMAC) | `ctr` 0.10.1 | 2026-05-20 | 1.85 | MIT OR Apache-2.0 | cipher 0.5.2 | 0.9.2 (2022-09-30, MSRV 1.56) | RustSec: keine Einträge. |
| HMAC-SHA256 (CTRMAC, `versionMac`, JWT HS256) | `hmac` 0.13.0 | 2026-03-29 | 1.85 | MIT OR Apache-2.0 | digest 0.11.2 | **0.12.1 im Bestand** | `verify_slice` vergleicht konstantzeitig (Annahme aus der API-Bezeichnung; Quelltext nicht gelesen). |
| SHA-1 (Verzeichnis-Hash, Kurznamen) | `sha1` 0.11.0 | 2026-07-10 | nicht angegeben | MIT OR Apache-2.0 | digest 0.11 | **0.10.7 im Bestand** (2026-07-10 als Pflegeversion) | SHA-1 dient hier nur als Namensabbildung, nicht als Sicherheitsprimitiv (Q-VAULT: Kürzung „for compatibility, not security“). |
| SHA-256 (Backup-Suffix, scrypt intern) | `sha2` 0.11.1 | 2026-10-12 | 1.88 | MIT OR Apache-2.0 | digest 0.11 | **0.10.9 im Bestand** | — |
| scrypt (KEK) | `scrypt` 0.12.0 | 2026-04-22 | 1.85 | MIT OR Apache-2.0 | pbkdf2 0.13, salsa20 0.11, sha2 0.11 | 0.11.0 (2023-03-05, MSRV 1.60; sha2 0.10) | RustSec: keine Einträge. `Params::new(log_n, r, p, len)` erwartet log₂ N (OxCrypt rechnet `log_2(scryptCostParam)` um). Speichergrenze (A.2) muss der Aufrufer prüfen. |
| AES Key Wrap RFC 3394 | `aes-kw` 0.3.1 | 2026-05-27 | 1.85 | MIT OR Apache-2.0 | aes 0.9 | 0.2.1 (2022-04-20, MSRV 1.56; aes 0.8) | RustSec: keine Einträge. |
| Schlüssel löschen | `zeroize` 1.9.1 | 2026-10-06 | 1.85 | Apache-2.0 OR MIT | — | **1.8 im Bestand** | — |
| Base64url mit Auffüllung | `base64` 0.23.1 | 2026-08-04 | 1.71.0 | MIT OR Apache-2.0 | — | **0.22 im Bestand** (`URL_SAFE` mit Padding) | RUSTSEC-2017-0004 betrifft nur Versionen vor 0.5.2. |
| Base32 RFC 4648 groß, ohne Padding-Bedarf | `data-encoding` 2.11.1 | 2026-08-03 | 1.48 | MIT | — | — | OxCrypt nutzt `data-encoding`; cryptomator-rs nutzt `base32 0.5`. |
| Base32 (Alternative) | `base32` 0.5.1 (stabil), 1.0.0-rc.1 (2026-08-23) | 2024-07-18 | nicht angegeben | MIT OR Apache-2.0 | — | — | — |
| NFC | `unicode-normalization` 0.1.26 | 2026-10-16 | 1.36 | (MIT OR Apache-2.0) AND Unicode-3.0 | — | — | Unicode-3.0-Lizenz für Datentabellen zusätzlich. |
| UUID für Verzeichnis-IDs und `jti` | `uuid` 1.28.0 | 2026-10-10 | **1.89.0** | Apache-2.0 OR MIT | — | — | Ersatz ohne Crate: 16 Zufallsbytes als UUIDv4-Text formatieren (Folgerung). |
| JWT HS256 | `jsonwebtoken` 11.1.0 | 2026-09-16 | 1.88.0 | MIT | optional `hmac 0.12.1`, `sha2 0.10.7`, `aws-lc-rs 1.15` | — | `Validation`-Standard verlangt `exp` (`required_spec_claims` = `{"exp"}`, `validate_exp = true`, `validate_aud = true`); Cryptomator-JWT hat kein `exp`, also beides abschalten. Eigene Prüfung = HMAC-SHA256 über `base64url(header) "." base64url(payload)` mit 64-Byte-Schlüssel, `alg` aus {HS256, HS384, HS512}. |
| konstantzeitiger Vergleich | `subtle` 2.6.1 | 2024-06-24 | nicht angegeben | BSD-3-Clause | — | — | — |
| CRC32 (Wiederherstellungsschlüssel) | `crc32fast` 1.5.2 | 2026-09-12 | 1.63 | MIT OR Apache-2.0 | — | — | — |
| AES-GCM, HMAC, SHA aus `ring` | `ring` 0.17 im Bestand | — | — | — | — | — | `ring` bietet AES-256-GCM, HMAC-SHA256/384/512, SHA-1 (nur `SHA1_FOR_LEGACY_USE_ONLY`) und SHA-256, aber weder AES-SIV noch scrypt noch AES Key Wrap noch rohes AES-CTR (Annahme aus der bekannten `ring`-API; in diesem Auftrag nicht nachgelesen, Lücke L6). |

no_std/Android: Alle RustCrypto-Crates der Tabelle sind `no_std`-fähig ausgelegt (Annahme aus der RustCrypto-Projektkonvention; je Crate nicht einzeln geprüft); Android-relevant ist nur die Hardwarebeschleunigung, die `cpufeatures` auf aarch64-Android per `getauxval(AT_HWCAP)` erkennt (`HWCAP_AES | HWCAP_PMULL`).

MSRV-Folge: Mit den neuesten Versionen verlangt `aes 0.9.3` Rust 1.89 und `uuid 1.28.0` Rust 1.89.0; die ältere Generation kommt mit Rust 1.60 aus. Welche Rust-Version die CI von se nutzt, steht nicht in `native/Cargo.toml` (Lücke L5).

### B.3 OxCrypt (`agucova/oxcrypt`)

- Stand: Zweig `main` unverändert gegenüber dem Vorbefund, HEAD `916864f66184dae6f97b0e97e02b879144fcfed6` (committed 2026-08-23 21:04 −07:00); keine Tags/Releases auf der Tag-Seite. Lizenz MPL-2.0 (GitHub-Lizenzerkennung und Workspace-`Cargo.toml`). Workspace: `edition = "2024"`, `rust-version = "1.95"`.
- Umfang (Zeilen `*.rs`, gezählt am HEAD-Archiv): `oxcrypt-core` `src` 21 197, Tests 15 243; weitere Crates `oxcrypt-fuse` 16 352, `oxcrypt-bench` 13 719, `oxcrypt-mount` 10 951, `oxcrypt-desktop` 10 481, `oxcrypt-cli` 5 153, `oxcrypt-fskit` 3 738, `oxcrypt-fileprovider` 3 356, `oxcrypt-nfs` 2 814, `oxcrypt-webdav` 2 691.
- Abhängigkeiten `oxcrypt-core` (Auswahl der Krypto- und Plattformteile): `aes 0.8`, `aes-gcm 0.10`, `aes-siv 0.7`, `ctr 0.9`, `scrypt 0.11`, `ring 0.17` (HMAC für CTRMAC), `jsonwebtoken 10` (Funktion `rust_crypto`), `data-encoding 2.9`, `unicode-normalization 0.1`, `uuid 1`, `secrecy 0.10`, `memsafe 0.4`, `zeroize 1`, `subtle 2`, `walkdir 2`, `tempfile 3`, `rayon 1.11`, `moka 0.12.12`, `dashmap 6`, `tokio 1` (optional).
- Mängel aus dem Vorbefund (am selben SHA weiterhin vorhanden, weil HEAD unverändert): (1) `vault/config.rs` Zeile 427 ff. `extract_master_key`: `decode_header` → `kid` → `Url::parse(&kid)` → `vault_path.join(Path::new(masterkey_uri.path()))` vor Signaturprüfung, ohne Bindung an die Tresorwurzel; (2) `vault/operations.rs` Zeile 2518 `rename_file` (Ablauf: Zielprüfung per Auflistung, gesamten Inhalt lesen, schreiben, Quelle löschen). Neu festgestellt: scrypt-Parameter aus der Datei werden ohne Obergrenze an `scrypt::Params::new` gereicht (`vault/master_key.rs` Zeile 183–189; keine Prüfung gegen eine Speichergrenze in diesem Bereich gefunden — nur dieser Bereich gelesen).
- Testfixture `test_vault/` (Wurzel des Repositories): 206 Dateien; `vault.cryptomator` Header `{"kid":"masterkeyfile:masterkey.cryptomator","alg":"HS256","typ":"JWT"}`, Payload `{"jti":"ea3282b3-3847-499b-82fd-a3723857a225","format":8,"cipherCombo":"SIV_GCM","shorteningThreshold":220}`; enthält `IMPORTANT.rtf`, `masterkey.cryptomator.F1B31BCD.bkup`, `vault.cryptomator.B4005D33.bkup`, 7 Unterordner unter `d/`, 2 × `dirid.c9r`, 0 × `.c9s`, 0 × `symlink.c9r`; Passwort in den Tests `"123456789"` (`crates/oxcrypt-fuse/tests/write_read_consistency.rs`). **Folgerung:** Die `.bkup`-Dateien und `IMPORTANT.rtf` deuten auf Erzeugung mit Cryptomator Desktop hin (Annahme, nicht belegt); als Interop-Fixture deckt sie lange Namen und Links nicht ab. Lizenz der Fixture: MPL-2.0 wie das Repository (Annahme: keine abweichende Lizenzdatei im Ordner; nicht geprüft).

### B.4 Dar9586/cryptomator-rs

- Stand: HEAD `c0c96cf45ecf17f18d6ece1e68adf51b7d18446d` (committed 2025-12-05 21:21 +01:00), unverändert gegenüber dem Vorbefund; 33 Commits; keine Tags. Lizenz Apache-2.0; `edition = "2024"`; Crate `cryptomator-rs-crypto` 0.1.3.
- Umfang: `cryptomator-rs-crypto` 1 305 Zeilen, `cryptomator-rs-fuse` 599, `cryptomator-rs-cli` 93.
- Abhängigkeiten `cryptomator-rs-crypto`: `scrypt 0.11`, `cmac 0.7`, `hmac 0.12`, `sha2 0.10`, `sha1 0.10`, `aes-kw 0.2`, `aes-siv 0.7`, `aes-gcm 0.10` (`std`), `jwt 0.16`, `base64 0.22`, `base32 0.5`, `unicode-normalization 0.1`, `uuid 1.18.1` (`v4`), `libc 0.2.177`, `serde`, `serde_json`, `serde_with`, `rand 0.9`, `itertools 0.14`, `log`, `thiserror 2`, `url 2`.
- Mängel (am selben SHA weiterhin vorhanden): `src/cryptomator.rs` Zeile 285 `vault_root.join(uri.path())` vor Signaturprüfung; Zeile 413–420 `rename`: `new_entry = old_dir.lookup(old_name)` (Ziel wird nie geprüft; `no_replace = true` → stets `EEXIST`). Weitere Befunde: nur `SIV_GCM` und `HS256` (Zeilen 278–283); Sicherungen heißen `masterkey.cryptomator.bak`/`vault.cryptomator.bak` (Zeilen 377–381), nicht Cryptomators `.<hex>.bkup`; Fehler als `libc`-Fehlercodes (`UnixError(libc::EEXIST)`), also Unix-gebunden.

### B.5 Übernahme gegenüber eigener Umsetzung (nur Fakten)

| Merkmal | Eigene Umsetzung in se | OxCrypt `oxcrypt-core` | cryptomator-rs-crypto | CryptoLib/CryptoFS (Java) |
|---|---|---|---|---|
| Lizenz | MIT (se) | MPL-2.0 (Datei-Copyleft) | Apache-2.0 | AGPLv3 oder kommerziell (Vorbefund) |
| Codeumfang Kern | nicht vorhanden | 21 197 Zeilen `src` | 1 305 Zeilen | nicht gezählt |
| Cipher-Kombinationen | — | SIV_GCM, SIV_CTRMAC (Vorbefund) | nur SIV_GCM | beide |
| E/A-Modell | über se-`vfs::Backend` möglich (Strangidee M-Schicht) | `std::fs` (Vorbefund) | `std::fs` + `libc` | `java.nio.file` |
| Rust-Version | Bestand Edition 2021 | Edition 2024, MSRV 1.95 | Edition 2024 | — |
| Konfliktbehandlung | — | keine Konfliktlogik in `src` gefunden (Suche nach „conflict“ trifft nur Fehlermeldungen in `crypto/mod.rs`, `fs/file.rs`, `fs/name.rs`; Inhalte nicht geprüft) | keine (Suche ohne Treffer) | automatisch (A.12) |
| bekannte offene Mängel | — | `kid`-Pfad, `rename_file`, scrypt-Grenze | `kid`-Pfad, `rename`, nur GCM | — |
| Pflege | — | letzter Commit 2026-08-23 | letzter Commit 2025-12-05 | aktiv (2.3.0/2.11.0 im Oktober 2026) |

## C. Sync und Vergleich

### C.1 Konfliktbehandlung je Produkt

**Cryptomator Desktop 1.19.3 / CryptoFS** — siehe A.12. Erkennung beim Auflisten am Chiffretextnamen (Base64-Teil + fremdes Suffix vor `.c9r`); automatische Umbenennung in `<Basis><Anbietersuffix>.<ext>` bzw. `<Basis> (1..9).<ext>`; identische Verzeichniskopien werden gelöscht; Anzeige in der Ereignisansicht (seit 1.16.0) mit Aktion „Im Dateimanager zeigen“; Empfehlung der Doku: „manually check both files and determine which one to keep“. Kein Vergleich in der App. Nicht erfasst: Namen, die nicht auf `.c9r`, `.c9s` oder `.c9u` enden, werden beim Auflisten stillschweigend übergangen (Q-FS `dir/NodeProcessor.java` Zeile 25–32: `else return Stream.empty()`); `.c9s`-Knoten laufen durch `C9sInflator` ohne Konfliktauflöser (Zeile 34–44).

**Syncthing** (`https://docs.syncthing.net/users/syncing.html`, „Written for v2.1.0“, Version „v2.1.0-25-g2590451“):
- Erkennung: Dateisystembeobachter (Standard an, Bündelung 10 s, Löschungen weitere 1 min verzögert) plus Vollscan (Standard stündlich, zufällig 3/4–5/4 des Intervalls); Vollscan prüft „modification time, size or permission bits“, bei Änderung Neuhashen in Blöcken 128 KiB–16 MiB mit SHA-256.
- Konflikt: Datei „modified on two devices simultaneously and the content actually differs“; die Fassung mit der **älteren** Änderungszeit wird umbenannt in `<filename>.sync-conflict-<date>-<time>-<modifiedBy>.<ext>`; bei gleicher Zeit verliert das Gerät mit dem größeren Wert der ersten 63 Bit der Geräte-ID; gewinnt eine Löschung gegen eine Änderung, wird die geänderte Datei zur Konfliktkopie. Konfliktkopien werden an alle Geräte verteilt.
- `maxConflicts` (`https://docs.syncthing.net/users/config.html`): Standard `10`; `-1` unbegrenzt; `0` schaltet Konfliktkopien ab.
- Benutzerführung: keine Vergleichsansicht in Syncthing selbst (in den gelesenen Seiten nicht erwähnt).
- Temporärdateien: `.syncthing.<name>.tmp` bzw. unter Windows `~syncthing~<name>.tmp`, danach Verschieben an den Zielort.

**Nextcloud Desktop** (Release `v35.0.0`, 2026-10-12; Doku Nextcloud-35-Handbuch `https://docs.nextcloud.com/server/stable/user_manual/en/desktop/conflicts.html`; Quelltext Tag `v35.0.0`; Architekturtext `doc/architecture.rst` am Tag `v3.16.0`, auf `master` nicht mehr vorhanden):
- Erkennung: lokal Vergleich der Änderungszeit mit dem Wert in der Journal-Datenbank; entfernt Vergleich der ETag je Datei mit dem gespeicherten Wert („If the ETag is the same, the file has not changed“). Prüfsummen (`checksumHeader`) werden mitgeführt und bei Übertragungen geprüft (`src/libsync/discovery.cpp`).
- Konflikt: beide Seiten seit dem letzten Lauf geändert → lokale Datei wird umbenannt, Serverfassung unter dem Originalnamen geladen; „Conflict files are always created on the client and never on the server.“ Konfliktkopie standardmäßig nicht hochgeladen; experimentell `OWNCLOUD_UPLOAD_CONFLICT_FILES=1`.
- Name (Code `src/common/utility.cpp` `makeConflictFileName`): `<Basis> (conflicted copy <Benutzer> yyyy-MM-dd hhmmss).<ext>`, Benutzer optional, Klammern im Benutzernamen durch `_` ersetzt; Einfügung vor dem **letzten** Punkt des Namens. Doku-Beispiel: `mydata (conflicted copy 2018-04-10 093612).txt`.
- Benutzerführung: Systembenachrichtigung, Tray-Symbol, gelbes Abzeichen „unresolved conflicts“ mit Liste; Konfliktdialog (`src/gui/conflictdialog.ui/.cpp`): Titel „Sync Conflict“, Text „Conflicting versions of %1.“, zwei Spalten „Local version“ / „Server version“ mit Änderungszeit (neuere fett), Größe, Vorschau, Links „Open local version“ / „Open server version“; Auswahl per Kontrollkästchen; Knopf „Keep local version“, „Keep server version“ oder „Keep both versions“ („If you select both versions, the local file will have a number added to its name.“).

**ownCloud Desktop** (neuester Release `v7.1.1`, 2026-09-01; Vorabversion `v7.2.0-rc.1`, 2026-10-06; Code `master` `src/common/utility.cpp`): Name `<Basis> (conflicted copy yyyy-MM-dd hhmmss).<ext>` (ohne Benutzer), Einfügung vor dem letzten Punkt. Ein eigener Konfliktdialog wurde im Ordner `src/gui` auf `master` nicht gefunden (Dateiliste ohne „conflict“ im Namen); Bedienführung nicht weiter geprüft (Lücke L7).

**Unison** (neuester Release `v2.54.0`, 2026-04-28; Quelltext `master`: `src/strings.ml`, `src/uitext.ml`, `src/files.ml`; Handbuch `doc/unison-manual.tex`):
- Erkennung: Archiv je Pfad nach erfolgreichem Abgleich; schneller Vorlauf über Änderungszeiten (Unix: Inode-Nummer und mtime unverändert → „not been changed“), sonst Inhalts-Fingerabdruck gegen den Archivstand.
- Konflikt: Pfad auf einer Seite geändert und (oder ein Nachfahre) auf der anderen, Inhalte verschieden; „By default, neither version will be propagated and both replicas will remain as they are.“ Gleiche Endstände („false conflicts“) werden stillschweigend als abgeglichen markiert.
- Aktionen je Pfad (Text-UI): `>`/`<` übertragen in eine Richtung, `/` überspringen, `d` „show differences“, `x` Details, `m` „merge the versions“, `]` neuere / `[` ältere Fassung, `c` „changed“, `r` Standardempfehlung, `i`/`I`/`E`/`N` dauerhaft ignorieren, `C` alle Konflikte auswählen, Auswahlfilter `A`, `1`, `2`, `M`, `X`, `&`, `|`.
- Einstellungen: `prefer` (Wurzel, `newer`, `older`) löst ohne Rückfrage auf; `copyonconflict` behält vor dem Überschreiben eine Kopie `<Name> (conflict_on_JJJJ-MM-TT)` bzw. `(conflict #n_on_…)`, bei Löschung `…_was_deleted` — das Suffix wird an den **vollständigen** Namen angehängt (`Path.addToFinalName`), also **hinter** der Erweiterung; `diff` (Befehl für Unterschiedsanzeige); `merge` (externes Werkzeug, Nicht-Null-Rückgabe = gescheitert, Replikate unverändert); `confirmbigdel` (Standard an: Rückfrage, wenn eine ganze Wurzel gelöscht scheint); `mountpoint` (Abbruch, wenn ein Pfad in einer Wurzel fehlt). Fehlt das Archiv, verhält sich Unison, als wären beide Seiten beim letzten Abgleich leer gewesen.

**Dropbox** (`https://help.dropbox.com/organize/conflicted-copy`): Konfliktkopie, wenn mehrere Personen gleichzeitig bearbeiten, offline bearbeitet wird oder Dateien auf einem anderen Rechner offen sind; „The editor's username, "conflicted copy", and the save date will be added to the filename.“; „The last version saved will always appear as the conflicted copy.“; Lösung: „compare both versions and merge them manually“. Genaues Namensmuster auf der Seite nicht angegeben (Lücke L8).

**Cyberduck / Mountain Duck** (`https://docs.cyberduck.io/cryptomator/`, `https://docs.mountainduck.io/mountainduck/connect/sync/`):
- Tresore: Formate 7 und 8, neue Tresore Format 8; Format 6 nicht unterstützt; Erkennung, wenn `masterkey.cryptomator` oder `vault.cryptomator` unter den ersten 10 Einträgen einer Auflistung liegt; Anlegen möglich, **Passwortwechsel nicht**; **„Name shortening is not supported“**, höchste Dateinamenlänge 146 Zeichen; Tresor in Tresor nicht unterstützt; Dateien vor dem Verschieben in den Tresor müssen über den lokalen Rechner laufen.
- Konflikt (Mountain Duck Smart Synchronization): erkannt, wenn „Last seen checksum differs from current checksum on server. (Or timestamp when server does not offer checksum verification)“; Konfliktdatei `<filename> timestamp.<extension>`; Meldung „Sync Conflict“; Nutzer vergleicht manuell und löscht die Kopie. Während pausierter Synchronisierung werden Serveränderungen nicht erkannt.

### C.2 Vergleichstabelle Produkte

| Produkt (Stand) | Änderungserkennung | Konfliktkriterium | Benennung der Kopie | Wer verliert den Namen | Bedienung | Vergleich in der App | Automatische Auflösung |
|---|---|---|---|---|---|---|---|
| Cryptomator 1.19.3 / CryptoFS 2.11.0 | keine eigene; liest, was der Sync-Client hinterlässt | fremdes Suffix im Chiffrenamen | `<Basis><Anbietersuffix>.<ext>`, sonst ` (1)`–` (9)` | die Kopie mit Suffix | Ereignisansicht, „im Dateimanager zeigen“ | nein | ja (Umbenennen; identische Ordner löschen) |
| Syncthing v2.1.0 | Watcher + Scan (mtime, Größe, Rechte) → SHA-256-Blöcke | beide geändert, Inhalt verschieden | `<name>.sync-conflict-<date>-<time>-<modifiedBy>.<ext>` | ältere mtime; Gleichstand: größere Geräte-ID | keine eigene Konfliktliste in den gelesenen Seiten | nein | ja (Gewinner bestimmt, Kopie behalten) |
| Nextcloud v35.0.0 | lokal mtime, entfernt ETag; Prüfsummen | beide seit letztem Lauf geändert | `<Basis> (conflicted copy [Benutzer] yyyy-MM-dd hhmmss).<ext>` | lokale Fassung | Benachrichtigung, Liste, Konfliktdialog | Metadaten nebeneinander (Zeit, Größe, Vorschau), Öffnen-Links | ja (Server gewinnt Namen), danach Dialog |
| ownCloud v7.1.1 | wie Nextcloud (gemeinsamer Ursprung; Annahme) | wie Nextcloud | `<Basis> (conflicted copy yyyy-MM-dd hhmmss).<ext>` | lokale Fassung | nicht geprüft | nicht geprüft | ja |
| Unison v2.54.0 | Archiv; mtime + Inode, dann Fingerabdruck | Pfad beidseitig geändert, Inhalte verschieden | nur mit `copyonconflict`: `<Name> (conflict_on_JJJJ-MM-TT)` hinter der Erweiterung | keiner (Standard) | interaktive Liste je Pfad | ja: `d` (externes diff), `m` (merge) | nein (Standard); ja mit `prefer` |
| Dropbox | nicht dokumentiert auf der gelesenen Seite | gleichzeitige Bearbeitung | Benutzername + „conflicted copy“ + Datum | zuletzt gespeicherte Fassung | keine | nein | ja |
| Mountain Duck | Prüfsumme, sonst Zeitstempel | Prüfsumme weicht vom zuletzt gesehenen Wert ab | `<filename> timestamp.<extension>` | die Serverdatei wird umbenannt (laut Tabelle der Doku) | Benachrichtigung „Sync Conflict“ | nein | ja |

### C.3 Zuverlässiges Erkennen „neuester Änderungen“ auf entfernten Speichern

| Backend-Typ in se | Verfügbares Signal | Quelle (abgerufen 2026-10-17) | Grenze |
|---|---|---|---|
| Google Drive | Änderungsfeed: `changes.getStartPageToken`, `changes.list` mit `pageToken`, `nextPageToken`, `newStartPageToken`; Einträge chronologisch („oldest changes appear first“); `changes.watch` meldet nur, dass es Änderungen gibt | `https://developers.google.com/workspace/drive/api/guides/manage-changes` | Felder wie `md5Checksum`, `modifiedTime` auf dieser Seite nicht beschrieben (Lücke L9) |
| WebDAV | `DAV:getetag` je Ressource (RFC 4918); optional `DAV:sync-collection`-Report mit Sync-Token (RFC 6578 §3.2) | RFC-Texte `rfc-editor.org` | `sync-collection` nur bei Serverunterstützung; ob die in se angebundenen Server sie haben, nicht ermittelt |
| FTP/FTPS | `MLSD`/`MLST`-Fakt `Modify` = `14DIGIT [ "." 1*DIGIT ]` (RFC 3659 §7.5.3) und `Size` | RFC 3659 | Sekundenauflösung ohne Bruchteil zulässig |
| SFTP (Protokollversion 3) | `ATTRS` mit `uint32 mtime` (Sekunden) bei `SSH_FILEXFER_ATTR_ACMODTIME` | `draft-ietf-secsh-filexfer-02` | 1-Sekunden-Auflösung, 32 Bit |
| lokal/UNC/SMB | mtime + Größe (+ Datei-ID/Inode), Beobachter | Syncthing- und Unison-Verfahren oben | — |
| Nextcloud-ähnlich | ETag je Datei und Ordner | Nextcloud `architecture.rst` | — |

Folgerungen für einen Abgleich auf **Chiffretextebene** (alle als **Folgerung** aus A und C.1–C.3; nicht durch Versuche belegt):
1. Jede Inhaltsänderung verschlüsselt die betroffenen Blöcke mit neuer zufälliger Nonce (A.7); ein Inhaltshash über den Chiffretext erkennt daher jede Änderung. mtime + Größe allein verfehlen eine gleich lange Änderung innerhalb derselben Sekunde auf SFTP v3 und FTP ohne Bruchteile.
2. Ein Umbenennen oder Verschieben im Klartext erscheint im Chiffretext als Löschen + Anlegen einer Datei mit anderem Namen (Name ist mit der Elternverzeichnis-ID verschlüsselt, A.5), bei Dateien mit identischen Inhaltsbytes. Ein Verzeichnis-Umbenennen verschiebt nur den Knoten `<name>.c9r/` mit `dir.c9r`; der Inhalt unter `d/` bleibt unverändert, weil die Verzeichnis-ID bleibt (A.6).
3. Legen beide Seiten im selben Ordner eine gleichnamige Datei an, entsteht derselbe Chiffrename (SIV ist deterministisch) — der Sync sieht eine echte Kollision. Legen beide denselben Ordner an, entstehen gleichnamige `<name>.c9r/`-Knoten mit **verschiedenen** `dir.c9r`-Inhalten und zwei verschiedenen `d/`-Bäumen — genau der Fall von CryptoFS-Issue #355.
4. Löscht eine Seite einen Ordner (Knoten + `d/`-Inhaltsverzeichnis) und fügt die andere darin eine Datei hinzu, bleibt ein Inhaltsverzeichnis ohne verweisenden `dir.c9r` übrig (Health-Check-Befund `OrphanContentDir`).
5. Übernimmt ein Abgleich einen Zwischenstand der Anlage-Reihenfolge (A.6), entstehen `MissingContentDir` oder `EmptyDirFile`; Übertragungsreihenfolge „erst `d/…`-Inhaltsverzeichnis mit `dirid.c9r`, dann `dir.c9r`“ vermeidet einen sichtbaren Knoten ohne Inhalt.
6. Konfliktkopien, die ein Abgleich anlegt, müssen so benannt sein, dass CryptoFS sie verarbeitet: Suffix **vor** `.c9r` (Nextcloud/ownCloud-Muster und Syncthing-Muster tun das, weil sie vor der letzten Erweiterung einfügen); Unisons `copyonconflict`-Suffix hinter `.c9r` macht die Kopie für Cryptomator unsichtbar (NodeProcessor). Konfliktkopien von `contents.c9r`/`name.c9s` **innerhalb** eines `.c9s`-Ordners (zum Beispiel `contents (conflicted copy …).c9r`) behandelt CryptoFS nicht als Konflikt (keine Auflösung im `C9sInflator`).
7. `vault.cryptomator`, `masterkey.cryptomator` und `*.bkup` sind Tresormetadaten; ein beidseitig geänderter `masterkey.cryptomator` (Passwortwechsel auf beiden Geräten) ergibt zwei gültige Schlüsseldateien mit verschiedenen Passwörtern für denselben Masterkey.

### C.4 Belegte Risiken beim Synchronisieren laufender Tresore

- Unvollständige Verzeichnisstruktur: Doku „Broken File Node Event“: „the encrypted directory is missing required identification files“, „The directory is missing essential files like dir.c9r“; Handlungsempfehlung „Ensure the encrypted directory is properly synced. Compare the content of the encrypted directory with what appears in your cloud provider's web interface or on other devices.“ (`https://docs.cryptomator.org/desktop/vault-events.md`).
- Beschädigte `dir.c9r`: „Broken Directory File Event“: „either because it's empty when it shouldn't be, or because it exceeds the maximum allowed size of 1000 bytes“.
- Doppelte Verzeichnis-IDs durch Konfliktkopien von Ordnern: CryptoFS-Issue #355 (A.12), betroffen CryptoFS 2.10.0 = Desktop 1.19.3.
- Konfliktauflösung während transienter Zustände: PR #356 „conflict resolution is now skipped if affected resources are in a transient state … (i.e. `dir.c9r` not written yet)“ und Reduktion von TOCTOU-Fällen.
- Health-Check-Befundtypen (Q-FS 2.11.0, Pakete `health/dirid`, `health/shortened`, `health/type`): `DirIdCheck`-Ergebnisse `HealthyDir`, `DirIdCollision`, `EmptyDirFile`, `LooseDirFile`, `MissingContentDir`, `MissingDirIdBackup`, `ObeseDirFile`, `OrphanContentDir`; `ShortenedNamesCheck`-Ergebnisse `ValidShortenedFile`, `ObeseNameFile`, `TrailingBytesInNameFile`; Typprüfung `KnownType`, `UnknownType`, `AmbiguousType`; allgemein `CheckFailed`, `PoisonResult`. Bedeutungen nur aus den Klassennamen abgeleitet (Lücke L10).
- Gleichzeitiges Öffnen auf mehreren Geräten: „Files in Use“ nur für Hub-Tresore (A.11); für Passwort-Tresore existiert keine geräteübergreifende Sperre (Vorbefund: „keine globale Sperre zwischen mehreren Rechnern“).
- Mountain Duck/Cyberduck kürzen keine Namen (146 Zeichen); Interop mit Tresoren, die `.c9s` enthalten, ist auf deren Seite nicht dokumentiert (Lücke L11).

## Widersprüchliche Angaben

| Nr. | Thema | Angabe 1 | Angabe 2 | Einordnung |
|---|---|---|---|---|
| W1 | CRC-Anteil im Wiederherstellungsschlüssel | Code-Kommentar Q-DESK `RecoveryKeyFactory` Zeile 67: „copy 16 most significant bits of CRC32(rawKey)“ | Guava `HashCode.fromInt`: „The underlying bytes are interpreted in little endian order“; `toString`: „everything else in the hashing API uniformly treats multibyte values as little-endian“ → `writeBytesTo(…, 2)` bzw. `copyOf(asBytes(), 2)` liefert die **zwei niederwertigsten** Bytes in Little-Endian-Reihenfolge | Folgerung: maßgeblich ist das Verhalten (`asBytes()[0..2]`), nicht der Kommentar; nur ein Referenz-Wiederherstellungsschlüssel kann es bestätigen (Lücke L2). |
| W2 | Inhalt von `dirid.c9r` | Doku Q-VAULT (Zusammenfassung des Abrufs): „contains the parent folder's directory ID“ | Q-FS `DirectoryIdBackup.write`: schreibt `ciphertextDirectory.dirId()` in `ciphertextDirectory.path()/dirid.c9r` — die ID des Verzeichnisses, dessen Kinder dort liegen | Kein Widerspruch in der Sache, wenn „parent“ aus Sicht der Kinder gemeint ist; maßgeblich ist der Code. |
| W3 | Leere Datei | CryptoFS-Größenformel: leere Datei = nur Header (68/88 Byte); 96 Byte wären ungültig (`cleartextSize(28)` wirft) | CryptoLib `EncryptingWritableByteChannel.close()` schreibt immer einen weiteren Block, auch leer | Folgerung: Leser müssen beide Formen tolerieren; Schreiber sollen der CryptoFS-Form folgen (Annahme: CryptoFS schreibt beim Anlegen leerer Dateien nur den Header; dessen `CleartextFileChannel` wurde nicht gelesen, Lücke L12). |
| W4 | Größe von `dir.c9r` | Q-FS `MAX_DIR_ID_LENGTH = 36` | Doku Ereignisse: „exceeds the maximum allowed size of 1000 bytes“ | Folgerung: 36 ist die ID-Länge beim Erzeugen und Vergleichen; 1000 Byte die Lesegrenze für „ObeseDirFile“ (Annahme; Lesegrenze im Code nicht nachgesehen). |
| W5 | Nextcloud-Konfliktname | `architecture.rst` (v3.16.0): `message_conflict-20160101-153110.txt` | Code v35.0.0: `… (conflicted copy [Benutzer] yyyy-MM-dd hhmmss).ext`; Handbuch 35: `mydata (conflicted copy 2018-04-10 093612).txt` | Architekturtext veraltet; maßgeblich ist der Code. |
| W6 | Doku-Inhalt Cyberduck | „Name shortening is not supported“ | Format 8 verlangt Kürzung ab `shorteningThreshold` | Cyberduck schreibt nur Namen ≤ 146 Zeichen; ob es vorhandene `.c9s` liest, sagt die Seite nicht (L11). |
| W7 | aarch64-Erkennung `aes` | docs.rs `aes` 0.9.3: Laufzeiterkennung „on Linux and macOS“ | `cpufeatures` `aarch64.rs`: `#[cfg(any(target_os = "linux", target_os = "android"))]` | Quelltext geht vor; Android ist abgedeckt. |

## Offene Lücken

- L1: Feste Prüfvektoren für Dateinamen (AES-SIV mit Verzeichnis-ID), Verzeichnis-ID-Hash und `.c9s`-Kurznamen fehlen in CryptoLib 2.3.0 und CryptoFS 2.11.0. Ein mit Cryptomator 1.19.3 erzeugter Referenztresor mit bekanntem Passwort, langen Namen, Symlink, leerer Datei und Datei genau 32 768 Byte lang existiert in den geprüften Quellen nicht; OxCrypts `test_vault` deckt `.c9s` und Symlinks nicht ab.
- L2: Kein Referenz-Wiederherstellungsschlüssel (Masterkey ↔ 44 Wörter) gefunden; Testdateien zu `RecoveryKeyFactory`/`WordEncoder` im Desktop-Repository nicht gelesen.
- L3: Lizenz der Wortliste `/i18n/4096words_en.txt` (Desktop-Repository, GPLv3) — ob eine Übernahme in MIT-Code zulässig ist bzw. ob die Liste aus einer frei lizenzierten Quelle stammt, wurde nicht ermittelt.
- L4: Aufrufstellen von `BackupHelper.attemptBackup` (wann `vault.cryptomator.<hex>.bkup` und `masterkey.cryptomator.<hex>.bkup` beim Entsperren entstehen) nicht gelesen.
- L5: `Cargo.lock` (transitive Krypto-Crates) und die von se-CI verwendete Rust-Version nicht geprüft (außerhalb der zugewiesenen Lesemenge).
- L6: `ring`-API-Umfang (AES-SIV, AES-KW, rohes AES-CTR, SHA-1-Einschränkung) nicht in dieser Sitzung nachgelesen.
- L7: ownCloud-Desktop-Konfliktbedienung (Dialog, Liste) nicht geprüft.
- L8: Exaktes Dropbox-Namensmuster der „conflicted copy“ (Reihenfolge, Datumsformat) auf der Hilfeseite nicht angegeben.
- L9: Felder `md5Checksum`, `sha256Checksum`, `headRevisionId`, `modifiedTime`, `version` der Drive-API für Chiffretext-Abgleich nicht nachgelesen.
- L10: Bedeutung und automatische Reparaturen der Health-Check-Befundtypen nur aus Klassennamen; Doku-Seite `docs.cryptomator.org/desktop/health-check/` lieferte HTTP 404.
- L11: Cyberduck/Mountain Duck: Lesen vorhandener `.c9s`-Knoten, Verhalten bei Konfliktkopien im Tresor nicht dokumentiert.
- L12: CryptoFS-Schreibpfad für leere Dateien (`ch/CleartextFileChannel`, `fh/*`) und ob ein Dateiheader bei Überschreiben/Truncate neu erzeugt wird, nicht gelesen; das bestimmt, ob eine Inhaltsänderung den Header (und damit die ersten 68 Byte) ändert.
- L13: `CryptoFileSystemProperties` 2.11.0 hat keine Eigenschaft zum Abschalten der Konfliktauflösung (Eigenschaften: `maxCleartextNameLength`, `shorteningThreshold`, `keyLoader`, `vaultConfigFilename`, `masterkeyFilename` (veraltet), `fsEventConsumer`, `flags`, `cipherCombo`, `ownerGetter`; einziger `FileSystemFlags`-Wert `READONLY`). Offen: ob `READONLY` die umbenennende Auflösung beim Auflisten unterdrückt (`C9rConflictResolver` prüft im gelesenen Bereich kein Nur-Lese-Flag).
