# Android: gepinnten Share-Server mit leerem Rootpfad vergleichen

Geprüft am 2026-10-04 gegen die Primärdokumentation von
[Android Uri](https://developer.android.com/reference/android/net/Uri),
[Uri.Builder](https://developer.android.com/reference/android/net/Uri.Builder)
und [RFC 6455, Abschnitt 3](https://www.rfc-editor.org/rfc/rfc6455#section-3).

Der vierte RV1-Gerätelauf liefert denselben gepinnten Server mit einem `/`
zwischen Port und `#sha256=…`. Der bisherige Rohstring-Substringvergleich
scheitert daran, bevor die eigentliche Room-Aufnahme beginnt.

`Uri.parse(String)` erzeugt eine URI. `isHierarchical()` und
`getEncodedAuthority()` erlauben die notwendige hierarchische Serverform zu
prüfen. `getEncodedPath()` liefert den noch kodierten Pfad.
`buildUpon()` übernimmt die Attribute der vorhandenen URI;
`encodedPath("/")` setzt ausschließlich den kodierten Pfad und `build()`
erzeugt die URI mit diesen Attributen. Die Stringdarstellung kann dann exakt
verglichen werden. Es wird nichts dekodiert oder aus Fragment/Query entfernt.

RFC 6455 setzt den WebSocket-Resource-Pfad bei leerem Pfad auf `/`.
Fragmente gehören dort nicht in den Transport-URI. Das `#sha256=…` in dieser
Fixture ist der separate Zertifikatpin der Share-Konfiguration und muss im
Konfigurationsvergleich vollständig erhalten bleiben.

Der Fixturehelfer normalisiert deshalb nur einen leeren Rootpfad zu `/`.
Schema, kodierte Authority mit Host/Port, jeder nichtleere Pfad, Query und
Pinfragment bleiben unverändert. Die vorhandenen Join-, Member-, Listing-
und Download-/SHA-256-Assertions bleiben der eigentliche Abnahmenachweis.
