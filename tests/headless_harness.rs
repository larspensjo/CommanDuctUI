use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use commanductui::{
    AppEvent, ControlId, DockStyle, LayoutRule, PlatformCommand, PlatformEventHandler,
    UiStateProvider, WindowId, headless::HeadlessHarness,
};

const BTN_CLICK_ME: ControlId = ControlId::new(101);

struct DemoHandler {
    command_queue: VecDeque<PlatformCommand>,
    click_count: u32,
    main_window_id: WindowId,
}

impl PlatformEventHandler for DemoHandler {
    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::ButtonClicked { control_id, .. } if control_id == BTN_CLICK_ME => {
                self.click_count += 1;
                self.command_queue
                    .push_back(PlatformCommand::SetWindowTitle {
                        window_id: self.main_window_id,
                        title: format!("You clicked {} times!", self.click_count),
                    });
                self.command_queue.push_back(PlatformCommand::Checkpoint {
                    label: "done".to_string(),
                });
            }
            _ => {}
        }
    }

    fn try_dequeue_command(&mut self) -> Option<PlatformCommand> {
        self.command_queue.pop_front()
    }
}

struct StaticUiState;

impl UiStateProvider for StaticUiState {
    fn is_tree_item_new(&self, _window_id: WindowId, _item_id: commanductui::TreeItemId) -> bool {
        false
    }
}

#[test]
fn demo_app_reaches_done_and_snapshot_is_stable() {
    let mut harness = HeadlessHarness::new("CommanDuctUIExample");
    let main_window_id = harness
        .create_window(commanductui::WindowConfig {
            title: "My App",
            width: 400,
            height: 300,
        })
        .unwrap();

    let initial_commands = vec![
        PlatformCommand::CreateButton {
            window_id: main_window_id,
            parent_control_id: None,
            control_id: BTN_CLICK_ME,
            text: "Click Me".to_string(),
        },
        PlatformCommand::DefineLayout {
            window_id: main_window_id,
            rules: vec![LayoutRule {
                control_id: BTN_CLICK_ME,
                parent_control_id: None,
                dock_style: DockStyle::Top,
                order: 0,
                fixed_size: Some(40),
                margin: (12, 12, 0, 12),
            }],
        },
        PlatformCommand::ShowWindow {
            window_id: main_window_id,
        },
    ];

    let handler: Arc<Mutex<dyn PlatformEventHandler>> = Arc::new(Mutex::new(DemoHandler {
        command_queue: VecDeque::new(),
        click_count: 0,
        main_window_id,
    }));
    let ui_state_provider: Arc<Mutex<dyn UiStateProvider>> = Arc::new(Mutex::new(StaticUiState));

    harness
        .start(handler.clone(), ui_state_provider, initial_commands)
        .unwrap();
    harness.click(main_window_id, BTN_CLICK_ME).unwrap();
    harness
        .wait_for("done", Duration::from_millis(100))
        .unwrap();

    let snapshot = harness.snapshot().unwrap();
    assert!(snapshot.contains("\"title\": \"You clicked 1 times!\""));
    assert!(snapshot.contains("\"label\": \"done\"") || snapshot.contains("\"markers\": ["));
}
