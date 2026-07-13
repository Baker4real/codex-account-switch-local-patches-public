use chrono::{Local, TimeZone, Utc};
use serde_json::{Map, Value};

use crate::models::{AdditionalQuotaWindow, AdditionalQuotaWindowKind};

fn value_at<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| object.get(*key))
}

fn scalar_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.trim().to_string(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        _ => String::new(),
    }
}

fn scalar_bool(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(value)) => Some(*value),
        Some(Value::String(value)) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn scalar_f64(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(value)) => value.as_f64(),
        Some(Value::String(value)) => value.trim().parse().ok(),
        _ => None,
    }
}

fn scalar_i64(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(value)) => value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok())),
        Some(Value::String(value)) => value.trim().parse().ok(),
        _ => None,
    }
}

fn normalize_timestamp(value: Option<&Value>) -> Option<i64> {
    let mut timestamp = scalar_i64(value)?;
    if timestamp > 1_000_000_000_000 {
        timestamp /= 1000;
    }
    (timestamp > 0).then_some(timestamp)
}

fn window_minutes(window: &Map<String, Value>) -> Option<i64> {
    let direct = scalar_i64(value_at(
        window,
        &["window_minutes", "windowMinutes", "windowDurationMins"],
    ))
    .filter(|value| *value > 0);
    if direct.is_some() {
        return direct;
    }

    scalar_i64(value_at(
        window,
        &["limit_window_seconds", "limitWindowSeconds"],
    ))
    .filter(|value| *value > 0)
    .map(|seconds| (seconds + 59) / 60)
}

fn reset_at_timestamp(window: &Map<String, Value>) -> Option<i64> {
    if let Some(timestamp) = normalize_timestamp(value_at(window, &["reset_at", "resetsAt"])) {
        return Some(timestamp);
    }

    let reset_after = scalar_i64(value_at(
        window,
        &["reset_after_seconds", "resetAfterSeconds"],
    ))?;
    if reset_after < 0 {
        return None;
    }
    Some(Utc::now().timestamp().saturating_add(reset_after))
}

fn format_reset_at(timestamp: Option<i64>) -> Option<String> {
    timestamp.and_then(|seconds| {
        Utc.timestamp_opt(seconds, 0).single().map(|datetime| {
            datetime
                .with_timezone(&Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
    })
}

fn window_label(minutes: Option<i64>, kind: AdditionalQuotaWindowKind) -> String {
    match minutes {
        Some(300) => "5h".to_string(),
        Some(10080) => "weekly".to_string(),
        Some(minutes) => format!("{minutes}m"),
        None => match kind {
            AdditionalQuotaWindowKind::Primary => "primary".to_string(),
            AdditionalQuotaWindowKind::Secondary => "secondary".to_string(),
        },
    }
}

fn normalized_limit_label(limit_name: &str, metered_feature: &str) -> String {
    let raw = if limit_name.is_empty() {
        metered_feature
    } else {
        limit_name
    };
    if raw.is_empty() {
        "Additional quota".to_string()
    } else {
        raw.replace(['_', '-'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn is_spark_limit(limit_name: &str, metered_feature: &str, label: &str) -> bool {
    [limit_name, metered_feature, label]
        .join(" ")
        .to_ascii_lowercase()
        .contains("spark")
}

fn parse_window(
    value: Option<&Value>,
    index: usize,
    kind: AdditionalQuotaWindowKind,
    limit_name: &str,
    metered_feature: &str,
    allowed: Option<bool>,
    limit_reached: Option<bool>,
    is_spark: bool,
) -> AdditionalQuotaWindow {
    let window = value.and_then(Value::as_object);
    let minutes = window.and_then(window_minutes);
    let reset_at_timestamp = window.and_then(reset_at_timestamp);
    let remaining_percent = window
        .and_then(|window| scalar_f64(value_at(window, &["used_percent", "usedPercent"])))
        .filter(|used| used.is_finite() && (0.0..=100.0).contains(used))
        .map(|used| (100.0 - used).round().clamp(0.0, 100.0) as u8);
    let base_label = normalized_limit_label(limit_name, metered_feature);
    let label = format!("{} · {}", base_label, window_label(minutes, kind));
    let kind_name = match kind {
        AdditionalQuotaWindowKind::Primary => "primary",
        AdditionalQuotaWindowKind::Secondary => "secondary",
    };

    AdditionalQuotaWindow {
        id: format!("additional:{index}:{kind_name}"),
        label,
        limit_name: limit_name.to_string(),
        metered_feature: metered_feature.to_string(),
        window_kind: kind,
        remaining_percent,
        refresh_at: format_reset_at(reset_at_timestamp),
        reset_at_timestamp,
        window_minutes: minutes,
        allowed,
        limit_reached,
        is_spark,
    }
}

/// Parse the allowlisted `additional_rate_limits` payload without retaining
/// the raw response. Missing fields are represented by `None`; a present empty
/// array remains `Some(Vec::new())` so callers can distinguish both states.
pub fn parse_additional_rate_limits(value: Option<&Value>) -> Option<Vec<AdditionalQuotaWindow>> {
    let entries = value?.as_array()?;
    let mut parsed = Vec::new();

    for (index, entry) in entries.iter().enumerate() {
        let Some(record) = entry.as_object() else {
            continue;
        };
        let limit_name = scalar_text(value_at(record, &["limit_name", "limitName"]));
        let metered_feature = scalar_text(value_at(record, &["metered_feature", "meteredFeature"]));
        let base_label = normalized_limit_label(&limit_name, &metered_feature);
        let is_spark = is_spark_limit(&limit_name, &metered_feature, &base_label);
        let rate_limit = value_at(record, &["rate_limit", "rateLimit"]).and_then(Value::as_object);
        let allowed = rate_limit.and_then(|value| scalar_bool(value_at(value, &["allowed"])));
        let limit_reached = rate_limit
            .and_then(|value| scalar_bool(value_at(value, &["limit_reached", "limitReached"])));

        let Some(rate_limit) = rate_limit else {
            parsed.push(parse_window(
                None,
                index,
                AdditionalQuotaWindowKind::Primary,
                &limit_name,
                &metered_feature,
                allowed,
                limit_reached,
                is_spark,
            ));
            continue;
        };

        let before = parsed.len();
        if rate_limit
            .get("primary_window")
            .or_else(|| rate_limit.get("primaryWindow"))
            .is_some()
        {
            parsed.push(parse_window(
                value_at(rate_limit, &["primary_window", "primaryWindow"]),
                index,
                AdditionalQuotaWindowKind::Primary,
                &limit_name,
                &metered_feature,
                allowed,
                limit_reached,
                is_spark,
            ));
        }
        if rate_limit
            .get("secondary_window")
            .or_else(|| rate_limit.get("secondaryWindow"))
            .is_some()
        {
            parsed.push(parse_window(
                value_at(rate_limit, &["secondary_window", "secondaryWindow"]),
                index,
                AdditionalQuotaWindowKind::Secondary,
                &limit_name,
                &metered_feature,
                allowed,
                limit_reached,
                is_spark,
            ));
        }
        if parsed.len() == before {
            parsed.push(parse_window(
                None,
                index,
                AdditionalQuotaWindowKind::Primary,
                &limit_name,
                &metered_feature,
                allowed,
                limit_reached,
                is_spark,
            ));
        }
    }

    Some(parsed)
}

#[cfg(test)]
mod tests {
    use super::parse_additional_rate_limits;
    use crate::models::AdditionalQuotaWindowKind;
    use serde_json::json;

    #[test]
    fn parses_model_specific_windows_and_marks_spark_noise() {
        let value = json!([
            {
                "limit_name": "gpt-5.4-codex",
                "metered_feature": "codex",
                "rate_limit": {
                    "allowed": true,
                    "limit_reached": false,
                    "primary_window": {
                        "used_percent": 25.0,
                        "limit_window_seconds": 300,
                        "reset_at": 1_800_000_000_i64
                    },
                    "secondary_window": {
                        "used_percent": "bad",
                        "limit_window_seconds": 10080,
                        "reset_at": 1_800_100_000_i64
                    }
                }
            },
            {
                "limit_name": "GPT-5.3-CODEX-SPARK",
                "metered_feature": "spark",
                "rate_limit": { "primary_window": null }
            }
        ]);

        let parsed = parse_additional_rate_limits(Some(&value)).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].remaining_percent, Some(75));
        assert_eq!(parsed[0].window_minutes, Some(5));
        assert_eq!(parsed[0].window_kind, AdditionalQuotaWindowKind::Primary);
        assert_eq!(parsed[1].remaining_percent, None);
        assert!(parsed[2].is_spark);
    }

    #[test]
    fn distinguishes_missing_and_empty_additional_arrays() {
        assert!(parse_additional_rate_limits(None).is_none());
        assert_eq!(
            parse_additional_rate_limits(Some(&json!([]))),
            Some(Vec::new())
        );
    }
}
