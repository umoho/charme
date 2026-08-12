//! Development-only startup states for quickly inspecting the native editor.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum DebugState {
    #[default]
    Startup,
    Editor,
    LayoutDefault,
}

pub(crate) fn state_from_args() -> DebugState {
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--debug" {
            return match arguments.next().as_deref() {
                Some("editor") => DebugState::Editor,
                Some("layout-default") => DebugState::LayoutDefault,
                Some(unknown) => {
                    eprintln!(
                        "unknown debug state '{unknown}', expected 'editor' or 'layout-default'"
                    );
                    DebugState::Startup
                }
                None => {
                    eprintln!(
                        "missing debug state after --debug, expected 'editor' or 'layout-default'"
                    );
                    DebugState::Startup
                }
            };
        }
    }

    DebugState::Startup
}
