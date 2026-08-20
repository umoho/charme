# AGENTS.md

## 项目概览

Charme 是基于 Bevy 和 PMX 的原生角色材质编辑器，采用 Cargo workspace 组织。各平台原生 UI 共享同一套文档模型、WGSL 工具链、Bevy 材质运行时与离屏渲染器；WGSL 文件在外部编辑，由应用负责重载。

修改前先阅读相关模块代码，以及 `README.md` 和 `docs/` 中对应的设计/打包文档。

## 模块与职责边界

### 模块职责

| 包 | 职责 | 边界 |
|---|---|---|
| `apps/charme-macos` | 原生应用壳：窗口、菜单、本地化、文件关联、渲染桥接 | 纯适配层，不持有领域状态 |
| `crates/charme-core` | 领域模型：稳定 ID、文档、命令、Undo/Redo、持久化 | 不含 Bevy、shader 编译器或平台 UI 类型 |
| `crates/charme-geometry` | 索引网格拓扑算法（primitive 连通分量分析） | 不依赖 Bevy、PMX 或平台 UI |
| `crates/charme-application` | 平台无关应用层：`EditorController`、`WorkspaceState`、`PreviewSynchronizer`、Inspector 注册表、材质对账 | 不依赖 cacao/AppKit，仅消费 renderer 契约类型 |
| `crates/charme-shader` | WGSL 组合、反射、元数据、校验、uniform 打包 | 可使用 naga，不依赖 Bevy |
| `crates/charme-bevy` | 可复用 Bevy 材质运行时：固定 ABI、`CharmeMaterial`/`Plugin` | 不依赖编辑器渲染器，可被普通 Bevy 应用消费 |
| `crates/charme-renderer` | 编辑器预览渲染器：场景、相机、离屏渲染、拾取、叠加 | 编辑器专用，不属于导出运行时依赖 |

### 边界规则

- Rust edition 2024，遵循现有模块边界与命名风格。
- 依赖方向保持单向：`core`、`geometry` 是叶子，`application` 只消费 renderer 的契约类型，macOS 是唯一的组合根。
- 平台 UI 逻辑不得放入 `core`；Bevy 与 ECS 类型不得越过 renderer/UI 边界。
- 平台无关的瞬态状态（选择、导入追踪、预览投影、Inspector、对账）放在 `charme-application`，不放入 macOS UI。
- 编辑器文档是唯一事实来源，渲染世界是投影；文档材质值只通过 `EditorCommand` 写入。
- 优先复用项目已有的错误类型与 `thiserror`，避免无必要地引入新的错误处理依赖。

## 模块连接与数据流

### 编译期依赖

```text
charme-core          （叶子，无内部依赖）
charme-geometry      （叶子，无内部依赖）
charme-shader        -> charme-core
charme-bevy          -> charme-core, charme-shader
charme-application   -> charme-core, charme-shader, charme-renderer(契约类型)
charme-renderer      -> charme-core, charme-geometry, charme-bevy
charme-macos         -> charme-application, charme-core, charme-renderer
```

- `charme-application` 依赖 `charme-renderer` 的契约类型（`PmxSourceIdentity`、`PmxLoadProgress`、`ViewportSelectionAction`、`PmxSceneInfo`），反向不成立。
- `charme-renderer` 依赖 `charme-geometry`（`split_primitive`）与 `charme-bevy`（材质 ABI）。
- `bevy_pmx` 是唯一作为依赖引用的外部参考项目。

### 运行时数据流

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

- 每次动作后，`PreviewSynchronizer` 从文档重新投影完整槽位参数，而不是增量 patch。
- 渲染器通知不直接改 UI 或文档，而是转成 `WorkspaceAction` 回流，由 `WorkspaceState` 校验请求归属、拒绝过期结果。
- 文档材质值只通过 `EditorCommand` 写入；原生视图不直接修改渲染器。

### 消息边界

| 消息 | 来源 | 消费方 |
|---|---|---|
| `Message::Editor(EditorMessage)` | 编辑器原生控件 | 单个编辑器窗口 |
| `Message::Preview(PreviewEvent)` | 渲染线程（经 `RenderBridge`） | 帧展示、通知处理 |
| `Message::Application(ApplicationEvent)` | 应用层 | 菜单刷新、错误提示 |

## 设计准则

### 数据流与依赖方向

- 数据流保持单向：动作 → 状态 → 效果 → 展示与渲染命令，通知作为动作回流。
- 文档是唯一事实来源，渲染世界是投影；`PreviewSynchronizer` 在每次动作后推导完整槽位更新。
- 平台无关状态放在 `charme-application`，不放入 macOS UI。

### 开闭原则

- 扩展优先于修改：新 Inspector 分区注册 `InspectorProvider`；新菜单项用 `MenuTag` + `MenuItemState` 描述符；新状态变更增加 `WorkspaceAction`/`WorkspaceEffect` 变体。
- 状态变更使用 reducer 风格：`dispatch(action)` 返回 `WorkspaceEffect` 列表，调用方按 effect 决定适配动作。
- 参数类型统一使用 `ParameterValue`，反射、打包、Inspector 与渲染 ABI 共享同一类型集合。

### 单一职责

- 模块按关注点拆分：调度（`RenderScheduler`）、拾取（`selection`）、叠加（`overlay`）、导入（`pmx_import`）与场景生成（`scene_runtime`）各自独立。
- 消息按来源分类：`ApplicationEvent`、`EditorMessage`、`PreviewEvent` 分开分发，不混入同一枚举。
- `RenderBridge` 只调度渲染操作并转发结果，不承担状态管理。

### 健壮性

- 稳定标识优先于位置/索引：菜单用 `MenuTag`、Inspector 分区用稳定 key、材质槽用 `MaterialSlotId`。
- 异步 PMX 导入保留候选状态，成功后以原子事务提交（`reconcile_pmx_materials`），过期或不匹配的结果被拒绝。
- 编译或参数校验失败时保留最后一次成功渲染的材质，并通过渲染器通知上报。

## 构建、测试与运行

### 代码检查

修改 Rust 代码后，根据改动范围运行相关检查：

```sh
cargo fmt --all
cargo test -p charme-shader --all-targets
cargo test -p charme-bevy --all-targets
cargo test -p charme-renderer --all-targets
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

提交前至少完成格式化，并运行受影响 crate 的测试和检查；无需每次跑完全部命令。

### 应用运行

macOS 上启动或验证应用时，优先使用项目脚本，而不是直接使用 `cargo run`：

```sh
scripts/run-macos-app.sh                 # 默认 debug：构建 Bundle 后运行
scripts/run-macos-app.sh --release       # release：构建 Bundle 后运行
scripts/run-macos-app.sh --build-only    # 只构建，不启动
scripts/run-macos-app.sh --run-only      # 运行已有的 debug Bundle
scripts/run-macos-app.sh --run-only --release
```

这样可以验证真正的 `Charme.app` 行为，包括 Bundle 资源、本地化、Info.plist、文件关联和 AppKit 菜单。默认 debug Bundle 位于 `target/debug/bundle/Charme.app`，release Bundle 位于 `target/release/bundle/Charme.app`。

`cargo run -p charme-macos` 仍可用于快速迭代、编译检查或不需要 Bundle 行为的场景；但不能替代对 `.app` 的启动验证。运行打包脚本前需要安装固定版本的 `cargo-packager`：

```sh
cargo install cargo-packager --version 0.11.8 --locked
```

### 调试与验证

- macOS Bundle 打包相关修改使用 `scripts/run-macos-app.sh --build-only` 验证。
- 修改涉及启动窗口、菜单、本地化或文件关联时，实际运行 Bundle 验证。
- 修改 WGSL、资源路径或 Bundle 配置时，同时检查对应的打包资源和运行时加载路径。
- Shell 脚本使用 `/bin/sh` 可用的语法，修改后运行 `sh -n scripts/run-macos-app.sh`。

## 版本管理规范

- 提交前运行 `git diff --check`，确保无输出。
- 不误提交构建产物、临时 Bundle 或本地资源。
- 提交信息使用简洁的英文动词短语，准确描述本次改动。
- 保持文档、命令示例和脚本参数的一致性。

## 文本要求

### 日志

- 运行时诊断优先使用 `tracing`，按严重程度使用 `error!`、`warn!`、`info!`、`debug!` 或 `trace!`，不使用 `eprintln!`、`print!` 或 `println!` 代替日志。
- 日志优先使用结构化字段记录上下文，例如 `path = %path`、`error = %error`，避免拼接成不可检索的字符串。
- 日志只用于诊断，不替代面向用户的本地化 UI 错误提示。
- 允许直接输出的例外：命令行参数错误或不支持平台的启动提示；`build.rs` 必需的 Cargo 指令输出；示例程序明确设计的标准输出。
- Bevy 相关代码优先接入其 `tracing`/`bevy_log` 体系，避免重复初始化全局日志订阅器。

### 界面与文档

- 出现在 UI 中的中英文混合文本无需用空格隔开。
- 面向用户的提示走本地化（`localization`），不使用日志代替。
- 文档、命令示例与脚本参数保持描述一致。
