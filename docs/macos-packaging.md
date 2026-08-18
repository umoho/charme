# macOS 应用打包

Charme 的原生运行形态是 `Charme.app`。项目使用固定版本的
[cargo-packager](https://github.com/crabnebula-dev/cargo-packager) 生成 Bundle；它支持当前需要的
`.app`、Resources 和 Info.plist，也可继续扩展 DMG、Developer ID 签名及 notarization。

## 准备工具

```sh
cargo install cargo-packager --version 0.11.8 --locked
```

## 构建与启动

从仓库根目录运行单个脚本即可构建并启动 debug 应用：

```sh
scripts/run-macos-app.sh                 # debug，默认启用 Bevy 动态链接
```

也可以指定 release、关闭 debug 动态链接，或只构建/只运行：

```sh
scripts/run-macos-app.sh --release
scripts/run-macos-app.sh --no-dynamic-linking
scripts/run-macos-app.sh --build-only --release
scripts/run-macos-app.sh --run-only
```

debug 产物位于 `target/debug/bundle/Charme.app`，release 产物位于
`target/release/bundle/Charme.app`。也可以直接运行：

```sh
cargo packager --config Packager.toml
open target/release/bundle/Charme.app
```

`Packager.toml` 会先以 `MACOSX_DEPLOYMENT_TARGET=11.0` 构建 release 二进制，再复制整个
本地化资源目录并合并应用的 Info.plist。Bundle ID 为 `com.umoho.charme`，同时声明
`.charme` 项目文档类型。

## 本地化与开发运行

打包应用通过主 `NSBundle` 的 `preferredLocalizations` 选择语言，并使用
`localizedStringForKey:value:table:` 读取 `Localizable.strings`。Info.plist 显式禁用 mixed
localizations，因此 AppKit framework 与应用采用同一个有效本地化。

构建脚本扫描 `resources/*.lproj/Localizable.strings`，为每种语言生成一个
`LanguageCatalog` 实现和统一 registry，并检查所有语言与开发语言 `en` 的键集合一致。新增语言
只需加入相应 `.lproj` 目录；Cargo 会重新生成实现，cargo-packager 会自动复制资源。

`cargo run -p charme-macos` 仍可用于快速开发。命令行二进制没有 Bundle 本地化资源，此时
Charme 使用 NSBundle 的原生语言协商 API，从编译期生成的语言实现中选择 fallback。标准菜单
自动补充项、Bundle 本地化、文档类型/文件关联和后续签名行为均应以通过 `open` 启动的 `.app`
为准。

## 发行

当前本地构建未配置 Developer ID。正式分发还需在 cargo-packager 的 macOS 配置中加入签名
身份和 notarization 凭据，并制作、签名与公证 DMG；这些凭据不应提交到仓库。
