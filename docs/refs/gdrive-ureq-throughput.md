# Google Drive API v3 & ureq 2.12.1 – Durchsatz-Referenz

Quelle: developers.google.com (Drive-API-v3-Referenz/-Guides, einzeln pro
Abschnitt verlinkt) · lokaler Crate-Quellcode `ureq-2.12.1`
(`.../index.crates.io-1949cf8c6b5b557f/ureq-2.12.1/src/{agent,pool,unit,
response,rtls,request}.rs`) · `native/Cargo.toml:35` (`ureq = { version =
"2", default-features = false, features = ["tls", "json"] }`, in
`Cargo.lock` auf `2.12.1` gepinnt). Abgerufen: 2026-09-28.

Kontext: baut auf `docs/lesungen/2026-09-28-gdrive-transfer-cost.md` auf
(hier nicht erneut gelesen außer zur Bestätigung einzelner Call-Shapes).
Reine Fakten-Referenz, keine Design-Entscheidung.

## 1. Upload-Typen (`uploadType`)

**multipart** (developers.google.com/workspace/drive/api/guides/manage-uploads):
`POST https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart`.
Body = `multipart/related` nach RFC 2387, genau zwei Teile: (1) Metadaten mit
`Content-Type: application/json; charset=UTF-8`, (2) Mediadaten mit eigenem
`Content-Type`; Request-Header `Content-Type: multipart/related;
boundary=<eigene boundary>` + `Content-Length`, Teile durch `--boundary`
getrennt, letzter durch `--boundary--` abgeschlossen. Empfohlene Obergrenze:
**"a small file (5 MB or less)"**.

**resumable**: `POST .../upload/drive/v3/files?uploadType=resumable`,
optional `X-Upload-Content-Type`/`X-Upload-Content-Length`. Antwort trägt
`Location`-Header (Session-URI, laut Doku **"expires after one week"**).
Chunks **"in multiples of 256 KB (256 x 1024 bytes)"**, außer dem letzten
(native `CHUNK_SIZE = 8 MiB` ist ein Vielfaches davon). Abschluss-Status:
`200`/`201` = fertig, `308 Resume Incomplete` = weiter (`Range`-Header,
committed offset), `404` = Session abgelaufen. Doku wörtlich: **"Resumable
uploads are also a good choice for most applications because they also work
for small files at a minimal cost of one additional HTTP request per
upload."** Kein oberes Größenlimit dokumentiert (Faustregel: größer 5 MB →
resumable statt multipart).

**Vorgenerierte IDs** (`files.generateIds`,
developers.google.com/drive/api/reference/rest/v3/files/generateIds):
`GET .../files/generateIds`, Query-Parameter `count` (Default 10, **Max.
1000**), `space` (`drive` Default | `appDataFolder`), `type` (`files` Default
| `shortcuts`). Antwort `{ids:[...], space, kind:"drive#generatedIds"}`.
Laut Referenz: **"Once an ID is generated, it can be passed to the create or
copy method through the id field"** – gilt für multipart, resumable *und*
`files.copy`. Einschränkung, wörtlich aus dem create-file-Guide
(developers.google.com/workspace/drive/api/guides/create-file): **"pre-
generated IDs aren't supported for the creation of Google Workspace files,
except for the `application/vnd.google-apps.drive-sdk` and
`application/vnd.google-apps.folder` MIME types."** D. h. normale
(Binär-)Dateien und Ordner erlaubt; native Workspace-Typen
(Docs/Sheets/Slides/Forms) nicht.

**Konflikt-Erkennung nach mehrdeutigem Fehlschlag**: derselbe Guide, wörtlich:
**"If the file is successfully created or copied, subsequent retries return
a `409 Conflict` HTTP status code response and duplicate files aren't
created."** Rezept: nach Timeout/5xx denselben Create-/Copy-Call mit
derselben vorgenerierten ID wiederholen – `409` heißt "existiert schon"
(Metadaten per `GET files/{id}` nachladen), `200/201` heißt "dieser Retry
hat es gerade angelegt". Einfacher als ein separates Verify-GET vor jedem
Retry.

## 2. Quotas und Limits

Zwei parallel gültige Modelle (developers.google.com/workspace/drive/api/guides/limits):
- **Bestandsprojekte** (vor 2026-05-01 angelegt): **20.000 Calls/100 s**,
  sowohl pro Nutzer als auch pro Projekt (Summe aus Lese- und Schreib-Calls).
- **Neue Projekte** (seit 2026-05-01): gewichtetes Quota-Unit-Modell – Projekt
  **1.000.000 Units/min**, pro Nutzer **325.000 Units/min**; Kosten:
  `files.get` 5 Units, Liste 100 Units, Edit (`files.update`) 50 Units,
  Download 200 Units, sonstige Aktionen 5 Units. Tagesschwellen: **400 Mio.
  Units/24 h** und **1 TB Egress/Tag** bevor Mehrkosten greifen.
- Upload: **750 GB/Tag** Obergrenze (My Drive + geteilte Ablagen zusammen),
  **5 TB** Maximalgröße pro Datei.
- **"Sustained write rate"** (knowledge.workspace.google.com, Google-Drive-
  Migrations-Best-Practices), wörtlich: **"avoid exceeding 3 requests per
  second of sustained write or insert requests, per account. This rate limit
  can't be increased."** Einzige dort genannte Abhilfe (Schreiblast über
  mehrere per Domain-Wide-Delegation impersonierte Nutzer verteilen) passt
  nicht zu diesem Consumer-OAuth-Backend; die 3-req/s-Zahl selbst bleibt aber
  die belastbare Zielgröße.

**Fehlercodes** (developers.google.com/workspace/drive/api/guides/handle-errors):
`403 rateLimitExceeded` ("project's rate limit … reached"),
`403 userRateLimitExceeded` ("per-user limit … reached"),
`403 dailyLimitExceeded`, `403 sharingRateLimitExceeded`,
`429 rateLimitExceeded` ("too many requests in a given amount of time"),
`500/502/503/504` (Serverfehler). Die Fehlerseite selbst dokumentiert keine
Backoff-Formel, sondern verweist auf den Limits-Guide, dort
(`.../guides/limits#exponential`) exakt: **`wait =
min(((2^n) + random_number_milliseconds), maximum_backoff)`**, `n` beginnt
bei 0 und wird je Retry inkrementiert, `random_number_milliseconds` ist ein
bei **jedem** Retry neu gewürfelter Jitter-Wert `≤ 1000 ms`,
`maximum_backoff` laut Doku **typischerweise 32–64 s**; danach im
Cap-Intervall weiterversuchen, irgendwann aufgeben (keine feste Retry-Zahl
dokumentiert). Rein faktisch zum Vergleich: der native Code nutzt
`RETRY_ATTEMPTS = 6`, Basis 400 ms verdoppelnd, Cap 16 s, ohne Jitter
(`api.rs:9-11`), sowie beim Resumable-PUT `MAX_RETRIES = 6`, Delay
`min(250ms<<n, 8s)`, ebenfalls ohne Jitter (`resumable.rs`).

## 3. Batch-Requests (`/batch/drive/v3`)

Der **API-spezifische** Batch-Endpunkt lebt weiter; nur der **globale**,
API-übergreifende Endpunkt `www.googleapis.com/batch` wurde am 2020-08-12
abgeschaltet (developers.googleblog.com/discontinuing-support-for-json-rpc-
and-global-http-batch-endpoints/), wörtlich: **"Non-Global HTTP Batch
endpoints that include the API name in the URI will continue to be
supported. Examples includes: `https://www.googleapis.com/batch/compute/v1`"**
– `https://www.googleapis.com/batch/drive/v3` fällt explizit darunter.

Format (developers.google.com/drive/api/v3/batch): eine HTTP-POST-Anfrage,
`multipart/mixed`, jeder Teil trägt `Content-Type: application/http` und
kapselt eine vollständige innere HTTP-Anfrage. Grenzen: **"limited to 100
calls in a single batch request"**, **"8,000 character limit on the length
of the URL for each inner request."** Medien-Ausschluss, wörtlich: **"Google
Drive doesn't support batch operations for media, either for upload or
download, or for exporting files."** Ob ein Call im Batch quota-technisch
günstiger ist als einzeln, ist nicht dokumentiert – belegt ist nur weniger
HTTP-Roundtrips, kein Unit-Rabatt.

Empfohlene Anwendungsfälle, wörtliche Liste: **"Retrieving metadata for a
large number of files."** / **"Updating metadata or properties in bulk."** /
**"Changing permissions for a large number of files…"** / **"Synchronizing
local client data for the first time or after being offline for an extended
time."** – Letzteres trifft exakt das Erst-Sync-Szenario der
Aufgabenstellung. Ordner-Erstellung (`files.create` ohne Media) und
Umbenennen (`files.update`/PATCH ohne Media) sind reine Metadaten-Calls,
fallen also unter "Updating metadata … in bulk" – batchbar. Hoch-/
Herunterladen von Dateiinhalt und Resumable-PUT-Chunks sind Medien und laut
obigem Zitat ausdrücklich **nicht** batchbar.

## 4. `files.list`

**`pageSize`** (developers.google.com/drive/api/reference/rest/v3/files/list),
Beschreibung wörtlich: **"The maximum number of files to return. The service
may return fewer than this value. If unspecified, at most 100 files will be
returned for shared drives, and the entire list of files for non-shared
drives. The maximum value is 1000; values above 1000 will be coerced to
1000."**

**`fields`** (developers.google.com/workspace/drive/api/guides/fields-parameter):
Default ohne `fields`-Parameter bei `files.list`: nur **`kind`, `id`,
`name`, `mimeType`**. Sub-Selektor-Syntax mit Klammern, z. B.
`fields=nextPageToken,files(id,name,createdTime,modifiedTime,size)`;
Wildcard `*` verfügbar, Doku warnt aber: **"using the wildcard can lead to
negative performance impacts on the request."** Der bereits im Backend
verwendete Shape `fields=...files(id,name,mimeType,size,modifiedTime,
createdTime,md5Checksum)` folgt exakt dieser Syntax.

**`q` für mehrere Elternordner auf einmal**
(developers.google.com/workspace/drive/api/guides/search-files): dokumentiert
ist die Einzel-Klausel `'<id>' in parents` sowie generisches Verknüpfen mit
`or`/`and`; ein wörtliches Beispiel, das mehrere Parent-IDs mit `or`
kombiniert (`'id1' in parents or 'id2' in parents`), liefert die Doku selbst
**nicht** – Syntax ist aus den Einzelbausteinen ableitbar, aber nicht
gegengeprüft. **Kein** dokumentiertes Limit für `q`-Stringlänge oder Anzahl
`or`-Klauseln gefunden (offener Punkt, s. u.); als einzige benachbarte,
tatsächlich dokumentierte Zahl gibt es die 8.000-Zeichen-Grenze pro innerer
Batch-Request-URL (Abschnitt 3) – nicht dasselbe Limit, aber die einzige
offiziell belegte Größenordnung in der Nähe.

**Zuordnung der Treffer zum jeweiligen Parent**: Das Files-Resource-Feld
`parents` (developers.google.com/drive/api/reference/rest/v3/files) ist Teil
jeder zurückgegebenen Datei, wörtlich: **"A file can only have one parent
folder; specifying multiple parents isn't supported."** Ein Treffer aus
einer Multi-Parent-`or`-Query lässt sich also über sein eigenes `parents[0]`
eindeutig zuordnen, auch ohne dass die Antwort das getroffene `q`-Glied
selbst benennt.

## 5. `files.create` (Ordner) und `files.copy`

**Ordner mit vorgenerierter ID**: `POST https://www.googleapis.com/drive/v3/files`
mit Body `{name, mimeType:"application/vnd.google-apps.folder",
parents:[parentId], id:<vorgeneriert>}` – laut §1 für
`application/vnd.google-apps.folder` ausdrücklich von der sonstigen
Workspace-Einschränkung ausgenommen, also erlaubt. `parents`, wörtlich:
**"If not specified as part of a create request, the file is placed
directly in the user's My Drive folder."** Drive v3 kennt nur genau einen
Parent pro Datei/Ordner, das native Ein-ID-pro-Pfad-Modell passt dazu.

**`files.copy`** (developers.google.com/drive/api/reference/rest/v3/files/copy):
`POST https://www.googleapis.com/drive/v3/files/{fileId}/copy`. Serverseitige
Kopie – laut Referenz **"creates a copy of a file and applies any requested
updates with patch semantics"**; der Client lädt keine Bytes hoch, Antwort
ist die vollständige `File`-Resource der Kopie. Die Copy-Referenzseite
selbst nennt `id`/`parents` im Body nicht explizit, aber die
`generateIds`-Referenz sagt ausdrücklich, IDs seien **"passed to the create
or copy method through the id field"** – `copy` unterstützt also dieselbe
vorgenerierte-ID-Mechanik wie `create`; `parents` ist ein normales
beschreibbares `File`-Feld, wirkt auf `copy` genauso platzierend (ein
Parent, s. o.). Fachlich relevant: `files.copy` ist die tatsächliche
serverseitige Kopieroperation, laut `2026-09-28-gdrive-transfer-cost.md` §7
im Backend aktuell **nicht** aufgerufen (dort nur clientseitiges
Re-Upload+Rename als "copy"-Ersatz).

## 6. HTTP Keep-Alive/HTTP2 bei googleapis.com

Google-eigene Cloud-API-HTTP-Guidelines (docs.cloud.google.com/apis/docs/http),
wörtlich: **"Supported protocols include HTTP/1.1, HTTP/2, and HTTP/3
(QUIC)."** Für HTTP/1.1 ausdrücklich: **"always reuse TCP connections
(Connection: Keep-Alive)."** GFE-artige Proxys **"can still enforce limits
on the maximum number of concurrent streams within a single HTTP/2 or
HTTP/3 connection … (for example, by limiting concurrent requests or
streams per connection to 100)"**; mit H2/H3 seien **"browser limits on the
number of parallel TCP connections to a single host (for example,
2-10)"** nicht mehr die primäre Grenze – reiner Browser-Kontext, keine
explizite Drive-API-Zahl für beliebige Clients. Keine Drive-spezifische
"maximal N gleichzeitige Verbindungen"-Empfehlung gefunden.

**Abgleich mit ureq**: ureq spricht ausschließlich HTTP/1.1 – die
Request-Zeile wird hart codiert geschrieben (`" HTTP/1.1\r\n"`,
`unit.rs:511`), im gesamten `src/`-Baum gibt es **keinen** Treffer für
`alpn`/`h2`/`HTTP/2` (grep-bestätigt), und das per Default gebaute
rustls-`ClientConfig` (`rtls.rs:119-136`) setzt keine ALPN-Protokollliste.
D. h. unabhängig davon, dass die Google-Frontends HTTP/2 könnten: diese
Crate-Version spricht mit `googleapis.com` immer klassisches
HTTP/1.1-Keep-Alive, kein Multiplexing. Echte Parallelität = Anzahl
gleichzeitig offener TCP/TLS-Verbindungen, nicht Streams auf einer
Verbindung.

## 7. `Agent`/`AgentBuilder`

Defaults aus `agent.rs` (Konstruktor `AgentBuilder::new()`, Zeilen 251-278):

| Option | Default | Fundstelle |
|---|---|---|
| `max_idle_connections` | **100** (`DEFAULT_MAX_IDLE_CONNECTIONS`) | `agent.rs:248,271` |
| `max_idle_connections_per_host` | **1** (`DEFAULT_MAX_IDLE_CONNECTIONS_PER_HOST`) | `agent.rs:249,272` |
| `timeout_connect` | `Some(30 s)` | `agent.rs:256` |
| `timeout_read` | `None` (blockiert unbegrenzt) | `agent.rs:257` |
| `timeout_write` | `None` (blockiert unbegrenzt) | `agent.rs:258` |
| `timeout` (Gesamtrequest) | `None` | `agent.rs:259` |
| `redirects` | `5` | `agent.rs:262` |
| `redirect_auth_headers` | `Never` | `agent.rs:263` |
| `no_delay` (`TCP_NODELAY`) | `true` | `agent.rs:261` |
| `tls_config` | `default_tls_config()` | `agent.rs:265` |

Feinheiten: `max_idle_connections_per_host`-Setter, wörtlich: **"By default,
this is set to 1. Setting this to zero would disable connection pooling."**
(`agent.rs:367-379`) – **pro Host wird standardmäßig nur eine einzige
Idle-Verbindung** vorgehalten, unabhängig vom globalen Cap von 100.
`timeout()` **"takes precedence over `.timeout_read()` and
`.timeout_write()`, but not `.timeout_connect()`"** (`agent.rs:471-493`).
`redirects()`-Warnung, wörtlich: **"for 307 and 308 redirects, this value is
ignored for methods that have a body. You must handle 307 redirects
yourself when sending a PUT, POST, PATCH, or DELETE request."**
(`agent.rs:515-546`) – erklärt, warum das Backend laut Lesung `redirects(0)`
setzt und den `Location`-Header manuell verfolgt.

**TLS-Default**: `rtls::default_tls_config()` (`rtls.rs:119-136`) baut
einmalig pro Prozess (`once_cell::sync::Lazy`) ein rustls-`ClientConfig` mit
`rustls::crypto::ring::default_provider()`, TLS 1.2 **und** 1.3, ohne
Client-Auth; Root-Store per Default `webpki_roots::TLS_SERVER_ROOTS`
(gebündelte Mozilla-Roots), **nicht** der OS-Truststore – Letzteres bräuchte
das hier laut `native/Cargo.toml:35` nicht aktivierte Feature
`native-certs` (`rtls.rs:62-86`). Überschreibbar per
`AgentBuilder::tls_config`/`tls_connector` (`agent.rs:590-639`).

**Nebenläufigkeit**: `Agent` ist `#[derive(Clone)]` über zwei `Arc`-Felder
(`Arc<AgentConfig>`, `Arc<AgentState>`, `agent.rs:111-116`); Doku-Kommentar
wörtlich: **"Agent uses an inner Arc, so cloning an Agent results in an
instance that shares the same underlying connection pool and other
state."** (`agent.rs:109-110`). Der Pool kapselt seinen Zustand hinter
`Mutex<Inner>` (`pool.rs:36-40,101-181`); Crate-Test
`agent_implements_send_and_sync` (`agent.rs:716-720`) verlangt `Agent: Send`
**und** `Agent: Sync`. ⇒ **ein geteilter `Agent` (oder Clones davon) kann
gefahrlos aus vielen Threads gleichzeitig genutzt werden** – Voraussetzung,
um vom Pool zu profitieren: jeder freistehende Aufruf
`ureq::get/post/request(...)` baut intern einen eigenen Einweg-Agent (kein
geteilter Pool), genau das Muster, das die Lesung für
`auth.rs`/`metadata.rs`/`promotion_api.rs`/`trash.rs` als nicht-poolend
beschreibt. Pro-Request-Deadline-Override: `Request::timeout(Duration)`
(`request.rs:60`).

## 8. Retry-Semantik bei wiederverwendeten Verbindungen (`unit.rs`)

`Unit::is_retryable(&self, body: &SizedReader) -> bool` (`unit.rs:132-152`):
idempotente Methoden sind exakt `DELETE | GET | HEAD | OPTIONS | PUT |
TRACE` – **`POST` und `PATCH` stehen nicht in dieser Liste und werden daher
nie automatisch wiederholt**, unabhängig vom Body. Zusätzlich muss der Body
`BodySize::Known(0)` oder `BodySize::Empty` sein; jede bekannte Länge `> 0`
(`BodySize::Known(_)`) oder ein ungrößter/gestreamter Body
(`BodySize::Unknown`) ergibt `retryable_body = false`. Damit gilt: selbst
`PUT` – obwohl in der idempotenten Liste – wird nur bei leerem Body
wiederholt; **ein `PUT`-Chunk mit echten Nutzdaten wird von ureq selbst nie
automatisch erneut gesendet.**

Zwei Retry-Pfade in `connect_inner` (`unit.rs:242-319`), beide auf einen
Zusatzversuch begrenzt (Rekursion übergibt beim zweiten Mal
`use_pooled=false`): (1) **vor dem Senden** (`unit.rs:266-277`) – schlägt
`send_prelude` auf einer wiederverwendeten Verbindung fehl, wird
bedingungslos einmal auf einer frischen Verbindung neu versucht (noch keine
Body-Bytes geschrieben, also unproblematisch); (2) **nach dem Senden**
(`unit.rs:279-309`) – `retryable = unit.is_retryable(&body)` wird vor dem
Senden berechnet; schlägt danach das Lesen der Antwort mit
`err.connection_closed() && retryable && is_recycled` fehl, wird einmal neu
versucht, aber mit `Payload::Empty` statt des Original-Bodys (konsistent
dazu, dass `retryable` ohnehin nur bei leerem Body wahr ist). Kommentar
zitiert RFC 7230 §6.3.1 und merkt an: **"we may do up to N+1 total tries,
where N is max_idle_connections_per_host."**

Fazit: keiner der realen Netzwerk-Bodies des Backends (JSON-POST/PATCH,
Resumable-PUT-Chunks mit echten Bytes) kann durch ureqs eigene
Connection-Recycling-Logik unsichtbar doppelt gesendet werden – das
Backend-Muster "mehrdeutig ⇒ per GET verifizieren" schließt also eine Lücke,
die auf HTTP-Client-Ebene sonst offen bliebe, statt sie zu duplizieren.

## 9. Response-Body-Konsum und Rückgabe an den Pool

Doku-Kommentar auf `struct Response` (`response.rs:55-65`), wörtlich:
**"Note that the socket connection is open and the body not read until one
of `into_reader()`, `into_json()`, or `into_string()` consumes the
response. When dropping a `Response` instance, one of two things can
happen. If the response has unread bytes, the underlying socket cannot be
reused, and the connection is closed. If there are no unread bytes, the
connection is returned to the `Agent` connection pool used."**

Mechanik in `stream_to_reader` (`response.rs:352-420`) – Rückgabe-Trigger ist
immer ein **0-Byte-`read()`**: `Transfer-Encoding: chunked` →
`PoolReturnRead<ChunkDecoder<..>>` (`pool.rs:261-307`) ruft
`Stream::return_to_pool()` sobald der innere Reader `0` liefert
(Chunk-Terminator); `Content-Length: 0` → Stream geht **sofort synchron**
zurück, kein Wrapper nötig (`response.rs:383-390`); `Content-Length: N>0` →
`LimitedRead` gibt nach exakt `N` gelesenen Bytes automatisch zurück
(bestätigt durch `pool.rs`s `read_exact`-Test, direkt danach
`agent.state.pool.len()==1`) bzw. sofort synchron, wenn der Body schon
komplett im Header-Lesepuffer lag; kein `Content-Length` und kein Chunking
(`CloseDelimited`, Body endet erst mit Socket-Close) → roher `Stream`
**ohne** Pool-Wrapper, strukturell nie poolbar; `Connection: close` (oder
HTTP/1.0 ohne `keep-alive`) → `set_unpoolable()` vorab (`response.rs:361`),
verworfen selbst bei vollständigem Lesen.

`Stream::drop` (`stream.rs:320-324`) ist **nur** ein Debug-Log, **kein**
Aufruf von `return_to_pool()`. Wird eine `Response`/ein Reader vor Erreichen
dieses 0-Byte-Reads verworfen (Abbruch eines Downloads, teilweises Lesen
eines großen `alt=media`-Bodys), schließt das den TCP/TLS-Socket statt ihn
zu poolen – der oben zitierte "unread bytes ⇒ closed"-Zweig, hier bis zum
Code zurückverfolgt. `into_string()` (`response.rs:456-481`, Limit 10 MiB)
und `into_json()` (`response.rs:531-547`, Feature `json`,
`serde_json::from_reader(...)`) treiben den Reader bis `Ok(0)` und geben die
Verbindung korrekt zurück, sofern der Aufrufer sie nicht vorher fallen lässt.

## Offene Punkte

- Kein dokumentiertes Limit für `q`-Stringlänge/Anzahl `or`-Klauseln bei
  `in parents` (§4); einzige benachbarte Zahl ist die 8.000-Zeichen-Grenze
  pro innerer Batch-Request-URL, nicht erwiesenermaßen dasselbe Limit.
- Batch-Rabatt auf Quota-Units nicht dokumentiert (§3); `files.copy`s
  Referenzseite nennt `id`/`parents` im Body nicht explizit, nur aus
  `generateIds`-Referenz + allgemeinem `File`-Schema abgeleitet (§5).
- Kein Drive-/googleapis.com-spezifisches "maximal N Verbindungen"-Guidance
  gefunden, nur allgemeine Cloud-API-HTTP-Guidelines (§6).
