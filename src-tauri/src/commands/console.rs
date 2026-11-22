//! The Console screen: run the `tmc` command-line tool and stream what it says.
//!
//! WHAT THIS DELIBERATELY WIDENS
//! -----------------------------
//! The rule at the top of `commands/mod.rs` is that no command takes a program
//! or a path from the webview. Here the webview sends a command LINE, so it is
//! worth stating what it still cannot do:
//!
//!   * **Choose the program.** It is the `tmc` installed beside the app (or the
//!     user's `TMC_CLI_PATH`, or `tmc` on `PATH`), chosen by Rust. The line
//!     becomes arguments to it and to nothing else.
//!   * **Reach a shell.** There is none; see [`tmc_core::console`].
//!   * **Choose the working directory or environment.** Both are Rust's.
//!
//! What it costs: `tmc` is a tool that uploads files and holds the user's API
//! key, so a script in the webview can now do what the user could type into a
//! terminal. That is the feature. It is bounded by desktop-only compilation,
//! one process at a time, and an audit entry per run with credential-shaped
//! arguments redacted (`console.run`).
//!
//! Output is pushed as `tmc://console` events rather than polled, for the
//! reason the download queue is: a build log is hundreds of messages a second
//! while it scrolls.

use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

use tmc_core::audit;
use tmc_core::console::{self, ConsoleEvent, StartError, Stream, MAX_STDIN};
use tmc_core::error::{AppError, AppResult};

use crate::state::AppState;

const EVENT: &str = "tmc://console";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleProbe {
    pub found: bool,
    /// Where `tmc` was found, for the header.
    pub path: Option<String>,
    /// `bundled` (shipped with the app), `override` (`TMC_CLI_PATH`) or `path`
    /// (the user's own install).
    pub source: Option<&'static str>,
    /// `tmc --version`'s first line.
    pub version: Option<String>,
    /// The app is signed in, so a run is signed in as the same user
    /// (`TMC_APP_TOKEN`) without asking for a key.
    pub signed_in: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleStarted {
    pub run_id: u64,
    /// The command as it should appear in the transcript and in history, with
    /// credential-shaped arguments redacted: a token typed on the line must not
    /// be written back into localStorage by the history.
    pub display: String,
}

/// Quote one argument for DISPLAY, so a copied transcript line re-splits to the
/// same arguments. Never executed.
fn quote_for_display(arg: &str) -> String {
    let plain = !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_alphanumeric() || "-_./=:@%+,~".contains(c));

    if plain {
        arg.to_string()
    } else {
        format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

#[tauri::command]
pub async fn console_probe(state: State<'_, AppState>) -> AppResult<ConsoleProbe> {
    let signed_in = state.auth.has_refresh();

    tauri::async_runtime::spawn_blocking(move || {
        let Some((path, source)) = console::locate_with_source() else {
            return ConsoleProbe {
                found: false,
                path: None,
                source: None,
                version: None,
                signed_in,
            };
        };

        ConsoleProbe {
            found: true,
            version: console::version_of(&path),
            path: Some(path.display().to_string()),
            source: Some(source.as_str()),
            signed_in,
        }
    })
    .await
    .map_err(|e| AppError::internal(format!("console probe failed: {e}")))
}

#[tauri::command]
pub async fn console_run(
    app: AppHandle,
    state: State<'_, AppState>,
    line: String,
) -> AppResult<ConsoleStarted> {
    let args =
        console::split_argv(&line, !cfg!(windows)).map_err(|e| AppError::invalid(e.to_string()))?;

    let redacted = console::redact_argv(&args);
    let display = redacted
        .iter()
        .map(|a| quote_for_display(a))
        .collect::<Vec<_>>()
        .join(" ");

    let Some(program) = console::locate() else {
        return Err(AppError::invalid(
            "The tmc command-line tool was not found. This build of the app does not include it; install it with pip (pip install tmc-cli), then reopen this screen.",
        ));
    };

    // Signed in as the app's own user, unless the line or the user's own
    // environment says otherwise; see `console::app_session_env`.
    let token = state.api.bearer_for_console().await;
    let session_env = console::app_session_env(
        &args,
        token.as_deref(),
        tmc_core::api::api_base(),
        tmc_core::api::PROD_API_BASE,
        std::env::var_os("TMC_APP_TOKEN").is_some_and(|v| !v.is_empty()),
        std::env::var("TMC_BASE_URL").ok().as_deref(),
    );
    let signed_in = !session_env.is_empty();

    let emitter = app.clone();
    let emit: console::Emit = Arc::new(move |event| {
        let payload = match event {
            ConsoleEvent::Output {
                run_id,
                stream,
                text,
            } => serde_json::json!({
                "type": "output",
                "runId": run_id,
                "stream": match stream { Stream::Stdout => "stdout", Stream::Stderr => "stderr" },
                "text": text,
            }),
            ConsoleEvent::Exit {
                run_id,
                code,
                stopped,
                millis,
            } => serde_json::json!({
                "type": "exit",
                "runId": run_id,
                "code": code,
                "stopped": stopped,
                "millis": millis,
            }),
        };

        let _ = emitter.emit(EVENT, payload);
    });

    match state
        .console
        .start_with_env(&program, &args, &session_env, emit)
    {
        Ok(run_id) => {
            let mut data = serde_json::Map::new();

            data.insert("argv".into(), serde_json::json!(redacted));
            data.insert("runId".into(), run_id.into());
            data.insert("appSession".into(), signed_in.into());

            audit!(
                state.audit,
                Info,
                App,
                "console.run",
                format!("tmc {display}"),
                data = data
            );

            Ok(ConsoleStarted { run_id, display })
        }
        Err(StartError::Busy) => Err(AppError::invalid(
            "A command is already running. Stop it first.",
        )),
        Err(StartError::Spawn(e)) => {
            let mut data = serde_json::Map::new();

            data.insert("argv".into(), serde_json::json!(redacted));
            data.insert("program".into(), program.display().to_string().into());
            data.insert("error".into(), e.to_string().into());

            audit!(
                state.audit,
                Error,
                App,
                "console.run",
                format!("tmc could not be started: {e}"),
                data = data
            );

            Err(AppError::internal(format!("could not start tmc: {e}")))
        }
    }
}

/// Send a line to the running process. `newline` appends the line break a typed
/// answer needs; the text itself is never logged, because it may be a secret.
#[tauri::command]
pub fn console_stdin(state: State<'_, AppState>, text: String, newline: bool) -> AppResult<()> {
    if text.len() > MAX_STDIN {
        return Err(AppError::invalid("That is too much to send at once."));
    }

    let mut text = text;

    if newline {
        text.push('\n');
    }

    if state.console.write_stdin(&text) {
        Ok(())
    } else {
        Err(AppError::invalid("Nothing is running that can take input."))
    }
}

/// End of input (Ctrl+D): for a command that reads until EOF.
#[tauri::command]
pub fn console_stdin_close(state: State<'_, AppState>) -> bool {
    state.console.close_stdin()
}

#[tauri::command]
pub fn console_stop(state: State<'_, AppState>) -> bool {
    state.console.stop()
}

/// The run in progress, so a remounted screen can re-attach after a route change.
#[tauri::command]
pub fn console_running(state: State<'_, AppState>) -> Option<u64> {
    state.console.running()
}
