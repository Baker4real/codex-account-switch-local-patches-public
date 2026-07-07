use crate::errors::CommandError;
use crate::models::{
    OpenAiUsageDashboard, OpenAiUsageDataSourceSummary, OpenAiUsageQuery, OpenAiUsageRequestLog,
    OpenAiUsageRequestLogPage, OpenAiUsageSummary, OpenAiUsageTrendPoint,
};

#[tauri::command]
pub async fn get_openai_usage_dashboard(
    query: Option<OpenAiUsageQuery>,
    page: u32,
    page_size: u32,
) -> Result<OpenAiUsageDashboard, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::shared::openai_usage::load_openai_usage_dashboard(None, query, page, page_size)
    })
    .await
    .map_err(|error| {
        CommandError::new(
            "OPENAI_USAGE_TASK_FAILED",
            format!("OpenAI usage task failed: {error}"),
        )
    })?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_openai_usage_summary(
    query: Option<OpenAiUsageQuery>,
) -> Result<OpenAiUsageSummary, CommandError> {
    tauri::async_runtime::spawn_blocking(|| {
        crate::shared::openai_usage::load_openai_usage_summary(None, query)
    })
    .await
    .map_err(|error| {
        CommandError::new(
            "OPENAI_USAGE_TASK_FAILED",
            format!("OpenAI usage task failed: {error}"),
        )
    })?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_openai_usage_trends(
    query: Option<OpenAiUsageQuery>,
) -> Result<Vec<OpenAiUsageTrendPoint>, CommandError> {
    tauri::async_runtime::spawn_blocking(|| {
        crate::shared::openai_usage::load_openai_usage_trends(None, query)
    })
    .await
    .map_err(|error| {
        CommandError::new(
            "OPENAI_USAGE_TASK_FAILED",
            format!("OpenAI usage task failed: {error}"),
        )
    })?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_openai_usage_request_logs(
    query: Option<OpenAiUsageQuery>,
    page: u32,
    page_size: u32,
) -> Result<OpenAiUsageRequestLogPage, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::shared::openai_usage::load_openai_usage_request_logs(None, query, page, page_size)
    })
    .await
    .map_err(|error| {
        CommandError::new(
            "OPENAI_USAGE_TASK_FAILED",
            format!("OpenAI usage task failed: {error}"),
        )
    })?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_openai_usage_request_detail(
    request_id: String,
) -> Result<Option<OpenAiUsageRequestLog>, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::shared::openai_usage::load_openai_usage_request_detail(None, &request_id)
    })
    .await
    .map_err(|error| {
        CommandError::new(
            "OPENAI_USAGE_TASK_FAILED",
            format!("OpenAI usage task failed: {error}"),
        )
    })?
    .map_err(Into::into)
}

#[tauri::command]
pub async fn get_openai_usage_data_sources(
    query: Option<OpenAiUsageQuery>,
) -> Result<Vec<OpenAiUsageDataSourceSummary>, CommandError> {
    tauri::async_runtime::spawn_blocking(|| {
        crate::shared::openai_usage::load_openai_usage_data_sources(None, query)
    })
    .await
    .map_err(|error| {
        CommandError::new(
            "OPENAI_USAGE_TASK_FAILED",
            format!("OpenAI usage task failed: {error}"),
        )
    })?
    .map_err(Into::into)
}
