use super::state::ControlKind;
use super::{
    DialogKind, DialogMatcher, DialogOutcome, DialogScriptEntry, HEADLESS_PROTOCOL_VERSION,
    HeadlessBackend, HeadlessHarness,
};
use crate::{
    AppEvent, ChartDataPacket, ChartLineData, ChartLineEmphasis, CheckState, Color, ControlId,
    ControlStyle, DockStyle, FormButtons, FormDialogDescriptor, FormField, FormFieldValue, FormRow,
    FormTextValidation, LabelClass, LayoutRule, ListBoxItemDescriptor, ListBoxItemId,
    ListBoxRowDensity, MenuActionId, MenuItemConfig, MessageSeverity, PlatformCommand,
    PlatformError, PlatformEventHandler, SplitterOrientation, StyleId, TreeItemDescriptor,
    TreeItemId, UiStateProvider, WindowConfig, WindowId,
};
use serde_json::Value;
use std::collections::VecDeque;
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const BTN_CLICK_ME: ControlId = ControlId::new(101);

struct TestHandler {
    events: Vec<AppEvent>,
    commands: VecDeque<PlatformCommand>,
}

impl PlatformEventHandler for TestHandler {
    fn handle_event(&mut self, event: AppEvent) {
        self.events.push(event);
    }

    fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
        self.commands.pop_front()
    }
}

struct SilentProvider;

impl UiStateProvider for SilentProvider {
    fn is_tree_item_new(&self, _window_id: WindowId, _item_id: TreeItemId) -> bool {
        false
    }
}

struct RecordingWriter {
    bytes: Vec<u8>,
    flushes: usize,
}

impl RecordingWriter {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            flushes: 0,
        }
    }

    fn into_string(self) -> String {
        String::from_utf8(self.bytes).unwrap()
    }
}

impl Write for RecordingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

fn parse_protocol_lines(output: &str) -> Vec<Value> {
    output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn started_harness(
    initial_commands: Vec<PlatformCommand>,
) -> (
    HeadlessHarness,
    WindowId,
    Arc<Mutex<TestHandler>>,
    Arc<Mutex<SilentProvider>>,
) {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(handler.clone(), provider.clone(), initial_commands)
        .unwrap();
    (harness, window_id, handler, provider)
}

#[test]
fn protocol_hello_is_first_and_flushes_output_groups() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler,
            provider,
            vec![PlatformCommand::ShowWindow { window_id }],
        )
        .unwrap();

    let input = Cursor::new(Vec::<u8>::new());
    let mut writer = RecordingWriter::new();
    harness.run_protocol(input, &mut writer).unwrap();

    let flushes = writer.flushes;
    let output = writer.into_string();
    let lines = parse_protocol_lines(&output);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["type"], "hello");
    assert_eq!(lines[0]["protocol_version"], HEADLESS_PROTOCOL_VERSION);
    assert_eq!(lines[1]["type"], "bye");
    assert_eq!(flushes, 2);
}

#[test]
fn protocol_actions_round_trip_and_wait_for_is_cursor_relative() {
    struct ProtocolHandler {
        events: Vec<AppEvent>,
        commands: VecDeque<PlatformCommand>,
        target_window: WindowId,
    }

    impl PlatformEventHandler for ProtocolHandler {
        fn handle_event(&mut self, event: AppEvent) {
            if matches!(
                event,
                AppEvent::ButtonClicked {
                    control_id,
                    ..
                } if control_id == BTN_CLICK_ME
            ) {
                self.commands.push_back(PlatformCommand::SetWindowTitle {
                    window_id: self.target_window,
                    title: "Clicked".into(),
                });
                self.commands.push_back(PlatformCommand::Checkpoint {
                    label: "done".into(),
                });
            }
            self.events.push(event);
        }

        fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
            self.commands.pop_front()
        }
    }

    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(ProtocolHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
        target_window: window_id,
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler.clone(),
            provider,
            vec![
                PlatformCommand::CreateButton {
                    window_id,
                    parent_control_id: None,
                    control_id: BTN_CLICK_ME,
                    text: "Click".into(),
                },
                PlatformCommand::ShowWindow { window_id },
            ],
        )
        .unwrap();

    let protocol_input = format!(
        "{}\n{}\n{}\n{}\n",
        serde_json::json!({
            "type": "snapshot",
            "request_id": 1
        }),
        serde_json::json!({
            "type": "action",
            "request_id": 2,
            "action": "click",
            "window_id": window_id.raw(),
            "control_id": BTN_CLICK_ME.raw()
        }),
        serde_json::json!({
            "type": "snapshot",
            "request_id": 3
        }),
        serde_json::json!({
            "type": "wait_for",
            "request_id": 4,
            "label": "done",
            "timeout_ms": 200
        })
    );

    let handler_for_thread = handler.clone();
    let delayed_marker = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(20));
        handler_for_thread
            .lock()
            .unwrap()
            .commands
            .push_back(PlatformCommand::Checkpoint {
                label: "done".into(),
            });
    });

    let mut writer = RecordingWriter::new();
    harness
        .run_protocol(Cursor::new(protocol_input.into_bytes()), &mut writer)
        .unwrap();
    delayed_marker.join().unwrap();

    let flushes = writer.flushes;
    let output = writer.into_string();
    let lines = parse_protocol_lines(&output);
    assert_eq!(
        lines
            .iter()
            .map(|line| line["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "hello", "snapshot", "marker", "ok", "snapshot", "marker", "ok", "bye",
        ]
    );
    assert_eq!(lines[1]["request_id"], 1);
    assert!(lines[1]["model"].get("markers").is_none());
    assert!(lines[1]["model"].get("quitting").is_none());
    assert_eq!(lines[3]["request_id"], 2);
    assert_eq!(lines[4]["model"]["windows"][0]["title"], "Clicked");
    assert!(lines[4]["model"].get("markers").is_none());
    assert!(lines[4]["model"].get("quitting").is_none());
    assert_eq!(lines[5]["label"], "done");
    assert_eq!(lines[6]["request_id"], 4);
    assert_eq!(flushes, 6);
}

#[test]
fn protocol_recovers_request_ids_for_malformed_input() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler,
            provider,
            vec![PlatformCommand::ShowWindow { window_id }],
        )
        .unwrap();

    let protocol_input = concat!(
        "{",
        "\"type\":\"action\",",
        "\"request_id\":42,",
        "\"action\":\"click\",",
        "\"window_id\":1",
        "}\n",
        "{\"request_id\":99}\n",
        "{\"type\":\"snapshot\",\"request_id\":7}\n",
        "{not-json}\n",
    );

    let mut writer = RecordingWriter::new();
    harness
        .run_protocol(Cursor::new(protocol_input.as_bytes().to_vec()), &mut writer)
        .unwrap();

    let lines = parse_protocol_lines(&writer.into_string());
    assert_eq!(lines[0]["type"], "hello");
    assert_eq!(lines[1]["type"], "error");
    assert_eq!(lines[1]["request_id"], 42);
    assert!(lines[1]["message"].as_str().unwrap().contains("control_id"));
    assert_eq!(lines[2]["type"], "error");
    assert_eq!(lines[2]["request_id"], 99);
    assert!(lines[2]["message"].as_str().unwrap().contains("type"));
    assert_eq!(lines[3]["type"], "snapshot");
    assert_eq!(lines[3]["request_id"], 7);
    assert_eq!(lines[4]["type"], "error");
    assert!(lines[4]["request_id"].is_null());
    assert_eq!(lines[5]["type"], "bye");
}

#[test]
fn protocol_dialogs_default_to_cancel_without_driver_scripting() {
    struct DialogHandler {
        events: Vec<AppEvent>,
        commands: VecDeque<PlatformCommand>,
        target_window: WindowId,
    }

    impl PlatformEventHandler for DialogHandler {
        fn handle_event(&mut self, event: AppEvent) {
            if matches!(event, AppEvent::ButtonClicked { .. }) {
                self.commands
                    .push_back(PlatformCommand::ShowSaveFileDialog {
                        window_id: self.target_window,
                        title: "Save".into(),
                        default_filename: "export.txt".into(),
                        filter_spec: "*.txt".into(),
                        initial_dir: None,
                    });
            }
            self.events.push(event);
        }

        fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
            self.commands.pop_front()
        }
    }

    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(DialogHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
        target_window: window_id,
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler.clone(),
            provider,
            vec![
                PlatformCommand::CreateButton {
                    window_id,
                    parent_control_id: None,
                    control_id: BTN_CLICK_ME,
                    text: "Save".into(),
                },
                PlatformCommand::ShowWindow { window_id },
            ],
        )
        .unwrap();

    let protocol_input = serde_json::json!({
        "type": "action",
        "request_id": 1,
        "action": "click",
        "window_id": window_id.raw(),
        "control_id": BTN_CLICK_ME.raw()
    })
    .to_string()
        + "\n";

    let mut writer = RecordingWriter::new();
    harness
        .run_protocol(Cursor::new(protocol_input.into_bytes()), &mut writer)
        .unwrap();

    let events = &handler.lock().unwrap().events;
    assert!(events.iter().any(|event| matches!(
        event,
        AppEvent::FileSaveDialogCompleted { result: None, .. }
    )));
    let lines = parse_protocol_lines(&writer.into_string());
    assert_eq!(lines[0]["type"], "hello");
    assert_eq!(lines[1]["type"], "ok");
    assert_eq!(lines[2]["type"], "bye");
}

#[test]
fn protocol_set_dialog_responder_scripts_form_dialog_and_round_trips_field_values() {
    struct DialogHandler {
        events: Vec<AppEvent>,
        commands: VecDeque<PlatformCommand>,
        target_window: WindowId,
    }

    impl PlatformEventHandler for DialogHandler {
        fn handle_event(&mut self, event: AppEvent) {
            if matches!(event, AppEvent::ButtonClicked { .. }) {
                self.commands.push_back(PlatformCommand::ShowFormDialog {
                    window_id: self.target_window,
                    form: FormDialogDescriptor {
                        title: "Form".into(),
                        context_tag: "form-tag".into(),
                        rows: vec![],
                        fields: vec![
                            FormField::TextInput {
                                field_id: "name".into(),
                                label: "Name".into(),
                                value: String::new(),
                                validation: FormTextValidation::Any,
                                live_warning: None,
                            },
                            FormField::CheckBox {
                                field_id: "enabled".into(),
                                label: "Enabled".into(),
                                checked: false,
                            },
                        ],
                        buttons: FormButtons {
                            confirm_label: "OK".into(),
                            cancel_label: "Cancel".into(),
                            confirm_enabled: true,
                        },
                    },
                });
            }
            self.events.push(event);
        }

        fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
            self.commands.pop_front()
        }
    }

    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(DialogHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
        target_window: window_id,
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler.clone(),
            provider,
            vec![
                PlatformCommand::CreateButton {
                    window_id,
                    parent_control_id: None,
                    control_id: BTN_CLICK_ME,
                    text: "Open".into(),
                },
                PlatformCommand::ShowWindow { window_id },
            ],
        )
        .unwrap();

    let protocol_input = format!(
        "{}\n{}\n{}\n",
        serde_json::json!({
            "type": "set_dialog_responder",
            "request_id": 1,
            "script": [
                {
                    "matcher": {
                        "kind": "message_box",
                        "window_id": window_id.raw(),
                        "title": "Notice",
                        "prompt": "Ignored",
                        "context_tag": null
                    },
                    "outcome": {
                        "kind": "message_box"
                    }
                },
                {
                    "matcher": {
                        "kind": "form",
                        "window_id": window_id.raw(),
                        "title": "Form",
                        "prompt": null,
                        "context_tag": "form-tag"
                    },
                    "outcome": {
                        "kind": "form",
                        "confirmed": true,
                        "field_values": [
                            {
                                "kind": "text",
                                "field_id": "name",
                                "value": "Alice"
                            },
                            {
                                "kind": "check_box",
                                "field_id": "enabled",
                                "checked": true
                            }
                        ]
                    }
                }
            ]
        }),
        serde_json::json!({
            "type": "action",
            "request_id": 2,
            "action": "click",
            "window_id": window_id.raw(),
            "control_id": BTN_CLICK_ME.raw()
        }),
        serde_json::json!({
            "type": "snapshot",
            "request_id": 3
        })
    );

    let mut writer = RecordingWriter::new();
    harness
        .run_protocol(Cursor::new(protocol_input.into_bytes()), &mut writer)
        .unwrap();

    let lines = parse_protocol_lines(&writer.into_string());
    assert_eq!(
        lines
            .iter()
            .map(|line| line["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["hello", "ok", "ok", "snapshot", "bye",]
    );
    assert_eq!(lines[0]["protocol_version"], HEADLESS_PROTOCOL_VERSION);
    assert_eq!(lines[1]["request_id"], 1);
    assert_eq!(lines[2]["request_id"], 2);
    assert_eq!(lines[3]["request_id"], 3);
    assert_eq!(
        lines[3]["model"]["dialog_requests"][0]["details"]["kind"],
        "form"
    );
    assert_eq!(
        lines[3]["model"]["dialog_requests"][0]["details"]["form"]["fields"][0]["kind"],
        "text_input"
    );
    assert_eq!(
        lines[3]["model"]["dialog_requests"][0]["details"]["form"]["buttons"]["confirm_label"],
        "OK"
    );

    let events = &handler.lock().unwrap().events;
    assert!(matches!(
        events.as_slice(),
        [
            AppEvent::ButtonClicked { control_id, .. },
            AppEvent::FormDialogCompleted {
                confirmed: true,
                field_values,
                ..
            }
        ] if *control_id == BTN_CLICK_ME && field_values == &vec![
            FormFieldValue::Text {
                field_id: "name".into(),
                value: "Alice".into(),
            },
            FormFieldValue::CheckBox {
                field_id: "enabled".into(),
                checked: true,
            },
        ]
    ));
    assert!(harness.backend.dialog_responder.is_empty());
}

#[test]
fn protocol_set_dialog_responder_rejects_mismatched_entry_kinds() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler,
            provider,
            vec![PlatformCommand::ShowWindow { window_id }],
        )
        .unwrap();

    let protocol_input = serde_json::json!({
        "type": "set_dialog_responder",
        "request_id": 11,
        "script": [
            {
                "matcher": {
                    "kind": "save_file",
                    "window_id": window_id.raw(),
                    "title": "Save",
                    "prompt": null,
                    "context_tag": null
                },
                "outcome": {
                    "kind": "input",
                    "text": "oops"
                }
            }
        ]
    })
    .to_string()
        + "\n";

    let mut writer = RecordingWriter::new();
    harness
        .run_protocol(Cursor::new(protocol_input.into_bytes()), &mut writer)
        .unwrap();

    let lines = parse_protocol_lines(&writer.into_string());
    assert_eq!(lines[0]["type"], "hello");
    assert_eq!(lines[1]["type"], "error");
    assert_eq!(lines[1]["request_id"], 11);
    assert!(
        lines[1]["message"]
            .as_str()
            .unwrap()
            .contains("does not match")
    );
    assert_eq!(lines[2]["type"], "bye");
}

#[test]
fn protocol_set_dialog_responder_rejects_unknown_matcher_kind() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler,
            provider,
            vec![PlatformCommand::ShowWindow { window_id }],
        )
        .unwrap();

    let protocol_input = serde_json::json!({
        "type": "set_dialog_responder",
        "request_id": 12,
        "script": [
            {
                "matcher": {
                    "kind": "unknown_dialog",
                    "window_id": window_id.raw(),
                    "title": "Save",
                    "prompt": null,
                    "context_tag": null
                },
                "outcome": {
                    "kind": "save_file",
                    "path": "export.txt"
                }
            }
        ]
    })
    .to_string()
        + "\n";

    let mut writer = RecordingWriter::new();
    harness
        .run_protocol(Cursor::new(protocol_input.into_bytes()), &mut writer)
        .unwrap();

    let lines = parse_protocol_lines(&writer.into_string());
    assert_eq!(lines[0]["type"], "hello");
    assert_eq!(lines[1]["type"], "error");
    assert_eq!(lines[1]["request_id"], 12);
    assert!(
        lines[1]["message"]
            .as_str()
            .unwrap()
            .contains("Unknown dialog responder matcher kind")
    );
    assert_eq!(lines[2]["type"], "bye");
}

#[test]
fn protocol_set_dialog_responder_reports_malformed_script_entries_with_request_id() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler,
            provider,
            vec![PlatformCommand::ShowWindow { window_id }],
        )
        .unwrap();

    let protocol_input = concat!(
        "{",
        "\"type\":\"set_dialog_responder\",",
        "\"request_id\":77,",
        "\"script\":[{",
        "\"matcher\":{",
        "\"kind\":\"form\",",
        "\"window_id\":1,",
        "\"title\":\"Form\"",
        "},",
        "\"outcome\":{",
        "\"kind\":\"form\",",
        "\"confirmed\":true,",
        "\"field_values\":[{",
        "\"field_id\":\"name\",",
        "\"value\":\"Alice\"",
        "}]", // missing `kind`
        "}",
        "}]",
        "}\n",
        "{\"type\":\"snapshot\",\"request_id\":78}\n",
    );

    let mut writer = RecordingWriter::new();
    harness
        .run_protocol(Cursor::new(protocol_input.as_bytes().to_vec()), &mut writer)
        .unwrap();

    let lines = parse_protocol_lines(&writer.into_string());
    assert_eq!(lines[0]["type"], "hello");
    assert_eq!(lines[1]["type"], "error");
    assert_eq!(lines[1]["request_id"], 77);
    assert!(lines[1]["message"].as_str().unwrap().contains("kind"));
    assert_eq!(lines[2]["type"], "snapshot");
    assert_eq!(lines[2]["request_id"], 78);
    assert_eq!(lines[3]["type"], "bye");
}

#[test]
fn protocol_nonmatching_dialog_script_defaults_to_cancel() {
    struct DialogHandler {
        events: Vec<AppEvent>,
        commands: VecDeque<PlatformCommand>,
        target_window: WindowId,
    }

    impl PlatformEventHandler for DialogHandler {
        fn handle_event(&mut self, event: AppEvent) {
            if matches!(event, AppEvent::ButtonClicked { .. }) {
                self.commands
                    .push_back(PlatformCommand::ShowSaveFileDialog {
                        window_id: self.target_window,
                        title: "Actual".into(),
                        default_filename: "export.txt".into(),
                        filter_spec: "*.txt".into(),
                        initial_dir: None,
                    });
            }
            self.events.push(event);
        }

        fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
            self.commands.pop_front()
        }
    }

    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(DialogHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
        target_window: window_id,
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler.clone(),
            provider,
            vec![
                PlatformCommand::CreateButton {
                    window_id,
                    parent_control_id: None,
                    control_id: BTN_CLICK_ME,
                    text: "Save".into(),
                },
                PlatformCommand::ShowWindow { window_id },
            ],
        )
        .unwrap();

    let protocol_input = format!(
        "{}\n{}\n",
        serde_json::json!({
            "type": "set_dialog_responder",
            "request_id": 1,
            "script": [
                {
                    "matcher": {
                        "kind": "save_file",
                        "window_id": window_id.raw(),
                        "title": "Different",
                        "prompt": null,
                        "context_tag": null
                    },
                    "outcome": {
                        "kind": "save_file",
                        "path": "scripted.txt"
                    }
                }
            ]
        }),
        serde_json::json!({
            "type": "action",
            "request_id": 2,
            "action": "click",
            "window_id": window_id.raw(),
            "control_id": BTN_CLICK_ME.raw()
        })
    );

    let mut writer = RecordingWriter::new();
    harness
        .run_protocol(Cursor::new(protocol_input.into_bytes()), &mut writer)
        .unwrap();

    let events = &handler.lock().unwrap().events;
    assert!(events.iter().any(|event| matches!(
        event,
        AppEvent::FileSaveDialogCompleted { result: None, .. }
    )));
    assert_eq!(harness.backend.dialog_responder.len(), 1);

    let lines = parse_protocol_lines(&writer.into_string());
    assert_eq!(
        lines
            .iter()
            .map(|line| line["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["hello", "ok", "ok", "bye"]
    );
}

#[test]
fn protocol_message_box_script_entries_are_accepted_but_not_queued() {
    struct MessageBoxHandler {
        events: Vec<AppEvent>,
        commands: VecDeque<PlatformCommand>,
        target_window: WindowId,
    }

    impl PlatformEventHandler for MessageBoxHandler {
        fn handle_event(&mut self, event: AppEvent) {
            if matches!(event, AppEvent::ButtonClicked { .. }) {
                self.commands.push_back(PlatformCommand::ShowMessageBox {
                    window_id: self.target_window,
                    title: "Notice".into(),
                    message: "Hello".into(),
                    severity: MessageSeverity::Information,
                });
            }
            self.events.push(event);
        }

        fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
            self.commands.pop_front()
        }
    }

    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(MessageBoxHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
        target_window: window_id,
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler.clone(),
            provider,
            vec![
                PlatformCommand::CreateButton {
                    window_id,
                    parent_control_id: None,
                    control_id: BTN_CLICK_ME,
                    text: "Notice".into(),
                },
                PlatformCommand::ShowWindow { window_id },
            ],
        )
        .unwrap();

    let protocol_input = format!(
        "{}\n{}\n",
        serde_json::json!({
            "type": "set_dialog_responder",
            "request_id": 1,
            "script": [
                {
                    "matcher": {
                        "kind": "message_box",
                        "window_id": window_id.raw(),
                        "title": "Notice",
                        "prompt": "Hello",
                        "context_tag": null
                    },
                    "outcome": {
                        "kind": "message_box"
                    }
                }
            ]
        }),
        serde_json::json!({
            "type": "action",
            "request_id": 2,
            "action": "click",
            "window_id": window_id.raw(),
            "control_id": BTN_CLICK_ME.raw()
        })
    );

    let mut writer = RecordingWriter::new();
    harness
        .run_protocol(Cursor::new(protocol_input.into_bytes()), &mut writer)
        .unwrap();

    let events = &handler.lock().unwrap().events;
    assert!(matches!(
        events.as_slice(),
        [AppEvent::ButtonClicked { .. }]
    ));
    assert!(harness.backend.dialog_responder.is_empty());

    let lines = parse_protocol_lines(&writer.into_string());
    assert_eq!(
        lines
            .iter()
            .map(|line| line["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["hello", "ok", "ok", "bye"]
    );
}

#[test]
fn define_layout_validates_rules_and_records_them() {
    let mut backend = HeadlessBackend::new("app".into());
    let window_id = backend.create_window(WindowConfig {
        title: "Window",
        width: 320,
        height: 240,
    });
    backend
        .create_control(window_id, None, ControlId::new(1), ControlKind::Panel)
        .unwrap();
    let err = backend
        .execute_platform_command(PlatformCommand::DefineLayout {
            window_id,
            rules: vec![LayoutRule {
                control_id: ControlId::new(1),
                parent_control_id: None,
                dock_style: DockStyle::Top,
                order: 0,
                fixed_size: None,
                margin: (1, 2, 3, 4),
            }],
        })
        .unwrap_err();
    assert!(matches!(err, PlatformError::OperationFailed(_)));

    backend
        .execute_platform_command(PlatformCommand::DefineLayout {
            window_id,
            rules: vec![LayoutRule {
                control_id: ControlId::new(1),
                parent_control_id: None,
                dock_style: DockStyle::Fill,
                order: 0,
                fixed_size: None,
                margin: (1, 2, 3, 4),
            }],
        })
        .unwrap();
    assert_eq!(
        backend
            .windows
            .get(&window_id.raw())
            .unwrap()
            .layout_rules
            .len(),
        1
    );
}

#[test]
fn checkpoint_is_recorded_and_setup_complete_is_followed_by_event() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler.clone(),
            provider,
            vec![
                PlatformCommand::Checkpoint {
                    label: "ready".into(),
                },
                PlatformCommand::SignalMainWindowUISetupComplete { window_id },
            ],
        )
        .unwrap();
    assert!(
        harness
            .backend
            .markers
            .iter()
            .any(|marker| marker == "ready")
    );
    let events = &handler.lock().unwrap().events;
    assert!(matches!(
        events.as_slice(),
        [AppEvent::MainWindowUISetupComplete { window_id: got }] if *got == window_id
    ));
}

#[test]
fn snapshot_is_deterministic_and_sorted() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    harness
        .backend
        .execute_platform_command(PlatformCommand::CreateLabel {
            window_id,
            parent_control_id: None,
            control_id: ControlId::new(20),
            initial_text: "B".into(),
            class: LabelClass::Default,
        })
        .unwrap();
    harness
        .backend
        .execute_platform_command(PlatformCommand::CreateLabel {
            window_id,
            parent_control_id: None,
            control_id: ControlId::new(10),
            initial_text: "A".into(),
            class: LabelClass::Default,
        })
        .unwrap();
    let snapshot = harness.snapshot().unwrap();
    let parsed: Value = serde_json::from_str(&snapshot).unwrap();
    let controls = parsed["windows"][0]["controls"].as_array().unwrap();
    assert_eq!(controls[0]["id"], 10);
    assert_eq!(controls[1]["id"], 20);
}

#[test]
fn core_commands_mutate_the_ui_model() {
    let mut backend = HeadlessBackend::new("app".into());
    let window_id = backend.create_window(WindowConfig {
        title: "Window",
        width: 320,
        height: 240,
    });

    for command in [
        PlatformCommand::SetWindowTitle {
            window_id,
            title: "Updated".into(),
        },
        PlatformCommand::ShowWindow { window_id },
        PlatformCommand::CreatePanel {
            window_id,
            parent_control_id: None,
            control_id: ControlId::new(1),
        },
        PlatformCommand::CreateButton {
            window_id,
            parent_control_id: Some(ControlId::new(1)),
            control_id: ControlId::new(2),
            text: "Click".into(),
        },
        PlatformCommand::CreateLabel {
            window_id,
            parent_control_id: Some(ControlId::new(1)),
            control_id: ControlId::new(3),
            initial_text: "Label".into(),
            class: LabelClass::StatusBar,
        },
        PlatformCommand::UpdateLabelText {
            window_id,
            control_id: ControlId::new(3),
            text: "Status".into(),
            severity: MessageSeverity::Warning,
        },
        PlatformCommand::CreateInput {
            window_id,
            parent_control_id: Some(ControlId::new(1)),
            control_id: ControlId::new(4),
            initial_text: "typed".into(),
            read_only: false,
            multiline: false,
            vertical_scroll: false,
        },
        PlatformCommand::CreateRichEdit {
            window_id,
            parent_control_id: Some(ControlId::new(1)),
            control_id: ControlId::new(5),
        },
        PlatformCommand::CreateListBox {
            window_id,
            parent_control_id: Some(ControlId::new(1)),
            control_id: ControlId::new(6),
        },
        PlatformCommand::PopulateListBox {
            window_id,
            control_id: ControlId::new(6),
            items: vec![ListBoxItemDescriptor {
                id: ListBoxItemId::new(99),
                badges: vec![],
                title: "Row".into(),
                metadata: "meta".into(),
                enabled: false,
            }],
            badge_column_width: 42,
        },
        PlatformCommand::SetListBoxRowDensity {
            window_id,
            control_id: ControlId::new(6),
            density: ListBoxRowDensity::Compact,
        },
        PlatformCommand::SetListBoxSelection {
            window_id,
            control_id: ControlId::new(6),
            item_id: ListBoxItemId::new(99),
        },
        PlatformCommand::CreateComboBox {
            window_id,
            parent_control_id: Some(ControlId::new(1)),
            control_id: ControlId::new(7),
        },
        PlatformCommand::SetComboBoxItems {
            window_id,
            control_id: ControlId::new(7),
            items: vec!["A".into(), "B".into()],
        },
        PlatformCommand::SetComboBoxSelection {
            window_id,
            control_id: ControlId::new(7),
            selected_index: Some(1),
        },
        PlatformCommand::CreateRadioButton {
            window_id,
            parent_control_id: Some(ControlId::new(1)),
            control_id: ControlId::new(8),
            text: "Radio".into(),
            group_start: true,
        },
        PlatformCommand::SetRadioButtonChecked {
            window_id,
            control_id: ControlId::new(8),
            checked: true,
        },
        PlatformCommand::CreateCheckBox {
            window_id,
            parent_control_id: Some(ControlId::new(1)),
            control_id: ControlId::new(9),
            text: "Check".into(),
        },
        PlatformCommand::SetCheckBoxChecked {
            window_id,
            control_id: ControlId::new(9),
            checked: true,
        },
        PlatformCommand::CreateTabBar {
            window_id,
            parent_control_id: Some(ControlId::new(1)),
            control_id: ControlId::new(10),
            items: vec!["One".into(), "Two".into()],
        },
        PlatformCommand::SetTabBarItems {
            window_id,
            control_id: ControlId::new(10),
            items: vec!["Three".into(), "Four".into()],
        },
        PlatformCommand::SetTabBarSelection {
            window_id,
            control_id: ControlId::new(10),
            selected_index: 1,
        },
        PlatformCommand::CreateToggleSwitch {
            window_id,
            parent_control_id: Some(ControlId::new(1)),
            control_id: ControlId::new(11),
            label: "Toggle".into(),
            checked: false,
        },
        PlatformCommand::SetToggleSwitchState {
            window_id,
            control_id: ControlId::new(11),
            checked: true,
        },
        PlatformCommand::CreateProgressBar {
            window_id,
            parent_control_id: Some(ControlId::new(1)),
            control_id: ControlId::new(12),
        },
        PlatformCommand::SetProgressBarRange {
            window_id,
            control_id: ControlId::new(12),
            min: 10,
            max: 90,
        },
        PlatformCommand::SetProgressBarPosition {
            window_id,
            control_id: ControlId::new(12),
            position: 55,
        },
        PlatformCommand::CreateSplitter {
            window_id,
            parent_control_id: Some(ControlId::new(1)),
            control_id: ControlId::new(13),
            orientation: SplitterOrientation::Vertical,
        },
        PlatformCommand::SetControlEnabled {
            window_id,
            control_id: ControlId::new(2),
            enabled: false,
        },
        PlatformCommand::SetInputText {
            window_id,
            control_id: ControlId::new(4),
            text: "edited".into(),
        },
        PlatformCommand::SetControlText {
            window_id,
            control_id: ControlId::new(2),
            text: "Press".into(),
        },
        PlatformCommand::SetViewerContent {
            window_id,
            control_id: ControlId::new(5),
            text: "viewer".into(),
        },
        PlatformCommand::SetRichEditContent {
            window_id,
            control_id: ControlId::new(5),
            rtf_text: "{\\rtf1}".into(),
        },
        PlatformCommand::SetFocus {
            window_id,
            control_id: ControlId::new(4),
            select_all: true,
        },
        PlatformCommand::DefineLayout {
            window_id,
            rules: vec![LayoutRule {
                control_id: ControlId::new(1),
                parent_control_id: None,
                dock_style: DockStyle::Fill,
                order: 0,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            }],
        },
    ] {
        backend.execute_platform_command(command).unwrap();
    }

    let snapshot = backend.snapshot();
    let window = &snapshot.windows[0];
    assert_eq!(window.title, "Updated");
    assert!(window.shown);
    assert_eq!(window.controls.len(), 13);
    let json = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(json["windows"][0]["controls"][1]["kind"], "button");
    assert_eq!(json["windows"][0]["controls"][2]["text"], "Status");
    assert_eq!(json["windows"][0]["controls"][2]["class"], "status_bar");
    assert_eq!(json["windows"][0]["controls"][2]["severity"], "warning");
    assert_eq!(json["windows"][0]["controls"][5]["density"], "compact");
    assert_eq!(
        json["windows"][0]["controls"][12]["orientation"],
        "vertical"
    );
    assert_eq!(json["windows"][0]["layout_rules"][0]["dock_style"], "fill");
    assert_eq!(json["windows"][0]["controls"][9]["selected_index"], 1);
    assert_eq!(json["windows"][0]["controls"][9]["items"][0], "Three");
    assert_eq!(json["windows"][0]["controls"][11]["position"], 55);
    assert_eq!(json["windows"][0]["controls"][5]["selected_item_id"], 99);
}

#[test]
fn snapshot_uses_stable_names_for_protocol_fields() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    harness
        .backend
        .execute_platform_command(PlatformCommand::CreateLabel {
            window_id,
            parent_control_id: None,
            control_id: ControlId::new(1),
            initial_text: "Label".into(),
            class: LabelClass::StatusBar,
        })
        .unwrap();
    harness
        .backend
        .execute_platform_command(PlatformCommand::UpdateLabelText {
            window_id,
            control_id: ControlId::new(1),
            text: "Updated".into(),
            severity: MessageSeverity::Error,
        })
        .unwrap();
    harness
        .backend
        .execute_platform_command(PlatformCommand::CreateListBox {
            window_id,
            parent_control_id: None,
            control_id: ControlId::new(2),
        })
        .unwrap();
    harness
        .backend
        .execute_platform_command(PlatformCommand::SetListBoxRowDensity {
            window_id,
            control_id: ControlId::new(2),
            density: ListBoxRowDensity::Expanded,
        })
        .unwrap();
    harness
        .backend
        .execute_platform_command(PlatformCommand::DefineLayout {
            window_id,
            rules: vec![LayoutRule {
                control_id: ControlId::new(1),
                parent_control_id: None,
                dock_style: DockStyle::ProportionalFill { weight: 1.0 },
                order: 0,
                fixed_size: None,
                margin: (0, 0, 0, 0),
            }],
        })
        .unwrap();
    harness
        .backend
        .execute_platform_command(PlatformCommand::ShowMessageBox {
            window_id,
            title: "Notice".into(),
            message: "Hello".into(),
            severity: MessageSeverity::Information,
        })
        .unwrap();
    harness
        .backend
        .execute_platform_command(PlatformCommand::ShowFormDialog {
            window_id,
            form: FormDialogDescriptor {
                title: "Form".into(),
                context_tag: "ctx".into(),
                rows: vec![FormRow::Note {
                    text: "Note".into(),
                    severity: MessageSeverity::Warning,
                }],
                fields: vec![FormField::TextInput {
                    field_id: "field".into(),
                    label: "Field".into(),
                    value: String::new(),
                    validation: FormTextValidation::PathSegment,
                    live_warning: None,
                }],
                buttons: FormButtons {
                    confirm_label: "OK".into(),
                    cancel_label: "Cancel".into(),
                    confirm_enabled: true,
                },
            },
        })
        .unwrap();

    let snapshot = serde_json::from_str::<Value>(&harness.snapshot().unwrap()).unwrap();
    assert_eq!(snapshot["windows"][0]["controls"][0]["class"], "status_bar");
    assert_eq!(snapshot["windows"][0]["controls"][0]["severity"], "error");
    assert_eq!(snapshot["windows"][0]["controls"][1]["density"], "expanded");
    assert_eq!(
        snapshot["windows"][0]["layout_rules"][0]["dock_style"],
        "proportional_fill"
    );
    assert_eq!(snapshot["dialog_requests"][0]["kind"], "message_box");
    assert_eq!(
        snapshot["dialog_requests"][0]["details"]["severity"],
        "information"
    );
    assert_eq!(
        snapshot["dialog_requests"][1]["details"]["form"]["rows"][0]["severity"],
        "warning"
    );
    assert_eq!(
        snapshot["dialog_requests"][1]["details"]["form"]["fields"][0]["validation"],
        "path_segment"
    );
}

#[test]
fn treeview_commands_update_state_and_snapshot_order() {
    let mut backend = HeadlessBackend::new("app".into());
    let window_id = backend.create_window(WindowConfig {
        title: "Window",
        width: 320,
        height: 240,
    });
    backend
        .execute_platform_command(PlatformCommand::CreateTreeView {
            window_id,
            parent_control_id: None,
            control_id: ControlId::new(1),
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::PopulateTreeView {
            window_id,
            control_id: ControlId::new(1),
            items: vec![TreeItemDescriptor {
                id: TreeItemId::new(10),
                text: "Parent".into(),
                is_folder: true,
                state: CheckState::Unchecked,
                style_override: Some(StyleId::TreeItemDisabled),
                children: vec![
                    TreeItemDescriptor {
                        id: TreeItemId::new(11),
                        text: "Child B".into(),
                        is_folder: false,
                        state: CheckState::Checked,
                        style_override: None,
                        children: vec![],
                    },
                    TreeItemDescriptor {
                        id: TreeItemId::new(12),
                        text: "Child A".into(),
                        is_folder: false,
                        state: CheckState::Unchecked,
                        style_override: Some(StyleId::DefaultText),
                        children: vec![],
                    },
                ],
            }],
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::UpdateTreeItemText {
            window_id,
            control_id: ControlId::new(1),
            item_id: TreeItemId::new(12),
            text: "Child A+".into(),
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::UpdateTreeItemVisualState {
            window_id,
            control_id: ControlId::new(1),
            item_id: TreeItemId::new(11),
            new_state: CheckState::Hidden,
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::SetTreeViewSelection {
            window_id,
            control_id: ControlId::new(1),
            item_id: TreeItemId::new(12),
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::ExpandAllTreeItems {
            window_id,
            control_id: ControlId::new(1),
        })
        .unwrap();

    let snapshot =
        serde_json::from_str::<Value>(&serde_json::to_string(&backend.snapshot()).unwrap())
            .unwrap();
    let tree = &snapshot["windows"][0]["controls"][0];
    assert_eq!(tree["kind"], "tree_view");
    assert_eq!(tree["selected_item_id"], 12);
    assert_eq!(tree["items"][0]["style_override"], "TreeItemDisabled");
    assert_eq!(tree["items"][0]["children"][0]["text"], "Child B");
    assert_eq!(tree["items"][0]["children"][1]["text"], "Child A+");
    assert_eq!(tree["items"][0]["children"][0]["expanded"], true);
    assert_eq!(tree["items"][0]["children"][1]["expanded"], true);
    assert_eq!(tree["items"][0]["children"][0]["state"], "Hidden");
    assert!(matches!(
        backend.follow_up_events.pop_front(),
        Some(AppEvent::TreeViewItemSelectionChanged {
            window_id: got_window_id,
            item_id: got_item_id,
        }) if got_window_id == window_id && got_item_id == TreeItemId::new(12)
    ));
}

#[test]
fn hidden_tree_toggle_is_silent() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler.clone(),
            provider,
            vec![
                PlatformCommand::CreateTreeView {
                    window_id,
                    parent_control_id: None,
                    control_id: ControlId::new(1),
                },
                PlatformCommand::PopulateTreeView {
                    window_id,
                    control_id: ControlId::new(1),
                    items: vec![TreeItemDescriptor {
                        id: TreeItemId::new(7),
                        text: "Row".into(),
                        is_folder: false,
                        state: CheckState::Hidden,
                        style_override: None,
                        children: vec![],
                    }],
                },
                PlatformCommand::ShowWindow { window_id },
            ],
        )
        .unwrap();

    harness
        .toggle_tree_item(window_id, ControlId::new(1), TreeItemId::new(7))
        .unwrap();
    assert!(handler.lock().unwrap().events.is_empty());
    let snapshot = serde_json::from_str::<Value>(&harness.snapshot().unwrap()).unwrap();
    assert_eq!(
        snapshot["windows"][0]["controls"][0]["items"][0]["state"],
        "Hidden"
    );
}

#[test]
fn chart_menu_style_and_scroll_commands_update_snapshot() {
    let mut backend = HeadlessBackend::new("app".into());
    let window_id = backend.create_window(WindowConfig {
        title: "Window",
        width: 320,
        height: 240,
    });
    backend
        .execute_platform_command(PlatformCommand::CreateButton {
            window_id,
            parent_control_id: None,
            control_id: ControlId::new(1),
            text: "Button".into(),
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::CreateChart {
            window_id,
            parent_control_id: None,
            control_id: ControlId::new(2),
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::CreateTabBar {
            window_id,
            parent_control_id: None,
            control_id: ControlId::new(3),
            items: vec!["One".into(), "Two".into()],
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::CreateToggleSwitch {
            window_id,
            parent_control_id: None,
            control_id: ControlId::new(4),
            label: "Toggle".into(),
            checked: false,
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::SetChartData {
            window_id,
            control_id: ControlId::new(2),
            data: ChartDataPacket {
                lines: vec![ChartLineData {
                    label: "Alpha".into(),
                    weekly_counts: vec![1, 2, 3],
                    color: 0x00FF00,
                    end_label: Some("A".into()),
                    emphasis: ChartLineEmphasis::Secondary,
                }],
                week_labels: vec!["W1".into(), "W2".into(), "W3".into()],
                is_loading: false,
                show_x_axis_labels: true,
                show_y_axis_labels: true,
                show_end_labels: true,
            },
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::CreateMainMenu {
            window_id,
            menu_items: vec![MenuItemConfig {
                action: None,
                text: "&File".into(),
                children: vec![MenuItemConfig {
                    action: Some(MenuActionId::new(42)),
                    text: "Exit".into(),
                    children: vec![],
                }],
            }],
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::DefineStyle {
            style_id: StyleId::PrimaryButton,
            style: ControlStyle::default(),
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::ApplyStyleToControl {
            window_id,
            control_id: ControlId::new(1),
            style_id: StyleId::PrimaryButton,
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::SetTabBarStyle {
            window_id,
            control_id: ControlId::new(3),
            background_color: Color::default(),
            text_color: Color::default(),
            accent_color: Color::default(),
            font: None,
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::SetToggleSwitchStyle {
            window_id,
            control_id: ControlId::new(4),
            background: Color::default(),
            pill_off: Color::default(),
            pill_on: Color::default(),
            knob: Color::default(),
            text: Color::default(),
        })
        .unwrap();
    backend
        .execute_platform_command(PlatformCommand::SetScrollPosition {
            window_id,
            control_id: ControlId::new(1),
            vertical_pos: 25,
            horizontal_pos: 75,
        })
        .unwrap();

    let snapshot =
        serde_json::from_str::<Value>(&serde_json::to_string(&backend.snapshot()).unwrap())
            .unwrap();
    let window = &snapshot["windows"][0];
    assert_eq!(window["menu"][0]["children"][0]["action"], 42);
    let chart = &window["controls"][1];
    assert_eq!(chart["kind"], "chart");
    assert_eq!(chart["data"]["lines"][0]["emphasis"], "Secondary");
    assert!(chart["data"]["lines"][0].get("color").is_none());
    assert_eq!(window["controls"][0]["style_id"], "PrimaryButton");
    assert_eq!(window["controls"][0]["scroll_vertical"], 25);
    assert_eq!(window["controls"][0]["scroll_horizontal"], 75);
}

#[test]
fn scroll_action_emits_event_and_rejects_list_box() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler.clone(),
            provider,
            vec![
                PlatformCommand::CreateInput {
                    window_id,
                    parent_control_id: None,
                    control_id: ControlId::new(1),
                    initial_text: String::new(),
                    read_only: false,
                    multiline: false,
                    vertical_scroll: true,
                },
                PlatformCommand::CreateListBox {
                    window_id,
                    parent_control_id: None,
                    control_id: ControlId::new(2),
                },
                PlatformCommand::ShowWindow { window_id },
            ],
        )
        .unwrap();

    harness
        .scroll(window_id, ControlId::new(1), 33, 44)
        .unwrap();
    let events = &handler.lock().unwrap().events;
    assert!(matches!(
        events.last(),
        Some(AppEvent::ControlScrolled {
            vertical_pos: 33,
            horizontal_pos: 44,
            ..
        })
    ));
    let snapshot = serde_json::from_str::<Value>(&harness.snapshot().unwrap()).unwrap();
    assert_eq!(snapshot["windows"][0]["controls"][0]["scroll_vertical"], 33);
    assert_eq!(
        snapshot["windows"][0]["controls"][0]["scroll_horizontal"],
        44
    );
    assert!(harness.scroll(window_id, ControlId::new(2), 1, 1).is_err());
}

#[test]
fn radio_buttons_are_scoped_by_group_start() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler,
            provider,
            vec![
                PlatformCommand::CreatePanel {
                    window_id,
                    parent_control_id: None,
                    control_id: ControlId::new(1),
                },
                PlatformCommand::CreateRadioButton {
                    window_id,
                    parent_control_id: Some(ControlId::new(1)),
                    control_id: ControlId::new(10),
                    text: "A".into(),
                    group_start: true,
                },
                PlatformCommand::CreateRadioButton {
                    window_id,
                    parent_control_id: Some(ControlId::new(1)),
                    control_id: ControlId::new(11),
                    text: "B".into(),
                    group_start: false,
                },
                PlatformCommand::CreateRadioButton {
                    window_id,
                    parent_control_id: Some(ControlId::new(1)),
                    control_id: ControlId::new(12),
                    text: "C".into(),
                    group_start: true,
                },
                PlatformCommand::CreateRadioButton {
                    window_id,
                    parent_control_id: Some(ControlId::new(1)),
                    control_id: ControlId::new(13),
                    text: "D".into(),
                    group_start: false,
                },
                PlatformCommand::ShowWindow { window_id },
            ],
        )
        .unwrap();

    harness.select_radio(window_id, ControlId::new(11)).unwrap();
    harness.select_radio(window_id, ControlId::new(13)).unwrap();
    let snapshot = serde_json::from_str::<Value>(&harness.snapshot().unwrap()).unwrap();
    assert_eq!(snapshot["windows"][0]["controls"][1]["checked"], false);
    assert_eq!(snapshot["windows"][0]["controls"][2]["checked"], true);
    assert_eq!(snapshot["windows"][0]["controls"][3]["checked"], false);
    assert_eq!(snapshot["windows"][0]["controls"][4]["checked"], true);
}

#[test]
fn semantic_actions_emit_events_and_mutate_state() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler.clone(),
            provider,
            vec![
                PlatformCommand::CreateButton {
                    window_id,
                    parent_control_id: None,
                    control_id: ControlId::new(1),
                    text: "Click".into(),
                },
                PlatformCommand::CreateInput {
                    window_id,
                    parent_control_id: None,
                    control_id: ControlId::new(2),
                    initial_text: String::new(),
                    read_only: false,
                    multiline: false,
                    vertical_scroll: false,
                },
                PlatformCommand::CreateListBox {
                    window_id,
                    parent_control_id: None,
                    control_id: ControlId::new(3),
                },
                PlatformCommand::PopulateListBox {
                    window_id,
                    control_id: ControlId::new(3),
                    items: vec![ListBoxItemDescriptor {
                        id: ListBoxItemId::new(7),
                        badges: vec![],
                        title: "Row".into(),
                        metadata: String::new(),
                        enabled: false,
                    }],
                    badge_column_width: 0,
                },
                PlatformCommand::CreateComboBox {
                    window_id,
                    parent_control_id: None,
                    control_id: ControlId::new(4),
                },
                PlatformCommand::SetComboBoxItems {
                    window_id,
                    control_id: ControlId::new(4),
                    items: vec!["A".into(), "B".into()],
                },
                PlatformCommand::CreateTabBar {
                    window_id,
                    parent_control_id: None,
                    control_id: ControlId::new(5),
                    items: vec!["One".into(), "Two".into()],
                },
                PlatformCommand::CreateCheckBox {
                    window_id,
                    parent_control_id: None,
                    control_id: ControlId::new(6),
                    text: "Check".into(),
                },
                PlatformCommand::CreateRadioButton {
                    window_id,
                    parent_control_id: None,
                    control_id: ControlId::new(7),
                    text: "Radio".into(),
                    group_start: true,
                },
                PlatformCommand::CreateToggleSwitch {
                    window_id,
                    parent_control_id: None,
                    control_id: ControlId::new(8),
                    label: "Toggle".into(),
                    checked: false,
                },
                PlatformCommand::ShowWindow { window_id },
            ],
        )
        .unwrap();

    harness
        .set_text(window_id, ControlId::new(2), "typed")
        .unwrap();
    harness
        .select_row(window_id, ControlId::new(3), ListBoxItemId::new(7))
        .unwrap();
    harness
        .select_combo(window_id, ControlId::new(4), 1)
        .unwrap();
    harness.select_tab(window_id, ControlId::new(5), 1).unwrap();
    harness.toggle(window_id, ControlId::new(6)).unwrap();
    harness.select_radio(window_id, ControlId::new(7)).unwrap();
    harness.toggle(window_id, ControlId::new(8)).unwrap();
    harness.click(window_id, ControlId::new(1)).unwrap();

    let events = &handler.lock().unwrap().events;
    assert!(matches!(events[0], AppEvent::InputTextChanged { .. }));
    assert!(matches!(
        events[1],
        AppEvent::ListBoxItemSelectionChanged { .. }
    ));
    assert!(matches!(
        events[2],
        AppEvent::ComboBoxSelectionChanged { .. }
    ));
    assert!(matches!(events[3], AppEvent::TabBarSelectionChanged { .. }));
    assert!(matches!(
        events[4],
        AppEvent::CheckBoxToggled { checked: true, .. }
    ));
    assert!(matches!(events[5], AppEvent::RadioButtonSelected { .. }));
    assert!(matches!(
        events[6],
        AppEvent::ToggleSwitchToggled { checked: true, .. }
    ));
    assert!(matches!(events[7], AppEvent::ButtonClicked { .. }));
    let snapshot = serde_json::from_str::<Value>(&harness.snapshot().unwrap()).unwrap();
    assert_eq!(snapshot["windows"][0]["controls"][1]["text"], "typed");
    assert_eq!(snapshot["windows"][0]["controls"][2]["selected_item_id"], 7);
    assert_eq!(snapshot["windows"][0]["controls"][3]["selected_index"], 1);
    assert_eq!(snapshot["windows"][0]["controls"][4]["selected_index"], 1);
    assert_eq!(snapshot["windows"][0]["controls"][5]["checked"], true);
    assert_eq!(snapshot["windows"][0]["controls"][6]["checked"], true);
    assert_eq!(snapshot["windows"][0]["controls"][7]["checked"], true);
}

#[test]
fn semantic_action_reaction_commands_apply_within_same_call() {
    struct ReactionHandler {
        events: Vec<AppEvent>,
        command_queue: VecDeque<PlatformCommand>,
        target_window: WindowId,
    }

    impl PlatformEventHandler for ReactionHandler {
        fn handle_event(&mut self, event: AppEvent) {
            self.events.push(event);
            if matches!(self.events.last(), Some(AppEvent::ButtonClicked { .. })) {
                self.command_queue
                    .push_back(PlatformCommand::SetWindowTitle {
                        window_id: self.target_window,
                        title: "Updated".into(),
                    });
            }
        }

        fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
            self.command_queue.pop_front()
        }
    }

    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(ReactionHandler {
        events: Vec::new(),
        command_queue: VecDeque::new(),
        target_window: window_id,
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler,
            provider,
            vec![
                PlatformCommand::CreateButton {
                    window_id,
                    parent_control_id: None,
                    control_id: ControlId::new(1),
                    text: "Click".into(),
                },
                PlatformCommand::ShowWindow { window_id },
            ],
        )
        .unwrap();

    harness.click(window_id, ControlId::new(1)).unwrap();
    let snapshot = serde_json::from_str::<Value>(&harness.snapshot().unwrap()).unwrap();
    assert_eq!(snapshot["windows"][0]["title"], "Updated");
}

#[test]
fn inject_raw_can_drive_user_requested_close_flow() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::from([PlatformCommand::CloseWindow { window_id }]),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler,
            provider,
            vec![PlatformCommand::ShowWindow { window_id }],
        )
        .unwrap();
    harness
        .inject_raw(AppEvent::WindowCloseRequestedByUser { window_id })
        .unwrap();

    let snapshot = harness.snapshot().unwrap();
    let parsed: Value = serde_json::from_str(&snapshot).unwrap();
    assert!(parsed["windows"][0]["closed"].as_bool().unwrap());
}

#[test]
fn semantic_actions_reject_hidden_and_read_only_inputs() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness
        .start(
            handler,
            provider,
            vec![
                PlatformCommand::CreateInput {
                    window_id,
                    parent_control_id: None,
                    control_id: ControlId::new(1),
                    initial_text: String::new(),
                    read_only: true,
                    multiline: false,
                    vertical_scroll: false,
                },
                PlatformCommand::ShowWindow { window_id },
            ],
        )
        .unwrap();

    assert!(
        harness
            .set_text(window_id, ControlId::new(1), "nope")
            .is_err()
    );
    assert!(
        harness
            .select_combo(window_id, ControlId::new(1), 0)
            .is_err()
    );
    assert!(harness.select_tab(window_id, ControlId::new(1), 0).is_err());
}

#[test]
fn wait_for_times_out_when_marker_missing() {
    let mut harness = HeadlessHarness::new("app");
    let err = harness
        .wait_for("missing", Duration::from_millis(1))
        .unwrap_err();
    assert!(matches!(err, PlatformError::OperationFailed(_)));
}

#[test]
fn wait_until_returns_immediately_when_condition_is_already_true() {
    let (mut harness, window_id, _handler, _provider) =
        started_harness(vec![PlatformCommand::ShowWindow {
            window_id: WindowId::new(1),
        }]);

    harness
        .wait_until(
            |snapshot| {
                snapshot["windows"][0]["shown"].as_bool() == Some(true)
                    && snapshot["windows"][0]["title"] == "Window"
            },
            Duration::from_millis(5),
        )
        .unwrap();

    assert_eq!(window_id.raw(), 1);
}

#[test]
fn wait_until_pumps_until_condition_becomes_true() {
    let (mut harness, window_id, _handler, _provider) =
        started_harness(vec![PlatformCommand::ShowWindow {
            window_id: WindowId::new(1),
        }]);
    harness
        .backend
        .command_queue
        .push_back(PlatformCommand::SetWindowTitle {
            window_id,
            title: "Updated".into(),
        });

    harness
        .wait_until(
            |snapshot| snapshot["windows"][0]["title"] == "Updated",
            Duration::from_millis(5),
        )
        .unwrap();
}

#[test]
fn wait_until_times_out_when_condition_is_never_true() {
    let mut harness = HeadlessHarness::new("app");
    let err = harness
        .wait_until(
            |snapshot| {
                snapshot["windows"]
                    .as_array()
                    .is_some_and(|windows| !windows.is_empty())
            },
            Duration::from_millis(1),
        )
        .unwrap_err();
    assert!(matches!(err, PlatformError::OperationFailed(_)));
}

#[test]
fn inject_raw_requires_an_event_handler() {
    let mut harness = HeadlessHarness::new("app");
    let err = harness
        .inject_raw(AppEvent::WindowCloseRequestedByUser {
            window_id: WindowId::new(1),
        })
        .unwrap_err();
    assert!(matches!(err, PlatformError::OperationFailed(_)));
}

#[test]
fn dialog_commands_emit_completions_and_record_requests() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness.set_dialog_responder(vec![
        DialogScriptEntry {
            matcher: DialogMatcher {
                kind: DialogKind::SaveFile,
                window_id: Some(window_id),
                title: Some("Save".into()),
                prompt: None,
                context_tag: None,
            },
            outcome: DialogOutcome::SaveFile {
                result: Some(PathBuf::from("export.txt")),
            },
        },
        DialogScriptEntry {
            matcher: DialogMatcher {
                kind: DialogKind::OpenFile,
                window_id: Some(window_id),
                title: Some("Open".into()),
                prompt: None,
                context_tag: None,
            },
            outcome: DialogOutcome::OpenFile {
                result: Some(PathBuf::from("profile.json")),
            },
        },
        DialogScriptEntry {
            matcher: DialogMatcher {
                kind: DialogKind::ProfileSelection,
                window_id: Some(window_id),
                title: Some("Profiles".into()),
                prompt: Some("Choose".into()),
                context_tag: None,
            },
            outcome: DialogOutcome::ProfileSelection {
                chosen_profile_name: Some("Default".into()),
                create_new_requested: false,
                user_cancelled: false,
            },
        },
        DialogScriptEntry {
            matcher: DialogMatcher {
                kind: DialogKind::Input,
                window_id: Some(window_id),
                title: Some("Prompt".into()),
                prompt: Some("Enter value".into()),
                context_tag: Some("ctx".into()),
            },
            outcome: DialogOutcome::Input {
                text: Some("typed".into()),
            },
        },
        DialogScriptEntry {
            matcher: DialogMatcher {
                kind: DialogKind::ExcludePatterns,
                window_id: Some(window_id),
                title: Some("Patterns".into()),
                prompt: None,
                context_tag: None,
            },
            outcome: DialogOutcome::ExcludePatterns {
                saved: true,
                patterns: "target/\n*.tmp".into(),
            },
        },
        DialogScriptEntry {
            matcher: DialogMatcher {
                kind: DialogKind::Form,
                window_id: Some(window_id),
                title: Some("Form".into()),
                prompt: None,
                context_tag: Some("form-tag".into()),
            },
            outcome: DialogOutcome::Form {
                confirmed: true,
                field_values: vec![
                    FormFieldValue::Text {
                        field_id: "name".into(),
                        value: "Alice".into(),
                    },
                    FormFieldValue::CheckBox {
                        field_id: "enabled".into(),
                        checked: true,
                    },
                ],
            },
        },
        DialogScriptEntry {
            matcher: DialogMatcher {
                kind: DialogKind::FolderPicker,
                window_id: Some(window_id),
                title: Some("Folder".into()),
                prompt: None,
                context_tag: None,
            },
            outcome: DialogOutcome::FolderPicker {
                path: Some(PathBuf::from("C:/tmp")),
            },
        },
    ]);
    harness
        .start(
            handler.clone(),
            provider,
            vec![
                PlatformCommand::ShowSaveFileDialog {
                    window_id,
                    title: "Save".into(),
                    default_filename: "export.txt".into(),
                    filter_spec: "*.txt".into(),
                    initial_dir: Some(PathBuf::from("C:/temp")),
                },
                PlatformCommand::ShowOpenFileDialog {
                    window_id,
                    title: "Open".into(),
                    filter_spec: "*.json".into(),
                    initial_dir: None,
                },
                PlatformCommand::ShowProfileSelectionDialog {
                    window_id,
                    available_profiles: vec!["Default".into()],
                    title: "Profiles".into(),
                    prompt: "Choose".into(),
                },
                PlatformCommand::ShowInputDialog {
                    window_id,
                    title: "Prompt".into(),
                    prompt: "Enter value".into(),
                    default_text: Some("seed".into()),
                    context_tag: Some("ctx".into()),
                },
                PlatformCommand::ShowExcludePatternsDialog {
                    window_id,
                    title: "Patterns".into(),
                    patterns: "target/\n*.tmp".into(),
                },
                PlatformCommand::ShowFormDialog {
                    window_id,
                    form: FormDialogDescriptor {
                        title: "Form".into(),
                        context_tag: "form-tag".into(),
                        rows: vec![
                            FormRow::ReadOnlyText {
                                label: "Info".into(),
                                value: "Value".into(),
                            },
                            FormRow::Note {
                                text: "Note".into(),
                                severity: MessageSeverity::Information,
                            },
                        ],
                        fields: vec![
                            FormField::TextInput {
                                field_id: "name".into(),
                                label: "Name".into(),
                                value: String::new(),
                                validation: FormTextValidation::Any,
                                live_warning: None,
                            },
                            FormField::CheckBox {
                                field_id: "enabled".into(),
                                label: "Enabled".into(),
                                checked: false,
                            },
                        ],
                        buttons: FormButtons {
                            confirm_label: "OK".into(),
                            cancel_label: "Cancel".into(),
                            confirm_enabled: true,
                        },
                    },
                },
                PlatformCommand::ShowFolderPickerDialog {
                    window_id,
                    title: "Folder".into(),
                    initial_dir: Some(PathBuf::from("C:/tmp")),
                },
                PlatformCommand::ShowMessageBox {
                    window_id,
                    title: "Notice".into(),
                    message: "Hello".into(),
                    severity: MessageSeverity::Information,
                },
            ],
        )
        .unwrap();

    let events = &handler.lock().unwrap().events;
    assert_eq!(events.len(), 7);
    assert!(matches!(
        events[0],
        AppEvent::FileSaveDialogCompleted {
            window_id: got,
            result: Some(_),
        } if got == window_id
    ));
    assert!(matches!(
        events[1],
        AppEvent::FileOpenProfileDialogCompleted {
            window_id: got,
            result: Some(_),
        } if got == window_id
    ));
    assert!(matches!(
        events[2],
        AppEvent::ProfileSelectionDialogCompleted {
            chosen_profile_name: Some(_),
            create_new_requested: false,
            user_cancelled: false,
            ..
        }
    ));
    assert!(matches!(
        events[3],
        AppEvent::GenericInputDialogCompleted {
            text: Some(_),
            context_tag: Some(_),
            ..
        }
    ));
    assert!(matches!(
        &events[4],
        AppEvent::ExcludePatternsDialogCompleted {
            saved: true,
            patterns,
            ..
        } if patterns == "target/\n*.tmp"
    ));
    assert!(matches!(
        &events[5],
        AppEvent::FormDialogCompleted {
            context_tag,
            confirmed: true,
            field_values,
            ..
        } if context_tag == "form-tag" && field_values.len() == 2
    ));
    assert!(matches!(
        events[6],
        AppEvent::FolderPickerDialogCompleted { path: Some(_), .. }
    ));

    let snapshot = serde_json::from_str::<Value>(&harness.snapshot().unwrap()).unwrap();
    assert_eq!(snapshot["dialog_requests"].as_array().unwrap().len(), 8);
    assert_eq!(
        snapshot["dialog_requests"][0]["details"]["kind"],
        "save_file"
    );
    assert_eq!(
        snapshot["dialog_requests"][0]["details"]["default_filename"],
        "export.txt"
    );
    assert_eq!(
        snapshot["dialog_requests"][2]["details"]["available_profiles"][0],
        "Default"
    );
    assert_eq!(
        snapshot["dialog_requests"][5]["details"]["form"]["fields"][0]["kind"],
        "text_input"
    );
    assert_eq!(
        snapshot["dialog_requests"][5]["details"]["form"]["buttons"]["confirm_label"],
        "OK"
    );
    assert_eq!(snapshot["dialog_requests"][7]["kind"], "message_box");
    assert_eq!(
        snapshot["dialog_requests"][7]["details"]["kind"],
        "message_box"
    );
    assert_eq!(
        snapshot["dialog_requests"][7]["details"]["severity"],
        "information"
    );
}

#[test]
fn dialog_responder_uses_ordered_matches_and_field_constraints() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness.set_dialog_responder(vec![
        DialogScriptEntry {
            matcher: DialogMatcher {
                kind: DialogKind::Input,
                window_id: Some(window_id),
                title: Some("First".into()),
                prompt: Some("Prompt 1".into()),
                context_tag: Some("tag".into()),
            },
            outcome: DialogOutcome::Input {
                text: Some("one".into()),
            },
        },
        DialogScriptEntry {
            matcher: DialogMatcher {
                kind: DialogKind::Input,
                window_id: Some(window_id),
                title: Some("Second".into()),
                prompt: Some("Prompt 2".into()),
                context_tag: Some("tag".into()),
            },
            outcome: DialogOutcome::Input {
                text: Some("two".into()),
            },
        },
    ]);
    harness
        .start(
            handler.clone(),
            provider,
            vec![
                PlatformCommand::ShowInputDialog {
                    window_id,
                    title: "First".into(),
                    prompt: "Prompt 1".into(),
                    default_text: None,
                    context_tag: Some("tag".into()),
                },
                PlatformCommand::ShowInputDialog {
                    window_id,
                    title: "Second".into(),
                    prompt: "Prompt 2".into(),
                    default_text: None,
                    context_tag: Some("tag".into()),
                },
            ],
        )
        .unwrap();

    let events = &handler.lock().unwrap().events;
    assert!(matches!(
        events.as_slice(),
        [
            AppEvent::GenericInputDialogCompleted { text: Some(first), .. },
            AppEvent::GenericInputDialogCompleted { text: Some(second), .. }
        ] if first == "one" && second == "two"
    ));
}

#[test]
fn unmatched_dialog_defaults_to_cancel_without_consuming_responder() {
    let mut harness = HeadlessHarness::new("app");
    let window_id = harness
        .create_window(WindowConfig {
            title: "Window",
            width: 320,
            height: 240,
        })
        .unwrap();
    let handler = Arc::new(Mutex::new(TestHandler {
        events: Vec::new(),
        commands: VecDeque::new(),
    }));
    let provider = Arc::new(Mutex::new(SilentProvider));
    harness.set_dialog_responder(vec![DialogScriptEntry {
        matcher: DialogMatcher {
            kind: DialogKind::SaveFile,
            window_id: Some(window_id),
            title: Some("Different".into()),
            prompt: None,
            context_tag: None,
        },
        outcome: DialogOutcome::SaveFile {
            result: Some(PathBuf::from("scripted.txt")),
        },
    }]);
    harness
        .start(
            handler.clone(),
            provider,
            vec![PlatformCommand::ShowSaveFileDialog {
                window_id,
                title: "Actual".into(),
                default_filename: "export.txt".into(),
                filter_spec: "*.txt".into(),
                initial_dir: None,
            }],
        )
        .unwrap();

    let events = &handler.lock().unwrap().events;
    assert!(matches!(
        events.as_slice(),
        [AppEvent::FileSaveDialogCompleted { result: None, .. }]
    ));
    assert_eq!(harness.backend.dialog_responder.len(), 1);
}
