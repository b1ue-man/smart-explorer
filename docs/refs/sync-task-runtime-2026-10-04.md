# Sync-Abnahme: tatsächliche Server und alte Android-Appdaten

Primärquellen und aktuelle Implementierung geprüft am 2026-10-04.
Diese Ref beschreibt ausschließlich die Remote-Task-Abnahme; sie ist kein
lokaler Build-/Testauftrag und keine Änderung produktiver TLS- oder Signaturregeln.

## Android-Update statt Neuinstallation

- [Android App Signing](https://developer.android.com/studio/publish/app-signing):
  ein APK-Update benötigt einen zur installierten App passenden Signaturschlüssel.
  Der automatisch erzeugte Debugschlüssel erfüllt das für eine veröffentlichte
  Release-APK nicht. App- und Test-APK erhalten für diesen Remote-Task explizit
  die vorhandene Release-Signatur; der normale Debugbuild bleibt unverändert.
- [Build Variants](https://developer.android.com/build/build-variants):
  `android.signingConfigs` definiert den Schlüssel, `buildTypes.debug.signingConfig`
  wählt ihn für den Development-Build aus. Die Auswahl erfolgt nur bei einer
  ausdrücklich gesetzten Task-Property und vollständig vorhandenen Secrets.
- [Advanced Test Setup](https://developer.android.com/studio/test/advanced-test-setup):
  das Instrumentation-APK hat ein eigenes Manifest und einen `<instrumentation>`-
  Eintrag; die Ziel-Package-ID/Runner werden aus installierter Metadatenantwort
  ermittelt. Standard-Testbuild ist debug. Eine Instrumentation kann den alten
  Zielprozess benutzen, sofern Signatur und verwendete öffentliche APIs passen.
- Aktueller Source: `android/app/build.gradle.kts`, `ReviewSyncTaskTest.kt`,
  `native/review-task-device.py`. Die alten APIs müssen zusätzlich gegen den
  Tag geprüft werden. Kein `run-as` für die nicht debuggable alte Release-App
  voraussetzen; Prepare läuft als passend signierte Instrumentation im Ziel.
- Statische Artefaktprüfung: v0.5.169-APK aus dem Tag stimmt mit dessen SHA-256
  überein und enthält `libsmart_explorer_android.so` für arm64-v8a und x86_64.
  Das belegt Eingangsbytes/ABI, noch keinen ausgeführten Updateablauf.

## SFTP und FTP/FTPS

- [atmoz/sftp README](https://raw.githubusercontent.com/atmoz/sftp/master/README.md):
  Usersyntax `user:pass[:e][:uid[:gid[:dir1[,dir2]...]]]`; Homes sind chrooted,
  daher liegt die schreibbare Fixture in einem Unterverzeichnis. Öffentliche
  Schlüssel kommen nach `/home/user/.ssh/keys/`; der Entrypoint erstellt
  `authorized_keys`. Hostkeys dürfen explizit gemountet werden.
- [delfer README](https://raw.githubusercontent.com/delfer/docker-alpine-ftp-server/master/README.md):
  `USERS=name|password|folder|uid|gid`, `ADDRESS`, `MIN_PORT`, `MAX_PORT` sowie
  `TLS_CERT`/`TLS_KEY`. Die Fixture verwendet die vorhandene Imageklasse.
- [delfer Entrypoint](https://raw.githubusercontent.com/delfer/docker-alpine-ftp-server/master/start_vsftpd.sh):
  nach Benutzeranlage wird ein übergebener Command ausgeführt. Für zuverlässigen
  Containerbesitz wird vsftpd explizit im Vordergrund gestartet, wie im
  vorhandenen `android/test-servers/servers.sh`. TLS-Zertifikat-/Keyoptionen
  müssen dabei im übergebenen Command erhalten bleiben.
- [vsftpd-Konfigurationsreferenz](https://manpages.debian.org/testing/vsftpd/vsftpd.conf.5.en.html):
  `ssl_enable`, `force_local_logins_ssl`, `force_local_data_ssl`,
  `rsa_cert_file`, `rsa_private_key_file`, `pasv_min_port`, `pasv_max_port`
  und `pasv_address` konfigurieren die echte Gegenstelle. `require_ssl_reuse`
  ist eine eigenständige Datenkanalbedingung und darf nicht versehentlich
  als Zertifikatsprüfung interpretiert werden.

## Testzertifikat und unveränderte Produktprüfung

Aktueller FTP-Source verwendet `webpki_roots::TLS_SERVER_ROOTS`, WebDAV den
ureq-Rustls-Agenten. Ein Eintrag im Runner-OS-Truststore beweist hier keine
Clientvertrauensstellung. Die isolierte Fixture-CA muss ausschließlich unter
`cfg(test)` in einen echten Rustls-RootCertStore aufgenommen und dem normalen
Transport übergeben werden. Kein unsicherer Zertifikatsverifier, keine
Produkt-Umgebungsvariable und keine gelockerte Produktionsprüfung.

Für selbst signierte FTPS-/HTTPS-Fixtures bleibt deshalb eine explizite
testbezogene Trust-Injektion nötig; die übrige gespeicherte Verbindung,
Resolver-, Auth-, Pfad- und Sync-Engine-Grenze bleibt produktiv. Die Abnahme
unterscheidet diese Vertrauensinjektion vom regulären OAuth-/Drive-Aufruf C10.
CA-/Ports-/Authwerte werden von der Suite erzeugt oder durch Readiness-
Kommandos entdeckt. Sämtliche Server/PIDs gehören genau diesem Remote-Lauf
und werden auch beim Fehler geschlossen.
