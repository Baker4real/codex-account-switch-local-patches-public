use crate::errors::CommandError;
use crate::models::OpenAiUsageSummary;

#[tauri::command]
pub async fn get_openai_usage_summary() -> Result<OpenAiUsageSummary, CommandError> {
    tauri::async_runtime::spawn_blocking(|| crate::shared::openai_usage::load_openai_usage_summary(None))
        .await
        .map_err(|error| {
            CommandError::new(
                "OPENAI_USAGE_TASK_FAILED",
                format!("OpenAI usage task failed: {error}"),
            )
        })?
        .map_err(Into::into)
}
