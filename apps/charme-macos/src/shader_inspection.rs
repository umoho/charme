use std::{fs, path::PathBuf, thread};

use cacao::appkit::App;
use charme_application::{ApplicationEvent, inspect_shader_source};

use crate::{
    app::{CharmeApp, Message},
    localization::{self, Key},
};

pub(crate) use charme_application::{ParameterControlKind, ParameterControlSpec, ShaderInspection};

/// Inspects a WGSL file off the main UI thread.
pub(crate) fn inspect_shader(path: PathBuf) {
    thread::Builder::new()
        .name("charme-shader-inspection".to_owned())
        .spawn(move || {
            let result = fs::read_to_string(&path)
                .map_err(|error| {
                    tracing::error!(
                        path = %path.display(),
                        error = %error,
                        "Failed to read Shader"
                    );
                    localization::format(Key::ShaderReadFailed, &[("path", &path.display())])
                })
                .and_then(|source| reflect_source(path.clone(), &source));
            App::<CharmeApp, Message>::dispatch_main(Message::Application(
                ApplicationEvent::ShaderInspected { path, result },
            ));
        })
        .expect("failed to start shader inspection worker");
}

fn reflect_source(path: PathBuf, source: &str) -> Result<ShaderInspection, String> {
    inspect_shader_source(path.clone(), source).map_err(|error| {
        tracing::error!(
            path = %path.display(),
            error = %error,
            "Failed to inspect WGSL Shader"
        );
        localization::text(Key::ShaderCompositionFailed).to_owned()
    })
}
