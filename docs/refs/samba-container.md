# dockurr/samba 4.23.10

Quelle: https://github.com/dockur/samba (Dockerfile, samba.sh, smb.conf, readme.md, master-Branch) · https://hub.docker.com/r/dockurr/samba/tags · https://github.com/dockur/samba/commits/master · https://github.com/dockur/samba/releases · https://github.com/dockur/samba/issues/69 · https://github.com/dockur/samba/issues/41 · Abgerufen: 2026-09-26

## Vergleich der 2026 gepflegten Images

| Image | Status 2026 | Konfiguration für 1 User + 1 Share |
|---|---|---|
| **dockurr/samba** (GitHub `dockur/samba`) | aktiv: Commits bis 2026-09-04, Releases bis `v4.23.10` (2026-08-22, Alpine `edge`) | genau `NAME`/`USER`/`PASS`/`RW`-Env-Vars, kein anderer Pflichtwert |
| `ghcr.io/servercontainers/samba` (`ServerContainers/samba`) | aktiv: Changelog-Eintrag 2026-07-11 | `ACCOUNT_<user>`- und `SAMBA_VOLUME_CONFIG_<name>`-Env-Vars; optionale wsdd2/Time-Machine-Layer brauchen `CAP_NET_ADMIN` – mehr Fläche als nötig |
| `crazymax/samba` (`crazy-max/docker-samba`) | aktiv (232 Commits) | Konfiguration über eine `/data/config.yml`-Datei statt Env-Vars, `--network host` empfohlen, kein einfacher Ein-User/Ein-Share-Weg über Env-Vars |
| `dperson/samba` | **unmaintained**: letztes Image-Push laut Docker-Hub-Tag-Liste >5 Jahre alt, Issues nennen es "full of security holes"; Community empfiehlt `ghcr.io/depau/docker-samba:main` als Drop-in-Ersatz (Quelle: https://github.com/dperson/samba/issues/459, https://github.com/depau/docker-samba) | – (nicht gewählt) |

Gewählt: **`dockurr/samba:4.23.10`** – einziges der vier Images, das einen einzelnen User + eine einzelne Freigabe rein über Env-Vars abbildet, ohne YAML/Compose-Datei oder Zusatz-Capabilities.

## Start command

```bash
docker run -d --name se-task-smb -p 445:445 \
  -e "NAME=share" -e "USER=seuser" -e "PASS=se-task-smb-pass" -e "RW=true" \
  -v "$PWD/smb-data:/shared" \
  dockurr/samba:4.23.10
```

Belege aus dem Dockerfile (`https://github.com/dockur/samba/blob/master/Dockerfile`):
- `ENV NAME="Shared"`, `ENV USER="samba"`, `ENV PASS="secret"`, `ENV RW="true"`, `ENV UID="auto"`, `ENV GID="auto"` – Defaults, hier durch die drei `-e`-Werte übersteuert.
- `VOLUME /shared`, `EXPOSE 139 445` – Freigabe-Pfad im Container ist `/shared` (legacy: `/storage`, falls dieses Verzeichnis existiert – `samba.sh`: `[ -d /storage ] && share="/storage"`).
- `ENTRYPOINT ["/sbin/tini", "--", "/usr/bin/samba.sh"]` – kein `--privileged` nötig, keine Doku/kein Issue verlangt erweiterte Capabilities für den reinen SMB-Betrieb.

Ownership/Homedir (aus `samba.sh`, Funktion `add_user` + Hauptlauf):
- `user_uid=$(resolve_id "${UID:-auto}" '%u' 1000)`, `user_gid=$(resolve_id "${GID:-auto}" '%g' 1000)` – bei `UID=auto`/`GID=auto` (Default) wird die UID/GID vom `/shared`-Verzeichnis per `stat` übernommen; ist das Verzeichnis neu/leer oder Owner `0`, fällt es auf `1000:1000` zurück. Bei einem frischen CI-Bind-Mount (`smb-data/` leer) landet `seuser` also auf UID/GID `1000:1000`.
- `homedir` des angelegten Users = `$share` (also `/shared`) – `add_user "$config" "$USER" "$user_uid" "$group" "$user_gid" "$PASS" "$share"`.
- `chmod 0770 "$share"` und – nur im Single-User-Modus ohne `users.conf` – `chown "$USER:$group" "$share"` (Gruppe fix `smb`), aber nur wenn `[ -z "$(ls -A "$share")" ]` (Verzeichnis leer/neu). Ein nicht-leeres Bind-Mount wird nicht umbesitzt.
- Da kein `users.conf` gebunden ist, setzt das Skript zusätzlich `force user = seuser` / `force group = smb` in die generierte `smb.conf` (`FORCE="Y"` Default) – SMB-Dateizugriffe laufen also unabhängig vom realen Host-UID immer als `seuser:smb`.

Protokoll/Sicherheit (aus `smb.conf`, `https://github.com/dockur/samba/blob/master/smb.conf`):
- `security = user`, `server min protocol = SMB2` – SMB1 ist deaktiviert, SMB2/3 ist erlaubt.
- Keine `server signing`/`client signing`/`smb encrypt`-Zeilen vorhanden → es gelten Sambas eingebaute Standardwerte, d.h. **Signing/Encryption sind nicht erzwungen** (Client kann unsigniert/unverschlüsselt verbinden).
- Kein `guest ok`, keine Anonymous-Freigabe im `[Shared]`-Stanza – Zugriff braucht die konfigurierten Zugangsdaten (`seuser`/`se-task-smb-pass`).
- Nur `smbd` wird gestartet (`exec smbd --configfile="$config" --foreground --debug-stdout -d "${DEBUG_LEVEL:-1}" --no-process-group`); **kein `nmbd`**, obwohl Port 139 exponiert ist. NetBIOS-Namensauflösung/-Browsing läuft nicht (README: "doesn't run a WS-Discovery service … server typically won't appear automatically under Network"). Für den Android-Emulator ist das irrelevant, da er ohnehin per IP (`10.0.2.2:445`) verbindet, nicht per NetBIOS-Name.

Erreichbarkeit vom Android-Emulator: `-p 445:445` bindet ohne explizite Host-IP auf `0.0.0.0` des Runners; der Emulator erreicht den Host-Loopback über `10.0.2.2`, also `\\10.0.2.2\share` bzw. SMB-Client-Verbindung zu `10.0.2.2:445`.

## Readiness

- Der Container selbst deklariert `HEALTHCHECK --interval=60s --timeout=15s CMD ["smbclient", "--configfile=/etc/samba.conf", "-L", "\\\\localhost", "-U", "%", "-m", "SMB3"]` (Dockerfile). Das ist eine **anonyme** (`-U %`) Share-Auflistung.
  - **Bekannter Fallstrick (Issue #69, https://github.com/dockur/samba/issues/69, gemeldet 2025-12-10):** Wer Anonymous-Zugriff explizit sperrt oder SMB3-Signing/Encryption zusätzlich erzwingt, bekommt `NT_STATUS_ACCESS_DENIED` von genau diesem Healthcheck-Aufruf und der Container zeigt dauerhaft `unhealthy`, obwohl die Freigabe normal funktioniert. Mit der hier gewählten Standardkonfiguration (kein zusätzliches Hardening, `security = user` ohne Anonymous-Sperre) sollte der eingebaute Healthcheck normal auf `healthy` laufen – aber verlässt man sich in der CI auf `docker inspect --format='{{.State.Health.Status}}'`, ist man von diesem Verhalten abhängig.
- **Empfehlung für die Actions-Runner-Suite:** ein reiner TCP-Connect-Probe auf Port 445, unabhängig vom eingebauten Healthcheck. Das ist mit dem Startskript verträglich: `samba.sh` führt vor dem `exec smbd`-Aufruf ausschließlich lokale, synchrone Setup-Schritte aus (Verzeichnisse anlegen, `smbpasswd`/`useradd`, `chmod`/`chown`) und öffnet keinen Port, bevor `smbd` läuft; danach übernimmt `smbd` als normaler Netzwerkdaemon die Verbindungsannahme. Ein bloßes Connect-und-wieder-Schließen (kein SMB-Handshake) ist damit nur "verbinde auf einen Port, an dem entweder noch nichts lauscht (Verbindung schlägt fehl → retry) oder `smbd` bereits lauscht (Verbindung gelingt)" – nichts im Skript oder in `smb.conf` reagiert speziell auf unvollständige/leere Verbindungen. Diese Einschätzung ist aus dem Skriptaufbau abgeleitet, nicht wörtlich in der Doku als "TCP-Probe ist sicher" belegt.
  ```bash
  for i in $(seq 1 30); do
    (exec 3<>/dev/tcp/127.0.0.1/445) 2>/dev/null && exec 3>&- 3<&- && break
    sleep 1
  done
  ```
- Zur Startdauer selbst macht keine der geprüften Quellen (Dockerfile, `samba.sh`, `readme.md`, Issues) eine explizite Zeitangabe. Die Vorbereitungsschritte in `samba.sh` sind rein lokale Alpine-Dateisystem-/`smbpasswd`-Operationen ohne Netzwerk-I/O; ein Polling-Timeout von ~30 s (wie oben) ist großzügig bemessen, ein belastbarer, dokumentierter Wert existiert nicht.

## Pitfalls

- **Healthcheck-Fallstrick** (Issue #69, s.o.): eingebauter `HEALTHCHECK` nutzt anonymen `smbclient -L`; bricht bei zusätzlichem Anonymous-/Signing-Hardening. Nicht als alleiniges CI-Readiness-Signal verwenden.
- **`msg.lock`-Rechte** (Issue #41, https://github.com/dockur/samba/issues/41, gemeldet 2025-02-12, laut Recherche-Stand ohne verlinkten Fix): `invalid permissions on directory '/var/cache/samba/msg.lock': has 0750 should be 0755`, sporadisch, gleiche Compose-Datei läuft auf anderen Docker-Hosts fehlerfrei. Das aktuelle `samba.sh` (Stand dieser Recherche) enthält bereits defensive Zeilen dagegen: `[ -d /run/samba/msg.lock ] && chmod -R 0755 /run/samba/msg.lock`, `[ -d /var/cache/samba/msg.lock ] && chmod -R 0755 /var/cache/samba/msg.lock` – diese greifen aber nur, **wenn das Verzeichnis zu diesem Zeitpunkt schon existiert**; bei einem komplett frischen Container legt `smbd` es ggf. erst danach selbst an, wodurch der Fallstrick weiter auftreten kann. Betrifft nur containerinterne Pfade (`/run`, `/var/cache`), nicht das gebundene `/shared`-Volume – kein Problem über CI-Runs hinweg, da bei `docker run` jedes Mal neu.
- **Kein `nmbd`/NetBIOS**, obwohl Port 139 exponiert ist – Browsing per Servername funktioniert nicht, nur direkte IP:Port-Verbindung (passt zum `10.0.2.2`-Zugriff vom Emulator).
- **`RW`/Read-only-Umschaltung** patcht `smb.conf` nur beim generierten Config (kein externer Bind-Mount von `smb.conf`); wird eine eigene `smb.conf` gebunden, ignoriert das Skript `RW`/`NAME` (`echo "Using provided configuration file: $config."`, kein weiteres Patchen).
- Kein Hinweis in Dockerfile/Skript/Issues darauf, dass der Start-Skript PIDs in einer gebundenen Datei ablegt oder `--privileged` braucht – anders als bei älteren/anderen Samba-Images (z. B. `dperson/samba`, dort mehrfach als Ursache für „stale PID"-Probleme bei Neustarts diskutiert, siehe Issue-Historie); für `dockurr/samba` wurde dieser Fallstrick in den geprüften Quellen nicht gefunden.
