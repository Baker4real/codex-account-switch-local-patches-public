use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::errors::{AppError, AppResult};
use crate::shared::dedicated_proxy::CODEX_SWITCH_PROXY_URL;

pub const OFFICIAL_PRICING_URL: &str = "https://developers.openai.com/api/docs/pricing";
pub const OFFICIAL_MODELS_URL: &str = "https://developers.openai.com/api/docs/models/all";
pub const OFFICIAL_GPT_5_6_SOL_URL: &str =
    "https://developers.openai.com/api/docs/models/gpt-5.6-sol";
pub const STANDARD_SHORT_CONTEXT: &str = "standard_short_context";
const SCHEMA_VERSION: u32 = 1;
const CATALOG_FILENAME: &str = "openai_pricing_catalog.json";
const MAX_MODELS: usize = 512;
const MAX_OFFICIAL_BODY_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RATE_MICROS_PER_MILLION: u64 = 1_000_000_000;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    Updated,
    NotModified,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct UpdateResult {
    pub status: UpdateStatus,
    pub catalog_hash: String,
    pub model_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelRate {
    pub canonical_id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub snapshots: Vec<String>,
    #[serde(default)]
    pub deprecated: bool,
    pub input_micros_per_million: u64,
    pub cached_input_micros_per_million: u64,
    pub output_micros_per_million: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceValidator {
    pub url: String,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    pub body_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PricingCatalog {
    pub schema_version: u32,
    pub fetched_at: String,
    pub published_at: String,
    pub source: String,
    pub source_urls: Vec<String>,
    #[serde(default)]
    pub source_validators: Vec<SourceValidator>,
    pub catalog_hash: String,
    pub models: Vec<ModelRate>,
}

#[derive(Clone, Debug)]
pub struct CatalogSnapshot {
    pub catalog: PricingCatalog,
    pub stale: bool,
    pub bundled_seed: bool,
    pub load_error: Option<String>,
}

impl PricingCatalog {
    pub fn rate_for_model(&self, requested: &str) -> Option<&ModelRate> {
        let requested = normalize_id(requested);
        self.models.iter().find(|rate| {
            normalize_id(&rate.canonical_id) == requested
                || rate
                    .aliases
                    .iter()
                    .any(|value| normalize_id(value) == requested)
                || rate
                    .snapshots
                    .iter()
                    .any(|value| normalize_id(value) == requested)
        })
    }

    pub fn content_hash(&self) -> AppResult<String> {
        let mut normalized = self.clone();
        normalized.catalog_hash.clear();
        normalized.fetched_at.clear();
        normalized.published_at.clear();
        normalized
            .source_validators
            .sort_by(|a, b| a.url.cmp(&b.url));
        normalized
            .models
            .sort_by(|a, b| a.canonical_id.cmp(&b.canonical_id));
        for model in &mut normalized.models {
            model.aliases.sort();
            model.snapshots.sort();
        }
        let bytes = serde_json::to_vec(&normalized).map_err(|error| {
            AppError::new("PRICING_CATALOG_SERIALIZE_FAILED", error.to_string())
        })?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub fn validate(&self) -> AppResult<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(invalid("unsupported schema version"));
        }
        if self.models.is_empty() || self.models.len() > MAX_MODELS {
            return Err(invalid("model count is outside the accepted range"));
        }
        if self.source_urls.is_empty()
            || self.source_urls.iter().any(|url| !is_official_url(url))
            || self.source_validators.iter().any(|item| {
                !is_official_url(&item.url)
                    || item.body_sha256.len() != 64
                    || !item.body_sha256.chars().all(|ch| ch.is_ascii_hexdigit())
            })
        {
            return Err(invalid("catalog contains a non-official or invalid source"));
        }
        if self.source != "bundled_seed"
            && (self.source_validators.len() != self.source_urls.len()
                || self.source_urls.iter().any(|url| {
                    self.source_validators
                        .iter()
                        .filter(|validator| validator.url == *url)
                        .count()
                        != 1
                }))
        {
            return Err(invalid(
                "catalog validators do not cover official sources exactly once",
            ));
        }
        let fetched_at = chrono::DateTime::parse_from_rfc3339(&self.fetched_at)
            .map_err(|_| invalid("fetched_at is not RFC3339"))?;
        if self.source != "bundled_seed"
            && fetched_at > chrono::Utc::now() + chrono::Duration::minutes(5)
        {
            return Err(invalid("fetched_at is unreasonably in the future"));
        }
        let mut identities = BTreeMap::<String, String>::new();
        for model in &self.models {
            let canonical = normalize_id(&model.canonical_id);
            if canonical.is_empty() {
                return Err(invalid("canonical model id is empty"));
            }
            if model.input_micros_per_million == 0
                || model.cached_input_micros_per_million == 0
                || model.output_micros_per_million == 0
                || model.input_micros_per_million > MAX_RATE_MICROS_PER_MILLION
                || model.cached_input_micros_per_million > MAX_RATE_MICROS_PER_MILLION
                || model.output_micros_per_million > MAX_RATE_MICROS_PER_MILLION
                || model.cached_input_micros_per_million > model.input_micros_per_million
            {
                return Err(invalid(&format!(
                    "model prices are outside accepted bounds: {}",
                    model.canonical_id
                )));
            }
            for identity in std::iter::once(&model.canonical_id)
                .chain(model.aliases.iter())
                .chain(model.snapshots.iter())
            {
                let identity = normalize_id(identity);
                if identity.is_empty() {
                    return Err(invalid("model alias or snapshot is empty"));
                }
                if let Some(owner) = identities.insert(identity.clone(), canonical.clone()) {
                    if owner != canonical {
                        return Err(invalid(&format!("model identity conflict: {identity}")));
                    }
                }
            }
        }
        let expected = self.content_hash()?;
        if self.catalog_hash != expected {
            return Err(invalid("catalog content hash does not match"));
        }
        Ok(())
    }
}

pub fn catalog_path(backup_root: &Path) -> PathBuf {
    backup_root.join(CATALOG_FILENAME)
}

pub fn load_snapshot(backup_root: &Path, now_ms: u64) -> CatalogSnapshot {
    let path = catalog_path(backup_root);
    match read_valid_catalog(&path).or_else(|primary_error| {
        let previous = path.with_extension("previous.json");
        read_valid_catalog(&previous)
            .map_err(|previous_error| format!("{primary_error}; previous: {previous_error}"))
    }) {
        Ok(catalog) => CatalogSnapshot {
            stale: catalog_age_ms(&catalog, now_ms).is_none_or(|age| age > 72 * 60 * 60 * 1000),
            bundled_seed: false,
            load_error: None,
            catalog,
        },
        Err(error) => CatalogSnapshot {
            catalog: bundled_seed(),
            stale: true,
            bundled_seed: true,
            load_error: if path.exists() { Some(error) } else { None },
        },
    }
}

fn read_valid_catalog(path: &Path) -> Result<PricingCatalog, String> {
    fs::read(path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| {
            serde_json::from_slice::<PricingCatalog>(&bytes).map_err(|error| error.to_string())
        })
        .and_then(|catalog| {
            catalog
                .validate()
                .map(|_| catalog)
                .map_err(|error| error.to_string())
        })
}

pub fn publish_validated(backup_root: &Path, candidate: &PricingCatalog) -> AppResult<()> {
    candidate.validate()?;
    fs::create_dir_all(backup_root)
        .map_err(|error| AppError::new("PRICING_CATALOG_DIR_FAILED", error.to_string()))?;
    let _publish_lock = PublishLock::acquire(backup_root)?;
    let target = catalog_path(backup_root);
    if let Ok(bytes) = fs::read(&target) {
        if let Ok(lkg) = serde_json::from_slice::<PricingCatalog>(&bytes) {
            if lkg.validate().is_ok() {
                validate_candidate_against_lkg(candidate, &lkg)?;
            }
        }
    }
    let temp = target.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let bytes = serde_json::to_vec_pretty(candidate)
        .map_err(|error| AppError::new("PRICING_CATALOG_SERIALIZE_FAILED", error.to_string()))?;
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|error| AppError::new("PRICING_CATALOG_TEMP_FAILED", error.to_string()))?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        if target.exists() {
            let previous = target.with_extension("previous.json");
            let _ = fs::remove_file(&previous);
            fs::copy(&target, &previous)?;
        }
        atomic_replace(&temp, &target)?;
        #[cfg(not(windows))]
        if let Some(parent) = target.parent() {
            OpenOptions::new().read(true).open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_file(&temp);
        return Err(AppError::new(
            "PRICING_CATALOG_PUBLISH_FAILED",
            error.to_string(),
        ));
    }
    Ok(())
}

struct PublishLock(PathBuf);

impl PublishLock {
    fn acquire(root: &Path) -> AppResult<Self> {
        let path = root.join("openai_pricing_catalog.lock");
        for _ in 0..50 {
            match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(_) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&path)
                        .and_then(|metadata| metadata.modified())
                        .and_then(|modified| modified.elapsed().map_err(std::io::Error::other))
                        .is_ok_and(|age| age > std::time::Duration::from_secs(300));
                    if stale {
                        let _ = fs::remove_file(&path);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => {
                    return Err(AppError::new(
                        "PRICING_CATALOG_LOCK_FAILED",
                        error.to_string(),
                    ))
                }
            }
        }
        Err(AppError::new(
            "PRICING_CATALOG_LOCK_TIMEOUT",
            "pricing catalog publisher is busy",
        ))
    }
}

impl Drop for PublishLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub fn conditional_headers(catalog: &PricingCatalog) -> BTreeMap<String, BTreeMap<String, String>> {
    catalog
        .source_validators
        .iter()
        .map(|source| {
            let mut headers = BTreeMap::new();
            if let Some(value) = &source.etag {
                headers.insert("If-None-Match".into(), value.clone());
            }
            if let Some(value) = &source.last_modified {
                headers.insert("If-Modified-Since".into(), value.clone());
            }
            (source.url.clone(), headers)
        })
        .collect()
}

/// Fetches the official pricing page outside the dashboard read path and atomically publishes it.
/// Any HTTP, parsing, or validation failure leaves the last-known-good catalog untouched.
pub fn update_from_official(backup_root: &Path) -> AppResult<UpdateResult> {
    let existing = load_snapshot(backup_root, current_time_ms()).catalog;
    let validators = conditional_headers(&existing);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .proxy(
            reqwest::Proxy::all(CODEX_SWITCH_PROXY_URL).map_err(|error| {
                AppError::new(
                    "CODEX_SWITCH_PROXY_INVALID",
                    format!("Failed to configure the dedicated Codex Switch proxy: {error}"),
                )
            })?,
        )
        .build()
        .map_err(|error| AppError::new("PRICING_UPDATE_CLIENT_FAILED", error.to_string()))?;
    let urls = [
        OFFICIAL_PRICING_URL,
        OFFICIAL_MODELS_URL,
        OFFICIAL_GPT_5_6_SOL_URL,
    ];
    let mut fetched = Vec::new();
    for url in urls {
        fetched.push(fetch_official_source(&client, url, validators.get(url))?);
    }
    if fetched.iter().all(|source| source.body.is_none()) {
        let mut refreshed = existing;
        refreshed.fetched_at =
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        publish_validated(backup_root, &refreshed)?;
        return Ok(UpdateResult {
            status: UpdateStatus::NotModified,
            catalog_hash: refreshed.catalog_hash,
            model_count: refreshed.models.len(),
        });
    }
    // A mixed 200/304 round cannot be parsed consistently from partial bodies. Re-fetch only
    // the unchanged sources without validators, then publish one all-source snapshot.
    for source in &mut fetched {
        if source.body.is_none() {
            *source = fetch_official_source(&client, &source.validator.url, None)?;
        }
    }
    let candidate = extract_official_catalog_from_sources(&fetched)?;
    publish_validated(backup_root, &candidate)?;
    Ok(UpdateResult {
        status: UpdateStatus::Updated,
        catalog_hash: candidate.catalog_hash,
        model_count: candidate.models.len(),
    })
}

struct FetchedSource {
    body: Option<Vec<u8>>,
    validator: SourceValidator,
}

fn fetch_official_source(
    client: &reqwest::blocking::Client,
    url: &str,
    headers: Option<&BTreeMap<String, String>>,
) -> AppResult<FetchedSource> {
    let mut request = client.get(url);
    if let Some(headers) = headers {
        for (name, value) in headers {
            request = request.header(name, value);
        }
    }
    let response = request
        .send()
        .map_err(|error| AppError::new("PRICING_UPDATE_FETCH_FAILED", error.to_string()))?;
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        let prior = headers.ok_or_else(|| invalid("unexpected 304 without validators"))?;
        return Ok(FetchedSource {
            body: None,
            validator: SourceValidator {
                url: url.into(),
                etag: prior.get("If-None-Match").cloned(),
                last_modified: prior.get("If-Modified-Since").cloned(),
                body_sha256: String::new(),
            },
        });
    }
    if !response.status().is_success() || response.url().as_str() != url {
        return Err(AppError::new(
            "PRICING_UPDATE_HTTP_FAILED",
            format!("official source {url} returned {}", response.status()),
        ));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_OFFICIAL_BODY_BYTES)
    {
        return Err(invalid("official response is too large"));
    }
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let last_modified = response
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let mut body = Vec::new();
    response
        .take(MAX_OFFICIAL_BODY_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|error| AppError::new("PRICING_UPDATE_BODY_FAILED", error.to_string()))?;
    if body.len() as u64 > MAX_OFFICIAL_BODY_BYTES {
        return Err(invalid("official response is too large"));
    }
    Ok(FetchedSource {
        validator: SourceValidator {
            url: url.into(),
            etag,
            last_modified,
            body_sha256: format!("{:x}", Sha256::digest(&body)),
        },
        body: Some(body),
    })
}

fn extract_official_catalog(
    body: &[u8],
    etag: Option<String>,
    last_modified: Option<String>,
) -> AppResult<PricingCatalog> {
    let html =
        std::str::from_utf8(body).map_err(|_| invalid("official pricing response is not UTF-8"))?;
    let standard_start = html
        .find("data-content-switcher-pane=\"true\" data-value=\"standard\"")
        .ok_or_else(|| invalid("official standard pricing pane was not found"))?;
    let after_start = &html[standard_start..];
    let standard_end = after_start
        .find("data-content-switcher-pane=\"true\" data-value=\"batch\"")
        .ok_or_else(|| invalid("official standard pricing pane was not bounded"))?;
    let pane = &after_start[..standard_end];
    let marker = "[1,[[0,&quot;";
    let mut cursor = 0;
    let mut models = BTreeMap::<String, ModelRate>::new();
    while let Some(relative) = pane[cursor..].find(marker) {
        let row_start = cursor + relative + marker.len();
        let Some(relative_end) = pane[row_start..].find("]]]") else {
            break;
        };
        let row = &pane[row_start..row_start + relative_end];
        cursor = row_start + relative_end + 3;
        let cells: Vec<_> = row.split("],[0,").collect();
        if !(cells.len() == 4 || cells.len() == 5) {
            continue;
        }
        let raw_id = cells[0].trim_end_matches("&quot;");
        let canonical_id = raw_id
            .split(" (&lt;")
            .next()
            .unwrap_or(raw_id)
            .trim()
            .to_ascii_lowercase();
        if !(canonical_id.starts_with("gpt-")
            || canonical_id.starts_with("codex-")
            || canonical_id.starts_with('o'))
        {
            continue;
        }
        let parse_price = |value: &str| -> Option<u64> {
            let value = value.trim().trim_matches('&').trim_matches('"');
            let dollars: f64 = value.parse().ok()?;
            if !dollars.is_finite() || dollars < 0.0 {
                return None;
            }
            Some((dollars * 1_000_000.0).round() as u64)
        };
        let Some(input) = parse_price(cells[1]) else {
            continue;
        };
        let Some(cached) = parse_price(cells[2]) else {
            continue;
        };
        let Some(output) = parse_price(cells[cells.len() - 1]) else {
            continue;
        };
        models.entry(canonical_id.clone()).or_insert(ModelRate {
            canonical_id,
            aliases: vec![],
            snapshots: vec![],
            deprecated: false,
            input_micros_per_million: input,
            cached_input_micros_per_million: cached,
            output_micros_per_million: output,
        });
    }
    // These independently listed GPT-5.6 variants are the required canary for the current
    // official page shape. Missing any one means the extractor must fail closed.
    for required in ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"] {
        if !models.contains_key(required) {
            return Err(invalid(&format!(
                "official standard pricing did not contain required model {required}"
            )));
        }
    }
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut catalog = PricingCatalog {
        schema_version: SCHEMA_VERSION,
        fetched_at: now.clone(),
        published_at: now,
        source: "OpenAI API pricing".into(),
        source_urls: vec![OFFICIAL_PRICING_URL.into()],
        source_validators: vec![SourceValidator {
            url: OFFICIAL_PRICING_URL.into(),
            etag,
            last_modified,
            body_sha256: format!("{:x}", Sha256::digest(body)),
        }],
        catalog_hash: String::new(),
        models: models.into_values().collect(),
    };
    catalog.catalog_hash = catalog.content_hash()?;
    catalog.validate()?;
    Ok(catalog)
}

fn extract_official_catalog_from_sources(sources: &[FetchedSource]) -> AppResult<PricingCatalog> {
    let find = |url: &str| {
        sources
            .iter()
            .find(|source| source.validator.url == url)
            .and_then(|source| source.body.as_deref())
            .ok_or_else(|| invalid(&format!("complete official source body is missing: {url}")))
    };
    let pricing_body = find(OFFICIAL_PRICING_URL)?;
    let models_body = find(OFFICIAL_MODELS_URL)?;
    let sol_body = find(OFFICIAL_GPT_5_6_SOL_URL)?;
    let models_html = std::str::from_utf8(models_body)
        .map_err(|_| invalid("official models response is not UTF-8"))?;
    let sol_html = std::str::from_utf8(sol_body)
        .map_err(|_| invalid("official GPT-5.6 Sol response is not UTF-8"))?;
    if !models_html.contains("gpt-5.6-sol") {
        return Err(invalid(
            "official models directory did not contain gpt-5.6-sol",
        ));
    }
    let alias_is_explicit = [
        ">&quot;gpt-5.6&lt;",
        "&quot;gpt-5.6&quot;",
        ">gpt-5.6<",
        "\"gpt-5.6\"",
    ]
    .iter()
    .any(|marker| sol_html.contains(marker));
    if !sol_html.contains("gpt-5.6-sol") || !alias_is_explicit {
        return Err(invalid(
            "official GPT-5.6 Sol detail did not explicitly declare alias gpt-5.6",
        ));
    }
    let pricing = sources
        .iter()
        .find(|source| source.validator.url == OFFICIAL_PRICING_URL)
        .ok_or_else(|| invalid("official pricing validator is missing"))?;
    let mut catalog = extract_official_catalog(
        pricing_body,
        pricing.validator.etag.clone(),
        pricing.validator.last_modified.clone(),
    )?;
    catalog
        .models
        .iter_mut()
        .find(|model| model.canonical_id == "gpt-5.6-sol")
        .expect("required model was checked by pricing extractor")
        .aliases
        .push("gpt-5.6".into());
    catalog.source_urls = sources
        .iter()
        .map(|source| source.validator.url.clone())
        .collect();
    catalog.source_validators = sources
        .iter()
        .map(|source| source.validator.clone())
        .collect();
    catalog.catalog_hash = catalog.content_hash()?;
    catalog.validate()?;
    Ok(catalog)
}

pub fn validate_candidate_against_lkg(
    candidate: &PricingCatalog,
    lkg: &PricingCatalog,
) -> AppResult<()> {
    candidate.validate()?;
    let old: BTreeSet<_> = lkg
        .models
        .iter()
        .map(|m| normalize_id(&m.canonical_id))
        .collect();
    let new: BTreeSet<_> = candidate
        .models
        .iter()
        .map(|m| normalize_id(&m.canonical_id))
        .collect();
    if old.len() >= 4 && new.len() * 2 < old.len() {
        return Err(invalid(
            "candidate unexpectedly removed more than half of the catalog",
        ));
    }
    for prior in &lkg.models {
        let Some(next) = candidate.rate_for_model(&prior.canonical_id) else {
            continue;
        };
        for (label, old_price, new_price) in [
            (
                "input",
                prior.input_micros_per_million,
                next.input_micros_per_million,
            ),
            (
                "cached input",
                prior.cached_input_micros_per_million,
                next.cached_input_micros_per_million,
            ),
            (
                "output",
                prior.output_micros_per_million,
                next.output_micros_per_million,
            ),
        ] {
            if old_price > 0
                && (new_price > old_price.saturating_mul(4)
                    || new_price.saturating_mul(4) < old_price)
            {
                return Err(invalid(&format!(
                    "{label} price changed by more than 4x for {}",
                    prior.canonical_id
                )));
            }
        }
    }
    Ok(())
}

pub fn bundled_seed() -> PricingCatalog {
    let mut catalog = PricingCatalog {
        schema_version: SCHEMA_VERSION,
        fetched_at: "2026-07-15T00:00:00Z".into(),
        published_at: "2026-07-15T00:00:00Z".into(),
        source: "bundled_seed".into(),
        source_urls: vec![OFFICIAL_PRICING_URL.into()],
        source_validators: vec![],
        catalog_hash: String::new(),
        models: vec![
            rate_with_aliases("gpt-5.6-sol", &["gpt-5.6"], 5_000_000, 500_000, 30_000_000),
            rate("gpt-5.6-terra", 2_500_000, 250_000, 15_000_000),
            rate("gpt-5.6-luna", 1_000_000, 100_000, 6_000_000),
            rate("gpt-5.5", 5_000_000, 500_000, 30_000_000),
            rate_with_snapshots(
                "gpt-5.4",
                &["gpt-5.4-2026-03-05"],
                2_500_000,
                250_000,
                15_000_000,
            ),
            rate("gpt-5.4-mini", 750_000, 75_000, 4_500_000),
            rate("gpt-5.4-nano", 200_000, 20_000, 1_250_000),
            rate("gpt-5.3-codex", 1_750_000, 175_000, 14_000_000),
            rate("gpt-5.2", 1_750_000, 175_000, 14_000_000),
            rate("gpt-5.2-codex", 1_750_000, 175_000, 14_000_000),
            rate("gpt-5.1", 1_250_000, 125_000, 10_000_000),
            rate("gpt-5.1-codex", 1_250_000, 125_000, 10_000_000),
            rate("gpt-5.1-codex-max", 1_250_000, 125_000, 10_000_000),
            rate("gpt-5", 1_250_000, 125_000, 10_000_000),
            rate("gpt-5-codex", 1_250_000, 125_000, 10_000_000),
        ],
    };
    catalog.catalog_hash = catalog.content_hash().expect("seed catalog serializes");
    catalog
}

fn rate(id: &str, input: u64, cached: u64, output: u64) -> ModelRate {
    ModelRate {
        canonical_id: id.into(),
        aliases: vec![],
        snapshots: vec![],
        deprecated: false,
        input_micros_per_million: input,
        cached_input_micros_per_million: cached,
        output_micros_per_million: output,
    }
}

fn rate_with_aliases(
    id: &str,
    aliases: &[&str],
    input: u64,
    cached: u64,
    output: u64,
) -> ModelRate {
    let mut model = rate(id, input, cached, output);
    model.aliases = aliases.iter().map(|value| (*value).to_string()).collect();
    model
}

fn rate_with_snapshots(
    id: &str,
    snapshots: &[&str],
    input: u64,
    cached: u64,
    output: u64,
) -> ModelRate {
    let mut model = rate(id, input, cached, output);
    model.snapshots = snapshots.iter().map(|value| (*value).to_string()).collect();
    model
}

fn normalize_id(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace('@', "-")
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string()
}

fn is_official_url(url: &str) -> bool {
    url.starts_with("https://developers.openai.com/") && !url.contains('@')
}

fn catalog_age_ms(catalog: &PricingCatalog, now_ms: u64) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(&catalog.fetched_at)
        .ok()
        .and_then(|value| {
            let published = u64::try_from(value.timestamp_millis()).ok()?;
            Some(now_ms.saturating_sub(published))
        })
}

fn current_time_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or_default()
}

#[cfg(windows)]
fn atomic_replace(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(source, target)
}

fn invalid(message: &str) -> AppError {
    AppError::new("PRICING_CATALOG_INVALID", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "codex-switch-pricing-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn seed_is_valid_and_unknown_models_are_not_guessed() {
        let seed = bundled_seed();
        seed.validate().unwrap();
        assert!(seed.rate_for_model("gpt-5.4").is_some());
        assert!(seed.rate_for_model("gpt-5.4-invented").is_none());
    }

    #[test]
    fn explicit_snapshot_resolves_without_prefix_guessing() {
        let mut catalog = bundled_seed();
        catalog
            .models
            .iter_mut()
            .find(|model| model.canonical_id == "gpt-5.4")
            .unwrap()
            .snapshots
            .push("gpt-5.4-2026-03-05".into());
        catalog.catalog_hash = catalog.content_hash().unwrap();
        assert_eq!(
            catalog
                .rate_for_model("gpt-5.4-2026-03-05")
                .unwrap()
                .canonical_id,
            "gpt-5.4"
        );
    }

    #[test]
    fn conflicting_alias_rejects_whole_candidate() {
        let mut catalog = bundled_seed();
        catalog.models[0].aliases.push("gpt-5.3-codex".into());
        catalog.catalog_hash = catalog.content_hash().unwrap();
        assert_eq!(
            catalog.validate().unwrap_err().error_code,
            "PRICING_CATALOG_INVALID"
        );
    }

    #[test]
    fn invalid_candidate_never_replaces_lkg() {
        let root = temp_root("atomic");
        let good = bundled_seed();
        publish_validated(&root, &good).unwrap();
        let mut bad = good.clone();
        bad.catalog_hash = "broken".into();
        assert!(publish_validated(&root, &bad).is_err());
        assert_eq!(
            load_snapshot(&root, u64::MAX / 2).catalog.catalog_hash,
            good.catalog_hash
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_active_catalog_recovers_previous_lkg() {
        let root = temp_root("recover-previous");
        let good = bundled_seed();
        fs::write(
            catalog_path(&root).with_extension("previous.json"),
            serde_json::to_vec(&good).unwrap(),
        )
        .unwrap();
        fs::write(catalog_path(&root), b"broken").unwrap();
        let recovered = load_snapshot(&root, current_time_ms());
        assert_eq!(recovered.catalog.catalog_hash, good.catalog_hash);
        assert!(!recovered.bundled_seed);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validators_create_conditional_request_headers() {
        let mut catalog = bundled_seed();
        catalog.source_validators.push(SourceValidator {
            url: OFFICIAL_PRICING_URL.into(),
            etag: Some("etag".into()),
            last_modified: Some("date".into()),
            body_sha256: "a".repeat(64),
        });
        let headers = conditional_headers(&catalog);
        assert_eq!(headers[OFFICIAL_PRICING_URL]["If-None-Match"], "etag");
        assert_eq!(headers[OFFICIAL_PRICING_URL]["If-Modified-Since"], "date");
    }

    #[test]
    fn official_standard_fixture_extracts_exact_gpt_5_6_prices_and_ignores_cache_write() {
        let body = br#"<div data-content-switcher-pane="true" data-value="standard"><astro-island props="{&quot;rows&quot;:[1,[[1,[[0,&quot;gpt-5.6-sol&quot;],[0,5],[0,0.5],[0,6.25],[0,30]]],[1,[[0,&quot;gpt-5.6-terra&quot;],[0,2.5],[0,0.25],[0,3.125],[0,15]]],[1,[[0,&quot;gpt-5.6-luna&quot;],[0,1],[0,0.1],[0,1.25],[0,6]]],[1,[[0,&quot;gpt-5.3-codex&quot;],[0,1.75],[0,0.175],[0,14]]]]]}"/></div><div data-content-switcher-pane="true" data-value="batch"></div>"#;
        let catalog =
            extract_official_catalog(body, Some("tag".into()), Some("date".into())).unwrap();
        let sol = catalog.rate_for_model("gpt-5.6-sol").unwrap();
        assert_eq!(
            (
                sol.input_micros_per_million,
                sol.cached_input_micros_per_million,
                sol.output_micros_per_million
            ),
            (5_000_000, 500_000, 30_000_000)
        );
        assert_eq!(catalog.source_validators[0].etag.as_deref(), Some("tag"));
        assert_eq!(catalog.source_validators[0].body_sha256.len(), 64);
    }

    #[test]
    fn changed_official_shape_fails_closed_without_partial_catalog() {
        let body = br#"<div data-content-switcher-pane="true" data-value="standard"></div><div data-content-switcher-pane="true" data-value="batch"></div>"#;
        assert_eq!(
            extract_official_catalog(body, None, None)
                .unwrap_err()
                .error_code,
            "PRICING_CATALOG_INVALID"
        );
    }

    #[test]
    fn abnormal_existing_model_price_jump_is_rejected() {
        let old = bundled_seed();
        let mut candidate = old.clone();
        candidate.models[0].input_micros_per_million *= 5;
        candidate.catalog_hash = candidate.content_hash().unwrap();
        assert!(validate_candidate_against_lkg(&candidate, &old).is_err());
    }

    #[test]
    fn zero_extreme_and_inverted_prices_are_rejected_for_new_models_too() {
        for mutate in 0..3 {
            let mut candidate = bundled_seed();
            let mut added = rate("brand-new-model", 1_000_000, 100_000, 2_000_000);
            match mutate {
                0 => added.input_micros_per_million = 0,
                1 => added.output_micros_per_million = MAX_RATE_MICROS_PER_MILLION + 1,
                _ => added.cached_input_micros_per_million = added.input_micros_per_million + 1,
            }
            candidate.models.push(added);
            candidate.catalog_hash = candidate.content_hash().unwrap();
            assert!(candidate.validate().is_err());
        }
    }

    #[test]
    fn multi_source_catalog_requires_explicit_sol_alias_and_records_every_validator() {
        let pricing = br#"<div data-content-switcher-pane="true" data-value="standard"><astro-island props="{&quot;rows&quot;:[1,[[1,[[0,&quot;gpt-5.6-sol&quot;],[0,5],[0,0.5],[0,6.25],[0,30]]],[1,[[0,&quot;gpt-5.6-terra&quot;],[0,2.5],[0,0.25],[0,3.125],[0,15]]],[1,[[0,&quot;gpt-5.6-luna&quot;],[0,1],[0,0.1],[0,1.25],[0,6]]]]]}"/></div><div data-content-switcher-pane="true" data-value="batch"></div>"#.to_vec();
        let make = |url: &str, body: Vec<u8>| FetchedSource {
            validator: SourceValidator {
                url: url.into(),
                etag: None,
                last_modified: None,
                body_sha256: format!("{:x}", Sha256::digest(&body)),
            },
            body: Some(body),
        };
        let sources = vec![
            make(OFFICIAL_PRICING_URL, pricing),
            make(OFFICIAL_MODELS_URL, b"gpt-5.6-sol".to_vec()),
            make(
                OFFICIAL_GPT_5_6_SOL_URL,
                b"gpt-5.6-sol alias: &quot;gpt-5.6&quot;".to_vec(),
            ),
        ];
        let catalog = extract_official_catalog_from_sources(&sources).unwrap();
        assert_eq!(catalog.source_validators.len(), 3);
        assert_eq!(
            catalog.rate_for_model("gpt-5.6").unwrap().canonical_id,
            "gpt-5.6-sol"
        );

        let mut missing_alias = sources;
        missing_alias[2] = make(OFFICIAL_GPT_5_6_SOL_URL, b"gpt-5.6-sol".to_vec());
        assert!(extract_official_catalog_from_sources(&missing_alias).is_err());
    }

    #[test]
    #[ignore = "requires live OpenAI network access"]
    fn live_official_page_matches_fail_closed_extractor() {
        let body = reqwest::blocking::get(OFFICIAL_PRICING_URL)
            .unwrap()
            .error_for_status()
            .unwrap()
            .bytes()
            .unwrap();
        let catalog = extract_official_catalog(&body, None, None).unwrap();
        assert!(catalog.models.len() >= 10);
        assert_eq!(
            catalog
                .rate_for_model("gpt-5.6-sol")
                .unwrap()
                .output_micros_per_million,
            30_000_000
        );
    }
}
