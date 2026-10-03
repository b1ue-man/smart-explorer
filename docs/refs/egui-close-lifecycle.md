# egui/eframe 0.29.1: Close- und Update-Grenze

Geprüft am 2026-10-03 gegen die lokal vorhandenen Primärquellen der durch Cargo.lock gebundenen Crates egui/eframe 0.29.1, ohne Compiler-/Formatterausführung.

`App::on_exit` liegt nach dem eigentlichen Schließen. Eine aktive Arbeit muss deshalb im normalen `update` über `ctx.input(|i| i.viewport().close_requested())` erkannt und mit `ViewportCommand::CancelClose` im selben Frame gehalten werden. Ein späteres ausdrücklich gesendetes `Close` darf erst nach terminaler Worker-Completion erfolgen.

Die UI darf nicht auf einen noch laufenden JoinHandle warten. Sie signalisiert den vorhandenen Cancelmarker, verarbeitet Ergebnisreceiver weiter und joint ausschließlich `is_finished()`-Handles. Eine abgelaufene UI-Frist bleibt sichtbar im offenen Fenster; sie erlaubt kein Beenden eines noch schreibenden Workers. Update-Verifikation und Anwendung erfolgen erst nach dieser Completion und dem vorhandenen Recovery-/Shutdown-Preflight.

Primärquellen: [eframe App](https://docs.rs/eframe/0.29.1/eframe/trait.App.html), [egui ViewportInfo](https://docs.rs/egui/0.29.1/egui/struct.ViewportInfo.html), [ViewportCommand](https://docs.rs/egui/0.29.1/egui/enum.ViewportCommand.html). Lokal: `eframe-0.29.1/src/epi.rs`, `egui-0.29.1/src/data/input.rs`, `egui-0.29.1/src/viewport.rs` im Cargo-Registry-Cache.
