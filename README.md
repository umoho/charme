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

See [`docs/architecture.md`](docs/architecture.md) for dependency boundaries.

## Status

The workspace structure is in place; product features have not been implemented yet.
