use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};

use commanductui::{
    AppEvent, ControlId, DockStyle, LayoutRule, PlatformCommand, PlatformEventHandler,
    PlatformResult, UiStateProvider, WindowId, headless::HeadlessHarness,
};

const BTN_CLICK_ME: ControlId = ControlId::new(101);
type AppLogicHandle = Arc<Mutex<dyn PlatformEventHandler>>;
type UiStateHandle = Arc<Mutex<dyn UiStateProvider>>;

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
    fn is_tree_item_new(&self, _window_id: WindowId, _item_id: commanductui::TreeItemId) -> bool {
        false
    }
}

fn build_app_core(
    main_window_id: WindowId,
) -> (AppLogicHandle, UiStateHandle, Vec<PlatformCommand>) {
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

    let app_logic: AppLogicHandle = Arc::new(Mutex::new(MyAppLogic {
        command_queue: VecDeque::new(),
        click_count: 0,
        main_window_id,
    }));
    let ui_state_provider: UiStateHandle = Arc::new(Mutex::new(StaticUiState));

    (app_logic, ui_state_provider, initial_commands)
}

#[cfg(target_os = "windows")]
fn headless_requested() -> bool {
    std::env::args().any(|arg| arg == "--headless")
        || std::env::var("COMMANDUCTUI_HEADLESS")
            .map(|value| {
                let value = value.trim().to_ascii_lowercase();
                !matches!(value.as_str(), "" | "0" | "false" | "off" | "no")
            })
            .unwrap_or(false)
}

fn run_headless_example() -> PlatformResult<()> {
    let mut harness = HeadlessHarness::new("CommanDuctUIExample");
    let main_window_id = harness.create_window(commanductui::WindowConfig {
        title: "My App",
        width: 400,
        height: 300,
    })?;
    let (app_logic, ui_state_provider, initial_commands) = build_app_core(main_window_id);
    harness.start(app_logic, ui_state_provider, initial_commands)?;
    harness.run_protocol(io::stdin().lock(), io::stdout().lock())
}

#[cfg(target_os = "windows")]
fn run_windows_example() -> PlatformResult<()> {
    let platform = commanductui::PlatformInterface::new("CommanDuctUIExample".to_string())?;
    let main_window_id = platform.create_window(commanductui::WindowConfig {
        title: "My App",
        width: 400,
        height: 300,
    })?;
    let (app_logic, ui_state_provider, initial_commands) = build_app_core(main_window_id);
    platform.main_event_loop(app_logic, ui_state_provider, initial_commands)
}

#[cfg(target_os = "windows")]
fn main() -> PlatformResult<()> {
    if headless_requested() {
        run_headless_example()
    } else {
        run_windows_example()
    }
}

#[cfg(not(target_os = "windows"))]
fn main() -> PlatformResult<()> {
    run_headless_example()
}
