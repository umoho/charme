use std::sync::OnceLock;

use cacao::{
    foundation::{NSString, id, nil},
    objc::{class, msg_send, sel, sel_impl},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Locale {
    ZhHans,
    En,
}

macro_rules! localization_keys {
    ($($key:ident),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        #[repr(usize)]
        pub(crate) enum Key {
            $($key),+
        }

        const KEYS: &[Key] = &[$(Key::$key),+];

        impl Key {
            fn resource_key(self) -> &'static str {
                match self {
                    $(Self::$key => stringify!($key)),+
                }
            }
        }
    };
}

localization_keys!(
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
);

struct Localization {
    strings: Vec<&'static str>,
}

static LOCALIZATION: OnceLock<Localization> = OnceLock::new();

pub(crate) fn text(key: Key) -> &'static str {
    localization().strings[key as usize]
}

fn localization() -> &'static Localization {
    LOCALIZATION.get_or_init(load)
}

/// Uses the main bundle's effective localization. A direct `cargo run` has no
/// localization resources, so it explicitly falls back to the first macOS
/// preferred language and the built-in strings below.
fn load() -> Localization {
    unsafe {
        let bundle: id = msg_send![class!(NSBundle), mainBundle];
        let resource = NSString::new("Localizable");
        let extension = NSString::new("strings");
        let table_path: id = msg_send![bundle, pathForResource: &*resource ofType: &*extension];
        let bundled = !table_path.is_null();
        let locale = if bundled {
            first_language(msg_send![bundle, preferredLocalizations]).unwrap_or(Locale::En)
        } else {
            first_language(msg_send![class!(NSLocale), preferredLanguages]).unwrap_or(Locale::En)
        };

        let strings = KEYS
            .iter()
            .map(|&key| {
                let fallback = fallback(locale, key);
                if !bundled {
                    return fallback;
                }

                let resource_key = NSString::new(key.resource_key());
                let fallback_value = NSString::new(fallback);
                let value: id = msg_send![bundle,
                    localizedStringForKey: &*resource_key
                    value: &*fallback_value
                    table: nil
                ];
                Box::leak(NSString::retain(value).to_string().into_boxed_str()) as &'static str
            })
            .collect();

        Localization { strings }
    }
}

unsafe fn first_language(languages: id) -> Option<Locale> {
    if languages.is_null() {
        return None;
    }
    let first: id = unsafe { msg_send![languages, firstObject] };
    if first.is_null() {
        return None;
    }
    let identifier = NSString::retain(first).to_string();
    Some(if identifier.starts_with("zh") {
        Locale::ZhHans
    } else {
        Locale::En
    })
}

fn fallback(locale: Locale, key: Key) -> &'static str {
    match (locale, key) {
        (Locale::ZhHans, Key::About) => "关于Charme",
        (Locale::ZhHans, Key::Services) => "服务",
        (Locale::ZhHans, Key::HideApp) => "隐藏Charme",
        (Locale::ZhHans, Key::HideOthers) => "隐藏其他",
        (Locale::ZhHans, Key::ShowAll) => "显示全部",
        (Locale::ZhHans, Key::Quit) => "退出Charme",
        (Locale::ZhHans, Key::CloseWindow) => "关闭窗口",
        (Locale::ZhHans, Key::Undo) => "撤销",
        (Locale::ZhHans, Key::Redo) => "重做",
        (Locale::ZhHans, Key::Cut) => "剪切",
        (Locale::ZhHans, Key::Copy) => "复制",
        (Locale::ZhHans, Key::Paste) => "粘贴",
        (Locale::ZhHans, Key::SelectAll) => "全选",
        (Locale::ZhHans, Key::Minimize) => "最小化",
        (Locale::ZhHans, Key::Zoom) => "缩放",
        (Locale::ZhHans, Key::OpenProject) => "打开项目",
        (Locale::ZhHans, Key::NewProject) => "新建项目",
        (Locale::ZhHans, Key::StartupTitle) => "开始使用Charme",
        (Locale::ZhHans, Key::StartupSubtitle) => "打开或新建一个项目以开始编辑角色材质",
        (Locale::ZhHans, Key::StartupFormats) => "支持.charme项目文件",
        (Locale::ZhHans, Key::RecentProjects) => "最近打开的项目",
        (Locale::ZhHans, Key::ProjectFallback) => "Charme项目",
        (Locale::ZhHans, Key::Scene) => "场景",
        (Locale::ZhHans, Key::EmptyScene) => "尚未导入角色\n\n请点击“导入PMX”开始。",
        (Locale::ZhHans, Key::Materials) => "材质槽",
        (Locale::ZhHans, Key::EmptyMaterials) => "暂无材质槽",
        (Locale::ZhHans, Key::Inspector) => "检查器",
        (Locale::ZhHans, Key::InspectorBody) => "打开文件后，这里将显示材质和Shader参数。",
        (Locale::ZhHans, Key::Brightness) => "视口亮度",
        (Locale::ZhHans, Key::RendererStarting) => "正在初始化渲染器…",
        (Locale::ZhHans, Key::ProjectOpened) => "项目已打开",
        (Locale::ZhHans, Key::WaitingCharacter) => "等待角色模型…",
        (Locale::ZhHans, Key::LoadingMaterials) => "正在加载材质槽…",
        (Locale::ZhHans, Key::LoadingPmxTextures) => "正在加载PMX和纹理…",
        (Locale::ZhHans, Key::InspectingShader) => "正在检查Shader…",
        (Locale::ZhHans, Key::ShaderError) => "Shader错误",
        (Locale::ZhHans, Key::ReflectionFailed) => "WGSL反射失败",
        (Locale::ZhHans, Key::MaterialInspector) => "材质检查器",
        (Locale::ZhHans, Key::WgslShader) => "WGSLShader",
        (Locale::ZhHans, Key::NoMaterials) => "没有材质槽",
        (Locale::ZhHans, Key::ErrorPrefix) => "错误：",
        (Locale::ZhHans, Key::ChooseProjectMessage) => "选择一个.charme项目文件。",
        (Locale::ZhHans, Key::SaveProjectMessage) => "选择项目保存位置。",
        (Locale::ZhHans, Key::ChoosePmxMessage) => "选择一个PMX角色模型导入当前项目。",
        (Locale::ZhHans, Key::ChooseShaderMessage) => "选择一个WGSL材质Shader进行检查。",
        (Locale::ZhHans, Key::FileMenu) => "文件",
        (Locale::ZhHans, Key::OpenProjectMenu) => "打开项目…",
        (Locale::ZhHans, Key::NewProjectMenu) => "新建项目",
        (Locale::ZhHans, Key::RecentProjectsMenu) => "打开最近项目",
        (Locale::ZhHans, Key::NoRecentProjects) => "暂无最近项目",
        (Locale::ZhHans, Key::ImportMenu) => "导入…",
        (Locale::ZhHans, Key::ImportPmxMenu) => "PMX…",
        (Locale::ZhHans, Key::SaveProjectMenu) => "保存",
        (Locale::ZhHans, Key::SaveAsProjectMenu) => "另存为…",
        (Locale::ZhHans, Key::InspectShaderMenu) => "检查WGSLShader…",
        (Locale::ZhHans, Key::EditMenu) => "编辑",
        (Locale::ZhHans, Key::ViewMenu) => "视图",
        (Locale::ZhHans, Key::WindowMenu) => "窗口",
        (Locale::ZhHans, Key::BringAllToFront) => "将所有窗口置于最前",

        (Locale::En, Key::About) => "About Charme",
        (Locale::En, Key::Services) => "Services",
        (Locale::En, Key::HideApp) => "Hide Charme",
        (Locale::En, Key::HideOthers) => "Hide Others",
        (Locale::En, Key::ShowAll) => "Show All",
        (Locale::En, Key::Quit) => "Quit Charme",
        (Locale::En, Key::CloseWindow) => "Close Window",
        (Locale::En, Key::Undo) => "Undo",
        (Locale::En, Key::Redo) => "Redo",
        (Locale::En, Key::Cut) => "Cut",
        (Locale::En, Key::Copy) => "Copy",
        (Locale::En, Key::Paste) => "Paste",
        (Locale::En, Key::SelectAll) => "Select All",
        (Locale::En, Key::Minimize) => "Minimize",
        (Locale::En, Key::Zoom) => "Zoom",
        (Locale::En, Key::OpenProject) => "Open Project",
        (Locale::En, Key::NewProject) => "New Project",
        (Locale::En, Key::StartupTitle) => "Start Using Charme",
        (Locale::En, Key::StartupSubtitle) => {
            "Open or create a project to start editing character materials"
        }
        (Locale::En, Key::StartupFormats) => "Supports .charme project files",
        (Locale::En, Key::RecentProjects) => "Recent Projects",
        (Locale::En, Key::ProjectFallback) => "Charme Project",
        (Locale::En, Key::Scene) => "Scene",
        (Locale::En, Key::EmptyScene) => "No character imported\n\nChoose “Import PMX” to begin.",
        (Locale::En, Key::Materials) => "Material Slots",
        (Locale::En, Key::EmptyMaterials) => "No material slots",
        (Locale::En, Key::Inspector) => "Inspector",
        (Locale::En, Key::InspectorBody) => {
            "Materials and Shader parameters will appear here after opening a file."
        }
        (Locale::En, Key::Brightness) => "Viewport Brightness",
        (Locale::En, Key::RendererStarting) => "Starting renderer…",
        (Locale::En, Key::ProjectOpened) => "Project opened",
        (Locale::En, Key::WaitingCharacter) => "Waiting for character model…",
        (Locale::En, Key::LoadingMaterials) => "Loading material slots…",
        (Locale::En, Key::LoadingPmxTextures) => "Loading PMX and textures…",
        (Locale::En, Key::InspectingShader) => "Inspecting Shader…",
        (Locale::En, Key::ShaderError) => "Shader Error",
        (Locale::En, Key::ReflectionFailed) => "WGSL reflection failed",
        (Locale::En, Key::MaterialInspector) => "Material Inspector",
        (Locale::En, Key::WgslShader) => "WGSL Shader",
        (Locale::En, Key::NoMaterials) => "No material slots",
        (Locale::En, Key::ErrorPrefix) => "Error: ",
        (Locale::En, Key::ChooseProjectMessage) => "Choose a .charme project file.",
        (Locale::En, Key::SaveProjectMessage) => "Choose where to save the project.",
        (Locale::En, Key::ChoosePmxMessage) => {
            "Choose a PMX character model to import into the current project."
        }
        (Locale::En, Key::ChooseShaderMessage) => "Choose a WGSL material Shader to inspect.",
        (Locale::En, Key::FileMenu) => "File",
        (Locale::En, Key::OpenProjectMenu) => "Open Project…",
        (Locale::En, Key::NewProjectMenu) => "New Project",
        (Locale::En, Key::RecentProjectsMenu) => "Open Recent",
        (Locale::En, Key::NoRecentProjects) => "No Recent Projects",
        (Locale::En, Key::ImportMenu) => "Import…",
        (Locale::En, Key::ImportPmxMenu) => "PMX…",
        (Locale::En, Key::SaveProjectMenu) => "Save",
        (Locale::En, Key::SaveAsProjectMenu) => "Save As…",
        (Locale::En, Key::InspectShaderMenu) => "Inspect WGSL Shader…",
        (Locale::En, Key::EditMenu) => "Edit",
        (Locale::En, Key::ViewMenu) => "View",
        (Locale::En, Key::WindowMenu) => "Window",
        (Locale::En, Key::BringAllToFront) => "Bring All to Front",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_has_both_fallbacks() {
        let tables = [
            include_str!("../resources/en.lproj/Localizable.strings"),
            include_str!("../resources/zh-Hans.lproj/Localizable.strings"),
        ];
        for &key in KEYS {
            assert!(!key.resource_key().is_empty());
            assert!(!fallback(Locale::En, key).is_empty());
            assert!(!fallback(Locale::ZhHans, key).is_empty());
            for table in tables {
                assert!(table.contains(&format!("\"{}\" =", key.resource_key())));
            }
        }
    }
}
