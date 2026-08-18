# AGENTS.md

## 项目概览

Charme 是基于 Bevy 和 PMX 的原生角色材质编辑器，采用 Cargo workspace：

- `apps/charme-macos`：macOS 原生应用壳。
- `crates/charme-core`：编辑器文档模型、命令、撤销/重做和持久化。
- `crates/charme-shader`：WGSL 组合、反射、校验和 uniform 打包。
- `crates/charme-bevy`：可复用的 Bevy 材质运行时。
- `crates/charme-renderer`：预览渲染器和 PMX 场景集成。

修改前先阅读相关模块代码，以及 `README.md` 和 `docs/` 中对应的设计/打包文档。

## 应用运行优先级

在 macOS 上需要启动或验证应用时，优先使用项目脚本，而不是直接使用 `cargo run`：

```sh
scripts/run-macos-app.sh                 # 默认 debug：构建 Bundle 后运行
scripts/run-macos-app.sh --release       # release：构建 Bundle 后运行
scripts/run-macos-app.sh --build-only    # 只构建，不启动
scripts/run-macos-app.sh --run-only      # 运行已有的 debug Bundle
scripts/run-macos-app.sh --run-only --release
```

这样可以验证真正的 `Charme.app` 行为，包括 Bundle 资源、本地化、Info.plist、文件关联和
AppKit 菜单。默认 debug Bundle 位于 `target/debug/bundle/Charme.app`，release Bundle 位于
`target/release/bundle/Charme.app`。

`cargo run -p charme-macos` 仍可用于快速迭代、编译检查或不需要 Bundle 行为的场景；但不能替代
对 `.app` 的启动验证。运行打包脚本前需要安装固定版本的 `cargo-packager`：

```sh
cargo install cargo-packager --version 0.11.8 --locked
```

## 构建与测试

修改 Rust 代码后，根据改动范围运行相关检查：

```sh
cargo fmt --all
cargo test -p charme-shader --all-targets
cargo test -p charme-bevy --all-targets
cargo test -p charme-renderer --all-targets
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

不需要每次运行所有命令，但提交前至少应完成格式化，并运行受影响 crate 的测试和检查。

macOS Bundle 打包相关修改应使用 `scripts/run-macos-app.sh --build-only` 验证；若修改涉及
启动窗口、菜单、本地化或文件关联，还应实际运行 Bundle 进行验证。

## 代码规范

- Rust edition 2024，遵循现有模块边界和命名风格。
- 保持 core、shader、renderer 和 macOS UI 之间的依赖方向，不要把平台 UI 逻辑放入 core。
- 优先使用项目已有的错误类型和 `thiserror`，避免无必要地引入新的错误处理依赖。
- 修改 WGSL、资源路径或 Bundle 配置时，同时检查对应的打包资源和运行时加载路径。
- Shell 脚本使用 `/bin/sh` 可用的语法，并在修改后运行 `sh -n scripts/run-macos-app.sh`。
- 出现在 UI 中的中英文混合文本无需用空格隔开。

## 日志规范

- 运行时诊断优先使用 `tracing`，按严重程度使用 `tracing::error!`、`warn!`、`info!`、`debug!` 或 `trace!`，不使用 `eprintln!`、`print!` 或 `println!` 代替日志。
- 日志优先使用结构化字段记录上下文，例如 `path = %path`、`error = %error`，避免只拼接成不可检索的字符串。
- 日志只用于诊断，不替代面向用户的本地化 UI 错误提示。
- 以下场景可以保留直接输出：
  - 命令行参数错误或不支持平台的启动提示；
  - `build.rs` 必需的 Cargo 指令输出；
  - 示例程序明确设计的标准输出。
- Bevy 相关代码应优先与其现有的 `tracing`/`bevy_log` 体系集成，避免重复初始化全局日志订阅器。

## 提交前检查

- `git diff --check` 无输出。
- 没有误提交构建产物、临时 Bundle 或本地资源。
- 文档、命令示例和脚本参数保持一致。
- 提交信息使用简洁的英文动词短语，准确描述本次改动。
