# Smart Explorer — Native (Rust + egui)

Schlanke, schnelle native Variante. Single-EXE, kein Chromium, kein Browser,
kein Node, kein Installer nötig. Einfach starten.

## Größe & Geschwindigkeit (im Vergleich zur Electron-Variante)

| Metrik | Electron-Version | Native-Version |
|---|---|---|
| Distribution | 79 MB Installer | **8.5 MB EXE** |
| Entpackt | ~280 MB | – (ist die EXE) |
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
# → target/release/smart_explorer.exe
```

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

- **Auto-Update:** noch nicht implementiert (optional via `self_update` crate)
- **Installer:** keiner — die EXE ist portabel, einfach kopieren

## Zur weiteren Beschleunigung Richtung WizTree

Aktuelle Leistung ~280k Einträge/s. WizTree liegt bei 1-3 Mio/s durch direktes
NTFS-MFT-Lesen. Ergänzungen für Folge-Versionen:

1. `FindFirstFileExW` mit `FIND_FIRST_EX_LARGE_FETCH` und `FindExInfoBasic`
   (überspringt 8.3-Aliase, batcht Kernel-Calls) → ~2× zusätzlich
2. NTFS-MFT-Reader via `\\.\C:` mit `FSCTL_ENUM_USN_DATA` (braucht Admin) →
   1-3 Mio/s, ähnlich WizTree
