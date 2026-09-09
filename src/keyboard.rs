use std::collections::BTreeMap;
use std::rc::Rc;

use gpui_kit::{
    Action, App, DummyKeyboardMapper, KeyBinding, KeyBindingContextPredicate, KeyContext,
    Keystroke, actions,
};

use crate::state::KeybindingSettings;

actions!(
    openmango,
    [
        QuitApp,
        NewConnection,
        OpenSelection,
        OpenSelectionPreview,
        EditConnection,
        DisconnectConnection,
        CopySelectionName,
        CopyConnectionUri,
        CopyTreeItem,
        PasteTreeItem,
        RenameCollection,
        DeleteSelection,
        TransferExport,
        TransferImport,
        TransferCopy,
        RunTransfer,
        CancelTransfer,
        SaveTransferQuery,
        CloseTransferQueryModal,
        CreateDatabase,
        CreateCollection,
        CreateIndex,
        InsertDocument,
        RefreshView,
        CloseTab,
        CloseEditorWindow,
        NextTab,
        PrevTab,
        SelectTab1,
        SelectTab2,
        SelectTab3,
        SelectTab4,
        SelectTab5,
        SelectTab6,
        SelectTab7,
        SelectTab8,
        SelectTab9,
        FindInResults,
        CloseSearch,
        SaveDocument,
        DiscardDocumentChanges,
        EditDocumentJson,
        CopyDocumentJson,
        DuplicateDocument,
        DeleteDocument,
        DeleteCollection,
        DeleteDatabase,
        DeleteConnection,
        PasteDocuments,
        EditValueType,
        RenameField,
        RemoveSelectedField,
        AddField,
        AddElement,
        RemoveMatchingValues,
        CopyValue,
        CopyKey,
        ShowDocumentsSubview,
        ShowIndexesSubview,
        ShowStatsSubview,
        ShowAggregationSubview,
        ShowSchemaSubview,
        ShowHistorySubview,
        RunAggregation,
        FormatAggregationStage,
        ClearAggregationStage,
        SelectPrevAggregationStage,
        SelectNextAggregationStage,
        MoveAggregationStageUp,
        MoveAggregationStageDown,
        DuplicateAggregationStage,
        DeleteAggregationStage,
        ToggleAggregationStageEnabled,
        FindInSidebar,
        CloseSidebarSearch,
        OpenActionBar,
        OpenQueryLibrary,
        OpenSettings,
        OpenForge,
        ToggleAiPanel,
        RunForgeAll,
        RunForgeSelectionOrStatement,
        CancelForgeRun,
        ClearForgeOutput,
        FocusForgeEditor,
        FocusForgeOutput,
        AcceptForgeCompletion,
        TriggerForgeCompletion,
        PreviousForgeCompletion,
        NextForgeCompletion,
        InsertForgeNewline,
        DeleteForgeWordBackward,
        DeleteForgeWordForward,
        MoveForgeWordBackward,
        MoveForgeWordForward,
        SelectForgeWordBackward,
        SelectForgeWordForward,
        FindInForgeOutput,
        CopyAs,
        CopyAsJson,
        CopyAsJsonLines,
        CopyAsCsv,
        CopyAsMarkdown,
        CopyAsTsv,
        FocusSidebar,
        FocusContent,
        NextSearchMatch,
        PrevSearchMatch,
        DownloadUpdate,
        InstallUpdate,
    ]
);

const DOCUMENT_EDIT_CONTEXT: &str = "Documents && !Input && !Aggregation";
const FOCUS_CONTENT_KEYS: [&str; 2] = ["cmd-shift-1", "ctrl-shift-1"];

pub fn bind_keymap(cx: &mut App, settings: &KeybindingSettings) {
    let (bindings, errors) = effective_keybindings(settings);
    for error in errors {
        log::error!("Ignoring invalid keybinding override: {error}");
    }
    cx.bind_keys(bindings);
}

fn default_keybindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("enter", OpenSelection, Some("Sidebar")),
        KeyBinding::new("return", OpenSelection, Some("Sidebar")),
        KeyBinding::new("cmd-enter", OpenSelectionPreview, Some("Sidebar")),
        KeyBinding::new("ctrl-enter", OpenSelectionPreview, Some("Sidebar")),
        KeyBinding::new("cmd-shift-f", OpenForge, Some("Sidebar")),
        KeyBinding::new("ctrl-shift-f", OpenForge, Some("Sidebar")),
        KeyBinding::new("cmd-e", EditConnection, Some("Sidebar")),
        KeyBinding::new("ctrl-e", EditConnection, Some("Sidebar")),
        KeyBinding::new("cmd-shift-d", DisconnectConnection, Some("Sidebar")),
        KeyBinding::new("ctrl-shift-d", DisconnectConnection, Some("Sidebar")),
        KeyBinding::new("cmd-c", CopyTreeItem, Some("Sidebar && !Input")),
        KeyBinding::new("ctrl-c", CopyTreeItem, Some("Sidebar && !Input")),
        KeyBinding::new("cmd-v", PasteTreeItem, Some("Sidebar && !Input")),
        KeyBinding::new("ctrl-v", PasteTreeItem, Some("Sidebar && !Input")),
        KeyBinding::new("cmd-shift-c", CopyConnectionUri, Some("Sidebar && !Input")),
        KeyBinding::new("ctrl-shift-c", CopyConnectionUri, Some("Sidebar && !Input")),
        KeyBinding::new("f2", RenameCollection, Some("Sidebar && !Input")),
        KeyBinding::new("backspace", DeleteSelection, Some("Sidebar && !Input")),
        KeyBinding::new("delete", DeleteSelection, Some("Sidebar && !Input")),
        KeyBinding::new("cmd-shift-n", CreateDatabase, Some("Sidebar")),
        KeyBinding::new("ctrl-shift-n", CreateDatabase, Some("Sidebar")),
        KeyBinding::new("cmd-n", CreateCollection, Some("Sidebar")),
        KeyBinding::new("ctrl-n", CreateCollection, Some("Sidebar")),
        KeyBinding::new("cmd-alt-e", TransferExport, Some("Sidebar && !Input")),
        KeyBinding::new("ctrl-alt-e", TransferExport, Some("Sidebar && !Input")),
        KeyBinding::new("cmd-alt-i", TransferImport, Some("Sidebar && !Input")),
        KeyBinding::new("ctrl-alt-i", TransferImport, Some("Sidebar && !Input")),
        KeyBinding::new("cmd-alt-c", TransferCopy, Some("Sidebar && !Input")),
        KeyBinding::new("ctrl-alt-c", TransferCopy, Some("Sidebar && !Input")),
        KeyBinding::new(
            "cmd-enter",
            RunTransfer,
            Some("Transfer && !TransferRunning && !TransferQueryModal"),
        ),
        KeyBinding::new(
            "ctrl-enter",
            RunTransfer,
            Some("Transfer && !TransferRunning && !TransferQueryModal"),
        ),
        KeyBinding::new(
            "escape",
            CancelTransfer,
            Some("Transfer && TransferRunning && !TransferQueryModal"),
        ),
        KeyBinding::new("cmd-enter", SaveTransferQuery, Some("Transfer && TransferQueryModal")),
        KeyBinding::new("ctrl-enter", SaveTransferQuery, Some("Transfer && TransferQueryModal")),
        KeyBinding::new("escape", CloseTransferQueryModal, Some("Transfer && TransferQueryModal")),
        KeyBinding::new("cmd-alt-f", OpenForge, Some("Workspace")),
        KeyBinding::new("ctrl-alt-f", OpenForge, Some("Workspace")),
        KeyBinding::new("cmd-enter", RunForgeAll, Some("ForgeView")),
        KeyBinding::new("cmd-return", RunForgeAll, Some("ForgeView")),
        KeyBinding::new("secondary-enter", RunForgeAll, Some("ForgeView")),
        KeyBinding::new("secondary-return", RunForgeAll, Some("ForgeView")),
        KeyBinding::new("ctrl-enter", RunForgeAll, Some("ForgeView")),
        KeyBinding::new("ctrl-return", RunForgeAll, Some("ForgeView")),
        KeyBinding::new("cmd-shift-enter", RunForgeSelectionOrStatement, Some("ForgeView")),
        KeyBinding::new("cmd-shift-return", RunForgeSelectionOrStatement, Some("ForgeView")),
        KeyBinding::new("secondary-shift-enter", RunForgeSelectionOrStatement, Some("ForgeView")),
        KeyBinding::new("secondary-shift-return", RunForgeSelectionOrStatement, Some("ForgeView")),
        KeyBinding::new("ctrl-shift-enter", RunForgeSelectionOrStatement, Some("ForgeView")),
        KeyBinding::new("ctrl-shift-return", RunForgeSelectionOrStatement, Some("ForgeView")),
        KeyBinding::new("cmd-enter", RunForgeAll, Some("ForgeView > Input")),
        KeyBinding::new("cmd-return", RunForgeAll, Some("ForgeView > Input")),
        KeyBinding::new("secondary-enter", RunForgeAll, Some("ForgeView > Input")),
        KeyBinding::new("secondary-return", RunForgeAll, Some("ForgeView > Input")),
        KeyBinding::new("ctrl-enter", RunForgeAll, Some("ForgeView > Input")),
        KeyBinding::new("ctrl-return", RunForgeAll, Some("ForgeView > Input")),
        KeyBinding::new("cmd-shift-enter", RunForgeSelectionOrStatement, Some("ForgeView > Input")),
        KeyBinding::new(
            "cmd-shift-return",
            RunForgeSelectionOrStatement,
            Some("ForgeView > Input"),
        ),
        KeyBinding::new(
            "secondary-shift-enter",
            RunForgeSelectionOrStatement,
            Some("ForgeView > Input"),
        ),
        KeyBinding::new(
            "secondary-shift-return",
            RunForgeSelectionOrStatement,
            Some("ForgeView > Input"),
        ),
        KeyBinding::new(
            "ctrl-shift-enter",
            RunForgeSelectionOrStatement,
            Some("ForgeView > Input"),
        ),
        KeyBinding::new(
            "ctrl-shift-return",
            RunForgeSelectionOrStatement,
            Some("ForgeView > Input"),
        ),
        KeyBinding::new("escape", CancelForgeRun, Some("ForgeView")),
        KeyBinding::new("cmd-alt-k", ClearForgeOutput, Some("ForgeView")),
        KeyBinding::new("ctrl-alt-k", ClearForgeOutput, Some("ForgeView")),
        KeyBinding::new("cmd-alt-k", ClearForgeOutput, Some("ForgeView > Input")),
        KeyBinding::new("ctrl-alt-k", ClearForgeOutput, Some("ForgeView > Input")),
        KeyBinding::new("cmd-e", FocusForgeEditor, Some("ForgeView")),
        KeyBinding::new("ctrl-e", FocusForgeEditor, Some("ForgeView")),
        KeyBinding::new("cmd-o", FocusForgeOutput, Some("ForgeView")),
        KeyBinding::new("ctrl-o", FocusForgeOutput, Some("ForgeView")),
        KeyBinding::new("tab", AcceptForgeCompletion, Some("ForgeView > Input")),
        KeyBinding::new("ctrl-space", TriggerForgeCompletion, Some("ForgeView > Input")),
        KeyBinding::new("up", PreviousForgeCompletion, Some("ForgeView > Input")),
        KeyBinding::new("down", NextForgeCompletion, Some("ForgeView > Input")),
        KeyBinding::new("enter", InsertForgeNewline, Some("ForgeView > Input")),
        KeyBinding::new("return", InsertForgeNewline, Some("ForgeView > Input")),
        KeyBinding::new(
            if cfg!(target_os = "macos") { "alt-backspace" } else { "ctrl-backspace" },
            DeleteForgeWordBackward,
            Some("ForgeView > Input"),
        ),
        KeyBinding::new(
            if cfg!(target_os = "macos") { "alt-delete" } else { "ctrl-delete" },
            DeleteForgeWordForward,
            Some("ForgeView > Input"),
        ),
        KeyBinding::new(
            if cfg!(target_os = "macos") { "alt-left" } else { "ctrl-left" },
            MoveForgeWordBackward,
            Some("ForgeView > Input"),
        ),
        KeyBinding::new(
            if cfg!(target_os = "macos") { "alt-right" } else { "ctrl-right" },
            MoveForgeWordForward,
            Some("ForgeView > Input"),
        ),
        KeyBinding::new(
            if cfg!(target_os = "macos") { "alt-shift-left" } else { "ctrl-shift-left" },
            SelectForgeWordBackward,
            Some("ForgeView > Input"),
        ),
        KeyBinding::new(
            if cfg!(target_os = "macos") { "alt-shift-right" } else { "ctrl-shift-right" },
            SelectForgeWordForward,
            Some("ForgeView > Input"),
        ),
        KeyBinding::new("cmd-f", FindInForgeOutput, Some("ForgeView && !Input")),
        KeyBinding::new("ctrl-f", FindInForgeOutput, Some("ForgeView && !Input")),
        KeyBinding::new("cmd-n", InsertDocument, Some("Documents && !Indexes && !Stats")),
        KeyBinding::new("ctrl-n", InsertDocument, Some("Documents && !Indexes && !Stats")),
        KeyBinding::new("cmd-n", CreateIndex, Some("Documents && Indexes")),
        KeyBinding::new("ctrl-n", CreateIndex, Some("Documents && Indexes")),
        KeyBinding::new("cmd-n", CreateCollection, Some("Database")),
        KeyBinding::new("ctrl-n", CreateCollection, Some("Database")),
        KeyBinding::new("cmd-n", NewConnection, Some("Workspace && Welcome")),
        KeyBinding::new("ctrl-n", NewConnection, Some("Workspace && Welcome")),
        KeyBinding::new("cmd-n", NewConnection, Some("Workspace && Databases")),
        KeyBinding::new("ctrl-n", NewConnection, Some("Workspace && Databases")),
        KeyBinding::new("cmd-n", NewConnection, Some("Workspace && Collections")),
        KeyBinding::new("ctrl-n", NewConnection, Some("Workspace && Collections")),
        KeyBinding::new("cmd-shift-n", CreateDatabase, Some("Workspace && !Documents")),
        KeyBinding::new("ctrl-shift-n", CreateDatabase, Some("Workspace && !Documents")),
        KeyBinding::new("cmd-w", CloseTab, Some("Workspace")),
        KeyBinding::new("ctrl-w", CloseTab, Some("Workspace")),
        KeyBinding::new("cmd-w", CloseEditorWindow, Some("JsonEditorWindow")),
        KeyBinding::new("ctrl-w", CloseEditorWindow, Some("JsonEditorWindow")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Workspace")),
        KeyBinding::new("ctrl-shift-tab", PrevTab, Some("Workspace")),
        KeyBinding::new("cmd-1", SelectTab1, Some("Workspace")),
        KeyBinding::new("ctrl-1", SelectTab1, Some("Workspace")),
        KeyBinding::new("cmd-2", SelectTab2, Some("Workspace")),
        KeyBinding::new("ctrl-2", SelectTab2, Some("Workspace")),
        KeyBinding::new("cmd-3", SelectTab3, Some("Workspace")),
        KeyBinding::new("ctrl-3", SelectTab3, Some("Workspace")),
        KeyBinding::new("cmd-4", SelectTab4, Some("Workspace")),
        KeyBinding::new("ctrl-4", SelectTab4, Some("Workspace")),
        KeyBinding::new("cmd-5", SelectTab5, Some("Workspace")),
        KeyBinding::new("ctrl-5", SelectTab5, Some("Workspace")),
        KeyBinding::new("cmd-6", SelectTab6, Some("Workspace")),
        KeyBinding::new("ctrl-6", SelectTab6, Some("Workspace")),
        KeyBinding::new("cmd-7", SelectTab7, Some("Workspace")),
        KeyBinding::new("ctrl-7", SelectTab7, Some("Workspace")),
        KeyBinding::new("cmd-8", SelectTab8, Some("Workspace")),
        KeyBinding::new("ctrl-8", SelectTab8, Some("Workspace")),
        KeyBinding::new("cmd-9", SelectTab9, Some("Workspace")),
        KeyBinding::new("ctrl-9", SelectTab9, Some("Workspace")),
        KeyBinding::new("cmd-r", RefreshView, Some("Workspace")),
        KeyBinding::new("ctrl-r", RefreshView, Some("Workspace")),
        KeyBinding::new("cmd-q", QuitApp, Some("Workspace")),
        KeyBinding::new("ctrl-q", QuitApp, Some("Workspace")),
        KeyBinding::new("cmd-f", FindInResults, Some("Documents")),
        KeyBinding::new("ctrl-f", FindInResults, Some("Documents")),
        KeyBinding::new("escape", CloseSearch, Some("Documents")),
        KeyBinding::new("escape", CloseSearch, Some("Documents && Input")),
        KeyBinding::new("cmd-f", FindInSidebar, Some("Sidebar")),
        KeyBinding::new("ctrl-f", FindInSidebar, Some("Sidebar")),
        KeyBinding::new("escape", CloseSidebarSearch, Some("Sidebar")),
        KeyBinding::new("cmd-s", SaveDocument, Some("Documents && !Input")),
        KeyBinding::new("ctrl-s", SaveDocument, Some("Documents && !Input")),
        KeyBinding::new("cmd-shift-s", DiscardDocumentChanges, Some("Documents && !Input")),
        KeyBinding::new("ctrl-shift-s", DiscardDocumentChanges, Some("Documents && !Input")),
        KeyBinding::new("cmd-e", EditDocumentJson, Some("Documents && !Input")),
        KeyBinding::new("ctrl-e", EditDocumentJson, Some("Documents && !Input")),
        KeyBinding::new("cmd-shift-j", CopyDocumentJson, Some("Documents && !Input")),
        KeyBinding::new("ctrl-shift-j", CopyDocumentJson, Some("Documents && !Input")),
        KeyBinding::new("cmd-d", DuplicateDocument, Some(DOCUMENT_EDIT_CONTEXT)),
        KeyBinding::new("ctrl-d", DuplicateDocument, Some(DOCUMENT_EDIT_CONTEXT)),
        KeyBinding::new("backspace", DeleteDocument, Some(DOCUMENT_EDIT_CONTEXT)),
        KeyBinding::new("delete", DeleteDocument, Some(DOCUMENT_EDIT_CONTEXT)),
        KeyBinding::new("cmd-shift-backspace", DeleteCollection, Some("Documents && !Input")),
        KeyBinding::new("ctrl-shift-backspace", DeleteCollection, Some("Documents && !Input")),
        KeyBinding::new("backspace", DeleteDatabase, Some("Database && !Input")),
        KeyBinding::new("delete", DeleteDatabase, Some("Database && !Input")),
        KeyBinding::new(
            "backspace",
            DeleteConnection,
            Some("Workspace && !Documents && !Database && !Input"),
        ),
        KeyBinding::new(
            "delete",
            DeleteConnection,
            Some("Workspace && !Documents && !Database && !Input"),
        ),
        KeyBinding::new("cmd-shift-v", PasteDocuments, Some("Documents && !Input")),
        KeyBinding::new("ctrl-shift-v", PasteDocuments, Some("Documents && !Input")),
        KeyBinding::new("alt-enter", EditValueType, Some("Documents && !Input")),
        KeyBinding::new("alt-return", EditValueType, Some("Documents && !Input")),
        KeyBinding::new("f2", RenameField, Some("Documents && !Input")),
        KeyBinding::new("alt-backspace", RemoveSelectedField, Some("Documents && !Input")),
        KeyBinding::new("alt-delete", RemoveSelectedField, Some("Documents && !Input")),
        KeyBinding::new("cmd-shift-a", AddField, Some("Documents && !Input")),
        KeyBinding::new("ctrl-shift-a", AddField, Some("Documents && !Input")),
        KeyBinding::new("cmd-alt-a", AddElement, Some("Documents && !Input")),
        KeyBinding::new("ctrl-alt-a", AddElement, Some("Documents && !Input")),
        KeyBinding::new("cmd-alt-backspace", RemoveMatchingValues, Some("Documents && !Input")),
        KeyBinding::new("ctrl-alt-backspace", RemoveMatchingValues, Some("Documents && !Input")),
        KeyBinding::new("cmd-c", CopyAs, Some("Documents && !Input && !Aggregation")),
        KeyBinding::new("ctrl-c", CopyAs, Some("Documents && !Input && !Aggregation")),
        KeyBinding::new("cmd-shift-c", CopyKey, Some("Documents && !Input")),
        KeyBinding::new("ctrl-shift-c", CopyKey, Some("Documents && !Input")),
        KeyBinding::new("cmd-k", OpenActionBar, Some("Workspace")),
        KeyBinding::new("ctrl-k", OpenActionBar, Some("Workspace")),
        KeyBinding::new("cmd-shift-h", OpenQueryLibrary, Some("Workspace")),
        KeyBinding::new("ctrl-shift-h", OpenQueryLibrary, Some("Workspace")),
        KeyBinding::new("cmd-,", OpenSettings, Some("Workspace")),
        KeyBinding::new("ctrl-,", OpenSettings, Some("Workspace")),
        KeyBinding::new("cmd-l", ToggleAiPanel, Some("Workspace")),
        KeyBinding::new("ctrl-l", ToggleAiPanel, Some("Workspace")),
        KeyBinding::new("cmd-0", FocusSidebar, Some("Workspace")),
        KeyBinding::new("ctrl-0", FocusSidebar, Some("Workspace")),
        KeyBinding::new(FOCUS_CONTENT_KEYS[0], FocusContent, Some("Workspace")),
        KeyBinding::new(FOCUS_CONTENT_KEYS[1], FocusContent, Some("Workspace")),
        KeyBinding::new("f3", NextSearchMatch, Some("Documents")),
        KeyBinding::new("shift-f3", PrevSearchMatch, Some("Documents")),
        KeyBinding::new("cmd-g", NextSearchMatch, Some("Documents")),
        KeyBinding::new("cmd-shift-g", PrevSearchMatch, Some("Documents")),
        KeyBinding::new("ctrl-g", NextSearchMatch, Some("Documents")),
        KeyBinding::new("ctrl-shift-g", PrevSearchMatch, Some("Documents")),
        KeyBinding::new("cmd-alt-1", ShowDocumentsSubview, Some("Documents")),
        KeyBinding::new("cmd-alt-2", ShowIndexesSubview, Some("Documents")),
        KeyBinding::new("cmd-alt-3", ShowStatsSubview, Some("Documents")),
        KeyBinding::new("cmd-alt-4", ShowAggregationSubview, Some("Documents")),
        KeyBinding::new("cmd-alt-5", ShowSchemaSubview, Some("Documents")),
        KeyBinding::new("ctrl-alt-1", ShowDocumentsSubview, Some("Documents")),
        KeyBinding::new("ctrl-alt-2", ShowIndexesSubview, Some("Documents")),
        KeyBinding::new("ctrl-alt-3", ShowStatsSubview, Some("Documents")),
        KeyBinding::new("ctrl-alt-4", ShowAggregationSubview, Some("Documents")),
        KeyBinding::new("ctrl-alt-5", ShowSchemaSubview, Some("Documents")),
        KeyBinding::new("cmd-enter", RunAggregation, Some("Documents && Aggregation")),
        KeyBinding::new("ctrl-enter", RunAggregation, Some("Documents && Aggregation")),
        KeyBinding::new("secondary-enter", RunAggregation, Some("Documents && Aggregation")),
        KeyBinding::new("cmd-shift-enter", RunAggregation, Some("Documents && Aggregation")),
        KeyBinding::new("ctrl-shift-enter", RunAggregation, Some("Documents && Aggregation")),
        KeyBinding::new("cmd-shift-f", FormatAggregationStage, Some("Documents && Aggregation")),
        KeyBinding::new("ctrl-shift-f", FormatAggregationStage, Some("Documents && Aggregation")),
        KeyBinding::new(
            "cmd-alt-k",
            ClearAggregationStage,
            Some("Documents && Aggregation && Input"),
        ),
        KeyBinding::new(
            "ctrl-alt-k",
            ClearAggregationStage,
            Some("Documents && Aggregation && Input"),
        ),
        KeyBinding::new(
            "cmd-shift-backspace",
            ClearAggregationStage,
            Some("Documents && Aggregation && Input"),
        ),
        KeyBinding::new(
            "ctrl-shift-backspace",
            ClearAggregationStage,
            Some("Documents && Aggregation && Input"),
        ),
        KeyBinding::new(
            "up",
            SelectPrevAggregationStage,
            Some("Documents && Aggregation && !Input"),
        ),
        KeyBinding::new(
            "down",
            SelectNextAggregationStage,
            Some("Documents && Aggregation && !Input"),
        ),
        KeyBinding::new("cmd-d", DuplicateAggregationStage, Some("Documents && Aggregation")),
        KeyBinding::new("ctrl-d", DuplicateAggregationStage, Some("Documents && Aggregation")),
        KeyBinding::new(
            "cmd-shift-e",
            ToggleAggregationStageEnabled,
            Some("Documents && Aggregation"),
        ),
        KeyBinding::new(
            "ctrl-shift-e",
            ToggleAggregationStageEnabled,
            Some("Documents && Aggregation && !Input"),
        ),
        KeyBinding::new(
            "delete",
            DeleteAggregationStage,
            Some("Documents && Aggregation && !Input"),
        ),
        KeyBinding::new(
            "backspace",
            DeleteAggregationStage,
            Some("Documents && Aggregation && !Input"),
        ),
        KeyBinding::new("cmd-shift-up", MoveAggregationStageUp, Some("Documents && Aggregation")),
        KeyBinding::new(
            "cmd-shift-down",
            MoveAggregationStageDown,
            Some("Documents && Aggregation"),
        ),
        KeyBinding::new("ctrl-shift-up", MoveAggregationStageUp, Some("Documents && Aggregation")),
        KeyBinding::new(
            "ctrl-shift-down",
            MoveAggregationStageDown,
            Some("Documents && Aggregation"),
        ),
    ]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeybindingIssueSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindingIssue {
    pub severity: KeybindingIssueSeverity,
    pub message: String,
    pub conflicting_binding_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindingCommand {
    pub id: String,
    pub label: String,
    pub category: String,
    pub context: String,
    pub default_shortcuts: Vec<String>,
    pub effective_shortcuts: Vec<String>,
    pub modified: bool,
    pub disabled: bool,
}

struct BindingSpec {
    id: String,
    action: Box<dyn Action>,
    label: String,
    category: String,
    context: Option<String>,
    defaults: Vec<String>,
}

pub fn keybinding_commands(settings: &KeybindingSettings) -> Vec<KeybindingCommand> {
    let mut commands = binding_specs()
        .into_iter()
        .map(|spec| {
            let override_value = settings.overrides.get(&spec.id);
            let effective_shortcuts = match override_value {
                None => spec.defaults.clone(),
                Some(None) => Vec::new(),
                Some(Some(shortcut)) => vec![
                    normalize_shortcut(shortcut).unwrap_or_else(|_| shortcut.trim().to_string()),
                ],
            };
            KeybindingCommand {
                id: spec.id,
                label: spec.label,
                category: spec.category,
                context: spec.context.unwrap_or_else(|| "Global".to_string()),
                default_shortcuts: spec.defaults,
                effective_shortcuts,
                modified: override_value.is_some(),
                disabled: matches!(override_value, Some(None)),
            }
        })
        .collect::<Vec<_>>();
    commands.sort_by(|a, b| {
        (&a.category, &a.label, &a.context).cmp(&(&b.category, &b.label, &b.context))
    });
    commands
}

pub fn validate_keybinding_override(
    settings: &KeybindingSettings,
    binding_id: &str,
    shortcut: &str,
) -> Result<Vec<KeybindingIssue>, String> {
    let shortcut = normalize_shortcut(shortcut)?;
    let conflict_key = canonical_conflict_key(&shortcut);
    let specs = binding_specs();
    let target = specs
        .iter()
        .find(|spec| spec.id == binding_id)
        .ok_or_else(|| "That keybinding no longer exists.".to_string())?;
    let mut issues = Vec::new();

    if is_reserved_shortcut(&shortcut) {
        issues.push(KeybindingIssue {
            severity: KeybindingIssueSeverity::Error,
            message: format!("{shortcut} is reserved by the operating system."),
            conflicting_binding_id: None,
        });
    }
    if is_unmodified_printable(&shortcut) && context_accepts_input(target.context.as_deref()) {
        issues.push(KeybindingIssue {
            severity: KeybindingIssueSeverity::Warning,
            message: "A printable key without modifiers may interfere with typing.".to_string(),
            conflicting_binding_id: None,
        });
    }

    for command in keybinding_commands(settings) {
        if command.id == binding_id
            || binding_action_name(&command.id) == binding_action_name(binding_id)
            || !command
                .effective_shortcuts
                .iter()
                .filter_map(|existing| normalize_shortcut(existing).ok())
                .any(|existing| canonical_conflict_key(&existing) == conflict_key)
        {
            continue;
        }
        let Some(other) = specs.iter().find(|spec| spec.id == command.id) else {
            continue;
        };
        if contexts_overlap(target.context.as_deref(), other.context.as_deref()) {
            issues.push(KeybindingIssue {
                severity: KeybindingIssueSeverity::Error,
                message: format!(
                    "{shortcut} conflicts with {} when {}.",
                    command.label, command.context
                ),
                conflicting_binding_id: Some(command.id),
            });
        }
    }

    Ok(issues)
}

pub fn effective_shortcuts_for_action(
    settings: &KeybindingSettings,
    action_name: &str,
) -> Vec<String> {
    let mut shortcuts = keybinding_commands(settings)
        .into_iter()
        .filter(|command| binding_action_name(&command.id) == action_name)
        .flat_map(|command| command.effective_shortcuts)
        .collect::<Vec<_>>();
    shortcuts.sort();
    shortcuts.dedup();
    shortcuts
}

pub fn format_keystroke(event: &gpui_kit::KeystrokeEvent) -> String {
    let modifiers = event.keystroke.modifiers;
    let mut parts = Vec::new();
    if modifiers.platform {
        parts.push("cmd");
    }
    if modifiers.control {
        parts.push("ctrl");
    }
    if modifiers.alt {
        parts.push("alt");
    }
    if modifiers.shift {
        parts.push("shift");
    }
    let key = event.keystroke.key.to_string();
    if parts.is_empty() {
        key
    } else {
        parts.push(&key);
        parts.join("-")
    }
}

pub fn normalize_shortcut(shortcut: &str) -> Result<String, String> {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() {
        return Err("Press a shortcut first.".to_string());
    }
    shortcut
        .split_whitespace()
        .map(|part| {
            Keystroke::parse(part).map(|key| key.unparse()).map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join(" "))
}

fn effective_keybindings(settings: &KeybindingSettings) -> (Vec<KeyBinding>, Vec<String>) {
    let mut bindings = Vec::new();
    let mut errors = Vec::new();
    for spec in binding_specs() {
        let shortcuts = match settings.overrides.get(&spec.id) {
            None => spec.defaults.clone(),
            Some(None) => continue,
            Some(Some(shortcut)) => match normalize_shortcut(shortcut) {
                Ok(shortcut) => vec![shortcut],
                Err(error) => {
                    errors.push(format!("{}: {error}", spec.label));
                    spec.defaults.clone()
                }
            },
        };
        let context = spec.context.as_deref().map(|context| {
            Rc::new(KeyBindingContextPredicate::parse(context).expect("default context is valid"))
        });
        for shortcut in shortcuts {
            match KeyBinding::load(
                &shortcut,
                spec.action.boxed_clone(),
                context.clone(),
                false,
                None,
                &DummyKeyboardMapper,
            ) {
                Ok(binding) => bindings.push(binding),
                Err(error) => errors.push(format!("{} ({shortcut}): {error}", spec.label)),
            }
        }
    }
    (bindings, errors)
}

fn binding_specs() -> Vec<BindingSpec> {
    let mut specs = Vec::<BindingSpec>::new();
    let mut indexes = BTreeMap::<(String, String), usize>::new();

    for binding in default_keybindings() {
        let action_name = binding.action().name().to_string();
        let context = binding.predicate().map(|predicate| predicate.to_string());
        let key = (action_name.clone(), context.clone().unwrap_or_default());
        let shortcut = binding
            .keystrokes()
            .iter()
            .map(|keystroke| keystroke.inner().unparse())
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(index) = indexes.get(&key).copied() {
            if !specs[index].defaults.contains(&shortcut) {
                specs[index].defaults.push(shortcut);
            }
            continue;
        }

        let id = binding_id(&action_name, context.as_deref());
        let index = specs.len();
        indexes.insert(key, index);
        specs.push(BindingSpec {
            id,
            action: binding.action().boxed_clone(),
            label: humanize_action(&action_name),
            category: binding_category(context.as_deref()).to_string(),
            context,
            defaults: vec![shortcut],
        });
    }
    specs
}

fn binding_id(action_name: &str, context: Option<&str>) -> String {
    format!(
        "{}.{}",
        slug(action_name.rsplit("::").next().unwrap_or(action_name)),
        slug(context.unwrap_or("global"))
    )
}

fn binding_action_name(binding_id: &str) -> &str {
    binding_id.split('.').next().unwrap_or(binding_id)
}

fn slug(raw: &str) -> String {
    let mut slug = String::new();
    let mut previous_separator = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() && !slug.is_empty() && !previous_separator {
                slug.push('-');
            }
            slug.push(ch.to_ascii_lowercase());
            previous_separator = false;
        } else if !slug.is_empty() && !previous_separator {
            slug.push('-');
            previous_separator = true;
        }
    }
    slug.trim_matches('-').to_string()
}

fn humanize_action(action_name: &str) -> String {
    let name = action_name.rsplit("::").next().unwrap_or(action_name);
    let mut label = String::new();
    for (index, ch) in name.chars().enumerate() {
        if index > 0 && ch.is_ascii_uppercase() {
            label.push(' ');
        }
        label.push(ch);
    }
    label.replace(" Ai ", " AI ").replace(" Json", " JSON")
}

fn binding_category(context: Option<&str>) -> &'static str {
    let context = context.unwrap_or_default();
    if context.contains("ForgeView") {
        "Forge"
    } else if context.contains("Transfer") {
        "Transfer"
    } else if context.contains("Aggregation") {
        "Aggregation"
    } else if context.contains("Indexes") {
        "Indexes"
    } else if context.contains("Stats") {
        "Schema"
    } else if context.contains("Sidebar") {
        "Sidebar"
    } else if context.contains("JsonEditorWindow") {
        "JSON Editor"
    } else if context.contains("Database") {
        "Database"
    } else if context.contains("Documents") {
        "Documents"
    } else {
        "Workspace"
    }
}

fn canonical_conflict_key(shortcut: &str) -> String {
    shortcut
        .split_whitespace()
        .map(|part| {
            let part = if cfg!(target_os = "macos") {
                part.replace("secondary-", "cmd-")
            } else {
                part.replace("secondary-", "ctrl-")
            };
            part.strip_suffix("return").map(|prefix| format!("{prefix}enter")).unwrap_or(part)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_reserved_shortcut(shortcut: &str) -> bool {
    matches!(
        shortcut,
        "cmd-space"
            | "cmd-tab"
            | "cmd-shift-3"
            | "cmd-shift-4"
            | "cmd-shift-5"
            | "alt-cmd-escape"
            | "ctrl-cmd-q"
    )
}

fn is_unmodified_printable(shortcut: &str) -> bool {
    let Some(key) = shortcut.split_whitespace().last() else {
        return false;
    };
    !key.contains('-') && key.chars().count() == 1
}

fn context_accepts_input(context: Option<&str>) -> bool {
    context.is_none_or(|context| !context.contains("!Input"))
}

fn contexts_overlap(left: Option<&str>, right: Option<&str>) -> bool {
    if left.is_none() || right.is_none() {
        return true;
    }
    let left = KeyBindingContextPredicate::parse(left.unwrap()).expect("default context is valid");
    let right =
        KeyBindingContextPredicate::parse(right.unwrap()).expect("default context is valid");
    context_samples()
        .iter()
        .any(|contexts| left.depth_of(contexts).is_some() && right.depth_of(contexts).is_some())
}

fn context_samples() -> Vec<Vec<KeyContext>> {
    let single = |context: &str| vec![KeyContext::parse(context).unwrap()];
    let path = |parent: &str, child: &str| {
        vec![KeyContext::parse(parent).unwrap(), KeyContext::parse(child).unwrap()]
    };
    vec![
        single("Workspace"),
        single("Workspace Welcome"),
        single("Workspace Databases"),
        single("Workspace Collections"),
        single("Workspace Database"),
        single("Workspace Documents"),
        single("Workspace Documents Input"),
        single("Workspace Documents Indexes"),
        single("Workspace Documents Stats"),
        single("Workspace Documents Schema"),
        single("Workspace Documents Aggregation"),
        single("Workspace Documents Aggregation Input"),
        single("Workspace ForgeView"),
        path("Workspace ForgeView", "Input"),
        single("Workspace Transfer"),
        single("Workspace Transfer TransferRunning"),
        single("Workspace Transfer TransferQueryModal"),
        single("Sidebar"),
        single("Sidebar Input"),
        single("JsonEditorWindow"),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use gpui_kit::{KeyBindingContextPredicate, KeyContext};

    use super::*;

    #[test]
    fn open_forge_shortcut_matches_sidebar_without_conflicting_with_aggregation() {
        let bindings = default_keybindings();
        for shortcut in ["cmd-shift-f", "ctrl-shift-f"] {
            assert!(bindings.iter().any(|binding| {
                binding.action().as_any().is::<OpenForge>()
                    && binding.keystrokes()
                        == KeyBinding::new(shortcut, OpenForge, Some("Sidebar")).keystrokes()
                    && binding.predicate().is_some_and(|context| {
                        context
                            .depth_of(&[
                                KeyContext::parse("Workspace").unwrap(),
                                KeyContext::parse("Sidebar").unwrap(),
                            ])
                            .is_some()
                            && context
                                .depth_of(&[
                                    KeyContext::parse("Workspace").unwrap(),
                                    KeyContext::parse("Documents Aggregation").unwrap(),
                                ])
                                .is_none()
                    })
            }));
        }
    }

    #[test]
    fn document_duplicate_and_delete_do_not_match_aggregation() {
        let document = KeyBindingContextPredicate::parse(DOCUMENT_EDIT_CONTEXT).unwrap();
        let aggregation =
            KeyBindingContextPredicate::parse("Documents && Aggregation && !Input").unwrap();
        let contexts = [KeyContext::parse("Documents Aggregation").unwrap()];

        assert!(document.depth_of(&contexts).is_none());
        assert!(aggregation.depth_of(&contexts).is_some());
    }

    #[test]
    fn focus_content_no_longer_conflicts_with_numbered_tab_selection() {
        assert!(!FOCUS_CONTENT_KEYS.contains(&"cmd-1"));
        assert!(!FOCUS_CONTENT_KEYS.contains(&"ctrl-1"));
        assert_eq!(FOCUS_CONTENT_KEYS, ["cmd-shift-1", "ctrl-shift-1"]);
    }

    #[test]
    fn feature_ten_shortcuts_are_in_the_default_catalog() {
        let commands = keybinding_commands(&KeybindingSettings::default());
        let shortcuts = |action: &str| {
            commands
                .iter()
                .filter(|command| command.id.starts_with(action))
                .flat_map(|command| command.default_shortcuts.iter().cloned())
                .collect::<Vec<_>>()
        };

        assert!(
            shortcuts("show-schema-subview.").contains(&normalize_shortcut("cmd-alt-5").unwrap())
        );
        assert!(
            shortcuts("show-schema-subview.").contains(&normalize_shortcut("ctrl-alt-5").unwrap())
        );
        assert!(shortcuts("run-transfer.").contains(&"cmd-enter".to_string()));
        assert!(shortcuts("run-transfer.").contains(&"ctrl-enter".to_string()));
        assert!(shortcuts("cancel-transfer.").contains(&"escape".to_string()));
        assert!(shortcuts("save-transfer-query.").contains(&"cmd-enter".to_string()));
        assert!(shortcuts("close-transfer-query-modal.").contains(&"escape".to_string()));
        assert!(!contexts_overlap(
            Some("Transfer && !TransferRunning && !TransferQueryModal"),
            Some("Transfer && TransferQueryModal")
        ));
    }

    #[test]
    fn transfer_shortcuts_match_base_and_modal_runtime_contexts() {
        let run = KeyBindingContextPredicate::parse(
            "Transfer && !TransferRunning && !TransferQueryModal",
        )
        .unwrap();
        let cancel =
            KeyBindingContextPredicate::parse("Transfer && TransferRunning && !TransferQueryModal")
                .unwrap();
        let modal = KeyBindingContextPredicate::parse("Transfer && TransferQueryModal").unwrap();
        let base_contexts =
            [KeyContext::parse("Workspace").unwrap(), KeyContext::parse("Transfer").unwrap()];
        let running_contexts = [
            KeyContext::parse("Workspace").unwrap(),
            KeyContext::parse("Transfer TransferRunning").unwrap(),
        ];
        let modal_contexts = [
            KeyContext::parse("Workspace").unwrap(),
            KeyContext::parse("Transfer TransferQueryModal").unwrap(),
            KeyContext::parse("Input").unwrap(),
        ];

        assert!(run.depth_of(&base_contexts).is_some());
        assert!(cancel.depth_of(&base_contexts).is_none());
        assert!(run.depth_of(&running_contexts).is_none());
        assert!(cancel.depth_of(&running_contexts).is_some());
        assert!(modal.depth_of(&base_contexts).is_none());
        assert!(run.depth_of(&modal_contexts).is_none());
        assert!(cancel.depth_of(&modal_contexts).is_none());
        assert!(modal.depth_of(&modal_contexts).is_some());
    }

    #[test]
    fn default_catalog_has_unique_ids_and_compiles() {
        let commands = keybinding_commands(&KeybindingSettings::default());
        let ids = commands.iter().map(|command| command.id.as_str()).collect::<HashSet<_>>();
        let (bindings, errors) = effective_keybindings(&KeybindingSettings::default());

        assert_eq!(ids.len(), commands.len());
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            bindings.len(),
            commands.iter().map(|command| command.default_shortcuts.len()).sum::<usize>()
        );
        assert!(commands.iter().any(|command| command.id == "select-tab1.workspace"));
    }

    #[test]
    fn validation_reports_overlapping_conflicts_but_allows_disjoint_contexts() {
        let settings = KeybindingSettings::default();
        let conflict =
            validate_keybinding_override(&settings, "open-action-bar.workspace", "cmd-,").unwrap();
        assert!(conflict.iter().any(|issue| {
            issue.severity == KeybindingIssueSeverity::Error
                && issue.conflicting_binding_id.as_deref() == Some("open-settings.workspace")
        }));

        let disjoint =
            validate_keybinding_override(&settings, "create-index.documents-indexes", "cmd-n")
                .unwrap();
        assert!(!disjoint.iter().any(|issue| issue.severity == KeybindingIssueSeverity::Error));
    }

    #[test]
    fn disabled_and_custom_overrides_replace_defaults() {
        let mut settings = KeybindingSettings::default();
        settings.overrides.insert("open-action-bar.workspace".into(), Some("cmd-alt-z".into()));
        settings.overrides.insert("open-settings.workspace".into(), None);

        let (bindings, errors) = effective_keybindings(&settings);
        assert!(errors.is_empty(), "{errors:?}");
        let action_bar = bindings
            .iter()
            .filter(|binding| binding.action().name().ends_with("OpenActionBar"))
            .flat_map(|binding| {
                binding.keystrokes().iter().map(|keystroke| keystroke.inner().unparse())
            })
            .collect::<Vec<_>>();
        assert_eq!(action_bar, vec![normalize_shortcut("cmd-alt-z").unwrap()]);
        assert!(bindings.iter().all(|binding| !binding.action().name().ends_with("OpenSettings")));
    }

    #[test]
    fn invalid_and_reserved_shortcuts_fail_safely() {
        assert!(normalize_shortcut("").is_err());
        let issues = validate_keybinding_override(
            &KeybindingSettings::default(),
            "open-action-bar.workspace",
            "cmd-space",
        )
        .unwrap();
        assert!(issues.iter().any(|issue| issue.severity == KeybindingIssueSeverity::Error));
    }
}
