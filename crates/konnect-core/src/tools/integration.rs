//! `integration` toolset — JLCPCB parts database, datasheet enrichment, and Freerouting autorouter.
//!
//! JLCPCB tools query a local SQLite cache of the JLCPCB parts database.
//! Freerouting wraps the Freerouting JAR via subprocess.
//! Datasheet enrichment uses the LCSC HTTP API.
//!
//! The three network calls (JLCPCB database download, LCSC datasheet lookups)
//! go through `get_with_backoff`, which retries transient failures (network
//! errors, 429, 5xx) with exponential backoff before giving up.
//!
//! The three JLCPCB query tools (`search_jlcpcb_parts`, `get_jlcpcb_part`,
//! `suggest_jlcpcb_alternatives`) cache results in `ToolContext::jlcpcb_cache`
//! (5-minute TTL) to avoid re-running an identical SQLite query for repeated
//! lookups within a session. Responses carry a `"cached"` field so callers
//! can see whether a given result came from cache.

use crate::mcp::protocol::CallToolResult;
use crate::tool;
use crate::tools::{get_path, require_str, ToolContext, ToolDef};
use anyhow::{bail, Context};
use serde_json::json;
use std::io::Write;
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

// ─── Tool definitions ─────────────────────────────────────────────────────────

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "download_jlcpcb_database",
            "Download or update the local JLCPCB component parts database cache (SQLite).",
            json!({
                "type": "object",
                "properties": {
                    "output_path": { "type": "string", "description": "Local path to store the SQLite database file (optional, uses config default)" },
                    "force": { "type": "boolean", "description": "Force re-download even if cache exists", "default": false },
                    "catalog": {
                        "type": "string",
                        "enum": ["current", "basic", "all"],
                        "description": "Catalog size: current excludes long-obsolete parts (recommended), basic contains Basic + Preferred parts, all contains the full historical catalog",
                        "default": "current"
                    }
                },
                "required": []
            }),
            |args, ctx| async move { handle_download_jlcpcb(args, ctx).await }
        ),
        tool!(
            "search_jlcpcb_parts",
            "Search the local JLCPCB component database by keyword, value, or category.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search string (MPN, description, or value)" },
                    "category": { "type": "string", "description": "Component category filter (optional)" },
                    "basic_only": { "type": "boolean", "description": "Restrict to JLCPCB Basic Library parts only", "default": false },
                    "in_stock": { "type": "boolean", "description": "Only return parts currently in stock", "default": true },
                    "limit": { "type": "integer", "description": "Maximum number of results", "default": 20 }
                },
                "required": ["query"]
            }),
            |args, ctx| async move { handle_search_jlcpcb_parts(args, ctx).await }
        ),
        tool!(
            "get_jlcpcb_part",
            "Retrieve full details for a single JLCPCB part by its LCSC part number.",
            json!({
                "type": "object",
                "properties": {
                    "lcsc_id": { "type": "string", "description": "LCSC part number (e.g. 'C14663')" }
                },
                "required": ["lcsc_id"]
            }),
            |args, ctx| async move { handle_get_jlcpcb_part(args, ctx).await }
        ),
        tool!(
            "suggest_jlcpcb_alternatives",
            "Suggest JLCPCB-stocked alternative parts for a given component value and footprint.",
            json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string", "description": "Component value (e.g. '100nF')" },
                    "footprint": { "type": "string", "description": "KiCAD footprint identifier" },
                    "max_price_usd": { "type": "number", "description": "Maximum unit price in USD (optional)" },
                    "limit": { "type": "integer", "description": "Maximum number of suggestions", "default": 5 }
                },
                "required": ["value", "footprint"]
            }),
            |args, ctx| async move { handle_suggest_alternatives(args, ctx).await }
        ),
        tool!(
            "get_jlcpcb_database_stats",
            "Return statistics about the local JLCPCB database cache: part count, last updated, file size.",
            json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            |args, ctx| async move { handle_jlcpcb_stats(args, ctx).await }
        ),
        tool!(
            "enrich_datasheets",
            "Fetch and cache datasheet URLs for all components in a schematic using the LCSC API.",
            json!({
                "type": "object",
                "properties": {
                    "schematic": { "type": "string", "description": "Path to .kicad_sch file" },
                    "overwrite_existing": { "type": "boolean", "description": "Replace existing Datasheet fields", "default": false }
                },
                "required": ["schematic"]
            }),
            |args, ctx| async move { handle_enrich_datasheets(args, ctx).await }
        ),
        tool!(
            "get_datasheet_url",
            "Retrieve the datasheet URL for a component by MPN or LCSC ID.",
            json!({
                "type": "object",
                "properties": {
                    "mpn": { "type": "string", "description": "Manufacturer part number (optional)" },
                    "lcsc_id": { "type": "string", "description": "LCSC part number (optional)" }
                },
                "required": []
            }),
            |args, ctx| async move { handle_get_datasheet_url(args, ctx).await }
        ),
        tool!(
            "autoroute",
            "Run Freerouting autorouter on the PCB: export DSN → autoroute → import SES result.",
            json!({
                "type": "object",
                "properties": {
                    "board": { "type": "string", "description": "Path to .kicad_pcb file" },
                    "passes": { "type": "integer", "description": "Number of autorouter passes", "default": 3 },
                    "timeout_seconds": { "type": "integer", "description": "Maximum autorouter runtime in seconds", "default": 120 },
                    "jar_path": { "type": "string", "description": "Path to freerouting.jar (optional, uses config default)" }
                },
                "required": ["board"]
            }),
            |args, ctx| async move { handle_autoroute(args, ctx).await }
        ),
        tool!(
            "check_freerouting",
            "Verify that the Freerouting JAR is available and return its version.",
            json!({
                "type": "object",
                "properties": {
                    "jar_path": { "type": "string", "description": "Path to freerouting.jar (optional, uses config default)" }
                },
                "required": []
            }),
            |args, ctx| async move { handle_check_freerouting(args, ctx).await }
        ),
    ]
}

// ─── JLCPCB database path helper ─────────────────────────────────────────────

fn default_jlcpcb_db_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_default();
        PathBuf::from(appdata).join("konnect").join("jlcpcb.db")
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(".konnect").join("jlcpcb.db")
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(dirs::data_local_dir)
            .unwrap_or_else(|| {
                dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".local/share")
            })
            .join("konnect")
            .join("jlcpcb.db")
    }
}

fn resolve_db_path(args: &serde_json::Value, ctx: &ToolContext) -> PathBuf {
    if let Some(p) = args["output_path"].as_str() {
        return PathBuf::from(p);
    }
    if let Some(p) = &ctx.config.jlcpcb_db_path {
        return p.clone();
    }
    default_jlcpcb_db_path()
}

const JLCPCB_CATALOG_BASE_URL: &str = "https://bouni.github.io/kicad-jlcpcb-tools";
const MAX_CATALOG_CHUNKS: usize = 100;

#[derive(Clone, Copy, Debug)]
struct CatalogSpec {
    name: &'static str,
    database_filename: &'static str,
    chunk_count_filename: &'static str,
}

fn catalog_spec(name: &str) -> Option<CatalogSpec> {
    match name {
        "current" => Some(CatalogSpec {
            name: "current",
            database_filename: "current-parts-fts5.db",
            chunk_count_filename: "chunk_num_current_parts_fts5.txt",
        }),
        "basic" => Some(CatalogSpec {
            name: "basic",
            database_filename: "basic-parts-fts5.db",
            chunk_count_filename: "chunk_num_basic_parts_fts5.txt",
        }),
        "all" => Some(CatalogSpec {
            name: "all",
            database_filename: "parts-fts5.db",
            chunk_count_filename: "chunk_num_fts5.txt",
        }),
        _ => None,
    }
}

#[derive(Debug)]
struct CatalogConversionStats {
    part_count: u64,
    basic_count: u64,
    source_date: String,
    source_last_updated: String,
}

type CatalogMetadata = (String, String, String);

// ─── Retry/backoff for external HTTP calls ────────────────────────────────────
//
// JLCPCB database download and LCSC datasheet lookups are the only genuinely
// networked calls in this toolset (everything else queries the local SQLite
// cache). Both are prone to transient failures — timeouts, connection resets,
// rate limiting — that a simple retry clears up without any user action.

/// Retry policy: 3 attempts total, exponential backoff starting at 300ms
/// (300ms, then 600ms between attempts).
const RETRY_MAX_ATTEMPTS: u32 = 3;
const RETRY_BASE_DELAY: std::time::Duration = std::time::Duration::from_millis(300);

/// Whether an HTTP status is worth retrying. 429 (rate limited) and 5xx
/// (server-side) are transient; other 4xx (404, 401, ...) are not — retrying
/// a "not found" or "unauthorized" wastes time and won't change the outcome.
fn is_transient_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// Delay before the next attempt, given the attempt number just made (1-based).
fn backoff_delay(attempt: u32) -> std::time::Duration {
    RETRY_BASE_DELAY * 2u32.pow(attempt.saturating_sub(1))
}

/// GET `url` with retry/backoff for transient failures (network-level errors,
/// 429, and 5xx). Returns the last response/error once attempts are exhausted.
async fn get_with_backoff(
    client: &reqwest::Client,
    url: &str,
) -> anyhow::Result<reqwest::Response> {
    let mut attempt = 0u32;
    loop {
        attempt += 1;
        match client.get(url).send().await {
            Ok(resp) => {
                let status = resp.status();
                if !is_transient_status(status) || attempt >= RETRY_MAX_ATTEMPTS {
                    return Ok(resp);
                }
                tracing::warn!(
                    "[BETA] {} returned {} (attempt {}/{}), retrying",
                    url,
                    status,
                    attempt,
                    RETRY_MAX_ATTEMPTS
                );
            }
            Err(e) => {
                if attempt >= RETRY_MAX_ATTEMPTS {
                    return Err(e.into());
                }
                tracing::warn!(
                    "[BETA] request to {} failed (attempt {}/{}): {}, retrying",
                    url,
                    attempt,
                    RETRY_MAX_ATTEMPTS,
                    e
                );
            }
        }
        tokio::time::sleep(backoff_delay(attempt)).await;
    }
}

async fn download_catalog_archive(
    client: &reqwest::Client,
    base_url: &str,
    spec: CatalogSpec,
    archive_path: &std::path::Path,
) -> anyhow::Result<(usize, u64)> {
    let base_url = base_url.trim_end_matches('/');
    let count_url = format!("{base_url}/{}", spec.chunk_count_filename);
    let count_response = get_with_backoff(client, &count_url).await?;
    if !count_response.status().is_success() {
        bail!(
            "catalog chunk manifest download failed: HTTP {} ({count_url})",
            count_response.status()
        );
    }
    let chunk_count: usize = count_response
        .text()
        .await?
        .trim()
        .parse()
        .context("catalog chunk manifest did not contain an integer")?;
    if !(1..=MAX_CATALOG_CHUNKS).contains(&chunk_count) {
        bail!("catalog chunk count {chunk_count} is outside the supported range");
    }

    let mut archive = tokio::fs::File::create(archive_path).await?;
    let mut downloaded_bytes = 0u64;
    for index in 1..=chunk_count {
        let chunk_filename = format!("{}.zip.{index:03}", spec.database_filename);
        let chunk_url = format!("{base_url}/{chunk_filename}");
        let mut response = get_with_backoff(client, &chunk_url).await?;
        if !response.status().is_success() {
            bail!(
                "catalog chunk {index}/{chunk_count} download failed: HTTP {} ({chunk_url})",
                response.status()
            );
        }
        while let Some(bytes) = response.chunk().await? {
            archive.write_all(&bytes).await?;
            downloaded_bytes += bytes.len() as u64;
        }
    }
    archive.flush().await?;
    archive.sync_all().await?;
    Ok((chunk_count, downloaded_bytes))
}

fn extract_catalog_database(
    archive_path: &std::path::Path,
    database_filename: &str,
    extracted_path: &std::path::Path,
) -> anyhow::Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).context("catalog download is not a valid ZIP")?;
    let mut database = archive
        .by_name(database_filename)
        .with_context(|| format!("catalog ZIP does not contain {database_filename}"))?;
    if database.is_dir() {
        bail!("catalog ZIP entry {database_filename} is a directory");
    }
    let mut output = std::fs::File::create(extracted_path)?;
    std::io::copy(&mut database, &mut output)?;
    output.flush()?;
    output.sync_all()?;
    Ok(())
}

fn sqlite_value_as_string(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<String> {
    use rusqlite::types::ValueRef;
    Ok(match row.get_ref(index)? {
        ValueRef::Null => String::new(),
        ValueRef::Integer(value) => value.to_string(),
        ValueRef::Real(value) => value.to_string(),
        ValueRef::Text(value) => String::from_utf8_lossy(value).into_owned(),
        ValueRef::Blob(_) => String::new(),
    })
}

fn first_tier_price(raw: &str) -> f64 {
    raw.split(',')
        .next()
        .and_then(|tier| tier.split_once(':').map(|(_, price)| price).or(Some(tier)))
        .and_then(|price| price.trim().parse::<f64>().ok())
        .filter(|price| price.is_finite() && *price >= 0.0)
        .unwrap_or(0.0)
}

fn contains_numeric_token(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let haystack = haystack.to_ascii_lowercase();
    let needle = needle.to_ascii_lowercase();
    haystack.match_indices(&needle).any(|(start, matched)| {
        let before = haystack[..start].chars().next_back();
        let after = haystack[start + matched.len()..].chars().next();
        !before.is_some_and(|character| character.is_ascii_digit() || character == '.')
            && !after.is_some_and(|character| character.is_ascii_digit() || character == '.')
    })
}

fn convert_catalog_database(
    source_path: &std::path::Path,
    target_path: &std::path::Path,
    spec: CatalogSpec,
    source_url: &str,
) -> anyhow::Result<CatalogConversionStats> {
    let source = rusqlite::Connection::open_with_flags(
        source_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let (source_part_count, source_date, source_last_updated) = source
        .query_row(
            "SELECT partcount, date, last_update FROM meta LIMIT 1",
            [],
            |row| {
                Ok((
                    sqlite_value_as_string(row, 0)?,
                    sqlite_value_as_string(row, 1)?,
                    sqlite_value_as_string(row, 2)?,
                ))
            },
        )
        .context("catalog metadata is missing or incompatible")?;

    let mut target = rusqlite::Connection::open(target_path)?;
    target.execute_batch(
        "PRAGMA journal_mode = OFF;
         PRAGMA synchronous = OFF;
         CREATE TABLE components (
             LCSC TEXT PRIMARY KEY,
             Category TEXT NOT NULL,
             MFR_Part TEXT NOT NULL,
             Package TEXT NOT NULL,
             Solder_Joint INTEGER NOT NULL,
             Manufacturer TEXT NOT NULL,
             Library_Type TEXT NOT NULL,
             Description TEXT NOT NULL,
             Datasheet TEXT NOT NULL,
             Price REAL NOT NULL,
             Stock INTEGER NOT NULL
         );
         CREATE TABLE catalog_metadata (
             catalog TEXT NOT NULL,
             source_url TEXT NOT NULL,
             source_date TEXT NOT NULL,
             source_last_updated TEXT NOT NULL,
             source_part_count INTEGER NOT NULL
         );",
    )?;

    let mut select = source.prepare(
        "SELECT \"LCSC Part\", \"First Category\", \"Second Category\", \
                \"MFR.Part\", Package, \"Solder Joint\", Manufacturer, \
                \"Library Type\", Description, Datasheet, Price, Stock \
         FROM parts",
    )?;
    let mut rows = select.query([])?;
    let transaction = target.transaction()?;
    let mut insert = transaction.prepare(
        "INSERT INTO components (
             LCSC, Category, MFR_Part, Package, Solder_Joint, Manufacturer,
             Library_Type, Description, Datasheet, Price, Stock
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
    )?;
    let mut part_count = 0u64;
    let mut basic_count = 0u64;
    while let Some(row) = rows.next()? {
        let lcsc = sqlite_value_as_string(row, 0)?;
        let first_category = sqlite_value_as_string(row, 1)?;
        let second_category = sqlite_value_as_string(row, 2)?;
        let mfr_part = sqlite_value_as_string(row, 3)?;
        let package = sqlite_value_as_string(row, 4)?;
        let solder_joint = sqlite_value_as_string(row, 5)?.parse::<i64>().unwrap_or(0);
        let manufacturer = sqlite_value_as_string(row, 6)?;
        let library_type = sqlite_value_as_string(row, 7)?;
        let description = sqlite_value_as_string(row, 8)?;
        let datasheet = sqlite_value_as_string(row, 9)?;
        let price = first_tier_price(&sqlite_value_as_string(row, 10)?);
        let stock = sqlite_value_as_string(row, 11)?.parse::<i64>().unwrap_or(0);
        let category = match (first_category.is_empty(), second_category.is_empty()) {
            (false, false) => format!("{first_category} / {second_category}"),
            (false, true) => first_category,
            (true, false) => second_category,
            (true, true) => String::new(),
        };
        insert.execute(rusqlite::params![
            lcsc,
            category,
            mfr_part,
            package,
            solder_joint,
            manufacturer,
            library_type,
            description,
            datasheet,
            price,
            stock,
        ])?;
        part_count += 1;
        if library_type == "Basic" {
            basic_count += 1;
        }
    }
    drop(insert);
    transaction.commit()?;

    target.execute_batch(
        "CREATE INDEX idx_components_library_stock ON components(Library_Type, Stock);",
    )?;
    target.execute(
        "INSERT INTO catalog_metadata VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            spec.name,
            source_url,
            source_date,
            source_last_updated,
            source_part_count
                .parse::<i64>()
                .unwrap_or(part_count as i64),
        ],
    )?;
    target.execute_batch(
        "PRAGMA optimize;
         PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = NORMAL;",
    )?;

    Ok(CatalogConversionStats {
        part_count,
        basic_count,
        source_date,
        source_last_updated,
    })
}

fn replace_database_atomically(
    replacement: &std::path::Path,
    destination: &std::path::Path,
    backup: &std::path::Path,
) -> anyhow::Result<()> {
    if destination.exists() {
        std::fs::rename(destination, backup).with_context(|| {
            format!(
                "could not move existing database {} aside",
                destination.display()
            )
        })?;
    }
    if let Err(error) = std::fs::rename(replacement, destination) {
        if backup.exists() {
            let _ = std::fs::rename(backup, destination);
        }
        return Err(error)
            .with_context(|| format!("could not install database at {}", destination.display()));
    }
    if backup.exists() {
        std::fs::remove_file(backup)?;
    }
    Ok(())
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

async fn handle_download_jlcpcb(
    args: &serde_json::Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let db_path = resolve_db_path(args, ctx);
    let force = args["force"].as_bool().unwrap_or(false);
    let catalog_name = args["catalog"].as_str().unwrap_or("current");
    let Some(spec) = catalog_spec(catalog_name) else {
        return Ok(CallToolResult::error(format!(
            "Unknown JLCPCB catalog '{catalog_name}'; expected current, basic, or all"
        )));
    };

    if db_path.exists() && !force {
        let meta = tokio::fs::metadata(&db_path).await?;
        return Ok(CallToolResult::text(
            serde_json::to_string_pretty(&json!({
                "status": "already_exists",
                "path": db_path.to_str().unwrap_or(""),
                "size_bytes": meta.len(),
                "note": "Use force=true to re-download"
            }))
            .unwrap(),
        ));
    }

    let parent = db_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let workspace = tempfile::Builder::new()
        .prefix(".konnect-jlcpcb-")
        .tempdir_in(parent)?;
    let archive_path = workspace.path().join("catalog.zip");
    let extracted_path = workspace.path().join(spec.database_filename);
    let converted_path = workspace.path().join("converted.db");
    let backup_path = workspace.path().join("previous.db");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;
    let (chunk_count, downloaded_bytes) =
        download_catalog_archive(&client, JLCPCB_CATALOG_BASE_URL, spec, &archive_path).await?;
    let source_url = format!("{JLCPCB_CATALOG_BASE_URL}/{}", spec.chunk_count_filename);
    let conversion = tokio::task::spawn_blocking({
        let archive_path = archive_path.clone();
        let extracted_path = extracted_path.clone();
        let converted_path = converted_path.clone();
        move || -> anyhow::Result<CatalogConversionStats> {
            extract_catalog_database(&archive_path, spec.database_filename, &extracted_path)?;
            convert_catalog_database(&extracted_path, &converted_path, spec, &source_url)
        }
    })
    .await??;
    replace_database_atomically(&converted_path, &db_path, &backup_path)?;
    ctx.jlcpcb_cache.clear();
    let installed_size = tokio::fs::metadata(&db_path).await?.len();

    Ok(CallToolResult::text(
        serde_json::to_string_pretty(&json!({
            "success": true,
            "catalog": spec.name,
            "path": db_path.to_str().unwrap_or(""),
            "part_count": conversion.part_count,
            "basic_part_count": conversion.basic_count,
            "catalog_date": conversion.source_date,
            "catalog_last_updated": conversion.source_last_updated,
            "chunk_count": chunk_count,
            "download_size_bytes": downloaded_bytes,
            "size_bytes": installed_size
        }))
        .unwrap(),
    ))
}

/// Build a deterministic cache key from a tool name, the resolved DB path
/// (so pointing at a different `output_path` never serves stale results),
/// and the query parameters that affect the result set.
fn cache_key(tool: &str, db_path: &std::path::Path, parts: &[&str]) -> String {
    format!("{}|{}|{}", tool, db_path.display(), parts.join("|"))
}

async fn handle_search_jlcpcb_parts(
    args: &serde_json::Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let db_path = resolve_db_path(args, ctx);
    if !db_path.exists() {
        return Ok(CallToolResult::error(
            "JLCPCB database not found. Run download_jlcpcb_database first.",
        ));
    }

    let query = args["query"].as_str().unwrap_or("").to_string();
    let basic_only = args["basic_only"].as_bool().unwrap_or(false);
    let in_stock = args["in_stock"].as_bool().unwrap_or(true);
    let limit = args["limit"].as_u64().unwrap_or(20).min(1000) as usize;
    let category = args["category"].as_str().map(String::from);

    let key = cache_key(
        "search_jlcpcb_parts",
        &db_path,
        &[
            &query,
            category.as_deref().unwrap_or(""),
            &basic_only.to_string(),
            &in_stock.to_string(),
            &limit.to_string(),
        ],
    );
    if let Some(cached) = ctx.jlcpcb_cache.get(&key) {
        let mut body = cached;
        body["cached"] = json!(true);
        return Ok(CallToolResult::text(
            serde_json::to_string_pretty(&body).unwrap(),
        ));
    }

    let results = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<serde_json::Value>> {
        let conn = rusqlite::Connection::open(&db_path)?;

        // The JLCPCB db schema has columns: LCSC, MFR_Part, Package, Solder_Joint,
        // Manufacturer, Library_Type, Description, Datasheet, Price, Stock
        let mut sql = String::from(
            "SELECT LCSC, MFR_Part, Package, Manufacturer, Library_Type, Description, Price, Stock \
             FROM components WHERE (Description LIKE ?1 OR MFR_Part LIKE ?1)"
        );
        if basic_only {
            sql.push_str(" AND Library_Type = 'Basic'");
        }
        if in_stock {
            sql.push_str(" AND Stock > 0");
        }
        if let Some(ref _cat) = category {
            sql.push_str(" AND Category LIKE ?2");
        }
        sql.push_str(&format!(" LIMIT {}", limit));

        let like_query = format!("%{}%", query);
        let mut stmt = conn.prepare(&sql)?;

        let rows: Vec<serde_json::Value> = if category.is_some() {
            let cat_like = format!("%{}%", category.as_deref().unwrap_or(""));
            stmt.query_map(rusqlite::params![like_query, cat_like], row_to_part_json)?
                .filter_map(|r| r.ok())
                .collect()
        } else {
            stmt.query_map(rusqlite::params![like_query], row_to_part_json)?
                .filter_map(|r| r.ok())
                .collect()
        };
        Ok(rows)
    })
    .await??;

    let body = json!({
        "query": args["query"].as_str().unwrap_or(""),
        "count": results.len(),
        "results": results
    });
    ctx.jlcpcb_cache.put(key, body.clone());

    let mut body = body;
    body["cached"] = json!(false);
    Ok(CallToolResult::text(
        serde_json::to_string_pretty(&body).unwrap(),
    ))
}

fn row_to_part_json(row: &rusqlite::Row) -> rusqlite::Result<serde_json::Value> {
    Ok(json!({
        "lcsc": row.get::<_, String>(0).unwrap_or_default(),
        "mpn": row.get::<_, String>(1).unwrap_or_default(),
        "package": row.get::<_, String>(2).unwrap_or_default(),
        "manufacturer": row.get::<_, String>(3).unwrap_or_default(),
        "library_type": row.get::<_, String>(4).unwrap_or_default(),
        "description": row.get::<_, String>(5).unwrap_or_default(),
        "price": row.get::<_, f64>(6).unwrap_or(0.0),
        "stock": row.get::<_, i64>(7).unwrap_or(0)
    }))
}

async fn handle_get_jlcpcb_part(
    args: &serde_json::Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let db_path = resolve_db_path(args, ctx);
    if !db_path.exists() {
        return Ok(CallToolResult::error(
            "JLCPCB database not found. Run download_jlcpcb_database first.",
        ));
    }
    let lcsc_id = require_str(args, "lcsc_id")
        .map_err(|e| anyhow::anyhow!("{:?}", e))?
        .to_string();

    let key = cache_key("get_jlcpcb_part", &db_path, &[&lcsc_id]);
    if let Some(mut cached) = ctx.jlcpcb_cache.get(&key) {
        cached["cached"] = json!(true);
        return Ok(CallToolResult::text(
            serde_json::to_string_pretty(&cached).unwrap(),
        ));
    }

    let result =
        tokio::task::spawn_blocking(move || -> anyhow::Result<Option<serde_json::Value>> {
            let conn = rusqlite::Connection::open(&db_path)?;
            let mut stmt = conn.prepare(
            "SELECT LCSC, MFR_Part, Package, Manufacturer, Library_Type, Description, Price, Stock \
             FROM components WHERE LCSC = ?1 LIMIT 1"
        )?;
            let mut rows = stmt.query_map(rusqlite::params![lcsc_id], row_to_part_json)?;
            Ok(rows.next().and_then(|r| r.ok()))
        })
        .await??;

    match result {
        Some(part) => {
            ctx.jlcpcb_cache.put(key, part.clone());
            let mut part = part;
            part["cached"] = json!(false);
            Ok(CallToolResult::text(
                serde_json::to_string_pretty(&part).unwrap(),
            ))
        }
        None => Ok(CallToolResult::error(format!(
            "Part not found in database: {}",
            args["lcsc_id"].as_str().unwrap_or("")
        ))),
    }
}

async fn handle_suggest_alternatives(
    args: &serde_json::Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let db_path = resolve_db_path(args, ctx);
    if !db_path.exists() {
        return Ok(CallToolResult::error(
            "JLCPCB database not found. Run download_jlcpcb_database first.",
        ));
    }
    let value = args["value"].as_str().unwrap_or("").to_string();
    let footprint = args["footprint"].as_str().unwrap_or("").to_string();
    let max_price = args["max_price_usd"].as_f64();
    let limit = args["limit"].as_u64().unwrap_or(5).min(1000) as usize;

    // Extract package from footprint (e.g. "Resistor_SMD:R_0402" → "0402")
    let package_hint = footprint
        .split(':')
        .next_back()
        .unwrap_or("")
        .split('_')
        .next_back()
        .unwrap_or("")
        .to_string();

    let key = cache_key(
        "suggest_jlcpcb_alternatives",
        &db_path,
        &[
            &value,
            &footprint,
            &max_price.map(|v| v.to_string()).unwrap_or_default(),
            &limit.to_string(),
        ],
    );
    if let Some(cached) = ctx.jlcpcb_cache.get(&key) {
        let mut body = cached;
        body["cached"] = json!(true);
        return Ok(CallToolResult::text(
            serde_json::to_string_pretty(&body).unwrap(),
        ));
    }

    let results = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<serde_json::Value>> {
        let conn = rusqlite::Connection::open(&db_path)?;
        let like_val = format!("%{}%", value);
        let like_pkg = format!("%{}%", package_hint);

        let candidate_limit = limit.saturating_mul(50).clamp(limit, 5000);
        let mut sql = String::from(
            "SELECT LCSC, MFR_Part, Package, Manufacturer, Library_Type, Description, Price, Stock \
             FROM components WHERE Description LIKE ?1 AND Package LIKE ?2 AND Stock > 0"
        );
        if let Some(max_p) = max_price {
            sql.push_str(&format!(" AND Price > 0 AND Price <= {}", max_p));
        }
        sql.push_str(&format!(
            " ORDER BY CASE WHEN Price > 0 THEN 0 ELSE 1 END, Price ASC LIMIT {}",
            candidate_limit
        ));

        let mut stmt = conn.prepare(&sql)?;
        let mut rows: Vec<serde_json::Value> = stmt
            .query_map(rusqlite::params![like_val, like_pkg], row_to_part_json)?
            .filter_map(|r| r.ok())
            .collect();
        rows.retain(|part| {
            part["description"]
                .as_str()
                .is_some_and(|description| contains_numeric_token(description, &value))
        });
        rows.truncate(limit);
        Ok(rows)
    })
    .await??;

    let body = json!({
        "value": args["value"].as_str().unwrap_or(""),
        "footprint": args["footprint"].as_str().unwrap_or(""),
        "alternatives": results
    });
    ctx.jlcpcb_cache.put(key, body.clone());

    let mut body = body;
    body["cached"] = json!(false);
    Ok(CallToolResult::text(
        serde_json::to_string_pretty(&body).unwrap(),
    ))
}

async fn handle_jlcpcb_stats(
    args: &serde_json::Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let db_path = resolve_db_path(args, ctx);
    if !db_path.exists() {
        return Ok(CallToolResult::text(
            serde_json::to_string_pretty(&json!({
                "exists": false,
                "note": "Run download_jlcpcb_database to fetch the parts database"
            }))
            .unwrap(),
        ));
    }

    let meta = tokio::fs::metadata(&db_path).await?;
    let size_bytes = meta.len();

    let (count, catalog_metadata) = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        move || -> anyhow::Result<(i64, Option<CatalogMetadata>)> {
            let conn = rusqlite::Connection::open(&db_path)?;
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM components", [], |r| r.get(0))?;
            let has_metadata: bool = conn.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM sqlite_master
                    WHERE type = 'table' AND name = 'catalog_metadata'
                )",
                [],
                |row| row.get(0),
            )?;
            let metadata = if has_metadata {
                conn.query_row(
                    "SELECT catalog, source_date, source_last_updated
                     FROM catalog_metadata LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .ok()
            } else {
                None
            };
            Ok((count, metadata))
        }
    })
    .await??;

    let mut body = json!({
        "exists": true,
        "path": db_path.to_str().unwrap_or(""),
        "size_bytes": size_bytes,
        "part_count": count
    });
    if let Some((catalog, source_date, source_last_updated)) = catalog_metadata {
        body["catalog"] = json!(catalog);
        body["catalog_date"] = json!(source_date);
        body["catalog_last_updated"] = json!(source_last_updated);
    }
    Ok(CallToolResult::text(
        serde_json::to_string_pretty(&body).unwrap(),
    ))
}

async fn handle_enrich_datasheets(
    args: &serde_json::Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let sch_path = get_path(args, "schematic")?;
    let overwrite = args["overwrite_existing"].as_bool().unwrap_or(false);

    let content = tokio::fs::read_to_string(&sch_path).await?;

    // Find all LCSC property values in the schematic
    let mut lcsc_ids: Vec<String> = Vec::new();
    let mut search = content.as_str();
    while let Some(pos) = search.find("(property \"LCSC\" \"") {
        let after = &search[pos + 18..];
        if let Some(end) = after.find('"') {
            lcsc_ids.push(after[..end].to_string());
        }
        search = &search[pos + 1..];
    }
    lcsc_ids.sort();
    lcsc_ids.dedup();

    if lcsc_ids.is_empty() {
        return Ok(CallToolResult::text(
            serde_json::to_string_pretty(&json!({
                "updated": 0,
                "note": "No LCSC property found in schematic components"
            }))
            .unwrap(),
        ));
    }

    // Query LCSC API for datasheet URLs
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let mut enriched = 0usize;
    let mut new_content = content.clone();

    for lcsc_id in &lcsc_ids {
        let url = format!(
            "https://wmsc.lcsc.com/ftps/wm/product/detail?productCode={}",
            lcsc_id
        );
        if let Ok(resp) = get_with_backoff(&client, &url).await {
            if resp.status().is_success() {
                if let Ok(json_resp) = resp.json::<serde_json::Value>().await {
                    if let Some(datasheet_url) = json_resp
                        .pointer("/result/dataManualUrl")
                        .and_then(|v| v.as_str())
                    {
                        // Find components with this LCSC ID and update their Datasheet property.
                        // Pattern: find (property "LCSC" "CxxxID") → walk back to symbol block →
                        // find (property "Datasheet" "...") and replace the URL.
                        let lcsc_pat = format!(r#"(property "LCSC" "{}")"#, lcsc_id);
                        let mut search_from = 0usize;
                        while let Some(lcsc_pos) = new_content[search_from..]
                            .find(&lcsc_pat)
                            .map(|i| i + search_from)
                        {
                            // Find the enclosing symbol block
                            let before = &new_content[..lcsc_pos];
                            if let Some(sym_start) = before.rfind("\n  (symbol") {
                                let sym_block = &new_content[sym_start..];
                                // Find Datasheet property within this symbol
                                let ds_pat = r#"(property "Datasheet" ""#;
                                if let Some(ds_offset) = sym_block.find(ds_pat) {
                                    let ds_abs = sym_start + ds_offset + ds_pat.len();
                                    if let Some(ds_end) = new_content[ds_abs..].find('"') {
                                        let existing = &new_content[ds_abs..ds_abs + ds_end];
                                        if overwrite || existing == "~" || existing.is_empty() {
                                            new_content = format!(
                                                "{}{}{}",
                                                &new_content[..ds_abs],
                                                datasheet_url,
                                                &new_content[ds_abs + ds_end..]
                                            );
                                            enriched += 1;
                                        }
                                    }
                                }
                            }
                            search_from = lcsc_pos + 1;
                        }
                    }
                }
            }
        }
    }

    // Write back if anything changed
    if enriched > 0 {
        konnect_sexp::writer::write_atomic(&sch_path, &new_content)?;
    }

    Ok(CallToolResult::text(
        serde_json::to_string_pretty(&json!({
            "lcsc_ids_found": lcsc_ids.len(),
            "datasheets_enriched": enriched,
            "schematic": sch_path.to_str().unwrap_or("")
        }))
        .unwrap(),
    ))
}

async fn handle_get_datasheet_url(
    args: &serde_json::Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let mpn = args["mpn"].as_str();
    let lcsc_id = args["lcsc_id"].as_str();

    if mpn.is_none() && lcsc_id.is_none() {
        return Ok(CallToolResult::error("Provide either 'mpn' or 'lcsc_id'"));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    // Try LCSC API with lcsc_id first
    if let Some(id) = lcsc_id {
        let url = format!(
            "https://wmsc.lcsc.com/ftps/wm/product/detail?productCode={}",
            id
        );
        if let Ok(resp) = get_with_backoff(&client, &url).await {
            if resp.status().is_success() {
                if let Ok(json_resp) = resp.json::<serde_json::Value>().await {
                    if let Some(ds_url) = json_resp
                        .pointer("/result/dataManualUrl")
                        .and_then(|v| v.as_str())
                    {
                        return Ok(CallToolResult::text(
                            serde_json::to_string_pretty(&json!({
                                "lcsc_id": id,
                                "datasheet_url": ds_url
                            }))
                            .unwrap(),
                        ));
                    }
                }
            }
        }
    }

    Ok(CallToolResult::text(
        serde_json::to_string_pretty(&json!({
            "mpn": mpn,
            "lcsc_id": lcsc_id,
            "datasheet_url": null,
            "note": "Datasheet not found via LCSC API"
        }))
        .unwrap(),
    ))
}

// ─── Freerouting ──────────────────────────────────────────────────────────────

fn find_freerouting_jar(args: &serde_json::Value) -> Option<PathBuf> {
    if let Some(p) = args["jar_path"].as_str() {
        return Some(PathBuf::from(p));
    }
    // Common locations
    let candidates = [
        "freerouting.jar",
        "/usr/local/lib/freerouting/freerouting.jar",
        "/opt/freerouting/freerouting.jar",
    ];
    for c in &candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

async fn handle_autoroute(
    _args: &serde_json::Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    // ponytail: Freerouting workflow requires Specctra DSN export + SES import,
    // both of which were removed from kicad-cli in KiCAD 10. The tool stays in the
    // registry so callers get a clear error; remove entirely once IPC round-trip lands.
    Ok(CallToolResult::error(
        "Autoroute via Freerouting is not available: kicad-cli in KiCAD 10 no longer \
         supports Specctra DSN export or SES import. Use KiCAD's PCB editor \
         (File > Export > Specctra DSN, then File > Import > Specctra Session) manually.",
    ))
}

async fn handle_check_freerouting(
    args: &serde_json::Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let jar = find_freerouting_jar(args);

    match jar {
        None => Ok(CallToolResult::text(
            serde_json::to_string_pretty(&json!({
                "available": false,
                "note": "freerouting.jar not found. Download from https://github.com/freerouting/freerouting/releases"
            }))
            .unwrap(),
        )),
        Some(jar_path) => {
            // Try to get version from java -jar freerouting.jar --version
            let output = tokio::process::Command::new("java")
                .args(["-jar", jar_path.to_str().unwrap_or(""), "--version"])
                .output()
                .await;

            let version = match output {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout);
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    format!("{}{}", stdout.trim(), stderr.trim())
                }
                Err(e) => format!("java not available: {e}"),
            };

            Ok(CallToolResult::text(
                serde_json::to_string_pretty(&json!({
                    "available": true,
                    "jar_path": jar_path.to_str().unwrap_or(""),
                    "version_output": version
                }))
                .unwrap(),
            ))
        }
    }
}

#[cfg(test)]
mod retry_backoff_tests {
    use super::*;

    /// End-to-end check against a real (hand-rolled) flaky HTTP server: two
    /// 503s followed by a 200 should be retried through to success, with
    /// real backoff delays elapsed in between — not just the status-code
    /// decision logic in isolation.
    #[tokio::test]
    async fn get_with_backoff_recovers_after_transient_failures() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            for resp in [
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n",
                "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok",
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let _ = socket.read(&mut buf).await;
                socket.write_all(resp.as_bytes()).await.unwrap();
            }
        });

        let client = reqwest::Client::new();
        let url = format!("http://{}/x", addr);

        let start = std::time::Instant::now();
        let resp = get_with_backoff(&client, &url).await.unwrap();
        let elapsed = start.elapsed();

        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        // Two retries at 300ms + 600ms = 900ms minimum before the 3rd (successful) attempt.
        assert!(
            elapsed >= std::time::Duration::from_millis(900),
            "expected backoff delays to have elapsed, got {:?}",
            elapsed
        );
    }

    /// A persistent (non-transient) failure should return immediately after
    /// the first attempt — no wasted retries on a 404.
    #[tokio::test]
    async fn get_with_backoff_does_not_retry_client_errors() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = socket.read(&mut buf).await;
            socket
                .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
            // If get_with_backoff retried, it would try to accept() again here
            // and this task would hang until the test times out.
        });

        let client = reqwest::Client::new();
        let url = format!("http://{}/x", addr);

        let start = std::time::Instant::now();
        let resp = get_with_backoff(&client, &url).await.unwrap();
        let elapsed = start.elapsed();

        assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
        assert!(
            elapsed < std::time::Duration::from_millis(200),
            "expected no retry delay for a 404, took {:?}",
            elapsed
        );
    }

    #[test]
    fn transient_on_rate_limit_and_server_errors() {
        assert!(is_transient_status(reqwest::StatusCode::TOO_MANY_REQUESTS));
        assert!(is_transient_status(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        ));
        assert!(is_transient_status(reqwest::StatusCode::BAD_GATEWAY));
        assert!(is_transient_status(
            reqwest::StatusCode::SERVICE_UNAVAILABLE
        ));
        assert!(is_transient_status(reqwest::StatusCode::GATEWAY_TIMEOUT));
    }

    #[test]
    fn not_transient_on_client_errors() {
        // Retrying a 404/401/403/400 wastes time — the request itself is
        // wrong, not the server having a bad moment.
        assert!(!is_transient_status(reqwest::StatusCode::BAD_REQUEST));
        assert!(!is_transient_status(reqwest::StatusCode::UNAUTHORIZED));
        assert!(!is_transient_status(reqwest::StatusCode::FORBIDDEN));
        assert!(!is_transient_status(reqwest::StatusCode::NOT_FOUND));
    }

    #[test]
    fn not_transient_on_success() {
        assert!(!is_transient_status(reqwest::StatusCode::OK));
        assert!(!is_transient_status(reqwest::StatusCode::NO_CONTENT));
    }

    #[test]
    fn backoff_delay_doubles_each_attempt() {
        assert_eq!(backoff_delay(1), std::time::Duration::from_millis(300));
        assert_eq!(backoff_delay(2), std::time::Duration::from_millis(600));
        assert_eq!(backoff_delay(3), std::time::Duration::from_millis(1200));
    }

    #[test]
    fn backoff_delay_never_panics_on_zero_attempt() {
        // attempt is 1-based in normal use, but the saturating_sub guards
        // against an accidental 0 causing an underflow panic.
        assert_eq!(backoff_delay(0), std::time::Duration::from_millis(300));
    }
}

#[cfg(test)]
mod jlcpcb_download_tests {
    use super::*;
    use axum::body::Bytes;
    use axum::routing::get;
    use axum::Router;

    fn seed_source_database(path: &std::path::Path) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE parts USING fts5(
                 'LCSC Part', 'First Category', 'Second Category', 'MFR.Part',
                 'Package', 'Solder Joint', 'Manufacturer', 'Library Type',
                 'Description', 'Datasheet', 'Price', 'Stock'
             );
             CREATE TABLE meta(filename, size, partcount, date, last_update);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO parts VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                "C1002",
                "Filters",
                "Ferrite Beads",
                "GZ1608D601TF",
                "0603",
                2,
                "Sunlord",
                "Basic",
                "600 ohm ferrite bead",
                "https://example.test/C1002.pdf",
                "1-199:0.019,200-599:0.016",
                "1078400",
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO meta VALUES ('cache.sqlite3', 1234, 1, '2026-07-21', '2026-07-21T08:21:41')",
            [],
        )
        .unwrap();
    }

    fn zip_database(database_path: &std::path::Path, filename: &str) -> Vec<u8> {
        let cursor = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(cursor);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .large_file(true);
        writer.start_file(filename, options).unwrap();
        writer
            .write_all(&std::fs::read(database_path).unwrap())
            .unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn catalog_variants_map_to_published_files() {
        let current = catalog_spec("current").unwrap();
        assert_eq!(current.database_filename, "current-parts-fts5.db");
        assert_eq!(
            current.chunk_count_filename,
            "chunk_num_current_parts_fts5.txt"
        );
        assert!(catalog_spec("unknown").is_none());
    }

    #[test]
    fn parses_first_quantity_price_tier() {
        assert_eq!(first_tier_price("1-199:0.019,200-599:0.016"), 0.019);
        assert_eq!(first_tier_price("0.25"), 0.25);
        assert_eq!(first_tier_price(""), 0.0);
        assert_eq!(first_tier_price("not-a-price"), 0.0);
    }

    #[test]
    fn numeric_value_matching_rejects_larger_values_containing_the_query() {
        assert!(contains_numeric_token("10kΩ thick film resistor", "10k"));
        assert!(contains_numeric_token("resistor 10K 1%", "10k"));
        assert!(!contains_numeric_token("110kΩ thick film resistor", "10k"));
        assert!(!contains_numeric_token("910kΩ thick film resistor", "10k"));
        assert!(!contains_numeric_token("10.2kΩ thick film resistor", "10k"));
    }

    #[test]
    fn converts_published_schema_to_konnect_schema() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.db");
        let target = dir.path().join("target.db");
        seed_source_database(&source);
        let spec = catalog_spec("current").unwrap();

        let stats =
            convert_catalog_database(&source, &target, spec, "https://example.test").unwrap();

        assert_eq!(stats.part_count, 1);
        assert_eq!(stats.basic_count, 1);
        assert_eq!(stats.source_date, "2026-07-21");
        let conn = rusqlite::Connection::open(target).unwrap();
        let row: (String, String, String, f64, i64) = conn
            .query_row(
                "SELECT LCSC, Category, MFR_Part, Price, Stock FROM components",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(row.0, "C1002");
        assert_eq!(row.1, "Filters / Ferrite Beads");
        assert_eq!(row.2, "GZ1608D601TF");
        assert_eq!(row.3, 0.019);
        assert_eq!(row.4, 1_078_400);
    }

    #[test]
    fn extracts_only_the_expected_database_entry() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.db");
        let archive = dir.path().join("catalog.zip");
        let extracted = dir.path().join("extracted.db");
        seed_source_database(&source);
        std::fs::write(&archive, zip_database(&source, "current-parts-fts5.db")).unwrap();

        extract_catalog_database(&archive, "current-parts-fts5.db", &extracted).unwrap();

        assert_eq!(
            std::fs::read(source).unwrap(),
            std::fs::read(extracted).unwrap()
        );
    }

    #[tokio::test]
    async fn downloads_and_reassembles_all_catalog_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.db");
        let archive_path = dir.path().join("download.zip");
        seed_source_database(&source);
        let archive = zip_database(&source, "current-parts-fts5.db");
        let split = archive.len() / 2;
        let first = Bytes::copy_from_slice(&archive[..split]);
        let second = Bytes::copy_from_slice(&archive[split..]);
        let app = Router::new()
            .route("/chunk_num_current_parts_fts5.txt", get(|| async { "2" }))
            .route(
                "/current-parts-fts5.db.zip.001",
                get(move || {
                    let bytes = first.clone();
                    async move { bytes }
                }),
            )
            .route(
                "/current-parts-fts5.db.zip.002",
                get(move || {
                    let bytes = second.clone();
                    async move { bytes }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = reqwest::Client::new();

        let (chunks, bytes) = download_catalog_archive(
            &client,
            &format!("http://{address}"),
            catalog_spec("current").unwrap(),
            &archive_path,
        )
        .await
        .unwrap();

        server.abort();
        assert_eq!(chunks, 2);
        assert_eq!(bytes as usize, archive.len());
        assert_eq!(std::fs::read(archive_path).unwrap(), archive);
    }

    #[test]
    fn atomic_replace_preserves_old_database_until_new_one_is_ready() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("jlcpcb.db");
        let replacement = dir.path().join("replacement.db");
        let backup = dir.path().join("backup.db");
        std::fs::write(&destination, b"old").unwrap();
        std::fs::write(&replacement, b"new").unwrap();

        replace_database_atomically(&replacement, &destination, &backup).unwrap();

        assert_eq!(std::fs::read(destination).unwrap(), b"new");
        assert!(!backup.exists());
    }
}

#[cfg(test)]
mod jlcpcb_cache_tests {
    use super::*;
    use crate::router::ToolRouter;
    use crate::tools::ServerConfig;
    use std::sync::Arc;

    fn test_ctx() -> ToolContext {
        ToolContext::new(
            ServerConfig {
                kicad_cli: String::new(),
                kicad_binary: String::new(),
                ipc_address: String::new(),
                project_dir: None,
                jlcpcb_db_path: None,
            },
            Arc::new(ToolRouter::new()),
        )
    }

    /// Builds a temp SQLite file with a `components` table matching the
    /// schema the handlers query, seeded with one part.
    fn seed_test_db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("jlcpcb.db");
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        conn.execute(
            "CREATE TABLE components (
                LCSC TEXT, MFR_Part TEXT, Package TEXT, Manufacturer TEXT,
                Library_Type TEXT, Description TEXT, Price REAL, Stock INTEGER
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO components VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                "C14663",
                "RC0402FR-0710KL",
                "0402",
                "YAGEO",
                "Basic",
                "10k resistor 0402",
                0.01,
                5000
            ],
        )
        .unwrap();
        (dir, db_path)
    }

    #[tokio::test]
    async fn search_jlcpcb_parts_caches_repeated_query() {
        let (_dir, db_path) = seed_test_db();
        let ctx = test_ctx();
        let args = json!({
            "query": "10k",
            "output_path": db_path.to_str().unwrap()
        });

        let first = handle_search_jlcpcb_parts(&args, &ctx).await.unwrap();
        let second = handle_search_jlcpcb_parts(&args, &ctx).await.unwrap();

        let first_body = response_json(&first);
        let second_body = response_json(&second);
        assert_eq!(first_body["cached"], json!(false));
        assert_eq!(second_body["cached"], json!(true));
        assert_eq!(first_body["results"], second_body["results"]);
        assert_eq!(first_body["count"], json!(1));
    }

    #[tokio::test]
    async fn search_jlcpcb_parts_different_query_is_a_cache_miss() {
        let (_dir, db_path) = seed_test_db();
        let ctx = test_ctx();

        let args_a = json!({ "query": "10k", "output_path": db_path.to_str().unwrap() });
        let args_b = json!({ "query": "100nF", "output_path": db_path.to_str().unwrap() });

        handle_search_jlcpcb_parts(&args_a, &ctx).await.unwrap();
        let second = handle_search_jlcpcb_parts(&args_b, &ctx).await.unwrap();

        assert_eq!(response_json(&second)["cached"], json!(false));
    }

    #[tokio::test]
    async fn get_jlcpcb_part_caches_repeated_lookup() {
        let (_dir, db_path) = seed_test_db();
        let ctx = test_ctx();
        let args = json!({
            "lcsc_id": "C14663",
            "output_path": db_path.to_str().unwrap()
        });

        let first = handle_get_jlcpcb_part(&args, &ctx).await.unwrap();
        let second = handle_get_jlcpcb_part(&args, &ctx).await.unwrap();

        assert_eq!(response_json(&first)["cached"], json!(false));
        assert_eq!(response_json(&second)["cached"], json!(true));
        assert_eq!(response_json(&first)["lcsc"], json!("C14663"));
    }

    #[tokio::test]
    async fn suggest_alternatives_caches_repeated_query() {
        let (_dir, db_path) = seed_test_db();
        let ctx = test_ctx();
        let args = json!({
            "value": "10k",
            "footprint": "Resistor_SMD:R_0402",
            "output_path": db_path.to_str().unwrap()
        });

        let first = handle_suggest_alternatives(&args, &ctx).await.unwrap();
        let second = handle_suggest_alternatives(&args, &ctx).await.unwrap();

        assert_eq!(response_json(&first)["cached"], json!(false));
        assert_eq!(response_json(&second)["cached"], json!(true));
    }

    #[tokio::test]
    async fn suggest_alternatives_requires_exact_value_and_known_price_for_budget() {
        let (_dir, db_path) = seed_test_db();
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO components VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                "C110K",
                "CHEAP-110K",
                "0402",
                "Example",
                "Extended",
                "110k resistor 0402",
                0.001,
                100
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO components VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                "CUNKNOWN",
                "UNKNOWN-10K",
                "0402",
                "Example",
                "Extended",
                "10k resistor 0402",
                0.0,
                100
            ],
        )
        .unwrap();
        drop(conn);
        let ctx = test_ctx();
        let args = json!({
            "value": "10k",
            "footprint": "Resistor_SMD:R_0402",
            "max_price_usd": 0.5,
            "limit": 5,
            "output_path": db_path.to_str().unwrap()
        });

        let result = response_json(&handle_suggest_alternatives(&args, &ctx).await.unwrap());

        assert_eq!(result["alternatives"].as_array().unwrap().len(), 1);
        assert_eq!(result["alternatives"][0]["lcsc"], json!("C14663"));
        assert_eq!(result["alternatives"][0]["price"], json!(0.01));
    }

    fn response_json(result: &CallToolResult) -> serde_json::Value {
        match &result.content[0] {
            crate::mcp::protocol::ToolContent::Text { text } => serde_json::from_str(text).unwrap(),
            _ => panic!("expected text content"),
        }
    }
}
