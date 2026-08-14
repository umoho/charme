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
- A native macOS application with a macOS-style startup window, automatic
  `.charme` project startup, recent Charme projects, and an editor window with
  project-scoped PMX import plus Scene/Materials, Viewport and Inspector columns.
- Retina-aware frame display, Orbit controls, native file dialogs, and background
  WGSL reflection with metadata-driven native float and integer Inspector controls.

Inspector scalar edits are recorded in the core document model and now flow
through the renderer into Charme's reusable Bevy material ABI. The first fixed
ABI uses a 16-lane uniform block (256 bytes); its roughness, rim strength,
outline width, toon bands, and base tint controls visibly update the viewport.
Unsupported parameter paths are rejected without replacing the last valid
material and are reported as renderer notifications.

## Development

```sh
cargo test -p charme-shader --all-targets
cargo test -p charme-bevy --all-targets
cargo test -p charme-renderer --all-targets
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

Build and open the native macOS application bundle with:

```sh
cargo install cargo-packager --version 0.11.8 --locked # once
scripts/run-macos-app.sh                 # debug: build then run
scripts/run-macos-app.sh --release       # release: build then run
scripts/run-macos-app.sh --run-only      # run an existing debug bundle
scripts/run-macos-app.sh --build-only    # build without running
```

The default debug bundle is written to `target/debug/bundle/Charme.app`; use
`--release` for `target/release/bundle/Charme.app`. See
[`docs/macos-packaging.md`](docs/macos-packaging.md) for localization, file
association, signing, and release details.

For a quick development launch, `cargo run -p charme-macos` remains available,
but AppKit menu localization and other native Bundle behavior must be verified
using `Charme.app` launched through `open`.
