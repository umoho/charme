//! Viewport tool abstraction shared by native frontends.
//!
//! A tool packages the behavior and presentation of one viewport
//! interaction mode. Tools are registered through [`ToolRegistry`] so that
//! frontends can render a palette, cycle with the keyboard and validate menu
//! state without knowing individual tool implementations.

use charme_renderer::ViewportSelectionAction;

/// Stable identifier of an active viewport tool.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum ViewportToolId {
    /// Click selects one imported material slot.
    #[default]
    SelectMaterialSlot,
    /// Click selects one or more source primitives.
    SelectPrimitive,
}

/// Semantic domain a tool applies click results to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionDomain {
    /// One imported PMX material slot.
    MaterialSlot,
    /// One source primitive (and its split components).
    Primitive,
}

/// Platform-neutral snapshot of modifier keys on a viewport click.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ModifierState {
    /// Command (⌘) key held.
    pub command: bool,
    /// Option (⌥) key held.
    pub option: bool,
    /// Shift (⇧) key held.
    pub shift: bool,
    /// Control (⌃) key held.
    pub control: bool,
}

/// Platform-neutral key-equivalent description of a tool shortcut.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolShortcut {
    /// Key equivalent character, for example `"1"`.
    pub key: &'static str,
    /// Modifier mask for the key equivalent.
    pub modifiers: ModifierState,
}

/// Behavior and presentation descriptor of one viewport tool.
///
/// Implementations are stateless: transient interaction state stays in the
/// native frontend, while finished interactions flow back as semantic
/// [`crate::WorkspaceAction`]s.
pub trait ViewportToolDescriptor: Send + Sync + 'static {
    /// Stable tool identity used in state and messages.
    fn id(&self) -> ViewportToolId;

    /// Domain viewport clicks are applied to while this tool is active.
    fn selection_domain(&self) -> SelectionDomain;

    /// Whether the tool allows multiple simultaneous targets.
    fn allows_multiple_selection(&self) -> bool;

    /// Maps modifier state to the viewport selection operation.
    ///
    /// The default matches the shared convention: option removes, command
    /// toggles, a plain click replaces.
    fn selection_action(&self, modifiers: ModifierState) -> ViewportSelectionAction {
        default_selection_action(modifiers)
    }

    /// Localization resource key of the tool title.
    fn title_key(&self) -> &'static str;

    /// Localization resource key of the palette tooltip.
    fn tooltip_key(&self) -> &'static str;

    /// Localization resource key of the status-bar hint.
    fn status_hint_key(&self) -> &'static str;

    /// SF Symbol name used by the macOS palette.
    fn symbol_name(&self) -> &'static str;

    /// Direct-selection shortcut, when one exists.
    fn shortcut(&self) -> Option<ToolShortcut>;
}

/// Default click mapping shared by the standard selection tools.
pub fn default_selection_action(modifiers: ModifierState) -> ViewportSelectionAction {
    if modifiers.option {
        ViewportSelectionAction::Remove
    } else if modifiers.command {
        ViewportSelectionAction::Toggle
    } else {
        ViewportSelectionAction::Replace
    }
}

/// Tool that selects one imported material slot.
#[derive(Clone, Copy, Debug, Default)]
pub struct SelectMaterialSlotTool;

impl ViewportToolDescriptor for SelectMaterialSlotTool {
    fn id(&self) -> ViewportToolId {
        ViewportToolId::SelectMaterialSlot
    }

    fn selection_domain(&self) -> SelectionDomain {
        SelectionDomain::MaterialSlot
    }

    fn allows_multiple_selection(&self) -> bool {
        false
    }

    fn title_key(&self) -> &'static str {
        "MaterialSlotSelectionLevel"
    }

    fn tooltip_key(&self) -> &'static str {
        "ToolMaterialSlotTooltip"
    }

    fn status_hint_key(&self) -> &'static str {
        "ToolMaterialSlotHint"
    }

    fn symbol_name(&self) -> &'static str {
        "paintbrush"
    }

    fn shortcut(&self) -> Option<ToolShortcut> {
        Some(ToolShortcut {
            key: "1",
            modifiers: ModifierState {
                command: true,
                ..ModifierState::default()
            },
        })
    }
}

/// Tool that selects one or more source primitives.
#[derive(Clone, Copy, Debug, Default)]
pub struct SelectPrimitiveTool;

impl ViewportToolDescriptor for SelectPrimitiveTool {
    fn id(&self) -> ViewportToolId {
        ViewportToolId::SelectPrimitive
    }

    fn selection_domain(&self) -> SelectionDomain {
        SelectionDomain::Primitive
    }

    fn allows_multiple_selection(&self) -> bool {
        true
    }

    fn title_key(&self) -> &'static str {
        "PrimitiveSelectionLevel"
    }

    fn tooltip_key(&self) -> &'static str {
        "ToolPrimitiveTooltip"
    }

    fn status_hint_key(&self) -> &'static str {
        "ToolPrimitiveHint"
    }

    fn symbol_name(&self) -> &'static str {
        "triangle"
    }

    fn shortcut(&self) -> Option<ToolShortcut> {
        Some(ToolShortcut {
            key: "2",
            modifiers: ModifierState {
                command: true,
                ..ModifierState::default()
            },
        })
    }
}

/// Ordered registry of viewport tools.
///
/// Registration order is the palette display order and the Tab-cycle order.
#[derive(Default)]
pub struct ToolRegistry {
    tools: Vec<Box<dyn ViewportToolDescriptor>>,
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ToolRegistry")
            .field("tool_count", &self.tools.len())
            .finish()
    }
}

impl ToolRegistry {
    /// Creates the registry with the standard selection tools.
    pub fn standard() -> Self {
        Self {
            tools: vec![
                Box::new(SelectMaterialSlotTool),
                Box::new(SelectPrimitiveTool),
            ],
        }
    }

    /// Appends a tool after all previously registered tools.
    pub fn register(&mut self, tool: impl ViewportToolDescriptor + 'static) {
        self.tools.push(Box::new(tool));
    }

    /// Returns registered tools in display order.
    pub fn tools(&self) -> impl Iterator<Item = &dyn ViewportToolDescriptor> {
        self.tools.iter().map(|tool| tool.as_ref())
    }

    /// Looks up a tool by its stable identifier.
    pub fn by_id(&self, id: ViewportToolId) -> Option<&dyn ViewportToolDescriptor> {
        self.tools
            .iter()
            .map(|tool| tool.as_ref())
            .find(|tool| tool.id() == id)
    }

    /// Returns the tool that follows `current` in registration order,
    /// cycling back to the first tool after the last one.
    ///
    /// Returns `current` when the registry has no tools.
    pub fn next_after(&self, current: ViewportToolId) -> ViewportToolId {
        let position = self.tools.iter().position(|tool| tool.id() == current);
        let Some(position) = position else {
            return self.tools.first().map(|tool| tool.id()).unwrap_or(current);
        };
        self.tools
            .get((position + 1) % self.tools.len())
            .map(|tool| tool.id())
            .unwrap_or(current)
    }

    /// Builds the palette presentation model for the active tool.
    pub fn palette_model(&self, active: ViewportToolId) -> ToolPaletteModel {
        ToolPaletteModel {
            entries: self
                .tools
                .iter()
                .map(|tool| ToolPaletteEntry {
                    id: tool.id(),
                    symbol_name: tool.symbol_name(),
                    title_key: tool.title_key(),
                    tooltip_key: tool.tooltip_key(),
                    shortcut: tool.shortcut(),
                    active: tool.id() == active,
                })
                .collect(),
        }
    }
}

/// One palette button presentation.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolPaletteEntry {
    /// Tool identity the button activates.
    pub id: ViewportToolId,
    /// SF Symbol name for the button image.
    pub symbol_name: &'static str,
    /// Localization resource key of the button title.
    pub title_key: &'static str,
    /// Localization resource key of the button tooltip.
    pub tooltip_key: &'static str,
    /// Direct-selection shortcut, shown in the tooltip.
    pub shortcut: Option<ToolShortcut>,
    /// Whether this entry is the active tool.
    pub active: bool,
}

/// Pure presentation model of the viewport tool palette.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolPaletteModel {
    /// Tool entries in display order.
    pub entries: Vec<ToolPaletteEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_action_maps_modifiers_to_operations() {
        assert_eq!(
            default_selection_action(ModifierState::default()),
            ViewportSelectionAction::Replace
        );
        assert_eq!(
            default_selection_action(ModifierState {
                command: true,
                ..ModifierState::default()
            }),
            ViewportSelectionAction::Toggle
        );
        assert_eq!(
            default_selection_action(ModifierState {
                option: true,
                ..ModifierState::default()
            }),
            ViewportSelectionAction::Remove
        );
    }

    #[test]
    fn standard_registry_orders_tools_and_cycles() {
        let registry = ToolRegistry::standard();
        let ids = registry.tools().map(|tool| tool.id()).collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                ViewportToolId::SelectMaterialSlot,
                ViewportToolId::SelectPrimitive
            ]
        );
        assert_eq!(
            registry.next_after(ViewportToolId::SelectMaterialSlot),
            ViewportToolId::SelectPrimitive
        );
        assert_eq!(
            registry.next_after(ViewportToolId::SelectPrimitive),
            ViewportToolId::SelectMaterialSlot
        );
    }

    #[test]
    fn palette_model_marks_only_the_active_tool() {
        let registry = ToolRegistry::standard();
        let model = registry.palette_model(ViewportToolId::SelectPrimitive);
        assert_eq!(model.entries.len(), 2);
        assert!(
            model
                .entries
                .iter()
                .all(|entry| { entry.active == (entry.id == ViewportToolId::SelectPrimitive) })
        );
        assert_eq!(
            model.entries[1].symbol_name,
            SelectPrimitiveTool.symbol_name()
        );
    }

    #[test]
    fn descriptors_expose_domains_and_shortcuts() {
        let registry = ToolRegistry::standard();
        let slot = registry.by_id(ViewportToolId::SelectMaterialSlot).unwrap();
        assert_eq!(slot.selection_domain(), SelectionDomain::MaterialSlot);
        assert!(!slot.allows_multiple_selection());
        let primitive = registry.by_id(ViewportToolId::SelectPrimitive).unwrap();
        assert_eq!(primitive.selection_domain(), SelectionDomain::Primitive);
        assert!(primitive.allows_multiple_selection());
        assert_eq!(primitive.shortcut().unwrap().key, "2");
    }
}
