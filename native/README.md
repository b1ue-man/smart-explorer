# Smart Explorer — Native (Rust + egui)

Schlanke, schnelle native Variante. Die GUI selbst ist ein einzelnes natives
Binary — kein Chromium, Browser oder Node. Sie ist weiterhin portabel startbar;
die reguläre per-user Installation ergänzt den verifizierten Update-Helfer und
den `se`-Terminal-Companion. Updates werden geprüft gestagt und erst nach
expliziter Bestätigung transaktional angewendet.

## Größe & Geschwindigkeit (im Vergleich zur Electron-Variante)

| Metrik | Electron-Version | Native-Version |
|---|---|---|
| Distribution | 79 MB Installer | **~7.7 MB Installer / ~22 MB EXE** |
| Entpackt | ~280 MB | **Native EXE + Updater-Helfer + `se` Companion** |
| Prozesse beim Start | 4 (main+renderer+gpu+util) | **1** |
| Scan `node_modules` (~12k Dateien) | 8.6k/s | **76k/s** |
| Scan `Program Files` (~514k Dateien, 89 GB) | 61.7s | **1.85s (33×)** |

Erreicht durch:
- `std::fs::read_dir` auf Windows nutzt `FindFirstFileW` und liefert alle Metadaten
  in einem Syscall pro Eintrag (statt readdir + stat)
- Rayon-basierte parallele Verzeichnis-Walker (Work-Stealing über alle Cores)
- Channels (crossbeam) streamen Resultate batchweise (1024er-Pakete oder 60ms)
- LTO + strip im Release-Build, panic = abort, codegen-units = 1

## Build

Voraussetzungen:
- Rust GNU-Toolchain (`rustup target add x86_64-pc-windows-gnu` mit `rustup install stable-gnu`)
- Strawberry Perl GCC oder MinGW-w64 als Linker (rustup-Bundle reicht)

```bash
cargo build --release
# → target/release/smart_explorer.exe und target/release/se.exe
```

Release-Artefakte werden nicht per Hand kopiert. Der aktuelle lokale
Release-Flow steht in [`../docs/RELEASING.md`](../docs/RELEASING.md); auf einem
Windows-Rechner ist `..\native\publish-release-local.ps1` der Standard, weil der
Wrapper Windows- und Linux-Feed-Payloads inklusive `se` gemeinsam in einem
isolierten Baum baut, prüft und erst danach rollback-geschützt veröffentlicht.
Die vollständigen Windows- und Linux-Pfade teilen
`release-native/.complete-release.lock` und brechen bei einem zweiten Lauf vor
dem Build ab. Ein fehlgeschlagener vollständiger Lauf behält seinen Stage und
meldet dessen Pfad; dieser ist Diagnosematerial, kein automatisch resumierbarer
oder manuell zu promotender Release-Kandidat.

Bench-Mode:
```bash
cargo build --release --bin bench
target/release/bench.exe "C:/Program Files"
```

## Stack

- `eframe 0.29` + `egui_extras` — immediate-mode GUI, virtualisierte Tabelle
- `rayon` — parallele Walker
- `crossbeam-channel` — Lock-free Channels für Stream-Updates
- `regex`, `globset` — Filter-Muster
- `chrono` — Datumshandling
- `rfd` — natives File-Dialog (Win32 Common Controls)
- `trash` — Papierkorb
- `windows-sys` — `GetLogicalDrives` für Laufwerksliste

## Limitierungen vs. Electron-Variante

- **Web-/Electron-Ökosystem:** bewusst nicht enthalten; Erweiterungen müssen in
  Rust/native umgesetzt werden.
- **Windows 11 modernes Kontextmenü:** COM-DLL und Sparse-Package-Manifest sind
  gebaut, aber die Aktivierung braucht ein vertrauenswürdiges Codesigning-Zertifikat
  (siehe [`../docs/WIN11_CONTEXT_MENU.md`](../docs/WIN11_CONTEXT_MENU.md)).
- **NTFS-MFT-Scan:** als spätere, erhöhte Windows-Option geplant; der normale
  parallele Walker bleibt der universelle Pfad.

## Zur weiteren Beschleunigung Richtung WizTree

Die schnelle Standardanalyse nutzt den parallelen Walker und die eigene
Storage-Analytics-Pipeline. WizTree liegt bei 1-3 Mio/s durch direktes
NTFS-MFT-Lesen. Mögliche Ergänzungen für Folge-Versionen:

1. `FindFirstFileExW` mit `FIND_FIRST_EX_LARGE_FETCH` und `FindExInfoBasic`
   (überspringt 8.3-Aliase, batcht Kernel-Calls) → ~2× zusätzlich
2. NTFS-MFT-Reader via `\\.\C:` mit `FSCTL_ENUM_USN_DATA` (braucht Admin) →
   1-3 Mio/s, ähnlich WizTree
