---
name: kicad-review
description: |
  Design review and validation workflow for KiCAD projects via MCP tools. Triggers on: "review my design",
  "check for errors", "audit", "DRC", "ERC", "find problems", "design review", "is this ready",
  "validate", "check my schematic", "check my PCB", "what's wrong", "run checks", "pre-fab review".
argument-hint: "[what to review]"
---

# KiCAD Design Review Workflow

A credible review separates deterministic checks, Konnect heuristics, and engineering judgments. Never report an item as verified when no tool or evidence checked it.

## Toolsets

```text
load_toolset("sch_analysis")
load_toolset("sch_batch")
load_toolset("sch_export")
load_toolset("verification")
load_toolset("design_review")
load_toolset("manufacturing")  # only for a pre-fab review
```

There are no `sch_query`, `pcb_query`, or `audit_esd_protection` tools.

## Review sequence

### 1. Structural schematic checks

Run the checks relevant to the design:

```text
find_orphan_items
find_shorted_nets
find_single_pin_nets
validate_wire_connections
validate_component_connections
check_schematic_overlaps
```

Inspect reported nets and pins with `get_net_connections`, `get_pin_connections`, `get_component_nets`, and `get_connected_items` before classifying a finding.

### 2. Formal KiCAD checks

- `sch_export.run_erc` runs KiCAD's Electrical Rules Check on the schematic.
- `verification.run_drc` runs KiCAD's Design Rules Check on the board.
- `pcb_export.get_drc_violations` exposes the same board check in an export-oriented report.

Formal ERC/DRC errors are not included automatically in `run_design_review`.

### 3. Heuristic audits

`run_design_review` combines:

- `audit_decoupling`: flags IC power nets with no capacitor; it does not verify capacitor value or physical PCB distance;
- `audit_connections`: pattern-based checks for selected pull-up, LED-resistor, floating-input, and output-sharing cases;
- `audit_power_rails`: heuristic bulk-capacitance, test-point, and regulator-output checks;
- `check_bom_health`: missing value, footprint, MPN, and LCSC fields only;
- `audit_manufacturing`: optional file-level outline, silkscreen, trace-width, and component-side heuristics.

These audits do not prove ESD protection, impedance, thermal behavior, part lifecycle, stock, current capacity, voltage compatibility, or general circuit correctness.

### 4. Engineering review

Use the schematic, datasheets, PCB geometry, and user requirements to evaluate anything beyond the implemented checks: voltage/current margins, decoupling values and placement, signal integrity, ESD/EMC protection, thermal design, footprints, mechanical clearances, testability, sourcing, and assembly orientation. Mark these as manually reviewed or unverified and cite the evidence used.

## Reporting

For every finding provide:

- severity and whether it came from formal KiCAD, a Konnect heuristic, or manual reasoning;
- exact component, pin, net, coordinate, or rule when available;
- the evidence/tool result;
- a specific correction or follow-up;
- verification performed after any fix.

Use verdicts conservatively:

- **Not ready**: unresolved formal errors or credible critical issues.
- **Ready with waivers**: no unresolved blockers and every waiver is explicit.
- **Ready for the next gate**: automated checks pass, but external gates such as fab upload preview or hardware validation remain.

Never use “fully verified” or “ready for fab” solely because `run_design_review` returned no findings.

## Convergence rule

After fixes, repeat the affected structural checks, ERC/DRC, and audits. Continue until there are no actionable findings or the remaining items are documented external/manual blockers.
