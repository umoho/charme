# Architecture

## Principles

- Platform UIs share commands and state, not widgets.
- `charme-core` contains no Bevy, shader compiler, or native UI types.
- Bevy and its ECS types do not cross the renderer/UI boundary.
- The editor document is the source of truth; the render world is a projection.
- Shader compilation failures preserve the last successfully rendered material.
- Exported Charme materials should be consumable by an ordinary Bevy application.

## Packages

```text
charme-core
    ▲
    ├── charme-shader
    │       ▲
    └───────┴── charme-bevy
                    ▲
                    │
             charme-renderer ─── bevy_pmx
                    ▲
                    │
              charme-macos
```

### `charme-core`

Owns stable IDs, editor documents, material instances, parameter values,
commands, events, persistence, and undo/redo semantics.

### `charme-shader`

Owns WGSL composition, metadata parsing, reflection, diagnostics, parameter
layout, and runtime value packing. It may use Naga but does not depend on Bevy.

### `charme-bevy`

Owns the fixed Charme shader ABI and the reusable Bevy material integration.
This is the package intended to be embedded in Bevy applications that consume
Charme output.

### `charme-renderer`

Owns the editor-only Bevy application, PMX preview scene, cameras, lighting,
offscreen targets, frame readback, thumbnails, and the command/event bridge.
It is not part of the exported runtime dependency set.

### Platform applications

Each operating system gets a native application package. The macOS frontend is
implemented first. Future Windows and Linux frontends should consume the same
core and renderer APIs without requiring a shared widget abstraction.

## Dependency policy

`bevy_pmx` is the only reference project used as a dependency. Other sibling
projects are experiments or applications and may be consulted, but Charme must
not depend on them. Third-party crates are declared centrally in the workspace.
