# Charme

Charme is a native character material editor built around Bevy and PMX.

The editor uses a platform-native UI on each operating system while sharing its
document model, WGSL tooling, Bevy material runtime, and offscreen renderer.
Charme does not provide a source-code editor; WGSL files are edited externally
and reloaded by the application.

## Workspace

- `crates/charme-core`: UI- and renderer-independent editor domain model.
- `crates/charme-shader`: WGSL composition, reflection, metadata, and packing.
- `crates/charme-bevy`: reusable Bevy 0.19 material runtime.
- `crates/charme-renderer`: editor preview renderer and PMX scene integration.
- `apps/charme-macos`: macOS native application shell.

See [`docs/architecture.md`](docs/architecture.md) for dependency boundaries and
[`docs/TODO.md`](docs/TODO.md) for the implementation checklist.

## Status

Implemented foundations:

- Versioned project documents with stable IDs, portable resource paths, material
  instances and slot bindings, transactional editor commands, Undo/Redo,
  dirty tracking, snapshots, and RON persistence.
- WGSL composition, metadata scanning, reflection, validation, and uniform packing.
- A private Bevy render worker with on-demand offscreen rendering, asynchronous
  CPU readback, resize suspension, and orbit camera controls.
- Asynchronous PMX loading from arbitrary file-system paths, texture fallback,
  material-slot summaries, scene replacement, and automatic camera framing.
- A native macOS editor window with Scene/Materials, Viewport and Inspector
  columns, Retina-aware frame display, Orbit controls, and an Open PMX dialog.

The next UI milestone is a reflection-driven native material Inspector, followed
by Charme's reusable Bevy material ABI.

## Development

```sh
cargo test -p charme-shader --all-targets
cargo test -p charme-renderer --all-targets
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

Run the current macOS UI with:

```sh
cargo run -p charme-macos
```
