# egui-winit 0.37.0 — local clipboard delegation

Source: the installed crates.io package egui-winit 0.37.0, checksum
`e4e449f4c4a2a60c7ccb4ba7fa4078417bfe1f9ddc03b5e05ce75748e33a70de`.
Upstream: https://github.com/emilk/egui/tree/0.37.0/crates/egui-winit
License: MIT OR Apache-2.0; both license texts retained alongside the source.
The licenses were fetched from that exact upstream tag on 2026-11-03.

Only functional modification: the keyboard paste branch checks the temporary
`egui::Context` widget ID at `Id::unique("egui_winit.defer_clipboard_paste")`.
It compares that owner with current keyboard focus when the native key arrives;
when equal, it emits `Event::Paste(String::new())` without querying the clipboard.
Codisseu registers its visible AI composer, removing registration when hidden;
focus is evaluated by the backend after render has processed focus transitions;
its bounded worker then reads native files, image, or text in that order.
The original backend path remains in effect for every other field. This fixes
native file MIME lacking a text fallback and image MIME accompanied by text,
and keeps the asynchronous clipboard operation scoped to the active draft.

No extension bundle or proprietary application code is incorporated.
