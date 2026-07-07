//! `integration` toolset — JLCPCB parts database, datasheet enrichment, and Freerouting autorouter.
//!
//! JLCPCB tools query a local SQLite cache of the JLCPCB parts database.
//! Freerouting wraps the Freerouting JAR via subprocess.
//! Datasheet enrichment uses the LCSC HTTP API.

use crate::mcp::protocol::CallToolResult;
use crate::tool;
use crate::tools::{get_path, require_str, ToolContext, ToolDef};
use serde_json::json;
use std::path::PathBuf;

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
                    "force": { "type": "boolean", "description": "Force re-download even if cache exists", "default": false }
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
    #[cfg(not(target_os = "windows"))]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(".konnect").join("jlcpcb.db")
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

// ─── Handlers ─────────────────────────────────────────────────────────────────

async fn handle_download_jlcpcb(
    args: &serde_json::Value,
    ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let db_path = resolve_db_path(args, ctx);
    let force = args["force"].as_bool().unwrap_or(false);

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

    // JLCPCB parts database is distributed as a CSV or SQLite download.
    // The official URL changes — we use a known community mirror format.
    let url = "https://bouni.github.io/kicad-jlcpcb-tools/jlcpcb_parts.db";

    if let Some(parent) = db_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Ok(CallToolResult::error(format!(
            "Download failed: HTTP {}",
            resp.status()
        )));
    }
    let bytes = resp.bytes().await?;
    tokio::fs::write(&db_path, &bytes).await?;

    Ok(CallToolResult::text(
        serde_json::to_string_pretty(&json!({
            "success": true,
            "path": db_path.to_str().unwrap_or(""),
            "size_bytes": bytes.len()
        }))
        .unwrap(),
    ))
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
    let limit = args["limit"].as_u64().unwrap_or(20) as usize;
    let category = args["category"].as_str().map(String::from);

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

    Ok(CallToolResult::text(
        serde_json::to_string_pretty(&json!({
            "query": args["query"].as_str().unwrap_or(""),
            "count": results.len(),
            "results": results
        }))
        .unwrap(),
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
        Some(part) => Ok(CallToolResult::text(
            serde_json::to_string_pretty(&part).unwrap(),
        )),
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
    let limit = args["limit"].as_u64().unwrap_or(5) as usize;

    // Extract package from footprint (e.g. "Resistor_SMD:R_0402" → "0402")
    let package_hint = footprint
        .split(':')
        .next_back()
        .unwrap_or("")
        .split('_')
        .next_back()
        .unwrap_or("")
        .to_string();

    let results = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<serde_json::Value>> {
        let conn = rusqlite::Connection::open(&db_path)?;
        let like_val = format!("%{}%", value);
        let like_pkg = format!("%{}%", package_hint);

        let mut sql = String::from(
            "SELECT LCSC, MFR_Part, Package, Manufacturer, Library_Type, Description, Price, Stock \
             FROM components WHERE Description LIKE ?1 AND Package LIKE ?2 AND Stock > 0"
        );
        if let Some(max_p) = max_price {
            sql.push_str(&format!(" AND Price <= {}", max_p));
        }
        sql.push_str(&format!(" ORDER BY Price ASC LIMIT {}", limit));

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params![like_val, like_pkg], row_to_part_json)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    })
    .await??;

    Ok(CallToolResult::text(
        serde_json::to_string_pretty(&json!({
            "value": args["value"].as_str().unwrap_or(""),
            "footprint": args["footprint"].as_str().unwrap_or(""),
            "alternatives": results
        }))
        .unwrap(),
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

    let count = tokio::task::spawn_blocking({
        let db_path = db_path.clone();
        move || -> anyhow::Result<i64> {
            let conn = rusqlite::Connection::open(&db_path)?;
            let count: i64 = conn.query_row("SELECT COUNT(*) FROM components", [], |r| r.get(0))?;
            Ok(count)
        }
    })
    .await??;

    Ok(CallToolResult::text(
        serde_json::to_string_pretty(&json!({
            "exists": true,
            "path": db_path.to_str().unwrap_or(""),
            "size_bytes": size_bytes,
            "part_count": count
        }))
        .unwrap(),
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
        if let Ok(resp) = client.get(&url).send().await {
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
        if let Ok(resp) = client.get(&url).send().await {
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
