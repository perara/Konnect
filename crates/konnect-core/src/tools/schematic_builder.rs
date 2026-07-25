//! SchematicBuilder — Structured writer for .kicad_sch files.
//!
//! KiCAD 10's parser requires elements in a specific order. This builder
//! parses an existing schematic into sections, allows adding elements to
//! the correct section, and serializes back with guaranteed valid ordering.
//!
//! Element order (enforced by this builder):
//!   1. Header (version, generator, uuid, paper, title_block)
//!   2. lib_symbols (library symbol definitions)
//!   3. Junctions, no_connects
//!   4. Wires, buses, bus_entries
//!   5. Text annotations
//!   6. Labels (net_label, global_label, hierarchical_label)
//!   7. Symbol instances (ALWAYS LAST)

use konnect_sexp::writer::{find_direct_child_blocks, write_atomic};
use std::path::Path;
use tracing::debug;

/// Structured representation of a .kicad_sch file.
/// Each section holds raw S-expression strings that are written in order.
pub struct SchematicBuilder {
    /// Everything before lib_symbols: version, generator, uuid, paper, title_block
    pub header: String,
    /// Contents inside (lib_symbols ...) — each entry is a complete (symbol "Lib:Name" ...) block
    pub lib_symbols: Vec<String>,
    /// Junction dots
    pub junctions: Vec<String>,
    /// No-connect flags
    pub no_connects: Vec<String>,
    /// Wire segments
    pub wires: Vec<String>,
    /// Bus segments
    pub buses: Vec<String>,
    /// Bus entry points
    pub bus_entries: Vec<String>,
    /// Text annotations
    pub texts: Vec<String>,
    /// Net labels (net_label, global_label, hierarchical_label)
    pub labels: Vec<String>,
    /// Symbol instances — ALWAYS serialized last
    pub symbols: Vec<String>,
}

fn sexp_tag(block: &str) -> &str {
    let Some(after_open) = block.strip_prefix('(') else {
        return "";
    };
    let end = after_open
        .find(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .unwrap_or(after_open.len());
    &after_open[..end]
}

impl Default for SchematicBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SchematicBuilder {
    /// Create an empty schematic with KiCAD 10 header.
    pub fn new() -> Self {
        let uuid = konnect_sexp::writer::new_uuid();
        SchematicBuilder {
            header: format!(
                "(kicad_sch\n\t(version 20250610)\n\t(generator \"konnect\")\n\t(generator_version \"10.0\")\n\t(uuid \"{}\")\n\t(paper \"A4\")",
                uuid
            ),
            lib_symbols: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            wires: Vec::new(),
            buses: Vec::new(),
            bus_entries: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
            symbols: Vec::new(),
        }
    }

    /// Parse an existing .kicad_sch file into structured sections.
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::parse(&content)
    }

    /// Parse schematic content into structured sections.
    pub fn parse(content: &str) -> anyhow::Result<Self> {
        let mut builder = SchematicBuilder {
            header: String::new(),
            lib_symbols: Vec::new(),
            junctions: Vec::new(),
            no_connects: Vec::new(),
            wires: Vec::new(),
            buses: Vec::new(),
            bus_entries: Vec::new(),
            texts: Vec::new(),
            labels: Vec::new(),
            symbols: Vec::new(),
        };

        let root_children = find_direct_child_blocks(content, "kicad_sch");
        let first_payload = root_children
            .iter()
            .find(|&&(start, end)| {
                !matches!(
                    sexp_tag(&content[start..end]),
                    "version"
                        | "generator"
                        | "generator_version"
                        | "uuid"
                        | "paper"
                        | "title_block"
                )
            })
            .map(|&(start, _)| start)
            .unwrap_or_else(|| content.rfind(')').unwrap_or(content.len()));
        builder.header = content[..first_payload].trim_end().to_string();

        for (start, end) in root_children {
            let block = &content[start..end];
            match sexp_tag(block) {
                "lib_symbols" => {
                    for (sym_start, sym_end) in find_direct_child_blocks(block, "lib_symbols") {
                        let symbol = &block[sym_start..sym_end];
                        if sexp_tag(symbol) == "symbol" {
                            builder.lib_symbols.push(symbol.trim().to_string());
                        }
                    }
                }
                "junction" => builder.junctions.push(block.to_string()),
                "no_connect" => builder.no_connects.push(block.to_string()),
                "wire" => builder.wires.push(block.to_string()),
                "bus" => builder.buses.push(block.to_string()),
                "bus_entry" => builder.bus_entries.push(block.to_string()),
                "text" => builder.texts.push(block.to_string()),
                "net_label" | "global_label" | "hierarchical_label" | "label" => {
                    builder.labels.push(block.to_string())
                }
                "symbol" => builder.symbols.push(block.to_string()),
                "version" | "generator" | "generator_version" | "uuid" | "paper"
                | "title_block" => {}
                elem_type => {
                    debug!(
                        "[SchematicBuilder] Unknown element type: '{}', block len: {}",
                        elem_type,
                        block.len()
                    );
                    builder.texts.push(block.to_string());
                }
            }
        }

        Ok(builder)
    }

    /// Add a lib_symbol definition (if not already present).
    pub fn add_lib_symbol(&mut self, definition: &str) {
        // Check if already present by matching the symbol name
        if let Some(name_start) = definition.find("(symbol \"") {
            let after = &definition[name_start + 9..];
            if let Some(name_end) = after.find('"') {
                let name = &after[..name_end];
                if self
                    .lib_symbols
                    .iter()
                    .any(|s| s.contains(&format!("(symbol \"{}\"", name)))
                {
                    return; // Already present
                }
            }
        }
        self.lib_symbols.push(definition.to_string());
    }

    /// Add a wire segment.
    pub fn add_wire(&mut self, sexp: &str) {
        self.wires.push(sexp.trim().to_string());
    }

    /// Add a junction.
    pub fn add_junction(&mut self, sexp: &str) {
        self.junctions.push(sexp.trim().to_string());
    }

    /// Add a no-connect flag.
    pub fn add_no_connect(&mut self, sexp: &str) {
        self.no_connects.push(sexp.trim().to_string());
    }

    /// Add a label (net_label, global_label, hierarchical_label).
    pub fn add_label(&mut self, sexp: &str) {
        self.labels.push(sexp.trim().to_string());
    }

    /// Add a text annotation.
    pub fn add_text(&mut self, sexp: &str) {
        self.texts.push(sexp.trim().to_string());
    }

    /// Add a symbol instance (always serialized last).
    pub fn add_symbol(&mut self, sexp: &str) {
        self.symbols.push(sexp.trim().to_string());
    }

    /// Serialize to a valid .kicad_sch string with correct element ordering.
    /// (Deliberately an inherent method — this is a file serialization, not a
    /// human-readable Display.)
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&self) -> String {
        let mut out = String::new();

        // Header
        out.push_str(&self.header);
        out.push('\n');

        // lib_symbols
        out.push_str("\t(lib_symbols\n");
        for sym in &self.lib_symbols {
            out.push_str("\t\t");
            out.push_str(sym);
            out.push('\n');
        }
        out.push_str("\t)\n");

        // Junctions
        for item in &self.junctions {
            out.push_str("  ");
            out.push_str(item);
            out.push('\n');
        }

        // No-connects
        for item in &self.no_connects {
            out.push_str("  ");
            out.push_str(item);
            out.push('\n');
        }

        // Wires
        for item in &self.wires {
            out.push_str("  ");
            out.push_str(item);
            out.push('\n');
        }

        // Buses
        for item in &self.buses {
            out.push_str("  ");
            out.push_str(item);
            out.push('\n');
        }

        // Bus entries
        for item in &self.bus_entries {
            out.push_str("  ");
            out.push_str(item);
            out.push('\n');
        }

        // Text
        for item in &self.texts {
            out.push_str("  ");
            out.push_str(item);
            out.push('\n');
        }

        // Labels
        for item in &self.labels {
            out.push_str("  ");
            out.push_str(item);
            out.push('\n');
        }

        // Symbols — ALWAYS LAST
        for item in &self.symbols {
            out.push_str("  ");
            out.push_str(item);
            out.push('\n');
        }

        // Close the root kicad_sch
        out.push_str(")\n");

        out
    }

    /// Write to file atomically (write to .tmp, fsync, rename).
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let content = self.to_string();
        write_atomic(path, &content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_builder_produces_valid_structure() {
        let builder = SchematicBuilder::new();
        let output = builder.to_string();
        assert!(output.starts_with("(kicad_sch"));
        assert!(output.contains("(version 20250610)"));
        assert!(output.contains("(lib_symbols"));
        assert!(output.ends_with(")\n"));
    }

    #[test]
    fn elements_are_ordered_correctly() {
        let mut builder = SchematicBuilder::new();
        // Add in wrong order — builder should serialize in correct order
        builder.add_symbol("(symbol (lib_id \"Device:R\") (at 100 100 0) (uuid \"sym1\"))");
        builder
            .add_wire("(wire (pts (xy 100 90) (xy 100 100)) (stroke (width 0)) (uuid \"wire1\"))");
        builder.add_label("(net_label \"VCC\" (at 100 85 0) (uuid \"label1\"))");
        builder.add_junction("(junction (at 100 90) (uuid \"junc1\"))");

        let output = builder.to_string();

        // Verify order: junction < wire < label < symbol
        let junc_pos = output.find("(junction").unwrap();
        let wire_pos = output.find("(wire").unwrap();
        let label_pos = output.find("(net_label").unwrap();
        let sym_pos = output.find("(symbol").unwrap();

        assert!(junc_pos < wire_pos, "junction should come before wire");
        assert!(wire_pos < label_pos, "wire should come before label");
        assert!(label_pos < sym_pos, "label should come before symbol");
    }

    #[test]
    fn parse_and_reserialize_preserves_elements() {
        let input = r#"(kicad_sch
	(version 20250610)
	(generator "konnect")
	(generator_version "10.0")
	(paper "A4")
	(lib_symbols
	)
  (wire (pts (xy 100 90) (xy 100 100)) (stroke (width 0) (type default)) (uuid "w1"))
  (net_label "VCC" (at 100 85 0) (effects (font (size 1.27 1.27))) (uuid "l1"))
  (symbol
    (lib_id "Device:R")
    (at 100 100 0)
    (uuid "s1")
    (property "Reference" "R1" (at 100 96 0) (effects (font (size 1.27 1.27))))
    (instances (project "" (path "/" (reference "R1") (unit 1))))
  )
)
"#;

        let builder = SchematicBuilder::parse(input).unwrap();
        assert_eq!(builder.wires.len(), 1);
        assert_eq!(builder.labels.len(), 1);
        assert_eq!(builder.symbols.len(), 1);

        let output = builder.to_string();
        assert!(output.contains("(wire"));
        assert!(output.contains("(net_label"));
        assert!(output.contains("(symbol"));
    }

    #[test]
    fn parse_handles_eeschema_tab_indentation_without_promoting_nested_symbols() {
        let input = "(kicad_sch\n\t(version 20260306)\n\t(generator \"eeschema\")\n\t(generator_version \"10.0\")\n\t(uuid \"root\")\n\t(paper \"A4\")\n\t(lib_symbols\n\t\t(symbol \"Device:R\"\n\t\t\t(symbol \"R_0_1\" (rectangle (start -1 -1) (end 1 1)))\n\t\t)\n\t)\n\t(wire\n\t\t(pts (xy 10 10) (xy 20 10))\n\t\t(uuid \"wire\")\n\t)\n\t(symbol\n\t\t(lib_id \"Device:R\")\n\t\t(at 20 10 0)\n\t\t(uuid \"placed\")\n\t)\n\t(sheet_instances (path \"/\" (page \"1\")))\n)\n";

        let builder = SchematicBuilder::parse(input).unwrap();
        assert_eq!(builder.lib_symbols.len(), 1);
        assert_eq!(builder.wires.len(), 1);
        assert_eq!(builder.symbols.len(), 1);
        assert!(
            builder
                .texts
                .iter()
                .any(|block| block.starts_with("(sheet_instances")),
            "unknown root items must be retained, not swallowed into the header"
        );

        let output = builder.to_string();
        assert_eq!(output.matches("(lib_id \"Device:R\")").count(), 1);
        assert!(konnect_sexp::parse_sexp(&output).is_ok());
    }

    #[test]
    fn parse_header_only_schematic_does_not_duplicate_root_close() {
        let input =
            "(kicad_sch\n\t(version 20260306)\n\t(generator \"eeschema\")\n\t(uuid \"root\")\n)\n";
        let output = SchematicBuilder::parse(input).unwrap().to_string();
        assert!(konnect_sexp::parse_sexp(&output).is_ok());
        assert_eq!(output.matches("(kicad_sch").count(), 1);
    }
}
