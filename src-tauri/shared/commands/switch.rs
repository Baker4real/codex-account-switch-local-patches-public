use crate::errors::CommandError;
use crate::models::{ProfilePayload, SwitchResponse};

use crate::windows as platform_runtime;

#[tauri::command]
pub async fn switch_profile(payload: ProfilePayload) -> Result<SwitchResponse, CommandError> {
    let profile = payload.profile;
    tauri::async_runtime::spawn_blocking(move || platform_runtime::switch::switch_profile(&profile))
        .await
        .map_err(|error| {
            CommandError::new("SWITCH_FAILED", format!("Switch task failed: {error}"))
        })?
        .map_err(Into::into)
}
