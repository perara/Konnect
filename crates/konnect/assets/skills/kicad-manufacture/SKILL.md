---
name: kicad-manufacture
description: |
  Manufacturing and fabrication workflow for KiCAD projects via MCP tools. Triggers on: "send to fab",
  "order boards", "gerbers", "JLCPCB", "manufacturing", "export for production", "pick and place",
  "assembly files", "generate fabrication outputs", "BOM for fab", "production files", "fab house".
argument-hint: "[fab house or export task]"
---

# KiCAD Manufacturing Workflow

Manufacturing output is a release process: save the design, run formal checks, generate files, inspect the actual archive contents, and keep the reports with the release.

## Toolsets

```text
load_toolset("verification")   # formal DRC and board rules
load_toolset("sch_export")     # formal ERC
load_toolset("design_review")  # heuristic design audits
load_toolset("manufacturing")  # package generation and pre-flight heuristic
load_toolset("pcb_export")     # Gerber/drill/PDF/3D/BOM/position/interchange exports
load_toolset("integration")    # optional local JLCPCB database and datasheets
```

There are no `sch_query`, `jlcpcb`, or `3d` toolsets.

## Pre-flight gate

1. Save the currently open PCB with `save_project` if live IPC edits were made.
2. Run schematic `run_erc` and board `run_drc`; treat formal errors as blockers unless the user documents an intentional waiver.
3. Run `find_orphan_items`, `find_shorted_nets`, and `find_single_pin_nets` when schematic connectivity changed.
4. Run `run_design_review` and `validate_for_manufacturing` as supplemental heuristics.
5. Confirm footprints, values, MPN/LCSC fields, assembly side, board origin, polarity, and component rotations from authoritative project/manufacturer information.

`validate_for_manufacturing` is intentionally limited: it checks file-level outline presence, footprints, minimum trace width, and an obviously completely-unrouted board. It does not replace DRC, BOM review, assembly-rule review, or a fab-house upload preview.

## Generate outputs

For a normal fabrication/assembly bundle, call `export_manufacturing_package` with `board`, `output_dir`, `fab_house`, `include_assembly`, and a `schematic` when a BOM is required. It generates Gerbers, drill output, and optionally BOM and position data.

Use `pcb_export` tools when individual control is required:

- `export_gerber`: board, output directory, optional layers, optional drill generation.
- `export_bom`: schematic, output file, optional DNP exclusion.
- `export_position_file`: board, output file, format, side, and units.
- `export_pdf`/`export_svg`: visual layer plots; options include exact layers and black-and-white mode.
- `export_3d`: supported 3D format plus whether unspecified models are included.
- `export_ipc2581` or `export_odb`: unified interchange formats when the manufacturer accepts them.

## Verify generated artifacts

Do not accept a success message alone. Inspect the output directory and verify:

- expected copper, mask, silkscreen, paste, and `Edge.Cuts` plots are present for the actual stackup;
- plated/non-plated drill outputs expected by the project exist;
- BOM references, values, footprints, DNP handling, and sourcing identifiers are correct;
- pick-and-place units, side, origin, and rotations match the assembly provider's current requirements;
- Gerber/drill alignment and board outline look correct in an independent viewer or the provider's upload preview;
- the package contains no stale outputs from an earlier run.

Provider requirements and prices change. Check the selected manufacturer's current documentation rather than treating values in prompts or prior releases as authoritative.

## Optional sourcing and cost tools

The `integration` toolset can maintain and search a local JLCPCB parts cache (`download_jlcpcb_database`, `search_jlcpcb_parts`, `get_jlcpcb_part`, `suggest_jlcpcb_alternatives`). Cache results are not proof of current stock; verify before ordering. `manufacturing.estimate_cost` is an estimate, not a quote.

## Release rule

Declare a package ready only when formal ERC/DRC results, heuristic findings, generated file inspection, and provider upload checks are all accounted for. Record any waiver explicitly.
