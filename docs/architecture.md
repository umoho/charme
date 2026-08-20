# Architecture

## Principles

- Platform UIs share commands and state, not widgets.
- `charme-core` contains no Bevy, shader compiler, or native UI types.
- Bevy and its ECS types do not cross the renderer/UI boundary.
- The editor document is the source of truth; the render world is a projection.
- Shader compilation failures preserve the last successfully rendered material.
- Parameter validation failures preserve the last successfully rendered material
  and are reported through renderer notifications.
- Exported Charme materials should be consumable by an ordinary Bevy application.

## Packages

```text
charme-core ──────┬── charme-application ──────┐
                  ├── charme-bevy ─────────────┤
charme-shader ────┤                            ├── charme-macos
charme-renderer ──┘                            │
      └── charme-bevy + bevy_pmx ──────────────┘
```

### `charme-core`

Owns stable IDs, editor documents, material instances, parameter values,
commands, events, persistence, and undo/redo semantics.

### `charme-geometry`

Owns renderer-independent indexed-mesh topology algorithms, including
primitive connected-component analysis. It does not depend on Bevy, PMX, or a
platform UI.

### `charme-shader`

Owns WGSL composition, metadata parsing, reflection, diagnostics, parameter
layout, and runtime value packing. It may use Naga but does not depend on Bevy.

### `charme-application`

Owns the platform-independent application layer shared by native frontends.
`EditorController` coordinates document actions against `EditorSession`.
`WorkspaceState` applies transient `WorkspaceAction`s and emits
`WorkspaceEffect`s for selection and asynchronous PMX import operations.
`PreviewSynchronizer` incrementally projects authoritative material state into
complete per-slot renderer updates, including Undo/Redo and removed overrides.
The Inspector registry composes provider-owned presentation sections and rows.
Shader inspection and view models expose presentation-ready state without
native widget types. The crate depends on core, shader tooling, and renderer
contract types, but not on cacao, AppKit, or another platform UI framework.

### `charme-bevy`

Owns the fixed Charme shader ABI and the reusable Bevy material integration.
This is the package intended to be embedded in Bevy applications that consume
Charme output. The first ABI uses Bevy material bind group 3, binding 0, with a
256-byte (16 `vec4`) uniform block. The first five semantic fields occupy
roughness, rim strength, highlight strength (the legacy outline-width slot),
toon bands, and base tint slots; the remaining lanes are reserved for
compatible additions. `CharmeMaterialPlugin`
embeds the runtime shader so consumers do not need Charme's editor assets.

### `charme-renderer`

Owns the editor-only Bevy application, PMX preview scene, cameras, lighting,
offscreen targets, frame readback, thumbnails, and the command/event bridge.
Its worker has one command execution path. `RenderScheduler` owns coalesced
viewport invalidation and readback state; background previews only start while
the viewport is idle. PMX scene assets, CPU selection/picking geometry, and
transient preview overlays live in separate modules. Complete material override
updates are validated before replacing the currently rendered ABI block. It is
not part of the exported runtime dependency set.

### Platform applications

Each operating system gets a native application package. The macOS frontend is
implemented first. Future Windows and Linux frontends should consume the shared
application actions and view models plus the renderer API, without requiring a
shared widget abstraction. Native UI messages remain platform adapters around
those application-level actions. The macOS frontend separates application
events, native editor messages, and preview transport events. `RenderBridge`
only schedules renderer operations and forwards results to the main thread.

## State and effect flow

```text
Native input
    -> EditorAction / WorkspaceAction
    -> EditorController / WorkspaceState
    -> EditorUpdate / WorkspaceEffect
    -> native presentation + PreviewSynchronizer
    -> renderer command

Renderer notification
    -> preview transport event
    -> WorkspaceAction when it changes application state
    -> native presentation effect
```

Document material values are written only through `EditorCommand`. Native views
do not separately mutate the renderer; `PreviewSynchronizer` derives complete
slot parameter state from the document after every editor action.

## Dependency policy

`bevy_pmx` is the only reference project used as a dependency. Other sibling
projects are experiments or applications and may be consulted, but Charme must
not depend on them. Third-party crates are declared centrally in the workspace.
