# CommanDuctUI

A declarative, command-driven Rust library for native Windows (Win32) UI development.

## What is CommanDuctUI?

CommanDuctUI provides a clean abstraction over the native Win32 API by using a command pattern. Instead of writing complex `WndProc` functions and managing native handles directly in your application logic, you interact with the UI in a declarative way:

1.  **Your application sends simple `PlatformCommand`s** to create windows, add controls, and update their state (e.g., `CreateButton`, `SetWindowTitle`).
2.  **The library receives user interactions and sends back platform-agnostic `AppEvent`s** for your application to handle (e.g., `ButtonClicked`, `WindowCloseRequestedByUser`).

This approach creates a strong boundary between your application's core logic and the UI implementation. It makes your application easier to test, maintain, and reason about, and is an ideal foundation for architectures like Model-View-Presenter (MVP) or MVVM.

### Core Concepts

-   **`PlatformCommand`**: An enum representing an instruction *to* the UI layer. Your application creates these and sends them to the library to execute.
-   **`AppEvent`**: An enum representing a notification *from* the UI layer. The library sends these to your application when a user does something.
-   **`PlatformEventHandler`**: A trait your application logic must implement to receive events and provide commands to the library.

## Headless / testable mode

Alongside the Win32 backend, CommanDuctUI ships a **headless backend** that interprets the
same `PlatformCommand` stream into an in-memory UI model and serializes it as JSON — no
window, no Win32, no human in the loop. It exists so you can drive a real application on
synthetic or live data and assert on the resulting UI state. It is **not** a TUI; the output
only needs to be faithful and machine-parseable.

The headless backend is pure Rust (model + `serde`) and is **always compiled, on every
platform** — including Linux and CI. Two ways to drive it:

- **In-process Rust harness** for integration tests — link your app-core and call
  `HeadlessHarness` methods directly:

  ```rust
  let mut harness = HeadlessHarness::new("MyApp");
  let window_id = harness.create_window(config)?;
  let (handler, provider, initial_commands) = build_app_core(window_id);
  harness.start(handler, provider, initial_commands)?;

  harness.click(window_id, some_button)?;
  harness.wait_for("done", std::time::Duration::from_secs(5))?;
  let json = harness.snapshot()?;   // assert on this
  ```

- **`--headless` stdio JSON protocol** for external / AI-driven harnesses — a separate
  process drives the shipped binary over JSON lines:

  ```powershell
  cargo run --example hello_window -- --headless
  ```

See **[docs/HeadlessMode.md](docs/HeadlessMode.md)** for the full guide (the app-core
pattern, both delivery shapes, the protocol, and dialog scripting).

## Integration as a Git Submodule

This library is designed to be integrated as a Git submodule, allowing for tight, coordinated development between the library and its consumer while maintaining a clean project separation.

### 1. Adding to a Project

To add `CommanDuctUI` to your main project (e.g., `MyProject`):

```bash
cd /path/to/MyProject

# Add the repository as a submodule in your source directory
git submodule add <url_to_CommanDuctUI_repo> src/CommanDuctUI
```

### 2. Updating `Cargo.toml`

Next, add the submodule as a local path dependency in your main project's `Cargo.toml`:

```toml
# In MyProject/Cargo.toml

[dependencies]
# ... your other dependencies
commanductui = { path = "src/CommanDuctUI" }
```

### 3. Cloning a Project with the Submodule

When cloning a project that contains this submodule, use the `--recurse-submodules` flag to ensure the submodule's code is also checked out:

```bash
git clone --recurse-submodules <url_to_MyProject_repo>
```

If you have already cloned the project, you can initialize the submodule with:

```bash
git submodule update --init --recursive
```

## Basic Usage Example

The repository ships a runnable example at [`examples/hello_window.rs`](examples/hello_window.rs).
Keep that file as the authoritative end-to-end sample. The minimal contract looks like this:

```rust
use commanductui::{
    AppEvent, PlatformCommand, PlatformEventHandler, PlatformInterface, WindowConfig,
};

impl PlatformEventHandler for MyAppLogic {
    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::ButtonClicked { .. } => { /* update app state, enqueue commands */ }
            AppEvent::WindowCloseRequestedByUser { window_id } =>
                self.enqueue(PlatformCommand::CloseWindow { window_id }),
            _ => {}
        }
    }
}

fn create_main_window(platform: &PlatformInterface) -> commanductui::PlatformResult<()> {
    let window_id = platform.create_window(WindowConfig {
        title: "My App",
        width: 400,
        height: 300,
    })?;

    // Build controls, define layout, then show the window.
    // See examples/hello_window.rs for the full runnable version.
    Ok(())
}
```

Two important details are easy to miss:

- `create_window` returns the `WindowId` you must keep and reuse in later commands.
- Clicking the native close button emits `AppEvent::WindowCloseRequestedByUser`; your host logic must decide what to do, typically by enqueuing `PlatformCommand::CloseWindow`.

## Testing the `hello_window` Example

On Windows, run the example from the repository root:

```powershell
cargo run --example hello_window
```

What to verify:

- A native window opens with a visible `Click Me` button near the top.
- Clicking the button updates the window title with the click count.
- Clicking the native close button closes the application.

Useful variants:

- Build without running: `cargo build --example hello_window`
- Type-check only: `cargo check --example hello_window`

If `cargo build --example hello_window` fails with `LNK1104` for `hello_window.exe`, a previous run is still open. Close the running example and rerun the command.

## Developer Workflow

Working with submodules requires a specific Git workflow to keep both repositories in sync.

### 1. Making Changes to `CommanDuctUI`

1.  Make your code changes inside the `src/CommanDuctUI` directory.
2.  Commit and push those changes **from within the submodule's directory**:
    ```bash
    cd src/CommanDuctUI
    git add .
    git commit -m "feat: Add new feature to CommanDuctUI"
    git push
    ```
3.  Go back to the main project root. `git status` will show `src/CommanDuctUI` as modified. This indicates that the main project's pointer to the submodule's commit has changed.
4.  Commit this pointer update in the main project to lock it in:
    ```bash
    cd ../..
    git add src/CommanDuctUI
    git commit -m "chore: Update CommanDuctUI to latest commit"
    git push
    ```

### 2. Pulling Updates

When you `git pull` in the main project, you must also update the submodule to get its latest code:

```bash
# Pull changes for the main project
git pull

# Update the submodule to the commit pointed to by the main project
git submodule update --recursive
```

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE).
