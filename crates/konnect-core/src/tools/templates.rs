//! `templates` toolset — Curated reference circuit starting points.
//!
//! Templates are JSON files stored in the platform user-data directory and
//! shipped as embedded defaults. On Linux, the directory is
//! `$XDG_DATA_HOME/konnect/templates` (normally `~/.local/share/konnect/templates`).
//! Claude retrieves a template and adapts it to the user's project. Values and
//! connections remain design inputs that must be checked against the selected parts.

use crate::mcp::protocol::CallToolResult;
use crate::tool;
use crate::tools::{get_path, require_str, ToolContext, ToolDef};
use konnect_schematic_editor as cse;
use konnect_sexp::geometry::snap_point;
use serde_json::json;
use std::path::PathBuf;
use tracing::{debug, info, warn};

// ─── Embedded starter templates ──────────────────────────────────────────────

fn builtin_templates() -> Vec<serde_json::Value> {
    vec![
        json!({
            "id": "usb_c_5v_sink",
            "name": "USB-C Power Sink (5V default)",
            "description": "USB Type-C receptacle with CC resistors for 5V default power and ESD protection on D+/D-.",
            "category": "connectivity/usb",
            "tags": ["usb-c", "power", "5v", "sink"],
            "components": [
                {"ref_prefix": "J", "lib_id": "Connector:USB_C_Receptacle_USB2.0_16P", "value": "USB_C", "notes": "Generic 16-pin USB 2.0 receptacle symbol; assign and verify the exact connector footprint/pinout"},
                {"ref_prefix": "R", "lib_id": "Device:R", "value": "5.1k", "quantity": 2, "package": "0402", "notes": "CC1 and CC2 pull-down — required for 5V default current"},
                {"ref_prefix": "D", "lib_id": "Device:D_TVS", "value": "PRTR5V0U2X", "quantity": 1, "notes": "ESD protection on D+/D-"},
                {"ref_prefix": "C", "lib_id": "Device:C", "value": "100nF", "quantity": 1, "package": "0402", "notes": "VBUS decoupling"}
            ],
            "connections": [
                {"from": "J.VBUS", "to_net": "VUSB", "notes": "5V power from USB host"},
                {"from": "J.CC1", "via": "R1", "to_net": "GND", "notes": "5.1k pull-down identifies as UFP sink"},
                {"from": "J.CC2", "via": "R2", "to_net": "GND", "notes": "5.1k pull-down"},
                {"from": "J.D+", "to_net": "USB_DP", "notes": "USB data positive"},
                {"from": "J.D-", "to_net": "USB_DN", "notes": "USB data negative"},
                {"from": "J.GND", "to_net": "GND", "notes": "Ground"}
            ],
            "design_notes": "Verify Rd and tolerance against the current USB Type-C specification and the exact receptacle. Rd identifies a sink; the source's Rp advertisement determines available current. Follow the chosen USB PHY reference design for data-line protection, series elements, and layout.",
            "references": ["USB Type-C Spec Rev 2.0, Table 4-25"]
        }),
        json!({
            "id": "ldo_3v3",
            "name": "3.3V LDO Regulator",
            "description": "Low-dropout 3.3V regulator with input/output capacitors. Generic topology, adapt MPN to your needs.",
            "category": "power/regulator",
            "tags": ["ldo", "3v3", "regulator", "power"],
            "components": [
                {"ref_prefix": "U", "lib_id": "Regulator_Linear:AMS1117-3.3", "value": "AMS1117-3.3", "notes": "3.3V 1A LDO — substitute AP2112, MCP1700, etc."},
                {"ref_prefix": "C", "lib_id": "Device:C", "value": "10uF", "quantity": 1, "package": "0805", "notes": "Input capacitor — ceramic X5R or X7R"},
                {"ref_prefix": "C", "lib_id": "Device:C", "value": "10uF", "quantity": 1, "package": "0805", "notes": "Output capacitor — ceramic, check ESR requirements in datasheet"},
                {"ref_prefix": "C", "lib_id": "Device:C", "value": "100nF", "quantity": 2, "package": "0402", "notes": "Bypass caps on input and output"}
            ],
            "connections": [
                {"from": "U.VIN", "to_net": "VIN", "notes": "Input voltage (check max Vin for chosen LDO)"},
                {"from": "U.VOUT", "to_net": "VCC_3V3", "notes": "3.3V regulated output"},
                {"from": "U.GND", "to_net": "GND", "notes": "Ground — ensure low-impedance path"}
            ],
            "design_notes": "Follow the exact regulator datasheet for input/output capacitance, ESR, voltage derating, stability, thermal limits, and placement. Do not substitute another LDO without recalculating the surrounding design.",
            "references": ["AMS1117 datasheet, Section 8.2"]
        }),
        json!({
            "id": "stm32_minimal",
            "name": "STM32 Minimal System",
            "description": "STM32 MCU with HSE crystal, decoupling caps, reset circuit, and SWD debug header.",
            "category": "mcu/stm32",
            "tags": ["stm32", "mcu", "minimal", "crystal", "swd"],
            "components": [
                {"ref_prefix": "U", "lib_id": "MCU_ST_STM32F4:STM32F411CEUx", "value": "STM32F411CEU6", "notes": "48-pin UFQFPN — substitute any STM32 in same package"},
                {"ref_prefix": "Y", "lib_id": "Device:Crystal", "value": "8MHz", "quantity": 1, "notes": "HSE crystal — check MCU datasheet for supported range"},
                {"ref_prefix": "C", "lib_id": "Device:C", "value": "20pF", "quantity": 2, "package": "0402", "notes": "Crystal load caps — calculate from datasheet CL spec"},
                {"ref_prefix": "C", "lib_id": "Device:C", "value": "100nF", "quantity": 5, "package": "0402", "notes": "Decoupling caps — one per VDD/VDDA pin"},
                {"ref_prefix": "C", "lib_id": "Device:C", "value": "4.7uF", "quantity": 1, "package": "0402", "notes": "Bulk decoupling on VDD"},
                {"ref_prefix": "C", "lib_id": "Device:C", "value": "1uF", "quantity": 1, "package": "0402", "notes": "VCAP pin — required for internal regulator"},
                {"ref_prefix": "R", "lib_id": "Device:R", "value": "10k", "quantity": 1, "package": "0402", "notes": "NRST pull-up"},
                {"ref_prefix": "C", "lib_id": "Device:C", "value": "100nF", "quantity": 1, "package": "0402", "notes": "NRST filter cap to GND"},
                {"ref_prefix": "J", "lib_id": "Connector:Conn_ARM_JTAG_SWD_10", "value": "SWD", "notes": "10-pin ARM JTAG/SWD debug header"}
            ],
            "connections": [
                {"from": "U.VDD", "to_net": "VCC_3V3", "notes": "All VDD pins to 3.3V"},
                {"from": "U.VDDA", "to_net": "VCC_3V3", "notes": "Analog supply — add ferrite bead for sensitive analog work"},
                {"from": "U.VSS", "to_net": "GND", "notes": "All VSS pins to ground"},
                {"from": "U.NRST", "to_net": "NRST", "notes": "Reset with 10k pull-up + 100nF cap"},
                {"from": "U.OSC_IN", "to_net": "HSE_IN", "notes": "Crystal input"},
                {"from": "U.OSC_OUT", "to_net": "HSE_OUT", "notes": "Crystal output"},
                {"from": "U.SWDIO", "to_net": "SWDIO", "notes": "SWD data to debug header"},
                {"from": "U.SWCLK", "to_net": "SWCLK", "notes": "SWD clock to debug header"}
            ],
            "design_notes": "Crystal load cap formula: CL = (C1*C2)/(C1+C2) + Cstray, where Cstray ≈ 3-5pF. Place all decoupling caps within 3mm of their VDD pin. VCAP capacitor value is critical — check your specific STM32 variant's datasheet.",
            "references": ["AN4488: Getting started with STM32F4 MCU hardware development"]
        }),
        json!({
            "id": "i2c_pullups",
            "name": "I2C Bus Pull-ups",
            "description": "Standard I2C pull-up resistors for SDA and SCL lines.",
            "category": "connectivity/i2c",
            "tags": ["i2c", "pull-up", "bus"],
            "components": [
                {"ref_prefix": "R", "lib_id": "Device:R", "value": "4.7k", "quantity": 2, "package": "0402", "notes": "Placeholder SDA/SCL pull-ups; calculate from bus voltage, capacitance, speed, rise-time limit, and sink-current limit"}
            ],
            "connections": [
                {"from": "R1.1", "to_net": "SDA", "notes": "I2C data line"},
                {"from": "R1.2", "to_net": "VCC_3V3", "notes": "Pull to I2C bus voltage"},
                {"from": "R2.1", "to_net": "SCL", "notes": "I2C clock line"},
                {"from": "R2.2", "to_net": "VCC_3V3", "notes": "Pull to I2C bus voltage"}
            ],
            "design_notes": "Account for every pull-up already present on the bus. Calculate the allowable resistance range from the current I2C specification, bus voltage/capacitance/speed, and every device's sink capability; 4.7k is only a placeholder.",
            "references": ["NXP UM10204: I2C-bus specification"]
        }),
        json!({
            "id": "led_indicator",
            "name": "LED Indicator Circuit",
            "description": "Simple LED with current-limiting resistor, driven by a GPIO pin.",
            "category": "misc/led",
            "tags": ["led", "indicator", "gpio"],
            "components": [
                {"ref_prefix": "D", "lib_id": "Device:LED", "value": "LED_Green", "quantity": 1, "package": "0603", "notes": "Standard indicator LED"},
                {"ref_prefix": "R", "lib_id": "Device:R", "value": "1k", "quantity": 1, "package": "0402", "notes": "Current limiter: R = (Vcc - Vf) / If. For 3.3V, green Vf≈2.1V, If=1.2mA → 1k"}
            ],
            "connections": [
                {"from": "GPIO", "to": "R1.1", "notes": "GPIO output drives LED through resistor"},
                {"from": "R1.2", "to": "D1.A", "notes": "Resistor to LED anode"},
                {"from": "D1.K", "to_net": "GND", "notes": "LED cathode to ground"}
            ],
            "design_notes": "R = (VCC - Vf) / If. For 3.3V GPIO: green (Vf=2.1V) → 1k gives 1.2mA. Red (Vf=1.8V) → 680R gives 2.2mA. Bright LEDs may only need 0.5mA. Check your LED's datasheet for Vf and recommended If.",
            "references": []
        }),
        json!({
            "id": "buck_converter",
            "name": "Buck Converter (Step-Down)",
            "description": "Synchronous buck converter with input/output caps, inductor, and feedback resistors. Generic topology — adapt MPN and passives to your voltage/current needs.",
            "category": "power/switching",
            "tags": ["buck", "step-down", "switching", "regulator", "power"],
            "components": [
                {"ref_prefix": "U", "lib_id": "Regulator_Switching:TPS563200", "value": "TPS563200DDCR", "notes": "3A sync buck, 4.5-17V input. Substitute: AP63356, MP2315, SY8089"},
                {"ref_prefix": "L", "lib_id": "Device:L", "value": "4.7uH", "quantity": 1, "notes": "Inductor — check datasheet for recommended value and saturation current > Iout*1.3"},
                {"ref_prefix": "C", "lib_id": "Device:C", "value": "10uF", "quantity": 2, "package": "0805", "notes": "Input capacitors — X5R/X7R ceramic, voltage rating > Vin*1.5"},
                {"ref_prefix": "C", "lib_id": "Device:C", "value": "22uF", "quantity": 2, "package": "0805", "notes": "Output capacitors — low ESR ceramic"},
                {"ref_prefix": "C", "lib_id": "Device:C", "value": "100nF", "quantity": 1, "package": "0402", "notes": "Boot cap"},
                {"ref_prefix": "R", "lib_id": "Device:R", "value": "100k", "quantity": 1, "package": "0402", "notes": "Feedback upper resistor — adjust for target Vout"},
                {"ref_prefix": "R", "lib_id": "Device:R", "value": "49.9k", "quantity": 1, "package": "0402", "notes": "Feedback lower resistor — Vout = Vref * (1 + Rtop/Rbot)"}
            ],
            "connections": [
                {"from": "U.VIN", "to_net": "VIN", "notes": "Input power"},
                {"from": "U.SW", "to": "L1.1", "notes": "Switch node to inductor"},
                {"from": "L1.2", "to_net": "VOUT", "notes": "Inductor output"},
                {"from": "U.FB", "via": "voltage divider R_top/R_bot", "to_net": "VOUT", "notes": "Feedback voltage divider"},
                {"from": "U.BOOT", "notes": "Bootstrap cap from BOOT to SW"},
                {"from": "U.GND", "to_net": "GND", "notes": "Power ground — kelvin sense to output cap GND"}
            ],
            "design_notes": "Layout is critical: keep input caps close to VIN/GND pins, keep SW trace short and wide (high di/dt), keep feedback divider close to FB pin away from SW node. Ground plane under inductor improves EMI. Calculate passives from datasheet — do NOT guess values.",
            "references": ["TI SLVA477: Application Note for TPS563200"]
        }),
    ]
}

// ─── Template storage paths ──────────────────────────────────────────────────

fn user_templates_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").unwrap_or_default();
        PathBuf::from(appdata).join("konnect").join("templates")
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(".konnect").join("templates")
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
            .join("templates")
    }
}

/// Load all templates: builtins + any user-created ones from disk.
async fn load_all_templates() -> Vec<serde_json::Value> {
    let mut templates = builtin_templates();

    let user_dir = user_templates_dir();
    if user_dir.is_dir() {
        if let Ok(mut rd) = tokio::fs::read_dir(&user_dir).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("json") {
                    match tokio::fs::read_to_string(&path).await {
                        Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
                            Ok(tmpl) => {
                                debug!(path = %path.display(), "Loaded user template");
                                templates.push(tmpl);
                            }
                            Err(e) => {
                                warn!(path = %path.display(), error = %e, "Failed to parse user template")
                            }
                        },
                        Err(e) => {
                            warn!(path = %path.display(), error = %e, "Failed to read user template")
                        }
                    }
                }
            }
        }
    }

    templates
}

// ─── Tool definitions ─────────────────────────────────────────────────────────

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "search_templates",
            "Search the reference circuit template library. Returns matching templates for \
             common subcircuits (USB-C, LDO, buck converter, MCU minimal system, I2C pull-ups, etc.). \
             Templates are curated starting points; verify values and connections for the selected parts.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search string: 'usb-c', '3.3v regulator', 'stm32', 'i2c', 'led', 'buck converter', etc."
                    },
                    "category": {
                        "type": "string",
                        "description": "Filter by category: 'power', 'connectivity', 'mcu', 'misc' (optional)"
                    }
                },
                "required": ["query"]
            }),
            |args, ctx| async move { handle_search_templates(args, ctx).await }
        ),
        tool!(
            "get_template",
            "Get full details for a reference circuit template including all components, \
             connections, and design notes. Use the template ID from search_templates.",
            json!({
                "type": "object",
                "properties": {
                    "template_id": { "type": "string", "description": "Template ID (e.g. 'usb_c_5v_sink', 'ldo_3v3', 'stm32_minimal')" }
                },
                "required": ["template_id"]
            }),
            |args, ctx| async move { handle_get_template(args, ctx).await }
        ),
        tool!(
            "apply_template",
            "Place a reference template's components into the current schematic and return its \
             mapped connection plan. Wiring is intentionally left for explicit follow-up calls after \
             pin verification.",
            json!({
                "type": "object",
                "properties": {
                    "schematic": { "type": "string", "description": "Path to .kicad_sch file" },
                    "template_id": { "type": "string", "description": "Template ID to instantiate" },
                    "position_x": { "type": "number", "description": "X position to place the subcircuit (mm)", "default": 100.0 },
                    "position_y": { "type": "number", "description": "Y position to place the subcircuit (mm)", "default": 100.0 },
                    "net_mappings": {
                        "type": "object",
                        "description": "Map template net names to your project's net names. E.g. {\"VUSB\": \"VCC_5V\", \"GND\": \"GND\"}"
                    },
                    "ref_start": {
                        "type": "integer",
                        "description": "Starting reference number (e.g. 10 → R10, C10, U10). Auto-detected if omitted."
                    }
                },
                "required": ["schematic", "template_id"]
            }),
            |args, ctx| async move { handle_apply_template(args, ctx).await }
        ),
        tool!(
            "list_template_categories",
            "List all available template categories and the number of templates in each.",
            json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            |args, ctx| async move { handle_list_categories(args, ctx).await }
        ),
    ]
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

async fn handle_search_templates(
    args: &serde_json::Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let query = args["query"].as_str().unwrap_or("").to_lowercase();
    let category_filter = args["category"].as_str().map(|s| s.to_lowercase());

    info!(query = %query, category = ?category_filter, "Searching templates");

    let templates = load_all_templates().await;
    let mut results = Vec::new();

    for tmpl in &templates {
        let id = tmpl["id"].as_str().unwrap_or("");
        let name = tmpl["name"].as_str().unwrap_or("");
        let desc = tmpl["description"].as_str().unwrap_or("");
        let category = tmpl["category"].as_str().unwrap_or("");
        let tags: Vec<&str> = tmpl["tags"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        // Category filter
        if let Some(ref cat) = category_filter {
            if !category.to_lowercase().contains(cat) {
                continue;
            }
        }

        // Search across name, description, tags, category
        let haystack =
            format!("{} {} {} {} {}", id, name, desc, category, tags.join(" ")).to_lowercase();

        let matches = query.split_whitespace().all(|word| haystack.contains(word));

        if matches {
            let component_count: usize =
                tmpl["components"].as_array().map(|a| a.len()).unwrap_or(0);
            results.push(json!({
                "id": id,
                "name": name,
                "description": desc,
                "category": category,
                "tags": tags,
                "component_count": component_count
            }));
        }
    }

    debug!(query = %query, results = results.len(), "Template search complete");

    Ok(CallToolResult::text(
        serde_json::to_string_pretty(&json!({
            "query": query,
            "count": results.len(),
            "templates": results
        }))
        .unwrap(),
    ))
}

async fn handle_get_template(
    args: &serde_json::Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let template_id = match require_str(args, "template_id") {
        Ok(v) => v.to_string(),
        Err(e) => return Ok(e),
    };

    info!(template_id = %template_id, "Loading template");

    let templates = load_all_templates().await;
    let tmpl = templates
        .iter()
        .find(|t| t["id"].as_str() == Some(&template_id));

    match tmpl {
        Some(t) => Ok(CallToolResult::text(
            serde_json::to_string_pretty(t).unwrap(),
        )),
        None => {
            warn!(template_id = %template_id, "Template not found");
            Ok(CallToolResult::error(format!(
                "Template '{}' not found. Use search_templates to find available templates.",
                template_id
            )))
        }
    }
}

async fn handle_apply_template(
    args: &serde_json::Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let sch_path = get_path(args, "schematic")?;
    let template_id = match require_str(args, "template_id") {
        Ok(v) => v.to_string(),
        Err(e) => return Ok(e),
    };
    let base_x = args["position_x"].as_f64().unwrap_or(100.0);
    let base_y = args["position_y"].as_f64().unwrap_or(100.0);
    let net_mappings: std::collections::HashMap<String, String> = args["net_mappings"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    info!(
        template_id = %template_id,
        schematic = %sch_path.display(),
        position = ?(base_x, base_y),
        net_mappings = ?net_mappings,
        "Applying template"
    );

    let templates = load_all_templates().await;
    let tmpl = match templates
        .iter()
        .find(|t| t["id"].as_str() == Some(&template_id))
    {
        Some(t) => t.clone(),
        None => {
            warn!(template_id = %template_id, "Template not found for apply");
            return Ok(CallToolResult::error(format!(
                "Template '{}' not found",
                template_id
            )));
        }
    };

    let content = std::fs::read_to_string(&sch_path)?;

    // Determine starting reference numbers by scanning existing components
    let ref_start = args["ref_start"]
        .as_u64()
        .map(|n| n as usize)
        .unwrap_or_else(|| find_next_ref_number(&content));

    let components = tmpl["components"].as_array().cloned().unwrap_or_default();
    let mut placed = Vec::new();
    let mut ref_counters: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut template_ref_counters: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    let mut reference_map = serde_json::Map::new();
    let mut sch = cse::Schematic::load(&sch_path)?;

    let effects_node = |hide: bool| -> cse::sexp::SexpNode {
        let font = cse::sexp::SexpNode::List(vec![
            cse::sexp::atom("font"),
            cse::sexp::SexpNode::List(vec![
                cse::sexp::atom("size"),
                cse::sexp::atom("1.27"),
                cse::sexp::atom("1.27"),
            ]),
        ]);
        let mut children = vec![cse::sexp::atom("effects"), font];
        if hide {
            children.push(cse::sexp::SexpNode::List(vec![
                cse::sexp::atom("hide"),
                cse::sexp::atom("yes"),
            ]));
        }
        cse::sexp::SexpNode::List(children)
    };
    let at_node = |x: f64, y: f64| -> cse::sexp::SexpNode {
        cse::sexp::SexpNode::List(vec![
            cse::sexp::atom("at"),
            cse::sexp::atom(cse::types::fmt_f64(x)),
            cse::sexp::atom(cse::types::fmt_f64(y)),
            cse::sexp::atom("0"),
        ])
    };

    // Place components in a column layout
    let spacing_y = 15.0; // mm between components
    for comp in components.iter() {
        let ref_prefix = comp["ref_prefix"].as_str().unwrap_or("U");
        let lib_id = comp["lib_id"].as_str().unwrap_or("Device:R");
        let value = comp["value"].as_str().unwrap_or("");
        let quantity = comp["quantity"].as_u64().unwrap_or(1) as usize;
        let notes = comp["notes"].as_str().unwrap_or("");

        for _q in 0..quantity {
            let template_counter = template_ref_counters
                .entry(ref_prefix.to_string())
                .or_insert(1);
            let is_first_for_prefix = *template_counter == 1;
            let template_reference = format!("{}{}", ref_prefix, template_counter);
            *template_counter += 1;
            let counter = ref_counters
                .entry(ref_prefix.to_string())
                .or_insert(ref_start);
            let reference = format!("{}{}", ref_prefix, counter);
            *counter += 1;

            let (x, y) = snap_point(base_x, base_y + (placed.len() as f64) * spacing_y, 1.27);
            cse::library::ensure_lib_symbol(&mut sch, lib_id)?;

            let mut symbol = cse::Symbol::new(lib_id, x, y);
            symbol.at.rotation = Some(0.0);
            let mut ref_property = cse::Property::new("Reference", &reference);
            ref_property.sub_nodes.push(at_node(x, y - 3.81));
            ref_property.sub_nodes.push(effects_node(false));
            symbol.properties.push(ref_property);
            let mut value_property = cse::Property::new("Value", value);
            value_property.sub_nodes.push(at_node(x, y + 3.81));
            value_property.sub_nodes.push(effects_node(false));
            symbol.properties.push(value_property);
            for name in ["Footprint", "Datasheet"] {
                let mut property = cse::Property::new(name, "");
                property.sub_nodes.push(at_node(x, y));
                property.sub_nodes.push(effects_node(true));
                symbol.properties.push(property);
            }
            symbol.raw_sub_nodes.push(cse::sexp::SexpNode::List(vec![
                cse::sexp::atom("instances"),
                cse::sexp::SexpNode::List(vec![
                    cse::sexp::atom("project"),
                    cse::sexp::qstr(""),
                    cse::sexp::SexpNode::List(vec![
                        cse::sexp::atom("path"),
                        cse::sexp::qstr("/"),
                        cse::sexp::SexpNode::List(vec![
                            cse::sexp::atom("reference"),
                            cse::sexp::qstr(&reference),
                        ]),
                        cse::sexp::SexpNode::List(vec![
                            cse::sexp::atom("unit"),
                            cse::sexp::atom("1"),
                        ]),
                    ]),
                ]),
            ]));
            sch.add_symbol(symbol);
            reference_map.insert(template_reference, json!(&reference));
            // Built-in connection plans use both numbered references (`R1.1`)
            // and a short alias for a single IC/connector (`U.VIN`, `J.VBUS`).
            if is_first_for_prefix {
                reference_map.insert(ref_prefix.to_string(), json!(&reference));
            }

            placed.push(json!({
                "reference": reference,
                "lib_id": lib_id,
                "value": value,
                "x": x, "y": y,
                "notes": notes
            }));

            debug!(reference = %reference, lib_id = %lib_id, value = %value, "Placed template component");
        }
    }

    // Persist the complete template insertion in one atomic write. Library
    // definitions are embedded before any component instance is committed.
    sch.overwrite()?;

    info!(
        template_id = %template_id,
        components_placed = placed.len(),
        "Template applied successfully"
    );

    // Build the net mapping guide for the user/Claude to wire up
    let connections = tmpl["connections"].as_array().cloned().unwrap_or_default();
    let mapped_connections: Vec<serde_json::Value> = connections
        .iter()
        .map(|conn| {
            let mut c = conn.clone();
            for field in ["from", "to", "via"] {
                if let Some(endpoint) = c[field].as_str() {
                    if let Some(mapped) = map_template_endpoint(endpoint, &reference_map) {
                        c[field] = json!(mapped);
                    }
                }
            }
            let original_net = c["to_net"].as_str().map(String::from);
            if let Some(net) = original_net {
                if let Some(mapped) = net_mappings.get(&net) {
                    c["to_net"] = json!(mapped);
                    c["mapped_from"] = json!(net);
                }
            }
            c
        })
        .collect();

    Ok(CallToolResult::text(
        serde_json::to_string_pretty(&json!({
            "template": template_id,
            "components_placed": placed,
            "reference_map": reference_map,
            "connections_to_wire": mapped_connections,
            "design_notes": tmpl["design_notes"],
            "next_steps": "Connection endpoints already use the assigned references. Verify actual pin numbers/coordinates, then wire explicitly with connect_to_net or connect_pins."
        }))
        .unwrap(),
    ))
}

/// Replace a template reference at the start of an endpoint while preserving
/// its pin suffix. Free-form notes such as "voltage divider R_top/R_bot" are
/// deliberately left untouched.
fn map_template_endpoint(
    endpoint: &str,
    reference_map: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let (reference, suffix) = endpoint
        .split_once('.')
        .map(|(reference, suffix)| (reference, Some(suffix)))
        .unwrap_or((endpoint, None));
    let mapped = reference_map.get(reference)?.as_str()?;
    Some(match suffix {
        Some(suffix) => format!("{mapped}.{suffix}"),
        None => mapped.to_string(),
    })
}

async fn handle_list_categories(
    _args: &serde_json::Value,
    _ctx: &ToolContext,
) -> anyhow::Result<CallToolResult> {
    let templates = load_all_templates().await;
    let mut categories: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for tmpl in &templates {
        let cat = tmpl["category"]
            .as_str()
            .unwrap_or("uncategorized")
            .to_string();
        *categories.entry(cat).or_insert(0) += 1;
    }

    let mut sorted: Vec<_> = categories.into_iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    Ok(CallToolResult::text(
        serde_json::to_string_pretty(&json!({
            "categories": sorted.iter().map(|(cat, count)| json!({"category": cat, "count": count})).collect::<Vec<_>>(),
            "total_templates": templates.len()
        }))
        .unwrap(),
    ))
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Find the next available reference number by scanning existing references in the schematic.
fn find_next_ref_number(content: &str) -> usize {
    let mut max_ref = 0usize;
    let mut pos = 0;
    while let Some(ref_pos) = content[pos..].find("(reference \"") {
        let abs = pos + ref_pos + 12;
        if let Some(end) = content[abs..].find('"') {
            let reference = &content[abs..abs + end];
            // Extract the numeric suffix
            let num_str: String = reference
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            if let Ok(num) = num_str.parse::<usize>() {
                if num > max_ref {
                    max_ref = num;
                }
            }
        }
        pos = abs + 1;
    }
    max_ref + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_endpoint_mapping_handles_numbered_and_short_aliases() {
        let reference_map = serde_json::Map::from_iter([
            ("U".to_string(), json!("U12")),
            ("R1".to_string(), json!("R12")),
        ]);

        assert_eq!(
            map_template_endpoint("U.VIN", &reference_map).as_deref(),
            Some("U12.VIN")
        );
        assert_eq!(
            map_template_endpoint("R1.1", &reference_map).as_deref(),
            Some("R12.1")
        );
        assert_eq!(
            map_template_endpoint("R1", &reference_map).as_deref(),
            Some("R12")
        );
        assert_eq!(map_template_endpoint("GPIO", &reference_map), None);
    }
}
