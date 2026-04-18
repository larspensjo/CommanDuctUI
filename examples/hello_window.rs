#[cfg(target_os = "windows")]
mod windows_example {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use commanductui::{
        AppEvent, ControlId, DockStyle, LayoutRule, PlatformCommand, PlatformEventHandler,
        PlatformInterface, PlatformResult, UiStateProvider, WindowConfig, WindowId,
    };

    const BTN_CLICK_ME: ControlId = ControlId::new(101);

    struct MyAppLogic {
        command_queue: VecDeque<PlatformCommand>,
        click_count: u32,
        main_window_id: WindowId,
    }

    impl PlatformEventHandler for MyAppLogic {
        fn handle_event(&mut self, event: AppEvent) {
            match event {
                AppEvent::ButtonClicked { control_id, .. } if control_id == BTN_CLICK_ME => {
                    self.click_count += 1;
                    self.command_queue
                        .push_back(PlatformCommand::SetWindowTitle {
                            window_id: self.main_window_id,
                            title: format!("You clicked {} times!", self.click_count),
                        });
                }
                AppEvent::WindowCloseRequestedByUser { window_id } => {
                    self.command_queue
                        .push_back(PlatformCommand::CloseWindow { window_id });
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
        fn is_tree_item_new(
            &self,
            _window_id: WindowId,
            _item_id: commanductui::TreeItemId,
        ) -> bool {
            false
        }
    }

    pub fn run() -> PlatformResult<()> {
        let platform = PlatformInterface::new("CommanDuctUIExample".to_string())?;
        let main_window_id = platform.create_window(WindowConfig {
            title: "My App",
            width: 400,
            height: 300,
        })?;

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

        let app_logic: Arc<Mutex<dyn PlatformEventHandler>> = Arc::new(Mutex::new(MyAppLogic {
            command_queue: VecDeque::new(),
            click_count: 0,
            main_window_id,
        }));
        let ui_state_provider: Arc<Mutex<dyn UiStateProvider>> =
            Arc::new(Mutex::new(StaticUiState));

        platform.main_event_loop(app_logic, ui_state_provider, initial_commands)
    }
}

#[cfg(target_os = "windows")]
fn main() -> commanductui::PlatformResult<()> {
    windows_example::run()
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("The hello_window example is only available on Windows.");
}
