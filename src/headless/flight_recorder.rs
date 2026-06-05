//! Pure, platform-agnostic flight-recorder core (Phase 2). Owns a shadow `HeadlessBackend`
//! and a generic `Write` sink; on each command it emits one flat JSON-line per changed
//! control. No `HWND`, no file I/O - fully CI-testable.
//!
//! Gated `#[cfg(test)]` until Phase 3 wires the `pub(crate)` facade and Win32 tap.

use super::backend::HeadlessBackend;
use super::snapshot::ControlSnapshot;
use super::state::{ControlState, WindowState};
use crate::{PlatformCommand, PlatformResult, WindowConfig, WindowId};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{self, Write};

/// Result of feeding one command to the recorder.
pub(super) struct RecordOutcome {
    /// Number of trace lines written for this command (one per changed control).
    pub(super) lines_emitted: usize,
    /// The shadow's accept/reject of the command. An `Err` is a headless/Win32 fidelity
    /// discrepancy the Phase 3 facade logs and ignores - never fatal.
    pub(super) shadow_result: PlatformResult<()>,
}

pub(super) struct RecorderCore<W: Write> {
    backend: HeadlessBackend,
    sink: W,
    seq: u64,
    prev: HashMap<(usize, i32), String>,
}

impl<W: Write> RecorderCore<W> {
    pub(super) fn new(app_name: impl Into<String>, sink: W) -> Self {
        Self {
            backend: HeadlessBackend::new(app_name.into()),
            sink,
            seq: 0,
            prev: HashMap::new(),
        }
    }

    /// Mirror a window the Win32 side created, preserving the exact id.
    /// Emits nothing and does not advance `seq` - windows have no row in the format.
    pub(super) fn record_window_created(
        &mut self,
        window_id: WindowId,
        title: &str,
        width: i32,
        height: i32,
    ) {
        self.backend.create_window_with_id(
            window_id,
            WindowConfig {
                title,
                width,
                height,
            },
        );
    }

    /// Apply one command to the shadow and emit a line per changed control.
    pub(super) fn record_command(&mut self, command: PlatformCommand) -> io::Result<RecordOutcome> {
        self.seq += 1;
        let shadow_result = self.backend.execute_platform_command(command);

        if shadow_result.is_err() {
            return Ok(RecordOutcome {
                lines_emitted: 0,
                shadow_result,
            });
        }

        let mut changed: Vec<(usize, i32, usize, u32, ControlSnapshot, String)> = Vec::new();
        for (&win_raw, window) in &self.backend.windows {
            for control in window.controls.values() {
                let snapshot = ControlSnapshot::from(control);
                let change_form =
                    serde_json::to_string(&snapshot).expect("ControlSnapshot serializes");
                let key = (win_raw, control.control_id.raw());
                if self.prev.get(&key) != Some(&change_form) {
                    let depth = control_depth(window, control);
                    changed.push((
                        win_raw,
                        control.control_id.raw(),
                        control.creation_order,
                        depth,
                        snapshot,
                        change_form,
                    ));
                }
            }
        }
        changed.sort_by_key(|(win, _id, order, ..)| (*win, *order));

        let mut lines_emitted = 0;
        for (win_raw, id_raw, _order, depth, snapshot, change_form) in changed {
            self.prev.insert((win_raw, id_raw), change_form);
            let line = project_line(self.seq, win_raw, depth, &snapshot);
            let indent = "  ".repeat(depth as usize);
            writeln!(self.sink, "{indent}{line}")?;
            lines_emitted += 1;
        }
        self.sink.flush()?;

        Ok(RecordOutcome {
            lines_emitted,
            shadow_result,
        })
    }
}

/// Nesting depth of a control within its window (0 = top-level), by walking the parent chain.
fn control_depth(window: &WindowState, control: &ControlState) -> u32 {
    let mut depth = 0;
    let mut current = control.parent_control_id;
    let bound = window.controls.len();
    while let Some(parent_id) = current {
        depth += 1;
        if depth as usize > bound {
            break;
        }
        current = window
            .controls
            .get(&parent_id.raw())
            .and_then(|parent| parent.parent_control_id);
    }
    depth
}

/// Flat trace projection of a `ControlSnapshot`: rename `parent_control_id` to `parent`
/// and inject `seq`/`win`/`depth`.
fn project_line(seq: u64, win: usize, depth: u32, snapshot: &ControlSnapshot) -> String {
    let mut value = serde_json::to_value(snapshot).expect("ControlSnapshot -> Value");
    if let Value::Object(map) = &mut value {
        let parent = map.remove("parent_control_id").unwrap_or(Value::Null);
        map.insert("parent".to_string(), parent);
        map.insert("seq".to_string(), Value::from(seq));
        map.insert("win".to_string(), Value::from(win));
        map.insert("depth".to_string(), Value::from(depth));
    }
    serde_json::to_string(&value).expect("Value -> String")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CheckState, ControlId, TreeItemDescriptor, TreeItemId};

    fn recorder_with_window() -> RecorderCore<Vec<u8>> {
        let mut core = RecorderCore::new("test", Vec::new());
        core.record_window_created(WindowId::new(1), "Main", 800, 600);
        core
    }

    fn lines(core: &RecorderCore<Vec<u8>>) -> Vec<Value> {
        String::from_utf8(core.sink.clone())
            .unwrap()
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str::<Value>(line.trim_start()).expect("valid json"))
            .collect()
    }

    #[test]
    fn first_appearance_of_control_emits_one_baseline_line() {
        let mut core = recorder_with_window();
        let before = core.sink.len();

        let outcome = core
            .record_command(PlatformCommand::CreateButton {
                window_id: WindowId::new(1),
                parent_control_id: None,
                control_id: ControlId::new(10),
                text: "OK".to_string(),
            })
            .expect("create button");

        assert_eq!(outcome.lines_emitted, 1);
        assert!(outcome.shadow_result.is_ok());

        let new_text = String::from_utf8(core.sink[before..].to_vec()).unwrap();
        let line: Value = serde_json::from_str(new_text.trim()).unwrap();
        assert_eq!(line["win"], serde_json::json!(1));
        assert_eq!(line["id"], serde_json::json!(10));
        assert_eq!(line["parent"], Value::Null);
        assert_eq!(line["depth"], serde_json::json!(0));
        assert_eq!(line["kind"], serde_json::json!("button"));
        assert_eq!(line["text"], serde_json::json!("OK"));
        assert_eq!(line["seq"], serde_json::json!(1));
        assert_eq!(lines(&core).len(), 1);
    }

    #[test]
    fn setting_a_control_to_its_current_value_emits_no_line() {
        let mut core = recorder_with_window();
        core.record_command(PlatformCommand::CreateProgressBar {
            window_id: WindowId::new(1),
            parent_control_id: None,
            control_id: ControlId::new(20),
        })
        .expect("create progress");

        let changed = core
            .record_command(PlatformCommand::SetProgressBarPosition {
                window_id: WindowId::new(1),
                control_id: ControlId::new(20),
                position: 40,
            })
            .expect("set position 40");
        assert_eq!(changed.lines_emitted, 1);

        let noop = core
            .record_command(PlatformCommand::SetProgressBarPosition {
                window_id: WindowId::new(1),
                control_id: ControlId::new(20),
                position: 40,
            })
            .expect("set position 40 again");
        assert_eq!(noop.lines_emitted, 0);
    }

    #[test]
    fn child_control_change_emits_only_the_child_not_the_parent() {
        let mut core = recorder_with_window();
        core.record_command(PlatformCommand::CreatePanel {
            window_id: WindowId::new(1),
            parent_control_id: None,
            control_id: ControlId::new(30),
        })
        .expect("create panel");
        core.record_command(PlatformCommand::CreateButton {
            window_id: WindowId::new(1),
            parent_control_id: Some(ControlId::new(30)),
            control_id: ControlId::new(31),
            text: "child".to_string(),
        })
        .expect("create child button");

        let before = core.sink.len();
        let outcome = core
            .record_command(PlatformCommand::SetControlText {
                window_id: WindowId::new(1),
                control_id: ControlId::new(31),
                text: "renamed".to_string(),
            })
            .expect("rename child");

        assert_eq!(outcome.lines_emitted, 1);
        let new_text = String::from_utf8(core.sink[before..].to_vec()).unwrap();
        let line: Value = serde_json::from_str(new_text.trim()).unwrap();
        assert_eq!(line["id"], serde_json::json!(31));
        assert_eq!(line["parent"], serde_json::json!(30));
        assert_eq!(line["depth"], serde_json::json!(1));
    }

    #[test]
    fn tree_item_change_emits_the_owning_treeview() {
        let mut core = recorder_with_window();
        core.record_command(PlatformCommand::CreateTreeView {
            window_id: WindowId::new(1),
            parent_control_id: None,
            control_id: ControlId::new(40),
        })
        .expect("create tree");
        core.record_command(PlatformCommand::PopulateTreeView {
            window_id: WindowId::new(1),
            control_id: ControlId::new(40),
            items: vec![TreeItemDescriptor {
                id: TreeItemId::new(100),
                text: "node".to_string(),
                is_folder: false,
                state: CheckState::Unchecked,
                style_override: None,
                children: Vec::new(),
            }],
        })
        .expect("populate tree");

        let before = core.sink.len();
        let outcome = core
            .record_command(PlatformCommand::UpdateTreeItemVisualState {
                window_id: WindowId::new(1),
                control_id: ControlId::new(40),
                item_id: TreeItemId::new(100),
                new_state: CheckState::Checked,
            })
            .expect("check item");

        assert_eq!(outcome.lines_emitted, 1);
        let new_text = String::from_utf8(core.sink[before..].to_vec()).unwrap();
        let line: Value = serde_json::from_str(new_text.trim()).unwrap();
        assert_eq!(line["id"], serde_json::json!(40));
        assert_eq!(line["kind"], serde_json::json!("tree_view"));
    }

    #[test]
    fn progress_two_scale_series_is_reproducible_from_trace() {
        let mut core = recorder_with_window();
        core.record_command(PlatformCommand::CreateProgressBar {
            window_id: WindowId::new(1),
            parent_control_id: None,
            control_id: ControlId::new(42),
        })
        .expect("create progress");

        let inputs = [12u32, 47, 15, 51, 18, 55];
        for pos in inputs {
            core.record_command(PlatformCommand::SetProgressBarPosition {
                window_id: WindowId::new(1),
                control_id: ControlId::new(42),
                position: pos,
            })
            .expect("set position");
        }

        let series: Vec<u64> = lines(&core)
            .into_iter()
            .filter(|line| line["id"] == serde_json::json!(42))
            .filter(|line| line["kind"] == serde_json::json!("progress_bar"))
            .filter_map(|line| line["position"].as_u64())
            .collect();

        assert_eq!(series, vec![0, 12, 47, 15, 51, 18, 55]);
    }

    #[test]
    fn rejected_command_mutates_nothing_and_emits_nothing() {
        let mut core = recorder_with_window();
        let before = core.sink.len();
        let outcome = core
            .record_command(PlatformCommand::SetProgressBarPosition {
                window_id: WindowId::new(1),
                control_id: ControlId::new(99),
                position: 50,
            })
            .expect("write itself must not fail");

        assert!(outcome.shadow_result.is_err());
        assert_eq!(outcome.lines_emitted, 0);
        assert_eq!(core.sink.len(), before);
    }

    struct FailingSink;

    impl Write for FailingSink {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("boom"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn write_failure_surfaces_io_error_without_panicking() {
        let mut core = RecorderCore::new("test", FailingSink);
        core.record_window_created(WindowId::new(1), "Main", 800, 600);

        let result = core.record_command(PlatformCommand::CreateButton {
            window_id: WindowId::new(1),
            parent_control_id: None,
            control_id: ControlId::new(10),
            text: "OK".to_string(),
        });
        assert!(result.is_err());
    }
}
