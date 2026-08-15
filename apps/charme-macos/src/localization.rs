use std::{fmt::Display, sync::OnceLock};

#[cfg(test)]
use std::collections::BTreeSet;

use cacao::{
    foundation::{NSString, id, nil},
    objc::{class, msg_send, sel, sel_impl},
};

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
    AppName,
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
    RecentProjects,
    ProjectFallback,
    Scene,
    Hierarchy,
    EmptyScene,
    Materials,
    EmptyMaterials,
    Inspector,
    InspectorBody,
    RendererStarting,
    ProjectOpened,
    WaitingCharacter,
    LoadingMaterials,
    LoadingPmxTextures,
    InspectingShader,
    ShaderError,
    ReflectionFailed,
    MaterialInspector,
    MaterialSubtitle,
    MaterialParameters,
    InspectorNoParameters,
    MaterialSource,
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
    NoRecentProjectsHint,
    RecentProjectRow,
    ImportMenu,
    ImportPmxMenu,
    SaveProjectMenu,
    SaveAsProjectMenu,
    InspectShaderMenu,
    EditMenu,
    ViewMenu,
    WindowMenu,
    BringAllToFront,
    EnterFullScreen,
    SaveProjectFailed,
    InvalidProjectFile,
    OpenProjectFailed,
    UntitledCharacter,
    UntitledProject,
    Unchanged,
    UnsavedChanges,
    RecentProjectTitle,
    FrameStatus,
    LoadingPmx,
    ShaderErrorDetails,
    ShaderSummary,
    ShaderSummaryWithNonScalar,
    ShaderReflected,
    ParameterUpdated,
    ParameterWaiting,
    PmxLoadFailed,
    ParameterRejected,
    SceneSummary,
    MaterialSlotListItem,
    MoreMaterials,
    SourceSlot,
    DiffuseTexture,
    SphereTexture,
    ToonTexture,
    MissingValue,
    SceneLoaded,
    SceneLoadedWithWarnings,
    FramePixelFormatUnsupported,
    ColorSpaceUnavailable,
    FrameImageCreationFailed,
    ShaderReadFailed,
    ShaderCompositionFailed,
    RendererFailed,
);

/// A compile-time localization catalog generated from one `.lproj` directory.
trait LanguageCatalog: Sync {
    fn identifier(&self) -> &'static str;
    fn text(&self, key: Key) -> &'static str;
}

include!(concat!(env!("OUT_DIR"), "/language_catalogs.rs"));

#[derive(Debug)]
struct Localization {
    strings: Vec<String>,
}

static LOCALIZATION: OnceLock<Localization> = OnceLock::new();

pub(crate) fn text(key: Key) -> &'static str {
    &localization().strings[key as usize]
}

/// Expands named placeholders such as `{path}` without imposing an argument
/// order on translations.
pub(crate) fn format(key: Key, arguments: &[(&str, &dyn Display)]) -> String {
    let template = text(key);
    let mut output = String::with_capacity(template.len());
    let mut remainder = template;

    while let Some(open) = remainder.find('{') {
        let after_open = &remainder[open + 1..];
        let Some(close) = after_open.find('}') else {
            break;
        };
        output.push_str(&remainder[..open]);
        let name = &after_open[..close];
        if let Some((_, value)) = arguments.iter().find(|(candidate, _)| *candidate == name) {
            use std::fmt::Write as _;
            let _ = write!(output, "{value}");
        } else {
            output.push_str(&remainder[open..open + close + 2]);
        }
        remainder = &after_open[close + 1..];
    }
    output.push_str(remainder);
    output
}

fn localization() -> &'static Localization {
    LOCALIZATION.get_or_init(load)
}

/// Uses the main bundle and its effective localization when resources are
/// available. A direct `cargo run` selects one of the generated language
/// implementations with the same native NSBundle language negotiation API.
fn load() -> Localization {
    unsafe {
        let bundle: id = msg_send![class!(NSBundle), mainBundle];
        let bundled = has_localizable_strings(bundle);
        let catalog = preferred_catalog(bundle, bundled).unwrap_or(DEVELOPMENT_CATALOG);

        let strings = KEYS
            .iter()
            .map(|&key| {
                let fallback = catalog.text(key);
                if bundled {
                    bundle_text(bundle, key, fallback)
                } else {
                    fallback.to_owned()
                }
            })
            .collect();

        Localization { strings }
    }
}

unsafe fn has_localizable_strings(bundle: id) -> bool {
    let resource = NSString::new("Localizable");
    let extension = NSString::new("strings");
    let path: id = unsafe { msg_send![bundle, pathForResource: &*resource ofType: &*extension] };
    !path.is_null()
}

unsafe fn preferred_catalog(bundle: id, bundled: bool) -> Option<&'static dyn LanguageCatalog> {
    let preferred: id = if bundled {
        unsafe { msg_send![bundle, preferredLocalizations] }
    } else {
        let available: id = unsafe {
            msg_send![class!(NSMutableArray), arrayWithCapacity: LANGUAGE_CATALOGS.len()]
        };
        for catalog in LANGUAGE_CATALOGS {
            let identifier = NSString::new(catalog.identifier());
            let _: () = unsafe { msg_send![available, addObject: &*identifier] };
        }
        let preferences: id = unsafe { msg_send![class!(NSLocale), preferredLanguages] };
        unsafe {
            msg_send![class!(NSBundle),
                preferredLocalizationsFromArray: available
                forPreferences: preferences
            ]
        }
    };

    unsafe { first_string(preferred) }.and_then(catalog_for)
}

unsafe fn first_string(array: id) -> Option<String> {
    if array.is_null() {
        return None;
    }
    let first: id = unsafe { msg_send![array, firstObject] };
    (!first.is_null()).then(|| NSString::retain(first).to_string())
}

fn catalog_for(identifier: String) -> Option<&'static dyn LanguageCatalog> {
    LANGUAGE_CATALOGS.iter().copied().find(|catalog| {
        catalog.identifier() == identifier || catalog.identifier().eq_ignore_ascii_case(&identifier)
    })
}

unsafe fn bundle_text(bundle: id, key: Key, fallback: &str) -> String {
    let resource_key = NSString::new(key.resource_key());
    let fallback = NSString::new(fallback);
    let value: id = unsafe {
        msg_send![bundle,
            localizedStringForKey: &*resource_key
            value: &*fallback
            table: nil
        ]
    };
    NSString::retain(value).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_catalogs_are_complete_and_unique() {
        let mut identifiers = BTreeSet::new();
        for catalog in LANGUAGE_CATALOGS {
            assert!(identifiers.insert(catalog.identifier()));
            for &key in KEYS {
                assert!(!catalog.text(key).is_empty());
            }
        }
        assert_eq!(DEVELOPMENT_CATALOG.identifier(), "en");
    }

    #[test]
    fn generated_catalogs_can_be_found_by_identifier() {
        for catalog in LANGUAGE_CATALOGS {
            let found = catalog_for(catalog.identifier().to_owned())
                .expect("generated catalog must be registered");
            assert_eq!(found.identifier(), catalog.identifier());
        }
    }
}
