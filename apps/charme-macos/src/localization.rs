use std::sync::OnceLock;

use cacao::{
    foundation::{NSString, id},
    objc::{class, msg_send, sel, sel_impl},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Locale {
    ZhCn,
    EnUs,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Key {
    About,
    Services,
    HideApp,
    HideOthers,
    ShowAll,
    Quit,
    CloseWindow,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    EnterFullScreen,
    ExitFullScreen,
    Minimize,
    Zoom,
    OpenProject,
    NewProject,
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
    ChooseProjectMessage,
    SaveProjectMessage,
    ChoosePmxMessage,
    ChooseShaderMessage,
    FileMenu,
    OpenProjectMenu,
    NewProjectMenu,
    RecentProjectsMenu,
    NoRecentProjects,
    ImportMenu,
    ImportPmxMenu,
    SaveProjectMenu,
    SaveAsProjectMenu,
    InspectShaderMenu,
    EditMenu,
    ViewMenu,
    WindowMenu,
    BringAllToFront,
}

/// Resolves the first preferred macOS language once per application launch.
pub(crate) fn current() -> Locale {
    static CURRENT: OnceLock<Locale> = OnceLock::new();
    *CURRENT.get_or_init(detect)
}

fn detect() -> Locale {
    let languages: id = unsafe { msg_send![class!(NSLocale), preferredLanguages] };
    let first: id = unsafe { msg_send![languages, firstObject] };
    if first.is_null() {
        return Locale::EnUs;
    }

    let language = NSString::retain(first).to_string();
    if language.starts_with("zh") {
        Locale::ZhCn
    } else {
        Locale::EnUs
    }
}

pub(crate) fn text(key: Key) -> &'static str {
    match (current(), key) {
        (Locale::ZhCn, Key::About) => "关于Charme",
        (Locale::ZhCn, Key::Services) => "服务",
        (Locale::ZhCn, Key::HideApp) => "隐藏Charme",
        (Locale::ZhCn, Key::HideOthers) => "隐藏其他",
        (Locale::ZhCn, Key::ShowAll) => "显示全部",
        (Locale::ZhCn, Key::Quit) => "退出Charme",
        (Locale::ZhCn, Key::CloseWindow) => "关闭窗口",
        (Locale::ZhCn, Key::Undo) => "撤销",
        (Locale::ZhCn, Key::Redo) => "重做",
        (Locale::ZhCn, Key::Cut) => "剪切",
        (Locale::ZhCn, Key::Copy) => "复制",
        (Locale::ZhCn, Key::Paste) => "粘贴",
        (Locale::ZhCn, Key::SelectAll) => "全选",
        (Locale::ZhCn, Key::EnterFullScreen) => "进入全屏",
        (Locale::ZhCn, Key::ExitFullScreen) => "退出全屏",
        (Locale::ZhCn, Key::Minimize) => "最小化",
        (Locale::ZhCn, Key::Zoom) => "缩放",
        (Locale::ZhCn, Key::OpenProject) => "打开项目",
        (Locale::ZhCn, Key::NewProject) => "新建项目",
        (Locale::ZhCn, Key::StartupTitle) => "开始使用Charme",
        (Locale::ZhCn, Key::StartupSubtitle) => "打开或新建一个项目以开始编辑角色材质",
        (Locale::ZhCn, Key::StartupFormats) => "支持.charme项目文件",
        (Locale::ZhCn, Key::RecentProjects) => "最近打开的项目",
        (Locale::ZhCn, Key::ProjectFallback) => "Charme项目",
        (Locale::ZhCn, Key::Scene) => "场景",
        (Locale::ZhCn, Key::EmptyScene) => "尚未导入角色\n\n请点击“导入PMX”开始。",
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
        (Locale::ZhCn, Key::ChooseProjectMessage) => "选择一个.charme项目文件。",
        (Locale::ZhCn, Key::SaveProjectMessage) => "选择项目保存位置。",
        (Locale::ZhCn, Key::ChoosePmxMessage) => "选择一个PMX角色模型导入当前项目。",
        (Locale::ZhCn, Key::ChooseShaderMessage) => "选择一个WGSL材质Shader进行检查。",
        (Locale::ZhCn, Key::FileMenu) => "文件",
        (Locale::ZhCn, Key::OpenProjectMenu) => "打开项目…",
        (Locale::ZhCn, Key::NewProjectMenu) => "新建项目",
        (Locale::ZhCn, Key::RecentProjectsMenu) => "打开最近项目",
        (Locale::ZhCn, Key::NoRecentProjects) => "暂无最近项目",
        (Locale::ZhCn, Key::ImportMenu) => "导入…",
        (Locale::ZhCn, Key::ImportPmxMenu) => "PMX…",
        (Locale::ZhCn, Key::SaveProjectMenu) => "保存",
        (Locale::ZhCn, Key::SaveAsProjectMenu) => "另存为…",
        (Locale::ZhCn, Key::InspectShaderMenu) => "检查WGSLShader…",
        (Locale::ZhCn, Key::EditMenu) => "编辑",
        (Locale::ZhCn, Key::ViewMenu) => "视图",
        (Locale::ZhCn, Key::WindowMenu) => "窗口",
        (Locale::ZhCn, Key::BringAllToFront) => "将所有窗口置于最前",

        (Locale::EnUs, Key::About) => "About Charme",
        (Locale::EnUs, Key::Services) => "Services",
        (Locale::EnUs, Key::HideApp) => "Hide Charme",
        (Locale::EnUs, Key::HideOthers) => "Hide Others",
        (Locale::EnUs, Key::ShowAll) => "Show All",
        (Locale::EnUs, Key::Quit) => "Quit Charme",
        (Locale::EnUs, Key::CloseWindow) => "Close Window",
        (Locale::EnUs, Key::Undo) => "Undo",
        (Locale::EnUs, Key::Redo) => "Redo",
        (Locale::EnUs, Key::Cut) => "Cut",
        (Locale::EnUs, Key::Copy) => "Copy",
        (Locale::EnUs, Key::Paste) => "Paste",
        (Locale::EnUs, Key::SelectAll) => "Select All",
        (Locale::EnUs, Key::EnterFullScreen) => "Enter Full Screen",
        (Locale::EnUs, Key::ExitFullScreen) => "Exit Full Screen",
        (Locale::EnUs, Key::Minimize) => "Minimize",
        (Locale::EnUs, Key::Zoom) => "Zoom",
        (Locale::EnUs, Key::OpenProject) => "Open Project",
        (Locale::EnUs, Key::NewProject) => "New Project",
        (Locale::EnUs, Key::StartupTitle) => "Start Using Charme",
        (Locale::EnUs, Key::StartupSubtitle) => {
            "Open or create a project to start editing character materials"
        }
        (Locale::EnUs, Key::StartupFormats) => "Supports .charme project files",
        (Locale::EnUs, Key::RecentProjects) => "Recent Projects",
        (Locale::EnUs, Key::ProjectFallback) => "Charme Project",
        (Locale::EnUs, Key::Scene) => "Scene",
        (Locale::EnUs, Key::EmptyScene) => "No character imported\n\nChoose “Import PMX” to begin.",
        (Locale::EnUs, Key::Materials) => "Material Slots",
        (Locale::EnUs, Key::EmptyMaterials) => "No material slots",
        (Locale::EnUs, Key::Inspector) => "Inspector",
        (Locale::EnUs, Key::InspectorBody) => {
            "Materials and Shader parameters will appear here after opening a file."
        }
        (Locale::EnUs, Key::Brightness) => "Viewport Brightness",
        (Locale::EnUs, Key::RendererStarting) => "Starting renderer…",
        (Locale::EnUs, Key::ProjectOpened) => "Project opened",
        (Locale::EnUs, Key::WaitingCharacter) => "Waiting for character model…",
        (Locale::EnUs, Key::LoadingMaterials) => "Loading material slots…",
        (Locale::EnUs, Key::LoadingPmxTextures) => "Loading PMX and textures…",
        (Locale::EnUs, Key::InspectingShader) => "Inspecting Shader…",
        (Locale::EnUs, Key::ShaderError) => "Shader Error",
        (Locale::EnUs, Key::ReflectionFailed) => "WGSL reflection failed",
        (Locale::EnUs, Key::MaterialInspector) => "Material Inspector",
        (Locale::EnUs, Key::WgslShader) => "WGSL Shader",
        (Locale::EnUs, Key::NoMaterials) => "No material slots",
        (Locale::EnUs, Key::ErrorPrefix) => "Error: ",
        (Locale::EnUs, Key::ChooseProjectMessage) => "Choose a .charme project file.",
        (Locale::EnUs, Key::SaveProjectMessage) => "Choose where to save the project.",
        (Locale::EnUs, Key::ChoosePmxMessage) => {
            "Choose a PMX character model to import into the current project."
        }
        (Locale::EnUs, Key::ChooseShaderMessage) => "Choose a WGSL material Shader to inspect.",
        (Locale::EnUs, Key::FileMenu) => "File",
        (Locale::EnUs, Key::OpenProjectMenu) => "Open Project…",
        (Locale::EnUs, Key::NewProjectMenu) => "New Project",
        (Locale::EnUs, Key::RecentProjectsMenu) => "Open Recent",
        (Locale::EnUs, Key::NoRecentProjects) => "No Recent Projects",
        (Locale::EnUs, Key::ImportMenu) => "Import…",
        (Locale::EnUs, Key::ImportPmxMenu) => "PMX…",
        (Locale::EnUs, Key::SaveProjectMenu) => "Save",
        (Locale::EnUs, Key::SaveAsProjectMenu) => "Save As…",
        (Locale::EnUs, Key::InspectShaderMenu) => "Inspect WGSL Shader…",
        (Locale::EnUs, Key::EditMenu) => "Edit",
        (Locale::EnUs, Key::ViewMenu) => "View",
        (Locale::EnUs, Key::WindowMenu) => "Window",
        (Locale::EnUs, Key::BringAllToFront) => "Bring All to Front",
    }
}
