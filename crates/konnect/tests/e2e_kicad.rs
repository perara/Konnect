//! End-to-end test against a real KiCAD installation (kicad-cli).
//!
//! Drives the shipped binary over stdio through a full design loop:
//! create project → place components → wire → ERC → export Gerbers → DRC.
//!
//! Requires kicad-cli and the standard symbol libraries, so it is `#[ignore]`
//! by default and run explicitly by the e2e-kicad workflow (and locally):
//!
//!     cargo test -p konnect --test e2e_kicad -- --ignored --nocapture

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Mcp {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    next_id: i64,
}

/// Locate kicad-cli: KICAD_CLI env override, PATH, then platform defaults.
fn find_kicad_cli() -> Option<String> {
    if let Ok(p) = std::env::var("KICAD_CLI") {
        if std::path::Path::new(&p).exists() {
            return Some(p);
        }
    }
    let name = if cfg!(windows) {
        "kicad-cli.exe"
    } else {
        "kicad-cli"
    };
    if Command::new(name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
    {
        return Some(name.to_string());
    }
    let candidates: &[&str] = if cfg!(windows) {
        &[
            r"C:\KiCad\10.0\bin\kicad-cli.exe",
            r"C:\Program Files\KiCad\10.0\bin\kicad-cli.exe",
        ]
    } else if cfg!(target_os = "macos") {
        &["/Applications/KiCad/KiCad.app/Contents/MacOS/kicad-cli"]
    } else {
        &["/usr/bin/kicad-cli", "/usr/local/bin/kicad-cli"]
    };
    candidates
        .iter()
        .find(|c| std::path::Path::new(c).exists())
        .map(|c| c.to_string())
}

impl Mcp {
    fn spawn(kicad_cli: &str) -> Self {
        let mut config = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
        write!(config, "{}", json!({"kicad_cli": kicad_cli})).unwrap();
        config.flush().unwrap();
        let (_persist, config_path) = config.keep().unwrap();

        let mut child = Command::new(env!("CARGO_BIN_EXE_konnect"))
            .arg("--config")
            .arg(&config_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let reader = BufReader::new(child.stdout.take().unwrap());
        let mut p = Mcp {
            child,
            stdin,
            reader,
            next_id: 1,
        };
        p.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "e2e", "version": "0"}
            }),
        );
        p
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        )
        .unwrap();
        self.stdin.flush().unwrap();
        loop {
            let mut line = String::new();
            assert!(self.reader.read_line(&mut line).unwrap() > 0, "server died");
            let v: Value = serde_json::from_str(line.trim()).unwrap();
            if v.get("id").and_then(Value::as_i64) == Some(id) {
                return v;
            }
        }
    }

    fn tool(&mut self, name: &str, args: Value) -> Value {
        let r = self.request("tools/call", json!({"name": name, "arguments": args}));
        let result = r["result"].clone();
        assert_ne!(
            result["isError"],
            json!(true),
            "tool {name} failed: {}",
            result["content"][0]["text"].as_str().unwrap_or("?")
        );
        result
    }

    fn load(&mut self, toolset: &str) {
        self.tool("load_toolset", json!({"name": toolset}));
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn body(result: &Value) -> Value {
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap_or("{}"))
        .unwrap_or(Value::Null)
}

#[test]
#[ignore = "requires kicad-cli + symbol libraries; run via e2e workflow"]
fn full_design_loop_with_real_kicad() {
    let Some(kicad_cli) = find_kicad_cli() else {
        panic!("kicad-cli not found — set KICAD_CLI or install KiCAD (this test is e2e-only)");
    };
    // KONNECT_E2E_KEEP_DIR: persist the generated project there (CI uploads
    // it as a failure artifact so file-format rejections can be diagnosed).
    let tmp = tempfile::tempdir().unwrap();
    let base: std::path::PathBuf = match std::env::var("KONNECT_E2E_KEEP_DIR") {
        Ok(d) => {
            std::fs::create_dir_all(&d).unwrap();
            d.into()
        }
        Err(_) => tmp.path().to_path_buf(),
    };
    let proj = base.join("e2e");
    let proj_s = proj.to_string_lossy().to_string();
    let sch = proj.join("e2e.kicad_sch");
    let pcb = proj.join("e2e.kicad_pcb");
    let mut p = Mcp::spawn(&kicad_cli);

    // ── Create ───────────────────────────────────────────────────────────
    p.tool("create_project", json!({"name": "e2e", "path": proj_s}));
    assert!(sch.exists() && pcb.exists());

    // ── Schematic: RC divider ────────────────────────────────────────────
    p.load("sch_components");
    p.load("sch_wiring");
    p.tool(
        "add_schematic_component",
        json!({
            "schematic": sch.to_string_lossy(), "lib_id": "Device:R",
            "reference": "R1", "value": "10k", "x": 100.0, "y": 100.0
        }),
    );
    p.tool(
        "add_schematic_component",
        json!({
            "schematic": sch.to_string_lossy(), "lib_id": "Device:C",
            "reference": "C1", "value": "100n", "x": 120.0, "y": 100.0
        }),
    );
    p.tool(
        "connect_pins",
        json!({
            "schematic": sch.to_string_lossy(),
            "ref1": "R1", "pin1": "2",
            "ref2": "C1", "pin2": "1"
        }),
    );

    // Template insertion must embed library definitions, remain parseable,
    // and report an explicit connection plan rather than claiming it wired
    // ambiguous template pin names automatically.
    p.load("templates");
    let applied = body(&p.tool(
        "apply_template",
        json!({
            "schematic": sch.to_string_lossy(),
            "template_id": "led_indicator",
            "position_x": 140.0,
            "position_y": 100.0
        }),
    ));
    assert_eq!(
        applied["components_placed"].as_array().map(Vec::len),
        Some(2)
    );
    assert!(applied["reference_map"].is_object());
    assert!(applied["connections_to_wire"].is_array());
    assert_eq!(
        applied["reference_map"]["D"],
        applied["reference_map"]["D1"]
    );
    assert_eq!(
        applied["reference_map"]["R"],
        applied["reference_map"]["R1"]
    );
    let placed_references: std::collections::HashSet<_> = applied["components_placed"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|component| component["reference"].as_str())
        .collect();
    for connection in applied["connections_to_wire"].as_array().unwrap() {
        for field in ["from", "to", "via"] {
            let Some(endpoint) = connection[field].as_str() else {
                continue;
            };
            let reference = endpoint.split('.').next().unwrap();
            if reference.starts_with('D') || reference.starts_with('R') {
                assert!(
                    placed_references.contains(reference),
                    "template endpoint was not translated to an assigned reference: {endpoint}"
                );
            }
        }
    }

    // Every bundled template must resolve against the standard KiCAD 10
    // libraries and produce a schematic that KiCAD itself can load.
    p.load("sch_export");
    for (index, template_id) in [
        "usb_c_5v_sink",
        "ldo_3v3",
        "stm32_minimal",
        "i2c_pullups",
        "led_indicator",
        "buck_converter",
    ]
    .iter()
    .enumerate()
    {
        let template_schematic = proj.join(format!("template-{index}.kicad_sch"));
        p.tool(
            "create_schematic",
            json!({"path": template_schematic.to_string_lossy()}),
        );
        p.tool(
            "apply_template",
            json!({
                "schematic": template_schematic.to_string_lossy(),
                "template_id": template_id
            }),
        );
        let template_erc = body(&p.tool(
            "run_erc",
            json!({"schematic": template_schematic.to_string_lossy()}),
        ));
        assert!(
            template_erc.get("errors").is_some()
                || template_erc.get("violations").is_some()
                || template_erc.get("summary").is_some(),
            "template {template_id} was not accepted by KiCAD: {template_erc}"
        );
    }

    // The written schematic must still parse and contain both parts.
    let content = std::fs::read_to_string(&sch).unwrap();
    let tree = konnect_sexp::parse_sexp(&content).expect("tool output must reparse");
    let refs: Vec<_> = konnect_sexp::schematic::extract_symbol_instances(&tree)
        .into_iter()
        .map(|s| s.reference)
        .collect();
    assert!(refs.contains(&"R1".to_string()) && refs.contains(&"C1".to_string()));
    assert_eq!(refs.len(), 4, "template components must remain parseable");

    // ── ERC through real eeschema ────────────────────────────────────────
    p.load("verification");
    let erc = body(&p.tool("run_erc", json!({"schematic": sch.to_string_lossy()})));
    // A 2-part net has floating-pin warnings; what matters is that eeschema
    // parsed OUR file and produced a structured report at all.
    assert!(
        erc.get("errors").is_some()
            || erc.get("violations").is_some()
            || erc.get("summary").is_some(),
        "unexpected ERC shape: {erc}"
    );

    // ── PCB: export Gerbers + DRC through real kicad-cli ─────────────────
    p.load("pcb_export");
    let out_dir = proj.join("gerbers");
    p.tool(
        "export_gerber",
        json!({
            "board": pcb.to_string_lossy(),
            "output_dir": out_dir.to_string_lossy()
        }),
    );
    let produced = std::fs::read_dir(&out_dir).map(|d| d.count()).unwrap_or(0);
    assert!(
        produced > 0,
        "no gerber files produced in {}",
        out_dir.display()
    );

    let pcb_pdf = proj.join("board.pdf");
    p.tool(
        "export_pdf",
        json!({
            "board": pcb.to_string_lossy(),
            "output": pcb_pdf.to_string_lossy(),
            "layers": ["F.Cu", "Edge.Cuts"],
            "black_and_white": true
        }),
    );
    assert!(pcb_pdf.is_file());

    let pcb_svg = proj.join("board.svg");
    p.tool(
        "export_svg",
        json!({
            "board": pcb.to_string_lossy(),
            "output": pcb_svg.to_string_lossy(),
            "layers": ["F.Cu", "Edge.Cuts"]
        }),
    );
    assert!(pcb_svg.is_file());

    let pcb_step = proj.join("board.step");
    p.tool(
        "export_3d",
        json!({
            "board": pcb.to_string_lossy(),
            "output": pcb_step.to_string_lossy(),
            "format": "step",
            "include_unspecified": false
        }),
    );
    assert!(pcb_step.is_file());

    let bom = proj.join("bom.csv");
    p.tool(
        "export_bom",
        json!({
            "schematic": sch.to_string_lossy(),
            "output": bom.to_string_lossy(),
            "exclude_dnp": true
        }),
    );
    assert!(bom.is_file());

    let sch_netlist = proj.join("schematic.net");
    p.tool(
        "export_netlist",
        json!({
            "board": sch.to_string_lossy(),
            "output": sch_netlist.to_string_lossy(),
            "format": "kicad"
        }),
    );
    assert!(sch_netlist.is_file());

    let ipc_netlist = proj.join("board.d356");
    p.tool(
        "export_netlist",
        json!({
            "board": pcb.to_string_lossy(),
            "output": ipc_netlist.to_string_lossy(),
            "format": "ipc"
        }),
    );
    assert!(ipc_netlist.is_file());

    let positions = proj.join("positions.csv");
    p.tool(
        "export_position_file",
        json!({
            "board": pcb.to_string_lossy(),
            "output": positions.to_string_lossy(),
            "format": "csv",
            "side": "both",
            "units": "mm"
        }),
    );
    assert!(positions.is_file());

    p.load("manufacturing");
    let manufacturing_dir = proj.join("manufacturing");
    let package = body(&p.tool(
        "export_manufacturing_package",
        json!({
            "board": pcb.to_string_lossy(),
            "schematic": sch.to_string_lossy(),
            "output_dir": manufacturing_dir.to_string_lossy(),
            "fab_house": "generic",
            "include_assembly": true
        }),
    ));
    assert_eq!(package["warnings"].as_array().map(Vec::len), Some(0));
    assert!(manufacturing_dir.join("bom.csv").is_file());
    assert!(manufacturing_dir.join("positions.csv").is_file());
    assert!(manufacturing_dir.join("gerbers").is_dir());

    let drc = body(&p.tool("run_drc", json!({"board": pcb.to_string_lossy()})));
    assert!(
        drc.get("errors").is_some()
            || drc.get("violations").is_some()
            || drc.get("summary").is_some(),
        "unexpected DRC shape: {drc}"
    );

    eprintln!(
        "E2E OK: project/templates created, wired, ERC'd, {produced} Gerbers, \
         PDF/SVG/STEP/BOM/netlists/positions/manufacturing package exported, DRC'd"
    );
}
