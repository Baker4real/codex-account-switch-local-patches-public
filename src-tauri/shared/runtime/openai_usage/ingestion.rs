use super::*;

use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::Value;

use super::super::paths::USAGE_EVIDENCE_DIRNAME;

fn usage_source_signature(codex_home: &Path) -> String {
    let mut entries = Vec::new();
    collect_usage_source_entries(codex_home, &mut entries);
    entries.sort();
    let mut hasher = DefaultHasher::new();
    PRICING_CHECKED_AT.hash(&mut hasher);
    EVIDENCE_PARSER_VERSION.hash(&mut hasher);
    EVIDENCE_CLASSIFIER_VERSION.hash(&mut hasher);
    EVIDENCE_DEDUPE_VERSION.hash(&mut hasher);
    LEDGER_SCHEMA_VERSION.hash(&mut hasher);
    LAYER_CLASSIFIER_VERSION.hash(&mut hasher);
    ATTRIBUTION_BACKFILL_VERSION.hash(&mut hasher);
    PRICING_VERSION.hash(&mut hasher);
    for entry in entries {
        entry.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

fn collect_usage_source_entries(codex_home: &Path, entries: &mut Vec<String>) {
    for sessions_root in [
        codex_home.join("sessions"),
        codex_home.join("archived_sessions"),
    ] {
        let mut files = Vec::new();
        collect_jsonl_files(&sessions_root, &mut files);
        for file in files {
            push_file_signature(codex_home, &file, entries);
        }
    }

    for file in [
        get_openai_usage_attribution_path(Some(codex_home)),
        get_current_profile_file(Some(codex_home)),
        get_backup_root(Some(codex_home)).join(".current_profile"),
    ] {
        push_file_signature(codex_home, &file, entries);
    }
    push_file_content_signature(
        codex_home,
        &codex_home.join("auth.json"),
        "root-auth",
        entries,
    );

    let backup_root = get_backup_root(Some(codex_home));
    for profile_dir in super::super::paths::list_profile_dirs(&backup_root) {
        push_file_signature(codex_home, &profile_dir.join("profile.json"), entries);
        push_file_content_signature(
            codex_home,
            &profile_dir.join("auth.json"),
            "profile-auth",
            entries,
        );
        push_file_signature(
            codex_home,
            &profile_dir.join(super::super::paths::ACTIVE_MARKER_FILE),
            entries,
        );
    }

    if let Some(ccswitch_db) = live_ccswitch_db_path(codex_home) {
        push_external_file_signature("ccswitch", &ccswitch_db, entries);
        push_external_file_signature(
            "ccswitch-wal",
            &ccswitch_db.with_extension("db-wal"),
            entries,
        );
    }
}

fn push_file_signature(codex_home: &Path, file: &Path, entries: &mut Vec<String>) {
    let Ok(metadata) = fs::metadata(file) else {
        return;
    };
    let relative = file.strip_prefix(codex_home).unwrap_or(file);
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .and_then(|value| u64::try_from(value.as_millis()).ok())
        .unwrap_or_default();
    entries.push(format!(
        "{}:{}:{}",
        relative.display(),
        metadata.len(),
        modified_ms
    ));
}

fn push_file_content_signature(
    codex_home: &Path,
    file: &Path,
    label: &str,
    entries: &mut Vec<String>,
) {
    let Ok(raw) = fs::read_to_string(file) else {
        return;
    };
    let relative = file.strip_prefix(codex_home).unwrap_or(file);
    entries.push(format!(
        "{label}:{}:{}",
        relative.display(),
        stable_hash(&[&raw])
    ));
}

fn push_external_file_signature(label: &str, file: &Path, entries: &mut Vec<String>) {
    let Ok(metadata) = fs::metadata(file) else {
        return;
    };
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .and_then(|value| u64::try_from(value.as_millis()).ok())
        .unwrap_or_default();
    entries.push(format!(
        "{label}:{}:{}:{}",
        file.display(),
        metadata.len(),
        modified_ms
    ));
}

fn live_ccswitch_db_path(codex_home: &Path) -> Option<PathBuf> {
    if codex_home != get_codex_home().as_path() {
        return None;
    }
    let user_profile = std::env::var_os("USERPROFILE").map(PathBuf::from)?;
    let db_path = user_profile.join(".cc-switch").join("cc-switch.db");
    db_path.is_file().then_some(db_path)
}

fn source_priority(source_type: &str) -> i64 {
    match source_type {
        SOURCE_OPENAI_RESPONSE_USAGE => 500,
        SOURCE_CCSWITCH_PROXY => 400,
        SOURCE_CODEX_SESSION_JSONL => 300,
        SOURCE_CCSWITCH_CODEX_SESSION => 200,
        SOURCE_CCSWITCH_ROLLUP => 100,
        _ => 0,
    }
}

#[derive(Deserialize)]
struct SessionLine {
    timestamp: Option<String>,
    #[serde(rename = "type")]
    line_type: String,
    payload: Option<Value>,
}

fn freeze_ccswitch_db(codex_home: &Path, db_path: &Path) -> Option<PathBuf> {
    let snapshot_dir = codex_home.join(USAGE_EVIDENCE_DIRNAME).join("ccswitch");
    fs::create_dir_all(&snapshot_dir).ok()?;
    let snapshot = snapshot_dir.join("cc-switch-latest.db");
    fs::copy(db_path, &snapshot).ok()?;
    for extension in ["db-wal", "db-shm"] {
        let source = db_path.with_extension(extension);
        let target = snapshot.with_extension(extension);
        if source.is_file() {
            let _ = fs::copy(source, target);
        }
    }
    Some(snapshot)
}

fn collect_ccswitch_evidence(codex_home: &Path) -> AppResult<Vec<UsageEvidence>> {
    let Some(live_db) = live_ccswitch_db_path(codex_home) else {
        return Ok(Vec::new());
    };
    let Some(snapshot) = freeze_ccswitch_db(codex_home, &live_db) else {
        return Ok(Vec::new());
    };
    let conn = Connection::open_with_flags(
        &snapshot,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        AppError::new(
            "CCSWITCH_USAGE_OPEN_FAILED",
            format!(
                "Failed to open cc-switch usage snapshot {}: {error}",
                snapshot.display()
            ),
        )
    })?;
    let mut evidence = Vec::new();
    collect_ccswitch_proxy_request_evidence(&conn, &snapshot, &mut evidence)?;
    collect_ccswitch_rollup_evidence(&conn, &snapshot, &mut evidence)?;
    Ok(evidence)
}

fn collect_ccswitch_proxy_request_evidence(
    conn: &Connection,
    snapshot: &Path,
    evidence: &mut Vec<UsageEvidence>,
) -> AppResult<()> {
    let columns = match table_columns(conn, "proxy_request_logs") {
        Ok(columns) => columns,
        Err(_) => return Ok(()),
    };
    let optional_column = |name: &str| {
        if columns.contains(name) {
            name.to_string()
        } else {
            format!("NULL AS {name}")
        }
    };
    let sql = format!(
        r#"
        SELECT request_id, provider_id, model, request_model, pricing_model,
               input_tokens, output_tokens, cache_read_tokens, status_code,
               session_id, data_source, created_at,
               {}, {}, {}, {}, {}
        FROM proxy_request_logs
        WHERE app_type = 'codex'
        "#,
        optional_column("account_identity_key"),
        optional_column("account_identity_source"),
        optional_column("profile_id"),
        optional_column("profile_name"),
        optional_column("effective_base_url"),
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(stmt) => stmt,
        Err(_) => return Ok(()),
    };
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                u64_from_i64(row.get::<_, i64>(5)?),
                u64_from_i64(row.get::<_, i64>(6)?),
                u64_from_i64(row.get::<_, i64>(7)?),
                row.get::<_, Option<i64>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
                row.get::<_, i64>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, Option<String>>(13)?,
                row.get::<_, Option<String>>(14)?,
                row.get::<_, Option<String>>(15)?,
                row.get::<_, Option<String>>(16)?,
            ))
        })
        .map_err(|error| {
            AppError::new(
                "CCSWITCH_USAGE_QUERY_FAILED",
                format!("Failed to query cc-switch request logs: {error}"),
            )
        })?;
    for row in rows {
        let (
            request_id,
            provider_id,
            model,
            request_model,
            pricing_model,
            input_tokens,
            output_tokens,
            cached_input_tokens,
            status_code,
            session_id,
            data_source,
            created_at_sec,
            external_account_identity_key,
            account_identity_source,
            profile_id,
            profile_name,
            effective_base_url,
        ) = row.map_err(|error| {
            AppError::new(
                "CCSWITCH_USAGE_ROW_FAILED",
                format!("Failed to read cc-switch request log row: {error}"),
            )
        })?;
        let data_source = data_source.unwrap_or_default();
        let source_type = if data_source == "codex_session" || provider_id == "_codex_session" {
            SOURCE_CCSWITCH_CODEX_SESSION
        } else {
            SOURCE_CCSWITCH_PROXY
        };
        let model = model.or(request_model.clone());
        let created_at_ms = u64_from_i64(created_at_sec).saturating_mul(1000);
        let normalized_account_identity_key =
            normalize_ccswitch_account_identity_key(external_account_identity_key);
        let has_strict_account_proof =
            source_type == SOURCE_CCSWITCH_PROXY && normalized_account_identity_key.is_some();
        let profile = if has_strict_account_proof {
            profile_id
                .clone()
                .or_else(|| profile_name.clone())
                .or_else(|| Some(provider_id.clone()))
        } else {
            None
        };
        let official_source = has_strict_account_proof.then(|| {
            effective_base_url
                .clone()
                .unwrap_or_else(|| "ChatGPT Codex OAuth".to_string())
        });
        let delta = UsageCounts {
            input_tokens,
            cached_input_tokens: cached_input_tokens.min(input_tokens),
            output_tokens,
            reasoning_output_tokens: 0,
        };
        let fingerprint = if let (Some(account_key), Some(model)) =
            (normalized_account_identity_key.as_deref(), model.as_deref())
        {
            Some(stable_hash(&[
                account_key,
                model,
                &delta.input_tokens.to_string(),
                &delta.cached_input_tokens.to_string(),
                &delta.output_tokens.to_string(),
                &(created_at_ms / 60_000).to_string(),
                SOURCE_CCSWITCH_PROXY,
            ]))
        } else {
            None
        };
        let reason = if has_strict_account_proof {
            "cc-switch proxy row carries stable account identity proof"
        } else if source_type == SOURCE_CCSWITCH_PROXY {
            "cc-switch proxy row requires account mapping before acceptance"
        } else {
            "cc-switch codex_session row has no account proof"
        };
        evidence.push(UsageEvidence {
            evidence_id: format!("{source_type}:{request_id}"),
            request_id: Some(request_id.clone()),
            fingerprint,
            session_id,
            source_type: source_type.to_string(),
            source_priority: source_priority(source_type),
            source_locator: format!("{}#proxy_request_logs:{request_id}", snapshot.display()),
            source_hash: stable_hash(&[
                &request_id,
                &provider_id,
                model.as_deref().unwrap_or_default(),
                &created_at_sec.to_string(),
                &input_tokens.to_string(),
                &output_tokens.to_string(),
                &cached_input_tokens.to_string(),
            ]),
            parser_version: EVIDENCE_PARSER_VERSION.to_string(),
            event_started_at_ms: None,
            event_completed_at_ms: Some(created_at_ms),
            observed_at_ms: Some(created_at_ms),
            turn_index: None,
            response_id: None,
            previous_response_id: None,
            model: model.as_deref().map(normalize_model),
            request_model,
            pricing_model: pricing_model.as_deref().map(normalize_model),
            model_reasoning_effort: None,
            model_verbosity: None,
            pricing_tier: Some(PRICING_TIER_STANDARD_SHORT_CONTEXT.to_string()),
            pricing_source_url: Some(PRICING_SOURCE_URL.to_string()),
            usage_shape: if source_type == SOURCE_CCSWITCH_PROXY {
                USAGE_SHAPE_PROXY_REQUEST.to_string()
            } else {
                USAGE_SHAPE_LAST_TOKEN_USAGE.to_string()
            },
            usage_confidence: if source_type == SOURCE_CCSWITCH_PROXY {
                CONFIDENCE_HIGH.to_string()
            } else {
                CONFIDENCE_LOW.to_string()
            },
            delta: Some(delta),
            raw_usage_json: None,
            profile,
            account_identity_key: normalized_account_identity_key,
            account_label: profile_name.or(profile_id),
            identity_source: account_identity_source,
            identity_observed_at_ms: has_strict_account_proof.then_some(created_at_ms),
            account_confidence: if has_strict_account_proof {
                CONFIDENCE_PROVEN.to_string()
            } else {
                CONFIDENCE_UNPROVEN.to_string()
            },
            effective_base_url: effective_base_url.clone(),
            official_source,
            official_confidence: if has_strict_account_proof {
                CONFIDENCE_PROVEN.to_string()
            } else {
                CONFIDENCE_UNPROVEN.to_string()
            },
            status_code: status_code.and_then(|value| u16::try_from(value).ok()),
            decision: if has_strict_account_proof {
                DECISION_ACCEPTED.to_string()
            } else {
                DECISION_CANDIDATE.to_string()
            },
            decision_reason: Some(reason.to_string()),
            superseded_by: None,
            ledger_layer: if has_strict_account_proof {
                LEDGER_LAYER_STRICT_PROVEN.to_string()
            } else {
                LEDGER_LAYER_CANDIDATE_UNATTRIBUTED.to_string()
            },
            layer_reason: Some(reason.to_string()),
            attribution_kind: if has_strict_account_proof {
                ATTRIBUTION_KIND_STRICT_ACCOUNT.to_string()
            } else {
                ATTRIBUTION_KIND_UNATTRIBUTED.to_string()
            },
            attribution_window_id: has_strict_account_proof.then(|| created_at_ms.to_string()),
            legacy_account_key: None,
            created_at_ms,
        });
    }
    Ok(())
}

fn normalize_ccswitch_account_identity_key(raw: Option<String>) -> Option<String> {
    let raw = raw?.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    if let Some(account_id) = raw.strip_prefix("chatgpt:") {
        let account_id = account_id.trim();
        if account_id.is_empty() {
            return None;
        }
        return Some(account_identity_key(&AccountIdentity {
            account_id: Some(account_id.to_string()),
            email: None,
        }));
    }
    Some(raw)
}

fn collect_ccswitch_rollup_evidence(
    conn: &Connection,
    snapshot: &Path,
    evidence: &mut Vec<UsageEvidence>,
) -> AppResult<()> {
    let mut stmt = match conn.prepare(
        r#"
        SELECT date, provider_id, model, request_model, pricing_model,
               SUM(input_tokens), SUM(output_tokens), SUM(cache_read_tokens),
               SUM(request_count)
        FROM usage_daily_rollups
        WHERE app_type = 'codex'
        GROUP BY date, provider_id, model, request_model, pricing_model
        "#,
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Ok(()),
    };
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                u64_from_i64(row.get::<_, i64>(5)?),
                u64_from_i64(row.get::<_, i64>(6)?),
                u64_from_i64(row.get::<_, i64>(7)?),
                u64_from_i64(row.get::<_, i64>(8)?),
            ))
        })
        .map_err(|error| {
            AppError::new(
                "CCSWITCH_ROLLUP_QUERY_FAILED",
                format!("Failed to query cc-switch rollups: {error}"),
            )
        })?;
    for row in rows {
        let (
            date,
            provider_id,
            model,
            request_model,
            pricing_model,
            input_tokens,
            output_tokens,
            cached_input_tokens,
            request_count,
        ) = row.map_err(|error| {
            AppError::new(
                "CCSWITCH_ROLLUP_ROW_FAILED",
                format!("Failed to read cc-switch rollup row: {error}"),
            )
        })?;
        let model = model.or(request_model.clone());
        let evidence_id = format!(
            "{}:{}:{}:{}",
            SOURCE_CCSWITCH_ROLLUP,
            date,
            provider_id,
            model.as_deref().unwrap_or("unknown")
        );
        evidence.push(UsageEvidence {
            evidence_id: evidence_id.clone(),
            request_id: None,
            fingerprint: None,
            session_id: None,
            source_type: SOURCE_CCSWITCH_ROLLUP.to_string(),
            source_priority: source_priority(SOURCE_CCSWITCH_ROLLUP),
            source_locator: format!("{}#usage_daily_rollups:{date}", snapshot.display()),
            source_hash: stable_hash(&[
                &date,
                &provider_id,
                model.as_deref().unwrap_or_default(),
                &request_count.to_string(),
                &input_tokens.to_string(),
                &output_tokens.to_string(),
                &cached_input_tokens.to_string(),
            ]),
            parser_version: EVIDENCE_PARSER_VERSION.to_string(),
            event_started_at_ms: None,
            event_completed_at_ms: None,
            observed_at_ms: None,
            turn_index: None,
            response_id: None,
            previous_response_id: None,
            model: model.as_deref().map(normalize_model),
            request_model,
            pricing_model: pricing_model.as_deref().map(normalize_model),
            model_reasoning_effort: None,
            model_verbosity: None,
            pricing_tier: Some(PRICING_TIER_STANDARD_SHORT_CONTEXT.to_string()),
            pricing_source_url: Some(PRICING_SOURCE_URL.to_string()),
            usage_shape: USAGE_SHAPE_DAILY_ROLLUP.to_string(),
            usage_confidence: CONFIDENCE_LOW.to_string(),
            delta: Some(UsageCounts {
                input_tokens,
                cached_input_tokens: cached_input_tokens.min(input_tokens),
                output_tokens,
                reasoning_output_tokens: 0,
            }),
            raw_usage_json: None,
            profile: None,
            account_identity_key: None,
            account_label: None,
            identity_source: None,
            identity_observed_at_ms: None,
            account_confidence: CONFIDENCE_UNPROVEN.to_string(),
            effective_base_url: None,
            official_source: None,
            official_confidence: CONFIDENCE_UNPROVEN.to_string(),
            status_code: None,
            decision: DECISION_CANDIDATE.to_string(),
            decision_reason: Some(
                "cc-switch rollup is aggregate evidence and cannot prove request account"
                    .to_string(),
            ),
            superseded_by: None,
            ledger_layer: LEDGER_LAYER_CANDIDATE_UNATTRIBUTED.to_string(),
            layer_reason: Some(
                "cc-switch rollup is aggregate evidence and cannot prove request account"
                    .to_string(),
            ),
            attribution_kind: ATTRIBUTION_KIND_UNATTRIBUTED.to_string(),
            attribution_window_id: None,
            legacy_account_key: None,
            created_at_ms: current_time_ms(),
        });
    }
    Ok(())
}

struct SessionEvidenceInput<'a> {
    file: &'a Path,
    line_number: usize,
    line: &'a str,
    request_id: &'a str,
    session_id: &'a str,
    event_index: u64,
    event_time_ms: Option<u64>,
    model: Option<&'a str>,
    request_model: Option<&'a str>,
    model_reasoning_effort: Option<&'a str>,
    model_verbosity: Option<&'a str>,
    parsed_usage: Option<ParsedUsage>,
    delta: Option<UsageCounts>,
    attribution: Option<&'a AttributionContext>,
    fingerprint: Option<String>,
    decision: &'a str,
    decision_reason: Option<&'a str>,
    superseded_by: Option<&'a str>,
}

fn session_usage_evidence(input: SessionEvidenceInput<'_>) -> UsageEvidence {
    let source_type = SOURCE_CODEX_SESSION_JSONL.to_string();
    let parsed_usage = input.parsed_usage;
    let attribution = input.attribution;
    let (source_locator, source_hash, evidence_id) =
        session_evidence_identity(input.file, input.line_number, input.line, input.request_id);
    let profile = attribution.map(|value| value.profile.clone());
    let account_identity_key = attribution.and_then(|value| value.account_identity_key.clone());
    let account_label = attribution.and_then(|value| value.account_label.clone());
    let model_reasoning_effort = input
        .model_reasoning_effort
        .map(ToString::to_string)
        .or_else(|| attribution.and_then(|value| value.model_reasoning_effort.clone()));
    let model_verbosity = input
        .model_verbosity
        .map(ToString::to_string)
        .or_else(|| attribution.and_then(|value| value.model_verbosity.clone()));
    let identity_source = attribution.and_then(|value| value.identity_source.clone());
    let identity_observed_at_ms = attribution.and_then(|value| value.identity_observed_at_ms);
    let official_source = attribution.map(|value| value.official_source.clone());
    let ledger_layer = ledger_layer_for_decision(input.decision, attribution);
    let attribution_kind = attribution
        .map(|value| value.attribution_kind.clone())
        .unwrap_or_else(|| ATTRIBUTION_KIND_UNATTRIBUTED.to_string());
    let attribution_window_id = attribution.and_then(|value| value.attribution_window_id.clone());
    let legacy_account_key = attribution.and_then(|value| value.legacy_account_key.clone());
    let layer_reason = input
        .decision_reason
        .map(ToString::to_string)
        .or_else(|| attribution.and_then(|value| value.layer_reason.clone()));
    UsageEvidence {
        evidence_id,
        request_id: Some(input.request_id.to_string()),
        fingerprint: input.fingerprint,
        session_id: Some(input.session_id.to_string()),
        source_type,
        source_priority: source_priority(SOURCE_CODEX_SESSION_JSONL),
        source_locator,
        source_hash,
        parser_version: EVIDENCE_PARSER_VERSION.to_string(),
        event_started_at_ms: None,
        event_completed_at_ms: input.event_time_ms,
        observed_at_ms: input.event_time_ms,
        turn_index: Some(input.event_index),
        response_id: None,
        previous_response_id: None,
        model: input.model.map(ToString::to_string),
        request_model: input.request_model.map(ToString::to_string),
        pricing_model: input.model.map(ToString::to_string),
        model_reasoning_effort,
        model_verbosity,
        pricing_tier: Some(PRICING_TIER_STANDARD_SHORT_CONTEXT.to_string()),
        pricing_source_url: Some(PRICING_SOURCE_URL.to_string()),
        usage_shape: parsed_usage
            .map(|usage| usage.usage_shape.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        usage_confidence: parsed_usage
            .map(|usage| usage.usage_confidence.to_string())
            .unwrap_or_else(|| CONFIDENCE_UNPROVEN.to_string()),
        delta: input.delta,
        raw_usage_json: Some(input.line.to_string()),
        profile,
        account_identity_key: account_identity_key.clone(),
        account_label,
        identity_source,
        identity_observed_at_ms,
        account_confidence: if account_identity_key.is_some() {
            CONFIDENCE_PROVEN.to_string()
        } else {
            CONFIDENCE_UNPROVEN.to_string()
        },
        effective_base_url: official_source.clone(),
        official_source: official_source.clone(),
        official_confidence: if official_source.is_some() {
            CONFIDENCE_PROVEN.to_string()
        } else {
            CONFIDENCE_UNPROVEN.to_string()
        },
        status_code: Some(200),
        decision: input.decision.to_string(),
        decision_reason: input.decision_reason.map(ToString::to_string),
        superseded_by: input.superseded_by.map(ToString::to_string),
        ledger_layer,
        layer_reason,
        attribution_kind,
        attribution_window_id,
        legacy_account_key,
        created_at_ms: input.event_time_ms.unwrap_or_else(current_time_ms),
    }
}

fn session_evidence_identity(
    file: &Path,
    line_number: usize,
    line: &str,
    request_id: &str,
) -> (String, String, String) {
    let source_locator = format!("{}:{}", file.display(), line_number);
    let source_hash = stable_hash(&[line]);
    let evidence_instance_hash = stable_hash(&[&source_locator, &source_hash]);
    (
        source_locator,
        source_hash,
        format!("{SOURCE_CODEX_SESSION_JSONL}:{request_id}:{evidence_instance_hash}"),
    )
}

fn collect_session_file_events(
    file: &Path,
    fallback_attribution: Option<&AttributionContext>,
    attribution_records: &[AttributionRecord],
    dedupe: &mut UsageDedupeEngine,
    events: &mut Vec<UsageEvent>,
    facts: &mut Vec<UsageFact>,
    evidence: &mut Vec<UsageEvidence>,
    decisions: &mut EvidenceDecisionCounts,
    summary: &mut OpenAiUsageSummary,
) {
    let Ok(raw) = fs::read_to_string(file) else {
        return;
    };
    let mut session_id = file
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown-session")
        .to_string();
    let mut current_model: Option<String> = None;
    let mut current_request_model: Option<String> = None;
    let mut current_reasoning_effort: Option<String> = None;
    let mut current_verbosity: Option<String> = None;
    let mut previous_usage: Option<UsageCounts> = None;
    let mut event_index = 0_u64;

    for (line_index, line) in raw.lines().enumerate() {
        let parsed = match serde_json::from_str::<SessionLine>(line) {
            Ok(value) => value,
            Err(_) => {
                summary.exclusions.malformed_lines =
                    summary.exclusions.malformed_lines.saturating_add(1);
                continue;
            }
        };
        let payload = parsed.payload.as_ref();
        match parsed.line_type.as_str() {
            "session_meta" => {
                if let Some(id) = payload
                    .and_then(|value| value.get("id"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    session_id = id.to_string();
                }
            }
            "turn_context" => {
                current_request_model = payload
                    .and_then(|value| value.get("model"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string);
                current_model = current_request_model
                    .as_deref()
                    .map(normalize_model)
                    .filter(|value| !value.is_empty());
                current_reasoning_effort = payload.and_then(extract_model_reasoning_effort);
                current_verbosity = payload.and_then(extract_model_verbosity);
            }
            "event_msg" => {
                if payload
                    .and_then(|value| value.get("type"))
                    .and_then(Value::as_str)
                    != Some("token_count")
                {
                    continue;
                }
                event_index = event_index.saturating_add(1);
                let event_time_ms = parsed.timestamp.as_deref().and_then(parse_timestamp_ms);
                let request_id = format!("codex_session:{session_id}:{event_index}");
                let parsed_usage = payload.and_then(parse_usage_counts);
                let Some(parsed_usage) = parsed_usage else {
                    summary.exclusions.malformed_lines =
                        summary.exclusions.malformed_lines.saturating_add(1);
                    decisions.rejected = decisions.rejected.saturating_add(1);
                    evidence.push(session_usage_evidence(SessionEvidenceInput {
                        file,
                        line_number: line_index + 1,
                        line,
                        request_id: &request_id,
                        session_id: &session_id,
                        event_index,
                        event_time_ms,
                        model: current_model.as_deref(),
                        request_model: current_request_model.as_deref(),
                        model_reasoning_effort: current_reasoning_effort.as_deref(),
                        model_verbosity: current_verbosity.as_deref(),
                        parsed_usage: None,
                        delta: None,
                        attribution: None,
                        fingerprint: None,
                        decision: DECISION_REJECTED,
                        decision_reason: Some("malformed token_count usage"),
                        superseded_by: None,
                    }));
                    continue;
                };
                let current_usage = parsed_usage.counts;
                let Some(model) = current_model.clone() else {
                    previous_usage = Some(current_usage);
                    summary.exclusions.missing_model =
                        summary.exclusions.missing_model.saturating_add(1);
                    decisions.rejected = decisions.rejected.saturating_add(1);
                    evidence.push(session_usage_evidence(SessionEvidenceInput {
                        file,
                        line_number: line_index + 1,
                        line,
                        request_id: &request_id,
                        session_id: &session_id,
                        event_index,
                        event_time_ms,
                        model: None,
                        request_model: current_request_model.as_deref(),
                        model_reasoning_effort: current_reasoning_effort.as_deref(),
                        model_verbosity: current_verbosity.as_deref(),
                        parsed_usage: Some(parsed_usage),
                        delta: None,
                        attribution: None,
                        fingerprint: None,
                        decision: DECISION_REJECTED,
                        decision_reason: Some("missing model"),
                        superseded_by: None,
                    }));
                    continue;
                };
                let Some(delta) = usage_delta(previous_usage, current_usage) else {
                    previous_usage = Some(current_usage);
                    summary.exclusions.invalid_delta =
                        summary.exclusions.invalid_delta.saturating_add(1);
                    decisions.rejected = decisions.rejected.saturating_add(1);
                    evidence.push(session_usage_evidence(SessionEvidenceInput {
                        file,
                        line_number: line_index + 1,
                        line,
                        request_id: &request_id,
                        session_id: &session_id,
                        event_index,
                        event_time_ms,
                        model: Some(&model),
                        request_model: current_request_model.as_deref(),
                        model_reasoning_effort: current_reasoning_effort.as_deref(),
                        model_verbosity: current_verbosity.as_deref(),
                        parsed_usage: Some(parsed_usage),
                        delta: None,
                        attribution: None,
                        fingerprint: None,
                        decision: DECISION_REJECTED,
                        decision_reason: Some("invalid negative token delta"),
                        superseded_by: None,
                    }));
                    continue;
                };
                previous_usage = Some(current_usage);
                if delta.input_tokens == 0 && delta.output_tokens == 0 {
                    decisions.rejected = decisions.rejected.saturating_add(1);
                    evidence.push(session_usage_evidence(SessionEvidenceInput {
                        file,
                        line_number: line_index + 1,
                        line,
                        request_id: &request_id,
                        session_id: &session_id,
                        event_index,
                        event_time_ms,
                        model: Some(&model),
                        request_model: current_request_model.as_deref(),
                        model_reasoning_effort: current_reasoning_effort.as_deref(),
                        model_verbosity: current_verbosity.as_deref(),
                        parsed_usage: Some(parsed_usage),
                        delta: Some(delta),
                        attribution: None,
                        fingerprint: None,
                        decision: DECISION_REJECTED,
                        decision_reason: Some("zero token delta"),
                        superseded_by: None,
                    }));
                    continue;
                }

                let Some(event_time_ms) = event_time_ms else {
                    summary.exclusions.unproven_history =
                        summary.exclusions.unproven_history.saturating_add(1);
                    decisions.rejected = decisions.rejected.saturating_add(1);
                    evidence.push(session_usage_evidence(SessionEvidenceInput {
                        file,
                        line_number: line_index + 1,
                        line,
                        request_id: &request_id,
                        session_id: &session_id,
                        event_index,
                        event_time_ms: None,
                        model: Some(&model),
                        request_model: current_request_model.as_deref(),
                        model_reasoning_effort: current_reasoning_effort.as_deref(),
                        model_verbosity: current_verbosity.as_deref(),
                        parsed_usage: Some(parsed_usage),
                        delta: Some(delta),
                        attribution: None,
                        fingerprint: None,
                        decision: DECISION_REJECTED,
                        decision_reason: Some("missing event timestamp"),
                        superseded_by: None,
                    }));
                    continue;
                };

                let attribution = match attribution_for_event(
                    event_time_ms,
                    fallback_attribution,
                    attribution_records,
                    &mut summary.exclusions,
                ) {
                    AttributionLookup::Attributed(value) => value,
                    AttributionLookup::CandidateUnattributed(reason) => {
                        decisions.candidate = decisions.candidate.saturating_add(1);
                        evidence.push(session_usage_evidence(SessionEvidenceInput {
                            file,
                            line_number: line_index + 1,
                            line,
                            request_id: &request_id,
                            session_id: &session_id,
                            event_index,
                            event_time_ms: Some(event_time_ms),
                            model: Some(&model),
                            request_model: current_request_model.as_deref(),
                            model_reasoning_effort: current_reasoning_effort.as_deref(),
                            model_verbosity: current_verbosity.as_deref(),
                            parsed_usage: Some(parsed_usage),
                            delta: Some(delta),
                            attribution: None,
                            fingerprint: None,
                            decision: DECISION_CANDIDATE,
                            decision_reason: Some(reason),
                            superseded_by: None,
                        }));
                        continue;
                    }
                    AttributionLookup::RejectedInvalid(reason) => {
                        decisions.rejected = decisions.rejected.saturating_add(1);
                        evidence.push(session_usage_evidence(SessionEvidenceInput {
                            file,
                            line_number: line_index + 1,
                            line,
                            request_id: &request_id,
                            session_id: &session_id,
                            event_index,
                            event_time_ms: Some(event_time_ms),
                            model: Some(&model),
                            request_model: current_request_model.as_deref(),
                            model_reasoning_effort: current_reasoning_effort.as_deref(),
                            model_verbosity: current_verbosity.as_deref(),
                            parsed_usage: Some(parsed_usage),
                            delta: Some(delta),
                            attribution: None,
                            fingerprint: None,
                            decision: DECISION_REJECTED,
                            decision_reason: Some(reason),
                            superseded_by: None,
                        }));
                        continue;
                    }
                };

                let fingerprint =
                    fingerprint_for_event(&attribution, &model, delta, Some(event_time_ms));
                let (_, _, evidence_id) =
                    session_evidence_identity(file, line_index + 1, line, &request_id);
                if let Some((reason, superseded_by)) =
                    dedupe.accept_or_reason(&request_id, &fingerprint, &evidence_id)
                {
                    summary.exclusions.duplicate_events =
                        summary.exclusions.duplicate_events.saturating_add(1);
                    decisions.superseded = decisions.superseded.saturating_add(1);
                    evidence.push(session_usage_evidence(SessionEvidenceInput {
                        file,
                        line_number: line_index + 1,
                        line,
                        request_id: &request_id,
                        session_id: &session_id,
                        event_index,
                        event_time_ms: Some(event_time_ms),
                        model: Some(&model),
                        request_model: current_request_model.as_deref(),
                        model_reasoning_effort: current_reasoning_effort.as_deref(),
                        model_verbosity: current_verbosity.as_deref(),
                        parsed_usage: Some(parsed_usage),
                        delta: Some(delta),
                        attribution: Some(&attribution),
                        fingerprint: Some(fingerprint),
                        decision: DECISION_SUPERSEDED,
                        decision_reason: Some(reason),
                        superseded_by: Some(&superseded_by),
                    }));
                    continue;
                }
                let accepted_evidence = session_usage_evidence(SessionEvidenceInput {
                    file,
                    line_number: line_index + 1,
                    line,
                    request_id: &request_id,
                    session_id: &session_id,
                    event_index,
                    event_time_ms: Some(event_time_ms),
                    model: Some(&model),
                    request_model: current_request_model.as_deref(),
                    model_reasoning_effort: current_reasoning_effort.as_deref(),
                    model_verbosity: current_verbosity.as_deref(),
                    parsed_usage: Some(parsed_usage),
                    delta: Some(delta),
                    attribution: Some(&attribution),
                    fingerprint: Some(fingerprint.clone()),
                    decision: DECISION_ACCEPTED,
                    decision_reason: Some(accepted_decision_reason(&attribution)),
                    superseded_by: None,
                });
                let evidence_id = accepted_evidence.evidence_id.clone();
                let cost = calculate_cost(&model, delta);
                let decision = EvidenceClassifier::accept_session_evidence(accepted_evidence, cost);
                evidence.push(decision.evidence);
                if let Some(event) = decision.accepted {
                    debug_assert_eq!(event.evidence_id, evidence_id);
                    events.push(event);
                    decisions.accepted = decisions.accepted.saturating_add(1);
                }
                if let Some(fact) = decision.fact {
                    facts.push(fact);
                }
            }
            _ => {}
        }
    }
}

fn parse_usage_counts(payload: &Value) -> Option<ParsedUsage> {
    let info = payload.get("info")?;
    let (usage, usage_shape, usage_confidence) = if let Some(usage) = info.get("total_token_usage")
    {
        (usage, USAGE_SHAPE_CUMULATIVE_TOTAL_DELTA, CONFIDENCE_HIGH)
    } else {
        (
            info.get("last_token_usage")?,
            USAGE_SHAPE_LAST_TOKEN_USAGE,
            CONFIDENCE_LOW,
        )
    };
    let input_tokens = usage.get("input_tokens")?.as_u64()?;
    let cached_input_tokens = usage
        .get("cached_input_tokens")
        .or_else(|| usage.get("cache_read_input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(input_tokens);
    Some(ParsedUsage {
        counts: UsageCounts {
            input_tokens,
            cached_input_tokens,
            output_tokens: usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            reasoning_output_tokens: usage
                .get("reasoning_output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        },
        usage_shape,
        usage_confidence,
    })
}

fn extract_model_reasoning_effort(payload: &Value) -> Option<String> {
    string_field(payload, &["model_reasoning_effort", "reasoning_effort"])
        .and_then(normalize_model_reasoning_effort)
}

fn extract_model_verbosity(payload: &Value) -> Option<String> {
    string_field(payload, &["model_verbosity", "verbosity"]).and_then(normalize_model_verbosity)
}

fn usage_delta(previous: Option<UsageCounts>, current: UsageCounts) -> Option<UsageCounts> {
    let Some(previous) = previous else {
        return Some(current);
    };
    if current.input_tokens < previous.input_tokens
        || current.cached_input_tokens < previous.cached_input_tokens
        || current.output_tokens < previous.output_tokens
        || current.reasoning_output_tokens < previous.reasoning_output_tokens
    {
        return None;
    }
    Some(UsageCounts {
        input_tokens: current.input_tokens - previous.input_tokens,
        cached_input_tokens: current.cached_input_tokens - previous.cached_input_tokens,
        output_tokens: current.output_tokens - previous.output_tokens,
        reasoning_output_tokens: current.reasoning_output_tokens - previous.reasoning_output_tokens,
    })
}

trait UsageEvidenceAdapter {
    fn source_type(&self) -> &'static str;

    fn collect(
        &self,
        codex_home: &Path,
        context: &mut IngestionContext<'_>,
    ) -> AppResult<Vec<UsageEvidence>>;
}

struct SessionJsonlAdapter;

impl UsageEvidenceAdapter for SessionJsonlAdapter {
    fn source_type(&self) -> &'static str {
        SOURCE_CODEX_SESSION_JSONL
    }

    fn collect(
        &self,
        codex_home: &Path,
        context: &mut IngestionContext<'_>,
    ) -> AppResult<Vec<UsageEvidence>> {
        let mut evidence = Vec::new();
        for sessions_root in [
            codex_home.join("sessions"),
            codex_home.join("archived_sessions"),
        ] {
            let mut files = Vec::new();
            collect_jsonl_files(&sessions_root, &mut files);
            files.sort();
            for file in files {
                collect_session_file_events(
                    &file,
                    context.fallback_attribution.as_ref(),
                    &context.attribution_records,
                    &mut context.dedupe,
                    &mut context.events,
                    &mut context.facts,
                    &mut evidence,
                    &mut context.decisions,
                    context.summary,
                );
            }
        }
        Ok(evidence)
    }
}

struct FutureOpenAiResponseUsageAdapter;

impl UsageEvidenceAdapter for FutureOpenAiResponseUsageAdapter {
    fn source_type(&self) -> &'static str {
        SOURCE_OPENAI_RESPONSE_USAGE
    }

    fn collect(
        &self,
        _codex_home: &Path,
        _context: &mut IngestionContext<'_>,
    ) -> AppResult<Vec<UsageEvidence>> {
        Ok(Vec::new())
    }
}

struct CcSwitchAdapter;

impl UsageEvidenceAdapter for CcSwitchAdapter {
    fn source_type(&self) -> &'static str {
        SOURCE_CCSWITCH_PROXY
    }

    fn collect(
        &self,
        codex_home: &Path,
        _context: &mut IngestionContext<'_>,
    ) -> AppResult<Vec<UsageEvidence>> {
        collect_ccswitch_evidence(codex_home)
    }
}

struct IngestionContext<'a> {
    fallback_attribution: Option<AttributionContext>,
    attribution_records: Vec<AttributionRecord>,
    dedupe: UsageDedupeEngine,
    events: Vec<UsageEvent>,
    facts: Vec<UsageFact>,
    evidence: Vec<UsageEvidence>,
    decisions: EvidenceDecisionCounts,
    summary: &'a mut OpenAiUsageSummary,
}

fn collect_from_adapters(
    adapters: &[&dyn UsageEvidenceAdapter],
    codex_home: &Path,
    context: &mut IngestionContext<'_>,
) -> AppResult<()> {
    for adapter in adapters {
        let _ = adapter.source_type();
        let evidence = adapter.collect(codex_home, context)?;
        context.evidence.extend(evidence);
    }
    Ok(())
}

pub(super) fn ingest_usage<F>(
    codex_home: &Path,
    summary: &mut OpenAiUsageSummary,
    cache_probe: F,
) -> AppResult<UsageLoadResult>
where
    F: FnOnce(&str, &mut OpenAiUsageSummary) -> AppResult<bool>,
{
    ingest_usage_with_adapters(
        &[
            &SessionJsonlAdapter,
            &FutureOpenAiResponseUsageAdapter,
            &CcSwitchAdapter,
        ],
        codex_home,
        summary,
        cache_probe,
    )
}

fn ingest_usage_with_adapters<F>(
    adapters: &[&dyn UsageEvidenceAdapter],
    codex_home: &Path,
    summary: &mut OpenAiUsageSummary,
    cache_probe: F,
) -> AppResult<UsageLoadResult>
where
    F: FnOnce(&str, &mut OpenAiUsageSummary) -> AppResult<bool>,
{
    let source_signature = usage_source_signature(codex_home);
    if cache_probe(&source_signature, summary)? {
        return Ok(UsageLoadResult {
            source_signature,
            cache_hit: true,
            events: Vec::new(),
            facts: Vec::new(),
            evidence: Vec::new(),
            decisions: EvidenceDecisionCounts::default(),
        });
    }

    let fallback_attribution = build_attribution_context(codex_home, &mut summary.exclusions)?;
    let attribution_records = load_attribution_records(codex_home);
    summary.attribution_since_ms = attribution_records
        .iter()
        .filter(|record| record.is_official_openai)
        .map(|record| record.observed_at_ms)
        .min()
        .or_else(|| {
            fallback_attribution
                .as_ref()
                .map(|record| record.attribution_since_ms)
        });

    let mut context = IngestionContext {
        fallback_attribution,
        attribution_records,
        dedupe: UsageDedupeEngine::default(),
        events: Vec::new(),
        facts: Vec::new(),
        evidence: Vec::new(),
        decisions: EvidenceDecisionCounts::default(),
        summary,
    };
    collect_from_adapters(adapters, codex_home, &mut context)?;
    let mut usage = finalize_usage_load(
        context.events,
        context.facts,
        context.evidence,
        context.decisions,
        context.summary,
    );
    usage.source_signature = source_signature;
    Ok(usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct RecordingAdapter {
        source_type: &'static str,
        calls: Rc<RefCell<Vec<&'static str>>>,
    }

    impl RecordingAdapter {
        fn new(source_type: &'static str, calls: Rc<RefCell<Vec<&'static str>>>) -> Self {
            Self { source_type, calls }
        }
    }

    impl UsageEvidenceAdapter for RecordingAdapter {
        fn source_type(&self) -> &'static str {
            self.source_type
        }

        fn collect(
            &self,
            _codex_home: &Path,
            _context: &mut IngestionContext<'_>,
        ) -> AppResult<Vec<UsageEvidence>> {
            self.calls.borrow_mut().push(self.source_type);
            Ok(Vec::new())
        }
    }

    struct PanicAdapter;

    impl UsageEvidenceAdapter for PanicAdapter {
        fn source_type(&self) -> &'static str {
            "panic_adapter"
        }

        fn collect(
            &self,
            _codex_home: &Path,
            _context: &mut IngestionContext<'_>,
        ) -> AppResult<Vec<UsageEvidence>> {
            panic!("cache hit must not collect evidence")
        }
    }

    fn test_context(summary: &mut OpenAiUsageSummary) -> IngestionContext<'_> {
        IngestionContext {
            fallback_attribution: None,
            attribution_records: Vec::new(),
            dedupe: UsageDedupeEngine::default(),
            events: Vec::new(),
            facts: Vec::new(),
            evidence: Vec::new(),
            decisions: EvidenceDecisionCounts::default(),
            summary,
        }
    }

    fn temp_codex_home(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("codex-switch-openai-usage-{name}-{unique}"))
    }

    fn ts(value: &str) -> u64 {
        parse_timestamp_ms(value).unwrap()
    }

    fn accepted_test_evidence(
        evidence_id: &str,
        source_type: &str,
        source_priority: i64,
    ) -> UsageEvidence {
        UsageEvidence {
            evidence_id: evidence_id.to_string(),
            request_id: Some("request-1".to_string()),
            fingerprint: Some("fingerprint-1".to_string()),
            session_id: Some("session-1".to_string()),
            source_type: source_type.to_string(),
            source_priority,
            source_locator: format!("test:{evidence_id}"),
            source_hash: evidence_id.to_string(),
            parser_version: EVIDENCE_PARSER_VERSION.to_string(),
            event_started_at_ms: Some(ts("2999-01-01T00:00:01.000Z")),
            event_completed_at_ms: Some(ts("2999-01-01T00:00:02.000Z")),
            observed_at_ms: Some(ts("2999-01-01T00:00:02.000Z")),
            turn_index: Some(1),
            response_id: Some("response-1".to_string()),
            previous_response_id: None,
            model: Some("gpt-5.3-codex".to_string()),
            request_model: Some("gpt-5.3-codex".to_string()),
            pricing_model: Some("gpt-5.3-codex".to_string()),
            model_reasoning_effort: None,
            model_verbosity: None,
            pricing_tier: Some(PRICING_TIER_STANDARD_SHORT_CONTEXT.to_string()),
            pricing_source_url: Some(PRICING_SOURCE_URL.to_string()),
            usage_shape: USAGE_SHAPE_CUMULATIVE_TOTAL_DELTA.to_string(),
            usage_confidence: CONFIDENCE_HIGH.to_string(),
            delta: Some(UsageCounts {
                input_tokens: 100,
                cached_input_tokens: 20,
                output_tokens: 10,
                reasoning_output_tokens: 0,
            }),
            raw_usage_json: None,
            profile: Some("a".to_string()),
            account_identity_key: Some("acct_key_a".to_string()),
            account_label: Some("acct_A".to_string()),
            identity_source: Some("test".to_string()),
            identity_observed_at_ms: Some(ts("2999-01-01T00:00:00.000Z")),
            account_confidence: CONFIDENCE_PROVEN.to_string(),
            effective_base_url: None,
            official_source: Some("OpenAI default".to_string()),
            official_confidence: CONFIDENCE_PROVEN.to_string(),
            status_code: Some(200),
            decision: DECISION_ACCEPTED.to_string(),
            decision_reason: Some("test accepted evidence".to_string()),
            superseded_by: None,
            ledger_layer: LEDGER_LAYER_STRICT_PROVEN.to_string(),
            layer_reason: Some("test strict proof".to_string()),
            attribution_kind: ATTRIBUTION_KIND_STRICT_ACCOUNT.to_string(),
            attribution_window_id: Some("window-1".to_string()),
            legacy_account_key: None,
            created_at_ms: ts("2999-01-01T00:00:02.000Z"),
        }
    }

    #[test]
    fn higher_priority_evidence_supersedes_session_fallback_before_facts_are_finalized() {
        let session = accepted_test_evidence(
            "session-evidence",
            SOURCE_CODEX_SESSION_JSONL,
            source_priority(SOURCE_CODEX_SESSION_JSONL),
        );
        let response = accepted_test_evidence(
            "response-evidence",
            SOURCE_OPENAI_RESPONSE_USAGE,
            source_priority(SOURCE_OPENAI_RESPONSE_USAGE),
        );
        let mut summary = new_usage_summary(&OpenAiUsageQuery::default());

        let result = finalize_usage_load(
            Vec::new(),
            Vec::new(),
            vec![session, response],
            EvidenceDecisionCounts::default(),
            &mut summary,
        );

        assert_eq!(result.events.len(), 1);
        assert_eq!(result.events[0].evidence_id, "response-evidence");
        assert_eq!(result.facts.len(), 1);
        assert_eq!(result.facts[0].evidence_id, "response-evidence");
        assert_eq!(result.decisions.accepted, 1);
        assert_eq!(result.decisions.superseded, 1);
        assert_eq!(summary.exclusions.superseded_events, 1);
        assert_eq!(summary.exclusions.duplicate_events, 0);
        let session_evidence = result
            .evidence
            .iter()
            .find(|entry| entry.evidence_id == "session-evidence")
            .unwrap();
        assert_eq!(session_evidence.decision, DECISION_SUPERSEDED);
        assert_eq!(
            session_evidence.superseded_by.as_deref(),
            Some("response-evidence")
        );
        assert_eq!(
            session_evidence.layer_reason.as_deref(),
            Some("superseded by higher-priority evidence source")
        );
    }

    #[test]
    fn ccswitch_rows_are_candidate_evidence_not_accepted_events() {
        let temp = temp_codex_home("ccswitch-evidence");
        fs::create_dir_all(&temp).unwrap();
        let db_path = temp.join("cc-switch.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE proxy_request_logs (
                request_id TEXT PRIMARY KEY,
                provider_id TEXT NOT NULL,
                app_type TEXT NOT NULL,
                model TEXT,
                request_model TEXT,
                pricing_model TEXT,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                cache_read_tokens INTEGER NOT NULL,
                status_code INTEGER,
                session_id TEXT,
                data_source TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE usage_daily_rollups (
                date TEXT NOT NULL,
                app_type TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                model TEXT,
                request_model TEXT,
                pricing_model TEXT,
                request_count INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                cache_read_tokens INTEGER NOT NULL
            );
            INSERT INTO proxy_request_logs VALUES
              ('proxy-1','openai','codex','gpt-5.3-codex','gpt-5.3-codex','gpt-5.3-codex',10,2,4,200,'s-proxy','proxy',32503680000),
              ('session-1','_codex_session','codex','gpt-5.3-codex','gpt-5.3-codex',NULL,20,3,5,200,'s-session','codex_session',32503680001);
            INSERT INTO usage_daily_rollups VALUES
              ('2999-01-01','codex','openai','gpt-5.3-codex','gpt-5.3-codex','gpt-5.3-codex',2,30,5,9);
            "#,
        )
        .unwrap();

        let mut evidence = Vec::new();
        collect_ccswitch_proxy_request_evidence(&conn, &db_path, &mut evidence).unwrap();
        collect_ccswitch_rollup_evidence(&conn, &db_path, &mut evidence).unwrap();

        assert_eq!(evidence.len(), 3);
        assert!(evidence
            .iter()
            .all(|entry| entry.decision == DECISION_CANDIDATE));
        assert!(evidence
            .iter()
            .any(|entry| entry.source_type == SOURCE_CCSWITCH_PROXY));
        assert!(evidence
            .iter()
            .any(|entry| entry.source_type == SOURCE_CCSWITCH_CODEX_SESSION));
        assert!(evidence
            .iter()
            .any(|entry| entry.source_type == SOURCE_CCSWITCH_ROLLUP));
        assert!(evidence
            .iter()
            .all(|entry| entry.account_confidence == CONFIDENCE_UNPROVEN));
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn ccswitch_proxy_rows_with_account_identity_are_strict_proven() {
        let temp = temp_codex_home("ccswitch-strict-identity");
        fs::create_dir_all(&temp).unwrap();
        let db_path = temp.join("cc-switch.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE proxy_request_logs (
                request_id TEXT PRIMARY KEY,
                provider_id TEXT NOT NULL,
                app_type TEXT NOT NULL,
                model TEXT,
                request_model TEXT,
                pricing_model TEXT,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                cache_read_tokens INTEGER NOT NULL,
                status_code INTEGER,
                session_id TEXT,
                data_source TEXT,
                created_at INTEGER NOT NULL,
                account_identity_key TEXT,
                account_identity_source TEXT,
                profile_id TEXT,
                profile_name TEXT,
                effective_base_url TEXT
            );
            INSERT INTO proxy_request_logs VALUES
              ('proxy-proof','codex-oauth','codex','gpt-5.3-codex','gpt-5.3-codex','gpt-5.3-codex',
               10,2,4,200,'s-proof','proxy',32503680000,'chatgpt:acct_ccswitch',
               'codex_oauth_chatgpt_account','profile-codex','Codex OAuth','https://chatgpt.com/backend-api/codex');
            "#,
        )
        .unwrap();

        let mut evidence = Vec::new();
        collect_ccswitch_proxy_request_evidence(&conn, &db_path, &mut evidence).unwrap();

        assert_eq!(evidence.len(), 1);
        let entry = &evidence[0];
        let expected_key = account_identity_key(&AccountIdentity {
            account_id: Some("acct_ccswitch".to_string()),
            email: None,
        });
        assert_eq!(entry.source_type, SOURCE_CCSWITCH_PROXY);
        assert_eq!(entry.decision, DECISION_ACCEPTED);
        assert_eq!(entry.ledger_layer, LEDGER_LAYER_STRICT_PROVEN);
        assert_eq!(entry.attribution_kind, ATTRIBUTION_KIND_STRICT_ACCOUNT);
        assert_eq!(
            entry.account_identity_key.as_deref(),
            Some(expected_key.as_str())
        );
        assert_eq!(entry.account_confidence, CONFIDENCE_PROVEN);
        assert_eq!(entry.official_confidence, CONFIDENCE_PROVEN);
        assert_eq!(entry.profile.as_deref(), Some("profile-codex"));
        assert!(entry.fingerprint.is_some());
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn freeze_ccswitch_db_preserves_wal_state_in_read_only_snapshot() {
        let codex_home = temp_codex_home("ccswitch-freeze-wal");
        let live_dir = codex_home.join("live-ccswitch");
        fs::create_dir_all(&live_dir).unwrap();
        let live_db = live_dir.join("cc-switch.db");
        let live_conn = Connection::open(&live_db).unwrap();
        let journal_mode = live_conn
            .query_row("PRAGMA journal_mode = WAL", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap();
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        live_conn
            .pragma_update(None, "wal_autocheckpoint", 0)
            .unwrap();
        live_conn
            .execute_batch(
                r#"
                CREATE TABLE wal_fixture (value TEXT NOT NULL);
                INSERT INTO wal_fixture VALUES ('visible-through-snapshot-wal');
                "#,
            )
            .unwrap();

        let live_wal = live_db.with_extension("db-wal");
        let live_shm = live_db.with_extension("db-shm");
        assert!(live_wal.is_file());
        assert!(live_shm.is_file());

        let snapshot = freeze_ccswitch_db(&codex_home, &live_db).unwrap();
        assert_eq!(
            snapshot,
            codex_home
                .join(USAGE_EVIDENCE_DIRNAME)
                .join("ccswitch")
                .join("cc-switch-latest.db")
        );
        assert!(snapshot.with_extension("db-wal").is_file());
        assert!(snapshot.with_extension("db-shm").is_file());

        let snapshot_conn = Connection::open_with_flags(
            &snapshot,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .unwrap();
        let value = snapshot_conn
            .query_row("SELECT value FROM wal_fixture", [], |row| {
                row.get::<_, String>(0)
            })
            .unwrap();
        assert_eq!(value, "visible-through-snapshot-wal");
        assert!(snapshot_conn
            .execute("INSERT INTO wal_fixture VALUES ('must-fail')", [])
            .is_err());

        drop(snapshot_conn);
        drop(live_conn);
        let _ = fs::remove_dir_all(&codex_home);
    }

    #[test]
    fn future_response_source_has_highest_priority_without_collecting_rows() {
        let adapter = FutureOpenAiResponseUsageAdapter;
        assert_eq!(adapter.source_type(), SOURCE_OPENAI_RESPONSE_USAGE);
        assert!(source_priority(adapter.source_type()) > source_priority(SOURCE_CCSWITCH_PROXY));
        let mut summary = new_usage_summary(&OpenAiUsageQuery::default());
        let mut context = test_context(&mut summary);
        assert!(adapter
            .collect(Path::new("unused"), &mut context)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn negative_delta_is_invalid() {
        assert!(usage_delta(
            Some(UsageCounts {
                input_tokens: 10,
                cached_input_tokens: 5,
                output_tokens: 2,
                reasoning_output_tokens: 1,
            }),
            UsageCounts {
                input_tokens: 9,
                cached_input_tokens: 5,
                output_tokens: 3,
                reasoning_output_tokens: 1,
            },
        )
        .is_none());
    }

    #[test]
    fn adapters_collect_in_stable_priority_order() {
        let mut summary = new_usage_summary(&OpenAiUsageQuery::default());
        let mut context = test_context(&mut summary);
        let calls = Rc::new(RefCell::new(Vec::new()));
        collect_from_adapters(
            &[
                &RecordingAdapter::new(SOURCE_CODEX_SESSION_JSONL, Rc::clone(&calls)),
                &RecordingAdapter::new(SOURCE_OPENAI_RESPONSE_USAGE, Rc::clone(&calls)),
                &RecordingAdapter::new(SOURCE_CCSWITCH_PROXY, Rc::clone(&calls)),
            ],
            Path::new("unused"),
            &mut context,
        )
        .unwrap();
        assert_eq!(
            calls.borrow().as_slice(),
            [
                SOURCE_CODEX_SESSION_JSONL,
                SOURCE_OPENAI_RESPONSE_USAGE,
                SOURCE_CCSWITCH_PROXY,
            ]
        );
    }

    #[test]
    fn cache_hit_returns_before_adapters_collect() {
        let codex_home = temp_codex_home("ingestion-cache-hit");
        let mut summary = new_usage_summary(&OpenAiUsageQuery::default());
        let result = ingest_usage_with_adapters(
            &[&PanicAdapter],
            &codex_home,
            &mut summary,
            |_source_signature, _summary| Ok(true),
        )
        .unwrap();

        assert!(result.cache_hit);
        let _ = fs::remove_dir_all(&codex_home);
    }
}
