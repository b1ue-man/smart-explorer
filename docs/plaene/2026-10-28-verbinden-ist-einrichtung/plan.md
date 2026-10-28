# Verbinden ist die Einrichtung; Tastatur am Linux-Anmeldebildschirm; schnelles Kopieren

Stand: 2026-10-28. Ausgangspunkt: veröffentlichtes v0.5.188, Quellstand
`23667a53`. Dieser Plan enthält keine lokalen Geräte-, Konto- oder Dienstdaten.

## A — Auftrag (Nutzernachrichten 2026-10-28)

| Id | Nutzeraussage (gekürzt, wörtlich im Aufgabenstand) | Ergebnis |
|---|---|---|
| r1 | Tastatur aus dem Stream wirkt auf dem Linux-Sperr-/Anmeldebildschirm nicht (Betrachter Windows und Handy), Maus schon; „nicht mal Enter“ | Tasten erreichen das Passwortfeld des Greeters |
| r2, r6 | Kopieren/Einfügen kleiner Dateien lastet CPU, RAM, Laufwerk und Netz nicht aus; „MUSS verbessert werden, massiv … gescheite Architektur, Bottlenecks ermitteln, reduzieren, parallelisieren, beschleunigen“ | gemessene Engpässe, Architektur, deutlich höhere Dateien/s |
| r3 | „bildschirm freigabe heißt bildschirm freigabe … die eine freigabe muss reichen, der rest muss laufen“ (RDP ohne Menüs, automatisch bei Migration und wenn sich Geräte online sehen) | RDP und Bildschirmbezogenes ohne weitere Option |
| r4 | „der nutzer soll so wenig wie möglich einrichten müssen, er verbindet zwei geräte … tcp, alles“ | Grundsatz für jede Funktion |
| r5 | „wenn n fenster offen ist sehen es andere se desktops, egal ob lokal der se desktop an ist … wenn auf localhost was offen ist wird das frei gegeben für andere geräte. per default.“ | Fenster und localhost-Dienste ohne Einrichtung |

Eigene Folgerungen (Annahmen, nicht Nutzeraussage):
- r1: „handy streamingh windows“ nennt das Handy als Betrachter eines Windows-Hosts; ob dort der Sperrbildschirm gemeint ist, sagt die Nachricht nicht.
- r3: „beim migrieren“ meint die Übernahme bestehender Freigaben in eine neue Version bzw. ein migriertes Gerät.
- r5: „localhost“ meint lokal lauschende TCP-Dienste (127.0.0.1/::1).

Grenzen: keine lokalen Builds/Tests; eine Remote-Suite; ein Release am Ende.

## B — Bestand und Befunde

### r1 Tastatur am Linux-Greeter (Ursache belegt)

- Eingabeweg vollständig: Viewer sendet `Key`/`Text`, wenn der Host `keyboard`/`text`
  anbietet (`app/core/screen_input_map.rs`); der X11-Host bietet beides an
  (`screen/os/linux_os/host_backend.rs`, `input_for`) und injiziert per XTest
  (`x11_input.rs`, evdev+8 bei evdev/pc105-Keymap). Kein Filter verwirft Tasten.
- Laufzeitbeleg am betroffenen X-Server (LightDM, unity-greeter 24.04.1, kein
  Fenstermanager: `_NET_SUPPORTING_WM_CHECK` fehlt): `XGetInputFocus` nannte das
  se-Anzeigefenster „Bildschirmfreigabe – Smart Explorer“ (`--screen-indicator`).
  Alle Tasten gingen an dieses Fenster; Mausklicks treffen das Fenster unter dem Zeiger.
- Ursache im Greeter-Quelltext: `unity-greeter-24.04.1/src/unity-greeter.vala`
  `focus_upon_map` (Z. 363–409) fokussiert jedes neu gemappte Fenster außer
  override-redirect, der eigenen Bildschirmtastatur und Fenstern mit Typ
  NOTIFICATION. lightdm-gtk-greeter (`src/lightdm-gtk-greeter.c`,
  `wm_window_filter`) fokussiert jedes Fenster außer COMBO, TOOLTIP, NOTIFICATION.
- Gegenprobe am selben Greeter: normales Testfenster und Fenster mit
  `WM_HINTS.input=False` bekommen den Fokus; ein Fenster mit
  `_NET_WM_WINDOW_TYPE_NOTIFICATION` nicht (Fokus bleibt beim Greeter).
- Mittel vorhanden: `egui::ViewportBuilder::with_window_type` (egui 0.29.1,
  `viewport.rs:607`) → egui-winit (Feature `x11` in `native/Cargo.toml` aktiv)
  → winit 0.30.13 `with_x11_window_type`, gesetzt vor dem Mappen.

### r2/r6 Kopieren kleiner Dateien

- Messung (installiertes 0.5.188, Linux, 4 Kerne): `se cp -r` lokal→lokal
  5000 × 4 KiB in 0,85 s (≈ 6000 Dateien/s, Urteil „limited by: none“);
  `cp -r` 0,27 s. Lokal→lokal ist nicht der Hauptengpass.
- Pfade: Einfügen in se (`app/core/transfer_route.rs` → Transfer-Engine),
  se → andere Programme (`transfer/os/shared/clipboard_parallel.rs`,
  `app/os/shared/clipboard_*.rs`), Drag & Drop, Android, `se cp`.
- Engine: Streaming mit adaptiver Nebenläufigkeit (`transfer/core/flow_control.rs`),
  Pakete kleiner Dateien für Share-Hosts/SSH-Agent (`engine/batch.rs`,
  ≈ 250 ms je Paket), Engpassurteil je Sekunde (`engine/bottleneck.rs`).
- Offen (Analyse läuft): Ablauf je Datei je Pfad, Bedingungen ohne Pakete,
  Hostseite je Paket, Ordneranlage, Journal-/Belegschreibungen.

### r3/r4/r5 Selbsteinrichtung

- `rdp/README.md`: der RDP-Adapter „konfiguriert weder RDP-Dienste noch NLA,
  TLS, Firewall oder Zugangsdaten“; das Formular `app/core/share_rdp_form.rs`
  verlangt Dienst, Client, App, Lebensdauer und Bestätigungskästchen.
- `docs/TODO.md` SELF-SETUP: erster Schritt geliefert (xrdp-autorun,
  Steuerung schließt Rechte ein); Rest offen.
- Offen (Inventur läuft): jede verbleibende manuelle Wahl auf jeder Oberfläche,
  TCP-Freigabe heute, Fensterliste ohne laufenden se Desktop, localhost-Dienste.

## C — Entscheidungen

### r1
- Gewählt: Das Anzeigefenster erhält unter X11 ohne Fenstermanager den
  Fenstertyp NOTIFICATION. Grund: genau diese Umgebung (Greeter mit eigener
  Fokuspolitik) ist der Fehlerort; beide verbreiteten LightDM-Greeter nehmen
  NOTIFICATION ausdrücklich aus; belegt durch Gegenprobe.
- Mit Fenstermanager bleibt das Fenster normal (Titelleiste, Verschieben,
  Schließen-Knopf „Schließen dieses Fensters beendet alle Sitzungen“).
- Verworfen: `WM_HINTS.input=False` (vom Greeter ignoriert, Gegenprobe);
  override-redirect (nur unity-greeter nimmt es aus, nicht lightdm-gtk-greeter;
  verliert auf Desktops die Fensterverwaltung); NOTIFICATION immer (Fenstermanager
  wie KWin/xfwm4 entfernen Dekoration und Schließen-Knopf).
- Zusätzlich `with_active(false)`: unter Windows erscheint das Fenster ohne
  Aktivierung (winit `with_active`, unter X11/Wayland ohne Wirkung), damit
  eine gesteuerte Windows-Anwendung den Tastaturfokus behält.

### r2/r6, r3–r5
Offen bis Analyse und Inventur vorliegen (Schleifenstand unten).

## D — Arbeitsplan

| # | Schritt | Dateien | Prüfbares Ergebnis |
|---|---|---|---|
| D1 | Anzeigefenster ohne Fokus am Greeter | `app/core/screen_indicator_ui.rs`, `app/os/linux_os.rs`, `app/os/windows.rs` | Suite: Xvfb ohne WM, Fenster mit Fokus gemappt, danach Indikator: Fokus unverändert, `_NET_WM_WINDOW_TYPE` = NOTIFICATION; mit WM-Check-Fenster: Typ unverändert |
| D2 | Kopierarchitektur | folgt aus Analyse | Suite-Messung Dateien/s vorher/nachher je Pfad |
| D3 | Selbsteinrichtung RDP/TCP/Fenster | folgt aus Inventur | je Oberfläche kein Formular mehr nötig |

## Schleifenstand (kreative Schleife)

- Ziel: r3–r6 — Lösungsweg für (a) schnelles Kopieren, (b) Einrichtung allein durch Verbinden.
- Kriterien: (a) gemessene Dateien/s je Pfad steigen deutlich, Invarianten
  (Absturzfestigkeit, kein Überschreiben, Behalten-beider, Abbruch, Wiederaufnahme)
  bleiben; (b) nach Kopplung und Bildschirmfreigabe ist für RDP, Fenster,
  Programme und TCP keine weitere Wahl nötig.
- Möglichkeiten: offen bis Befunde vorliegen.
- Nächster Schritt: Ergebnisse der Codeanalyse (Kopierpfade) und der Inventur
  (manuelle Schritte) auswerten; danach Ansätze bilden.
- Status: aktiv.
