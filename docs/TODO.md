# Charme TODO

本文档记录 Charme 的产品与工程任务。完成任务时应同步更新 checkbox，并在必要时补充测试与文档。

## 当前实现优先级

1. 做出可体验的 macOS 原生编辑器 UI，并持续完善 Viewport、PMX 打开和材质面板。
2. 将 Shader reflection 接入原生 Inspector 控件。
3. 实现 `charme-bevy` 固定材质 ABI，让 UI 参数可以真正改变角色材质。
4. 完善项目保存、导出和跨平台前端。

## 0. 项目基础

- [x] 建立 Cargo workspace。
- [x] 拆分 `charme-core`、`charme-shader`、`charme-bevy`、`charme-renderer` 和 `charme-macos`。
- [x] 统一使用 Bevy 0.19。
- [x] 通过固定 Git revision 引入 `bevy_pmx`。
- [x] 确保不依赖其他实验参考项目。
- [x] 建立基础架构和依赖边界文档。
- [ ] 确定项目许可证。
- [ ] 配置 CI：format、check、clippy 和非 GPU 测试。
- [ ] 区分普通测试与需要真实 GPU 的测试。

## 1. `charme-core`：编辑器文档模型

- [x] 定义稳定的 `DocumentId`、`ShaderId`、`MaterialId` 和 `MaterialSlotId`。
- [ ] 评估并定义纹理等独立资源是否需要稳定 ID。
- [x] 定义 Charme 项目文档及版本号。
- [x] 定义角色模型资源引用，首期支持 PMX。
- [x] 定义 PMX material slot 到 Charme material instance 的绑定。
- [x] 定义与渲染后端无关的参数值类型。
- [x] 定义材质实例、Shader 引用、纹理引用和渲染状态。
- [x] 设计安全的项目相对路径和机器本地绝对路径规则。
- [ ] 定义缺失资源的文档状态和重新定位流程。
- [x] 定义 `EditorCommand`、`EditorEvent` 和只读 UI snapshot。
- [x] 实现事务式命令应用和文档 dirty 状态。
- [x] 实现 Undo/Redo 文档历史和保存状态恢复。
- [ ] 实现 slider、拖动等连续编辑的事务合并。
- [x] 实现新建、打开、保存和另存为。
- [ ] 为项目格式实现向前兼容的版本迁移。
- [x] 为文档模型、命令、路径和序列化添加单元测试。

## 2. `charme-shader`：WGSL 工具链

- [x] 支持 naga-oil composition 和 `#import`。
- [x] 支持 Shader defs。
- [x] 解析 WGSL doc comment 中的 `%{ ... }` metadata。
- [x] 将 metadata 与 WGSL 声明及源码位置关联。
- [x] 反射 entry point、资源和参数块。
- [x] 使用 Naga 布局信息打包 scalar 和 vector 参数。
- [x] 对不支持的参数类型产生可恢复诊断。
- [ ] 定义 Charme metadata 的正式 schema 和版本策略。
- [ ] 校验 `ui.min`、`ui.max`、`ui.step`、`ui.color` 等属性的类型和组合。
- [ ] 支持参数分组、排序、tooltip、枚举和隐藏属性。
- [ ] 将诊断 byte span 转换为文件、行号和列号。
- [ ] 实现从磁盘加载根 Shader 及其 import graph。
- [ ] 实现文件监视和防抖热重载。
- [ ] 编译失败时保留最后一次成功的 Shader interface。
- [ ] 按字段路径和类型迁移热重载前的参数值。
- [ ] 明确 matrix、array、嵌套 struct 的支持范围。
- [ ] 增加恶意输入、超大输入及 Unicode 路径测试。

## 3. `charme-bevy`：Bevy 材质运行时

- [x] 确定第一版固定 bind group ABI（group 3、binding 0 参数块）。
- [x] 确定参数 buffer 的容量、对齐和更新策略（256 bytes、16 个 vec4 lane）。
- [ ] 确定 diffuse、normal、sphere、toon/ramp 等固定纹理槽位。
- [x] 定义 `CharmeMaterial` Bevy asset。
- [x] 实现 `CharmeMaterialPlugin`。
- [x] 将第一版固定参数布局连接到 GPU uniform buffer。
- [ ] 实现纹理和 sampler 绑定。
- [ ] 实现 opaque、mask、blend 和 double-sided 渲染状态。
- [ ] 实现 Shader pipeline 创建、缓存和失效。
- [ ] 实现保留最后一次成功 pipeline 的热重载。
- [ ] 定义可由普通 Bevy 应用加载的导出资产格式。
- [ ] 实现 Charme 材质 AssetLoader。
- [ ] 提供最小 Bevy runtime 示例。
- [ ] 验证编辑器预览与 runtime 示例的渲染一致性。

## 4. `charme-renderer`：编辑器渲染服务

- [x] 在私有线程运行无窗口 Bevy App。
- [x] 支持按需渲染和 redraw 请求合并。
- [x] 支持异步 GPU readback。
- [x] 支持 BGRA8/RGBA8 sRGB 输出。
- [x] 支持动态尺寸和零尺寸暂停。
- [x] 支持 Orbit、Zoom 和相机重置。
- [x] 从任意文件系统路径加载 PMX。
- [x] 生成静态 PMX primitive 和 StandardMaterial 预览。
- [x] 枚举 PMX material slot 及其纹理引用。
- [x] 缺失纹理时使用占位资源并返回 warning。
- [x] 根据模型包围盒自动居中、落地和 framing。
- [x] 加载失败时保留当前场景。
- [x] 替换 PMX 场景时清理对应 mesh、material 和 image assets。
- [x] 将 StandardMaterial 预览替换为 `charme-bevy` 材质（首版固定参数 ABI）。
- [ ] 支持选择和高亮 material slot。
- [x] 支持从 UI 更新固定材质参数并立即重绘；纹理仍待接入。
- [ ] 增加材质球与材质缩略图 render session。
- [ ] 增加灯光、背景和环境 preset。
- [ ] 增加网格、法线、UV、材质 ID 等调试视图。
- [ ] 增加加载进度、取消和连续请求的处理策略。
- [ ] 处理多次快速加载时的过期结果。
- [ ] 支持 PMX sphere texture 和共享 toon texture 的准确预览。
- [ ] 支持 PMX 蒙皮和基础骨骼姿态。
- [ ] 支持 Morph 预览。
- [ ] 评估物理预览是否属于 Charme 范围。
- [ ] 消除 renderer shutdown 时可能出现的 readback channel warning。
- [ ] 在 Metal、Vulkan 和 DX12 上运行 GPU 集成测试。

## 5. `charme-macos`：原生 macOS UI

- [x] 创建 Cacao/AppKit 应用和原生主窗口。
- [x] 建立主线程 UI 与 renderer worker 的生命周期桥接。
- [ ] 完善顶部工具栏；原生应用菜单和“Open PMX”菜单已可用。
- [x] 建立 Viewport、Scene/Materials 和 Inspector 三栏初始界面。
- [ ] 增加 Diagnostics 面板。
- [ ] 将固定三栏升级为可调整尺寸的 Docking 布局。
- [x] 将 BGRA frame 显示为 `CGImage`/`NSImage`。
- [x] 正确处理 points、物理像素和 Retina scale factor。
- [x] 将鼠标拖动和滚轮事件转换为 Orbit/Zoom 命令。
- [x] 实现打开 PMX 和选择外部 WGSL 的原生文件对话框。
- [ ] 支持拖放 PMX、WGSL 和纹理文件。
- [x] 展示 PMX material slot 列表、纹理摘要和加载 warning 数量。
- [ ] 增加可展开的加载 warning 详情。
- [x] 在后台反射 WGSL，并根据 metadata 生成原生 Inspector scalar 控件。
- [x] 支持 float、i32 和 u32 原生 slider，显示 label、范围和当前值。
- [ ] 支持 boolean、vector、color 和 texture 控件。
- [x] 将 Inspector scalar 编辑写入 `charme-core` 材质实例和 dirty 历史。
- [x] 将 Inspector 参数编辑连接到 renderer 并实时改变角色材质。
- [ ] 实现参数编辑的连续事务与 Undo/Redo。
- [ ] 实现 Shader diagnostics 列表及源码位置跳转。
- [ ] 提供“在外部编辑器中打开”操作。
- [ ] 实现最近打开项目和窗口状态恢复。
- [ ] 实现未保存修改的关闭确认。
- [ ] 实现键盘快捷键和基础无障碍标签。

## 6. 项目保存、导出与 Bevy 工作流

- [ ] 确定 Charme 项目文件扩展名和目录结构。
- [ ] 保存模型、Shader、纹理和材质实例的相对引用。
- [ ] 保存 material slot 绑定。
- [ ] 保存相机、灯光和预览环境，但与 runtime 数据分离。
- [ ] 导出 Bevy runtime 材质资产。
- [ ] 设计复制资源与仅保存引用两种导出策略。
- [ ] 生成可直接加入 Bevy `AssetServer` 的目录结构。
- [ ] 检测缺失资源、路径越界和大小写不一致。
- [ ] 实现导出前验证报告。
- [ ] 用独立 Bevy 示例加载并渲染导出结果。

## 7. Windows 与 Linux 原生前端

- [ ] 在 macOS 工作流稳定后冻结 UI-facing command/snapshot 协议。
- [ ] 调研 Windows 原生 UI 技术栈并制作小型 viewport bridge 原型。
- [ ] 调研 Linux GTK4/libadwaita 前端并制作小型 viewport bridge 原型。
- [ ] 避免在平台之间抽象通用 widget；只共享状态、命令和语义。
- [ ] 实现 `charme-windows`。
- [ ] 实现 `charme-linux`。
- [ ] 验证项目文件在不同操作系统间可移植。

## 8. 质量、性能和发布

- [ ] 为公开 crate 开启严格 rustdoc 检查。
- [ ] 为项目格式准备 golden files。
- [ ] 为 Shader reflection 准备回归 fixture。
- [ ] 为真实 PMX 模型建立不提交私有资产的测试策略。
- [ ] 记录加载时间、首次出帧时间和参数更新延迟。
- [ ] 限制模型、纹理和 Shader 输入的资源占用。
- [ ] 检查 renderer worker 的 panic、断连和 shutdown 路径。
- [ ] 实现日志文件和可复制的诊断报告。
- [ ] 解决或替换 Cacao 的 future-incompatibility 依赖。
- [ ] 添加应用图标、版本信息和 About 窗口。
- [ ] 生成 macOS `.app` 并完成签名/公证流程。
- [ ] 撰写用户手册、Shader 作者指南和 Bevy 集成指南。
