# Experimental Haiku frontend

This separate binary uses Wordcraft's existing WordApp and document engine with
a CPU renderer and Haiku's in-process Wayland server. It does not replace the
normal desktop frontend. Its standalone Cargo workspace keeps temporary fork
overrides out of the other applications.

On Haiku x86_64 with Rust 1.99 and the system Wayland development files:

```sh
export LIBRARY_PATH='%A/lib:/boot/home/config/non-packaged/lib:/boot/home/config/lib:/boot/system/non-packaged/lib:/boot/system/lib'
cargo build --release --manifest-path apps/wordcraft-haiku/Cargo.toml
apps/wordcraft-haiku/target/release/wordcraft-haiku
```

The dependency revisions are pinned public fork commits. Their PRs cover RFD,
Wayland, Winit, calloop, SCTK and the egui software renderer. They are temporary
integration dependencies, not a claim that released crates support Haiku today.

`--sample` opens the built-in sample; a filename opens that document.
`--control-stdio` accepts Wordcraft control requests on stdin and writes replies
on stdout. It opens no listening network socket.

The original prototype passed physical keyboard input in a blank document,
native Open and Save As, Cancel, DOCX content checks and closing through Haiku's
native tab. This draft replaces local paths with public fork revisions, enables
the renderer's texture-cache fix, and makes the process-exit workaround explicit.
Those changes need a complete application build and repeat native tests before
this draft can be called ready to merge.

Known limits: system printing, system clipboard integration, settings persistence
and file associations are not wired into this frontend. The RFD command bridge
cannot enforce file filters or parent its dialogs. International composition and
long-duration stability are not established.

On Haiku the executable explicitly enables the renderer's process-exit workaround
for successful close. It runs after the editor's save/close decision and before
Wayland state is dropped, skipping process destructors. Proper system-library
teardown is still unresolved. The renderer option defaults to off for other
applications. The placement of this workaround is an open review question.
