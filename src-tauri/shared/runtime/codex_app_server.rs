//! JSON-RPC client for the upstream `codex app-server` subcommand.
//!
//! Replaces the legacy `codex exec "Reply with the single word OK."`
//! fallback used to provoke a session-file write so we could sample
//! quota. That path costs the user a real LLM round-trip (30–90 s) and
//! actual token quota; this one calls the same backend through a stdio
//! JSON-RPC handshake and returns within seconds without touching the
//! model.
//!
//! Wire format (verified against `openai/codex` `codex-rs/app-server`):
//!   * stdio transport, newline-delimited JSON (one JSON object per line)
//!   * mandatory `initialize` request + `initialized` notification
//!     handshake before any other call
//!   * methods used: `account/login/start` for browser login URLs owned
//!     by this UI, `account/read` (with `refreshToken: false` so a
//!     normal quota fallback observes the existing authorization without
//!     forcing an OAuth rotation),
//!     `account/rateLimits/read` (no params)
//!
//! On error the server returns standard JSON-RPC error objects. Codes we
//! care about: `-32600` ("authentication required") → relogin signal,
//! `-32601` (method not found) → outdated codex CLI.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{Local, TimeZone, Utc};
use serde_json::{json, Value};

use crate::errors::{AppError, AppResult};
use crate::models::{QuotaSummary, QuotaWindow};

use super::login_cancel::{
    cancel_login_in_progress, drop_killed_child, peek_login_status, register_login_child, LoginPeek,
};
use super::quota_routing::{slot_from_window_minutes, QuotaSlot};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const LOGIN_START_TIMEOUT: Duration = Duration::from_secs(10);
/// ChatGPT browser login is user-paced. Match the device-code window
/// Codex documents for its app-server flow so users have enough time to
/// complete MFA without leaving a child process alive indefinitely.
const LOGIN_COMPLETE_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// `account/rateLimits/read` is one HTTPS GET to the same endpoint
/// `chatgpt_api.rs` polls with `HTTP_TIMEOUT = 15s`, so match that
/// budget — slow-network users are exactly the population this
/// fallback exists to serve.
const RATE_LIMITS_TIMEOUT: Duration = Duration::from_secs(15);
/// `account/read` is intentionally non-rotating during a normal refresh,
/// but the app-server launch plus authenticated account lookup can still be
/// slower than a single rate-limit GET on a high-latency link.
const ACCOUNT_READ_TIMEOUT: Duration = Duration::from_secs(25);
/// Hard upper bound on the whole RPC session. Defends against a child
/// that hangs after responding to one method but not the next. Sized
/// to comfortably cover handshake + account/read + rateLimits/read at
/// their per-method ceilings, with a small margin.
const SESSION_TIMEOUT: Duration = Duration::from_secs(60);
const LOGIN_SESSION_TIMEOUT: Duration = Duration::from_secs(16 * 60);

/// Pulled from `account/rateLimits/read` + `account/read`. `quota` is
/// `None` when the response carried no rate-limit data (typical for free
/// tier or accounts without an enforced window) — callers should clear
/// stale paid-window numbers in that case.
#[derive(Debug, Clone, Default)]
pub struct AppServerSnapshot {
    pub plan_type: Option<String>,
    pub quota: Option<QuotaSummary>,
}

/// Drive a one-shot JSON-RPC session against `codex app-server`.
/// Caller-provided `command` must already point at the resolved real
/// codex binary (callers also typically set `CODEX_HOME` and any
/// platform-specific stdio flags such as Windows' CREATE_NO_WINDOW).
///
/// The function pipes stdio internally and kills the child on every
/// exit path, so caller does not need a Drop guard of its own.
pub fn fetch_account_snapshot(mut command: Command) -> AppResult<AppServerSnapshot> {
    configure_app_server_command(&mut command);

    let child = command.spawn().map_err(|error| {
        AppError::new(
            "APP_SERVER_SPAWN_FAILED",
            format!("Failed to spawn `codex app-server`: {error}"),
        )
    })?;

    let mut guard = ChildGuard::new(child);
    drive_snapshot_session(guard.child_mut())
}

/// Start a ChatGPT browser login through `codex app-server` and let the
/// caller decide where to open the returned auth URL. This avoids the
/// `codex login` CLI path, which always asks Windows to open the system
/// default browser before printing the URL.
pub fn run_chatgpt_browser_login<F>(mut command: Command, open_auth_url: F) -> AppResult<()>
where
    F: FnOnce(&str) -> AppResult<()>,
{
    configure_app_server_command(&mut command);

    let child = command.spawn().map_err(|error| {
        AppError::new(
            "APP_SERVER_SPAWN_FAILED",
            format!("Failed to spawn `codex app-server`: {error}"),
        )
    })?;

    drive_cancellable_login_session(child, open_auth_url)
}

fn configure_app_server_command(command: &mut Command) {
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
    // RUST_LOG=error silences the protocol-level chatter codex emits on
    // stderr by default. Without this the pipe buffer can fill on a
    // slow client and back-pressure the child into a deadlock; a
    // background drain thread (below) backs this up belt-and-braces.
    command.env("RUST_LOG", "error");
}

struct ChildGuard {
    child: Option<Child>,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child
            .as_mut()
            .expect("ChildGuard outlived its Child handle")
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn open_session(child: &mut Child, session_timeout: Duration) -> AppResult<Session> {
    let stdin = child.stdin.take().ok_or_else(|| {
        AppError::new(
            "APP_SERVER_PIPE_FAILED",
            "Failed to acquire stdin for codex app-server.",
        )
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        AppError::new(
            "APP_SERVER_PIPE_FAILED",
            "Failed to acquire stdout for codex app-server.",
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        AppError::new(
            "APP_SERVER_PIPE_FAILED",
            "Failed to acquire stderr for codex app-server.",
        )
    })?;

    drain_stderr_in_background(stderr);
    let stdout_rx = spawn_stdout_reader(stdout);
    Ok(Session {
        stdin,
        stdout_rx,
        next_id: 0,
        session_deadline: Instant::now() + session_timeout,
    })
}

fn drive_snapshot_session(child: &mut Child) -> AppResult<AppServerSnapshot> {
    let mut session = open_session(child, SESSION_TIMEOUT)?;

    handshake(&mut session)?;
    let plan_type = read_account_plan(&mut session)?;
    let quota = read_rate_limits(&mut session)?;

    Ok(AppServerSnapshot { plan_type, quota })
}

fn drive_cancellable_login_session<F>(mut child: Child, open_auth_url: F) -> AppResult<()>
where
    F: FnOnce(&str) -> AppResult<()>,
{
    let mut session = open_session(&mut child, LOGIN_SESSION_TIMEOUT)?;
    register_login_child(child);
    finish_cancellable_login(drive_login_session_with_open_session(
        &mut session,
        open_auth_url,
    ))
}

fn drive_login_session_with_open_session<F>(
    session: &mut Session,
    open_auth_url: F,
) -> AppResult<()>
where
    F: FnOnce(&str) -> AppResult<()>,
{
    handshake(session)?;
    let (login_id, auth_url) = start_chatgpt_browser_login(session)?;
    open_auth_url(&auth_url)?;
    wait_for_chatgpt_login_completed(session, &login_id)
}

fn finish_cancellable_login(result: AppResult<()>) -> AppResult<()> {
    match result {
        Ok(()) => {
            let _ = cancel_login_in_progress();
            Ok(())
        }
        Err(error) => match peek_login_status() {
            Ok(LoginPeek::Cancelled) => Err(AppError::new(
                "LOGIN_CANCELLED",
                "Login was cancelled before the OAuth flow finished.",
            )),
            Ok(LoginPeek::Exited(child)) => {
                drop_killed_child(child);
                Err(error)
            }
            Ok(LoginPeek::Running) | Err(_) => {
                let _ = cancel_login_in_progress();
                Err(error)
            }
        },
    }
}

struct Session {
    stdin: ChildStdin,
    stdout_rx: Receiver<std::io::Result<String>>,
    next_id: i64,
    session_deadline: Instant,
}

impl Session {
    fn call(
        &mut self,
        method: &'static str,
        params: Option<Value>,
        per_request_timeout: Duration,
    ) -> AppResult<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let mut payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "id": id,
        });
        if let Some(params) = params {
            payload["params"] = params;
        }
        write_frame(&mut self.stdin, &payload)?;

        let request_deadline = (Instant::now() + per_request_timeout).min(self.session_deadline);
        loop {
            let parsed = self.recv_message(request_deadline, method)?;
            // Skip notifications or replies addressed to a different id —
            // the server may emit progress events while a request is in
            // flight. Safe because `next_id` is monotonic and calls are
            // serialized through `&mut self`, so a stale reply's id can
            // only be lower than the current one and never collide.
            if parsed.get("id").and_then(Value::as_i64) != Some(id) {
                continue;
            }
            if let Some(error_obj) = parsed.get("error") {
                return Err(map_rpc_error(method, error_obj));
            }
            return Ok(parsed.get("result").cloned().unwrap_or(Value::Null));
        }
    }

    fn recv_message(&mut self, deadline: Instant, context: &str) -> AppResult<Value> {
        loop {
            let timeout = deadline
                .checked_duration_since(Instant::now())
                .ok_or_else(|| timeout_error(context))?;
            let line = match self.stdout_rx.recv_timeout(timeout) {
                Ok(Ok(line)) => line,
                Ok(Err(error)) => {
                    return Err(AppError::new(
                        "APP_SERVER_READ_FAILED",
                        format!("Failed to read codex app-server stdout: {error}"),
                    ));
                }
                Err(RecvTimeoutError::Timeout) => return Err(timeout_error(context)),
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(AppError::new(
                        "APP_SERVER_PIPE_CLOSED",
                        "codex app-server stdout closed unexpectedly.",
                    ));
                }
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            return serde_json::from_str(trimmed).map_err(|error| {
                AppError::new(
                    "APP_SERVER_PARSE_FAILED",
                    format!("Failed to parse codex app-server message ({error}): {trimmed}"),
                )
            });
        }
    }

    fn notify(&mut self, method: &str, params: Option<Value>) -> AppResult<()> {
        let mut payload = json!({
            "jsonrpc": "2.0",
            "method": method,
        });
        if let Some(params) = params {
            payload["params"] = params;
        }
        write_frame(&mut self.stdin, &payload)
    }
}

fn timeout_error(method: &str) -> AppError {
    AppError::new(
        "APP_SERVER_TIMEOUT",
        format!("`codex app-server` `{method}` timed out."),
    )
}

fn write_frame(stdin: &mut ChildStdin, value: &Value) -> AppResult<()> {
    let mut bytes = serde_json::to_vec(value).map_err(|error| {
        AppError::new(
            "APP_SERVER_SERIALIZE_FAILED",
            format!("Failed to serialize app-server request: {error}"),
        )
    })?;
    bytes.push(b'\n');
    stdin.write_all(&bytes).map_err(|error| {
        AppError::new(
            "APP_SERVER_WRITE_FAILED",
            format!("Failed to write to codex app-server stdin: {error}"),
        )
    })?;
    stdin.flush().map_err(|error| {
        AppError::new(
            "APP_SERVER_WRITE_FAILED",
            format!("Failed to flush codex app-server stdin: {error}"),
        )
    })
}

fn spawn_stdout_reader(stdout: ChildStdout) -> Receiver<std::io::Result<String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if tx.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = tx.send(Err(error));
                    break;
                }
            }
        }
    });
    rx
}

fn drain_stderr_in_background(stderr: ChildStderr) {
    thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut sink = std::io::sink();
        let _ = std::io::copy(&mut reader, &mut sink);
    });
}

fn handshake(session: &mut Session) -> AppResult<()> {
    let params = json!({
        "clientInfo": {
            "name": "codex_account_switch",
            "title": "Codex Account Switch",
            "version": env!("CARGO_PKG_VERSION"),
        }
    });
    session.call("initialize", Some(params), HANDSHAKE_TIMEOUT)?;
    // `ClientNotification::Initialized` serializes (via serde
    // `rename_all = "camelCase"`) as method `"initialized"` with no
    // params payload — matches `codex-rs/app-server-protocol`'s wire
    // format, not the LSP-style `notifications/initialized`.
    session.notify("initialized", None)
}

fn start_chatgpt_browser_login(session: &mut Session) -> AppResult<(String, String)> {
    let result = session.call(
        "account/login/start",
        Some(json!({ "type": "chatgpt" })),
        LOGIN_START_TIMEOUT,
    )?;
    parse_login_start_response(&result)
}

fn parse_login_start_response(result: &Value) -> AppResult<(String, String)> {
    let login_id = result
        .get("loginId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "APP_SERVER_LOGIN_START_INVALID",
                "codex app-server did not return a loginId.",
            )
        })?;
    let auth_url = result
        .get("authUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "APP_SERVER_LOGIN_START_INVALID",
                "codex app-server did not return an authUrl.",
            )
        })?;
    Ok((login_id.to_string(), auth_url.to_string()))
}

fn wait_for_chatgpt_login_completed(session: &mut Session, login_id: &str) -> AppResult<()> {
    let deadline = (Instant::now() + LOGIN_COMPLETE_TIMEOUT).min(session.session_deadline);
    loop {
        let message = session.recv_message(deadline, "account/login/completed")?;
        if message.get("id").is_some() {
            continue;
        }
        if message.get("method").and_then(Value::as_str) != Some("account/login/completed") {
            continue;
        }
        if let Some(result) = parse_login_completed_notification(&message, login_id) {
            return result;
        }
    }
}

fn parse_login_completed_notification(message: &Value, login_id: &str) -> Option<AppResult<()>> {
    let params = message.get("params")?;
    let message_login_id = params.get("loginId").and_then(Value::as_str)?;
    if message_login_id != login_id {
        return None;
    }
    let success = params
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if success {
        return Some(Ok(()));
    }

    let error = params
        .get("error")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("ChatGPT login failed.");
    Some(Err(AppError::new("LOGIN_FAILED", error)))
}

fn account_read_params() -> Value {
    // The direct API path already rotates when the access token is expired,
    // near expiry, or returns 401. A transient API failure must not make the
    // app-server fallback consume a refresh token just to read account data.
    json!({ "refreshToken": false })
}

fn read_account_plan(session: &mut Session) -> AppResult<Option<String>> {
    let params = account_read_params();
    let result = session.call("account/read", Some(params), ACCOUNT_READ_TIMEOUT)?;
    Ok(extract_plan_type(&result))
}

fn extract_plan_type(result: &Value) -> Option<String> {
    let account = result.get("account")?;
    if account.is_null() {
        return None;
    }
    account
        .get("planType")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn read_rate_limits(session: &mut Session) -> AppResult<Option<QuotaSummary>> {
    let result = session.call("account/rateLimits/read", None, RATE_LIMITS_TIMEOUT)?;
    Ok(parse_rate_limits_response(&result))
}

fn parse_rate_limits_response(value: &Value) -> Option<QuotaSummary> {
    let rate_limits = value.get("rateLimits").filter(|v| !v.is_null());
    let additional_value = rate_limits
        .and_then(|value| {
            value
                .get("additionalRateLimits")
                .or_else(|| value.get("additional_rate_limits"))
        })
        .or_else(|| {
            value
                .get("additionalRateLimits")
                .or_else(|| value.get("additional_rate_limits"))
        });
    if rate_limits.is_none() && additional_value.is_none() {
        return None;
    }
    let mut summary = QuotaSummary::default();
    summary.additional = super::additional_quota::parse_additional_rate_limits(additional_value);
    let mut any_data = summary.additional.is_some();

    // Position is fallback only — `windowDurationMins` is authoritative,
    // mirrors the JSONL routing in `session_usage` and the HTTP path's
    // `chatgpt_api::quota_summary_from_payload`.
    for (window_key, fallback) in [
        ("primary", QuotaSlot::FiveHour),
        ("secondary", QuotaSlot::Weekly),
    ] {
        let Some(rate_limits) = rate_limits else {
            break;
        };
        let Some(window) = rate_limits.get(window_key).filter(|v| !v.is_null()) else {
            continue;
        };
        let mapped = quota_window_from_app_server(window);
        if mapped.remaining_percent.is_none() && mapped.refresh_at.is_none() {
            continue;
        }
        any_data = true;
        let window_minutes = window.get("windowDurationMins").and_then(Value::as_i64);
        match slot_from_window_minutes(window_minutes, fallback) {
            QuotaSlot::FiveHour => summary.five_hour = mapped,
            QuotaSlot::Weekly => summary.weekly = mapped,
        }
    }

    if any_data {
        Some(summary)
    } else {
        None
    }
}

fn quota_window_from_app_server(window: &Value) -> QuotaWindow {
    let used_percent = window
        .get("usedPercent")
        .and_then(|value| value.as_f64().or_else(|| value.as_i64().map(|v| v as f64)));
    let remaining_percent = used_percent.map(|used| (100.0 - used).round().clamp(0.0, 100.0) as u8);
    let reset_at_timestamp = window.get("resetsAt").and_then(Value::as_i64);
    let refresh_at = reset_at_timestamp.and_then(|seconds| {
        Utc.timestamp_opt(seconds, 0).single().map(|datetime| {
            datetime
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
    });
    QuotaWindow {
        remaining_percent,
        refresh_at,
        reset_at_timestamp,
    }
}

fn map_rpc_error(method: &str, error: &Value) -> AppError {
    let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
    let raw_message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let normalized = raw_message.to_ascii_lowercase();

    if normalized.contains("token_invalidated")
        || normalized.contains("refresh_token_reused")
        || normalized.contains("authentication token has been invalidated")
        || normalized.contains("refresh token has already been used")
        || normalized.contains("please try signing in again")
        || normalized.contains("please log out and sign in again")
    {
        return AppError::new(
            "AUTH_REFRESH_RELOGIN_REQUIRED",
            "This account session has expired. Please log in again.",
        );
    }

    // `-32600 codex account authentication required to read rate limits` is
    // the canonical "auth missing" path the upstream tests exercise; treat
    // it the same as the relogin-required errors above so the dashboard
    // surfaces a consistent toast.
    if code == -32600 && normalized.contains("authentication required") {
        return AppError::new(
            "AUTH_REFRESH_RELOGIN_REQUIRED",
            "This account session has expired. Please log in again.",
        );
    }

    if code == -32601 {
        return AppError::new(
            "APP_SERVER_METHOD_UNSUPPORTED",
            format!(
                "`{method}` not supported by your codex CLI. Upgrade `codex` to the latest version."
            ),
        );
    }

    AppError::new(
        "APP_SERVER_RPC_ERROR",
        format!("`{method}` returned error {code}: {raw_message}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    #[test]
    fn parse_rate_limits_routes_primary_5h_and_secondary_weekly() {
        let payload = json!({
            "rateLimits": {
                "primary": {
                    "usedPercent": 36.5,
                    "resetsAt": 1_715_000_000_i64,
                    "windowDurationMins": 300_i64,
                },
                "secondary": {
                    "usedPercent": 7,
                    "resetsAt": 1_716_000_000_i64,
                    "windowDurationMins": 10080_i64,
                }
            }
        });
        let quota = parse_rate_limits_response(&payload).expect("quota");
        assert_eq!(quota.five_hour.remaining_percent, Some(64));
        assert_eq!(quota.weekly.remaining_percent, Some(93));
        assert!(quota.five_hour.refresh_at.is_some());
        assert!(quota.weekly.refresh_at.is_some());
        assert_eq!(quota.provenance.source, crate::models::QuotaSource::Unknown);
    }

    #[test]
    fn parse_rate_limits_routes_by_window_minutes_when_weekly_in_primary_slot() {
        let payload = json!({
            "rateLimits": {
                "primary": {
                    "usedPercent": 12,
                    "resetsAt": 1_716_000_000_i64,
                    "windowDurationMins": 10080_i64,
                },
                "secondary": null
            }
        });
        let quota = parse_rate_limits_response(&payload).expect("quota");
        assert_eq!(quota.weekly.remaining_percent, Some(88));
        assert!(quota.five_hour.remaining_percent.is_none());
        assert!(quota.five_hour.refresh_at.is_none());
    }

    #[test]
    fn parse_rate_limits_returns_none_for_empty_payload() {
        let payload = json!({ "rateLimits": null });
        assert!(parse_rate_limits_response(&payload).is_none());
        let payload = json!({ "rateLimits": { "primary": null, "secondary": null } });
        assert!(parse_rate_limits_response(&payload).is_none());
    }

    #[test]
    fn parse_rate_limits_reads_allowlisted_additional_windows_when_present() {
        let payload = json!({
            "rateLimits": {
                "primary": null,
                "secondary": null,
                "additionalRateLimits": [{
                    "limitName": "gpt-5.4-codex",
                    "meteredFeature": "codex",
                    "rateLimit": {
                        "primaryWindow": {
                            "usedPercent": 40,
                            "windowDurationMins": 300,
                            "resetsAt": 1_800_000_000_i64
                        }
                    }
                }]
            }
        });

        let quota = parse_rate_limits_response(&payload).expect("additional quota");
        let additional = quota.additional.expect("additional entries");
        assert_eq!(additional.len(), 1);
        assert_eq!(additional[0].remaining_percent, Some(60));
        assert_eq!(additional[0].window_minutes, Some(300));
    }

    #[test]
    fn parse_rate_limits_keeps_explicit_empty_additional_array() {
        let payload = json!({
            "rateLimits": {
                "primary": null,
                "secondary": null,
                "additionalRateLimits": []
            }
        });

        let quota = parse_rate_limits_response(&payload).expect("explicit empty additional quota");
        assert_eq!(quota.additional, Some(Vec::new()));
    }

    #[test]
    fn extract_plan_type_handles_chatgpt_account() {
        let result = json!({
            "account": { "type": "chatgpt", "email": "u@example.com", "planType": "plus" },
            "requiresOpenaiAuth": false
        });
        assert_eq!(extract_plan_type(&result).as_deref(), Some("plus"));
    }

    #[test]
    fn account_read_params_keep_oauth_refresh_lazy() {
        assert_eq!(account_read_params(), json!({ "refreshToken": false }));
    }

    #[test]
    fn extract_plan_type_returns_none_for_null_account() {
        let result = json!({ "account": null, "requiresOpenaiAuth": true });
        assert!(extract_plan_type(&result).is_none());
    }

    #[test]
    fn parse_login_start_response_extracts_login_id_and_auth_url() {
        let result = json!({
            "type": "chatgpt",
            "loginId": "login-123",
            "authUrl": "https://chatgpt.com/auth"
        });

        let (login_id, auth_url) = parse_login_start_response(&result).unwrap();
        assert_eq!(login_id, "login-123");
        assert_eq!(auth_url, "https://chatgpt.com/auth");
    }

    #[test]
    fn parse_login_completed_notification_matches_success_for_login_id() {
        let message = json!({
            "method": "account/login/completed",
            "params": {
                "loginId": "login-123",
                "success": true,
                "error": null
            }
        });

        assert!(parse_login_completed_notification(&message, "login-123")
            .unwrap()
            .is_ok());
        assert!(parse_login_completed_notification(&message, "other-login").is_none());
    }

    #[test]
    fn parse_login_completed_notification_maps_failure() {
        let message = json!({
            "method": "account/login/completed",
            "params": {
                "loginId": "login-123",
                "success": false,
                "error": "denied"
            }
        });

        let error = parse_login_completed_notification(&message, "login-123")
            .unwrap()
            .unwrap_err();
        assert_eq!(error.error_code, "LOGIN_FAILED");
        assert_eq!(error.message, "denied");
    }

    #[test]
    fn browser_login_registers_app_server_child_for_cancel() {
        let script = write_fake_login_app_server_script();
        let command = fake_login_app_server_command(&script);
        let opened = Arc::new(AtomicBool::new(false));
        let opened_for_login = Arc::clone(&opened);

        let handle = thread::spawn(move || {
            run_chatgpt_browser_login(command, move |_auth_url| {
                opened_for_login.store(true, Ordering::SeqCst);
                Ok(())
            })
        });

        let deadline = Instant::now() + Duration::from_secs(3);
        while !opened.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "fake app-server did not reach login URL open"
            );
            thread::sleep(Duration::from_millis(10));
        }

        let cancelled = super::super::login_cancel::cancel_login_in_progress();
        let result = handle.join().expect("login thread should not panic");
        let _ = std::fs::remove_file(&script);

        assert!(cancelled, "cancel should find the app-server login child");
        let error = result.expect_err("cancelled login should return an error");
        assert_eq!(error.error_code, "LOGIN_CANCELLED");
    }

    #[cfg(windows)]
    fn write_fake_login_app_server_script() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "codex-switch-fake-app-server-{}.cmd",
            unique_test_suffix()
        ));
        std::fs::write(
            &path,
            concat!(
                "@echo off\r\n",
                "echo {\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{}}\r\n",
                "echo {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"loginId\":\"login-123\",\"authUrl\":\"https://example.invalid/auth\"}}\r\n",
                "ping -n 3 127.0.0.1 >NUL\r\n",
            ),
        )
        .expect("write fake app-server cmd");
        path
    }

    #[cfg(windows)]
    fn fake_login_app_server_command(script: &std::path::Path) -> Command {
        let mut command = Command::new("cmd");
        command.arg("/C").arg(script);
        command
    }

    #[cfg(unix)]
    fn write_fake_login_app_server_script() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "codex-switch-fake-app-server-{}.sh",
            unique_test_suffix()
        ));
        std::fs::write(
            &path,
            concat!(
                "#!/bin/sh\n",
                "printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{}}'\n",
                "printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"loginId\":\"login-123\",\"authUrl\":\"https://example.invalid/auth\"}}'\n",
                "sleep 2\n",
            ),
        )
        .expect("write fake app-server shell");
        path
    }

    #[cfg(unix)]
    fn fake_login_app_server_command(script: &std::path::Path) -> Command {
        let mut command = Command::new("sh");
        command.arg(script);
        command
    }

    fn unique_test_suffix() -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time before unix epoch")
            .as_nanos();
        format!("{}-{nanos}", std::process::id())
    }

    #[test]
    fn map_rpc_error_classifies_authentication_required_as_relogin() {
        let error = json!({
            "code": -32600_i64,
            "message": "codex account authentication required to read rate limits"
        });
        let mapped = map_rpc_error("account/rateLimits/read", &error);
        assert_eq!(mapped.error_code, "AUTH_REFRESH_RELOGIN_REQUIRED");
    }

    #[test]
    fn map_rpc_error_classifies_method_not_found_as_unsupported() {
        let error = json!({ "code": -32601_i64, "message": "method not found" });
        let mapped = map_rpc_error("account/read", &error);
        assert_eq!(mapped.error_code, "APP_SERVER_METHOD_UNSUPPORTED");
    }

    #[test]
    fn map_rpc_error_classifies_invalidated_token_message_as_relogin() {
        let error = json!({
            "code": -32603_i64,
            "message": "Your refresh token has already been used. Please try signing in again."
        });
        let mapped = map_rpc_error("account/read", &error);
        assert_eq!(mapped.error_code, "AUTH_REFRESH_RELOGIN_REQUIRED");
    }

    #[test]
    fn map_rpc_error_falls_through_to_generic_for_unknown_codes() {
        let error = json!({ "code": -32603_i64, "message": "something broke" });
        let mapped = map_rpc_error("account/read", &error);
        assert_eq!(mapped.error_code, "APP_SERVER_RPC_ERROR");
        assert!(mapped.message.contains("something broke"));
    }

    #[test]
    fn quota_window_from_app_server_handles_integer_used_percent() {
        let window = json!({ "usedPercent": 12, "resetsAt": 1_715_000_000_i64 });
        let mapped = quota_window_from_app_server(&window);
        assert_eq!(mapped.remaining_percent, Some(88));
        assert!(mapped.refresh_at.is_some());
    }

    #[test]
    fn map_rpc_error_classifies_token_invalidated_substring_as_relogin() {
        // The legacy `codex exec` path used to surface this exact substring
        // verbatim from the CLI's stderr; covering it here keeps that
        // regression-catch alive after the test moved off the renamed
        // `classify_auth_refresh_failure` helper.
        let error = json!({
            "code": -32603_i64,
            "message": "401 Unauthorized: token_invalidated"
        });
        let mapped = map_rpc_error("account/read", &error);
        assert_eq!(mapped.error_code, "AUTH_REFRESH_RELOGIN_REQUIRED");
    }

    #[test]
    fn parse_rate_limits_keeps_window_when_only_used_percent_is_set() {
        // `resetsAt` may be absent on some plans; partial data is still
        // useful (the dashboard renders the percent without a reset
        // time). Asserts the partial-data branch in
        // `quota_window_from_app_server` remains live.
        let payload = json!({
            "rateLimits": {
                "primary": { "usedPercent": 25, "windowDurationMins": 300_i64 },
                "secondary": null,
            }
        });
        let quota = parse_rate_limits_response(&payload).expect("quota");
        assert_eq!(quota.five_hour.remaining_percent, Some(75));
        assert!(quota.five_hour.refresh_at.is_none());
        assert!(quota.weekly.remaining_percent.is_none());
    }

    #[test]
    fn parse_rate_limits_skips_window_with_no_data_fields() {
        // A window object with neither `usedPercent` nor `resetsAt` must
        // be treated as missing — `any_data` should stay false and the
        // function returns None instead of an empty-but-allocated quota.
        let payload = json!({
            "rateLimits": {
                "primary": { "windowDurationMins": 300_i64 },
                "secondary": { "windowDurationMins": 10080_i64 },
            }
        });
        assert!(parse_rate_limits_response(&payload).is_none());
    }
}
