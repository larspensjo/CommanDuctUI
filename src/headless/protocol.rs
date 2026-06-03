use super::snapshot::ProtocolSnapshot;
use super::{DialogKind, DialogMatcher, DialogOutcome, DialogScriptEntry};
use crate::{PlatformError, PlatformResult, WindowId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub(super) const HEADLESS_PROTOCOL_VERSION: u32 = 3;

#[derive(Debug, Deserialize)]
pub(super) struct ProtocolRequestEnvelope {
    pub(super) request_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ProtocolRequest {
    Action {
        request_id: u64,
        #[serde(flatten)]
        action: ProtocolActionRequest,
    },
    /// Replaces the ordered dialog responder script used by later `Show*Dialog` commands.
    /// Install this before the action that opens the dialog.
    SetDialogResponder {
        request_id: u64,
        script: Vec<ProtocolDialogScriptEntry>,
    },
    Snapshot {
        request_id: u64,
    },
    WaitFor {
        request_id: u64,
        label: String,
        timeout_ms: u64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(super) enum ProtocolActionRequest {
    Click {
        window_id: usize,
        control_id: i32,
    },
    SetText {
        window_id: usize,
        control_id: i32,
        text: String,
    },
    SelectRow {
        window_id: usize,
        control_id: i32,
        item_id: u64,
    },
    SelectCombo {
        window_id: usize,
        control_id: i32,
        index: usize,
    },
    SelectTab {
        window_id: usize,
        control_id: i32,
        index: usize,
    },
    Toggle {
        window_id: usize,
        control_id: i32,
    },
    SelectRadio {
        window_id: usize,
        control_id: i32,
    },
    SelectTree {
        window_id: usize,
        control_id: i32,
        item_id: u64,
    },
    ToggleTree {
        window_id: usize,
        control_id: i32,
        item_id: u64,
    },
    ClickMenu {
        window_id: usize,
        action_id: u32,
    },
    Scroll {
        window_id: usize,
        control_id: i32,
        vertical_pos: u32,
        horizontal_pos: u32,
    },
    #[serde(rename = "scroll_listbox")]
    ScrollListBox {
        window_id: usize,
        control_id: i32,
        position: u32,
    },
    #[serde(rename = "key_listbox")]
    KeyListBox {
        window_id: usize,
        control_id: i32,
        key_code: u16,
    },
    #[serde(rename = "key_input")]
    KeyInput {
        window_id: usize,
        control_id: i32,
        key_code: u16,
        ctrl: bool,
        shift: bool,
        alt: bool,
    },
}

#[derive(Debug, Deserialize)]
pub(super) struct ProtocolDialogScriptEntry {
    pub(super) matcher: ProtocolDialogMatcher,
    outcome: ProtocolDialogOutcome,
}

#[derive(Debug, Deserialize)]
pub(super) struct ProtocolDialogMatcher {
    kind: String,
    window_id: Option<usize>,
    title: Option<String>,
    prompt: Option<String>,
    context_tag: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum ProtocolDialogOutcome {
    SaveFile {
        path: Option<String>,
    },
    OpenFile {
        path: Option<String>,
    },
    ProfileSelection {
        chosen_profile_name: Option<String>,
        create_new_requested: bool,
        user_cancelled: bool,
    },
    Input {
        text: Option<String>,
    },
    ExcludePatterns {
        saved: bool,
        patterns: String,
    },
    Form {
        confirmed: bool,
        field_values: Vec<ProtocolFormFieldValue>,
    },
    FolderPicker {
        path: Option<String>,
    },
    MessageBox,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum ProtocolFormFieldValue {
    Text { field_id: String, value: String },
    CheckBox { field_id: String, checked: bool },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum ProtocolResponse {
    Hello {
        protocol_version: u32,
    },
    Snapshot {
        request_id: u64,
        model: ProtocolSnapshot,
    },
    Ok {
        request_id: u64,
    },
    Error {
        request_id: Option<u64>,
        message: String,
    },
    Marker {
        label: String,
    },
    Bye,
}

impl ProtocolDialogScriptEntry {
    pub(super) fn into_runtime(self) -> PlatformResult<DialogScriptEntry> {
        let matcher = self.matcher.into_runtime()?;
        let outcome = self.outcome.into_runtime()?;
        if matcher.kind != outcome.kind() {
            return Err(PlatformError::OperationFailed(format!(
                "Dialog responder matcher kind '{}' does not match outcome kind '{}'",
                matcher.kind.stable_name(),
                outcome.kind().stable_name()
            )));
        }
        Ok(DialogScriptEntry { matcher, outcome })
    }
}

impl ProtocolDialogMatcher {
    pub(super) fn into_runtime(self) -> PlatformResult<DialogMatcher> {
        let kind = DialogKind::from_stable_name(&self.kind).ok_or_else(|| {
            PlatformError::OperationFailed(format!(
                "Unknown dialog responder matcher kind '{}'",
                self.kind
            ))
        })?;
        Ok(DialogMatcher {
            kind,
            window_id: self.window_id.map(WindowId::new),
            title: self.title,
            prompt: self.prompt,
            context_tag: self.context_tag,
        })
    }
}

impl ProtocolDialogOutcome {
    pub(super) fn into_runtime(self) -> PlatformResult<DialogOutcome> {
        Ok(match self {
            Self::SaveFile { path } => DialogOutcome::SaveFile {
                result: path.map(PathBuf::from),
            },
            Self::OpenFile { path } => DialogOutcome::OpenFile {
                result: path.map(PathBuf::from),
            },
            Self::ProfileSelection {
                chosen_profile_name,
                create_new_requested,
                user_cancelled,
            } => DialogOutcome::ProfileSelection {
                chosen_profile_name,
                create_new_requested,
                user_cancelled,
            },
            Self::Input { text } => DialogOutcome::Input { text },
            Self::ExcludePatterns { saved, patterns } => {
                DialogOutcome::ExcludePatterns { saved, patterns }
            }
            Self::Form {
                confirmed,
                field_values,
            } => DialogOutcome::Form {
                confirmed,
                field_values: field_values
                    .into_iter()
                    .map(ProtocolFormFieldValue::into_runtime)
                    .collect::<PlatformResult<Vec<_>>>()?,
            },
            Self::FolderPicker { path } => DialogOutcome::FolderPicker {
                path: path.map(PathBuf::from),
            },
            Self::MessageBox => DialogOutcome::MessageBox,
        })
    }
}

impl ProtocolFormFieldValue {
    pub(super) fn into_runtime(self) -> PlatformResult<crate::FormFieldValue> {
        Ok(match self {
            Self::Text { field_id, value } => crate::FormFieldValue::Text { field_id, value },
            Self::CheckBox { field_id, checked } => {
                crate::FormFieldValue::CheckBox { field_id, checked }
            }
        })
    }
}
