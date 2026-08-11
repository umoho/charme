#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Locale {
    ZhCn,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Key {
    OpenFile,
    StartupTitle,
    StartupSubtitle,
    StartupFormats,
    RecentProjects,
    ProjectFallback,
    Scene,
    EmptyScene,
    Materials,
    EmptyMaterials,
    Inspector,
    InspectorBody,
    Brightness,
    RendererStarting,
    ProjectOpened,
    WaitingCharacter,
    LoadingMaterials,
    LoadingPmxTextures,
    InspectingShader,
    ShaderError,
    ReflectionFailed,
    MaterialInspector,
    WgslShader,
    NoMaterials,
    ErrorPrefix,
    ChooseFileMessage,
    ChooseShaderMessage,
    FileMenu,
    InspectShaderMenu,
    EditMenu,
    ViewMenu,
    WindowMenu,
}

pub(crate) fn current() -> Locale {
    Locale::ZhCn
}

pub(crate) fn text(key: Key) -> &'static str {
    match (current(), key) {
        (Locale::ZhCn, Key::OpenFile) => "打开文件",
        (Locale::ZhCn, Key::StartupTitle) => "开始使用Charme",
        (Locale::ZhCn, Key::StartupSubtitle) => "打开一个文件以开始编辑角色材质",
        (Locale::ZhCn, Key::StartupFormats) => "支持.charme、.pmx和.wgsl文件",
        (Locale::ZhCn, Key::RecentProjects) => "最近打开的项目",
        (Locale::ZhCn, Key::ProjectFallback) => "Charme项目",
        (Locale::ZhCn, Key::Scene) => "场景",
        (Locale::ZhCn, Key::EmptyScene) => "尚未打开文件\n\n请点击“打开文件”开始。",
        (Locale::ZhCn, Key::Materials) => "材质槽",
        (Locale::ZhCn, Key::EmptyMaterials) => "暂无材质槽",
        (Locale::ZhCn, Key::Inspector) => "检查器",
        (Locale::ZhCn, Key::InspectorBody) => "打开文件后，这里将显示材质和Shader参数。",
        (Locale::ZhCn, Key::Brightness) => "视口亮度",
        (Locale::ZhCn, Key::RendererStarting) => "正在初始化渲染器…",
        (Locale::ZhCn, Key::ProjectOpened) => "项目已打开",
        (Locale::ZhCn, Key::WaitingCharacter) => "等待角色模型…",
        (Locale::ZhCn, Key::LoadingMaterials) => "正在加载材质槽…",
        (Locale::ZhCn, Key::LoadingPmxTextures) => "正在加载PMX和纹理…",
        (Locale::ZhCn, Key::InspectingShader) => "正在检查Shader…",
        (Locale::ZhCn, Key::ShaderError) => "Shader错误",
        (Locale::ZhCn, Key::ReflectionFailed) => "WGSL反射失败",
        (Locale::ZhCn, Key::MaterialInspector) => "材质检查器",
        (Locale::ZhCn, Key::WgslShader) => "WGSLShader",
        (Locale::ZhCn, Key::NoMaterials) => "没有材质槽",
        (Locale::ZhCn, Key::ErrorPrefix) => "错误：",
        (Locale::ZhCn, Key::ChooseFileMessage) => "选择一个Charme项目、PMX角色模型或WGSLShader。",
        (Locale::ZhCn, Key::ChooseShaderMessage) => "选择一个WGSL材质Shader进行检查。",
        (Locale::ZhCn, Key::FileMenu) => "文件",
        (Locale::ZhCn, Key::InspectShaderMenu) => "检查WGSLShader…",
        (Locale::ZhCn, Key::EditMenu) => "编辑",
        (Locale::ZhCn, Key::ViewMenu) => "视图",
        (Locale::ZhCn, Key::WindowMenu) => "窗口",
    }
}
