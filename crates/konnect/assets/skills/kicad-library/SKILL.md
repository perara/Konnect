---
name: kicad-library
description: |
  Library management workflow for KiCAD — creating symbols, footprints, and managing libraries
  via MCP tools. Triggers on: "create a symbol", "make a footprint", "custom component",
  "register library", "find a part", "pin numbering", "new symbol", "new footprint",
  "add to library", "library path", "pad layout".
argument-hint: "[component or library task]"
---

# KiCAD Library Management Workflow

Use the `library` toolset for searches, inspection, creation, editing, and registration. Never patch `.kicad_sym`, `.kicad_mod`, `sym-lib-table`, or `fp-lib-table` with generic text-writing tools.

## Search and identify first

```text
load_toolset("library")
search_symbols
search_footprints
get_symbol_info
get_footprint_info
```

Confirm the exact manufacturer part number, package variant, datasheet revision, and KiCAD library version. Similar names frequently have different pin mappings, exposed pads, pitches, or body dimensions.

## Symbol creation gate

Before `create_symbol`, derive and review:

- every pin number/name and electrical type from the authoritative datasheet;
- hidden/stacked pins and multi-unit behavior if relevant;
- reference prefix, value, footprint link, datasheet URL, description, and sourcing fields;
- a readable logical layout that does not alter the real pin mapping.

Pin numbering is part-specific. Do not apply generic BJT/MOSFET/connector conventions without checking the chosen part.

## Footprint creation gate

Before `create_footprint`, use the manufacturer's recommended land pattern or a documented IPC-based calculation. Review:

- finished pad numbers, shapes, sizes, pitch, row spacing, and exposed/thermal pads;
- solder-mask and paste behavior, including paste segmentation where required;
- courtyard, fabrication outline, body height, polarity/pin-1 marks, and assembly tolerances;
- through-hole drill versus finished-hole dimensions, plating, slots, and annular rings;
- the selected assembler's process constraints.

Do not copy nominal pad sizes from a package-name table. Package labels such as SOIC, QFN, or SOT-23 do not uniquely determine a land pattern.

Use `edit_footprint_pad` only after inspecting the existing footprint and confirming the intended pad number.

## Registration

Use `register_symbol_library` or `register_footprint_library` with the explicit path and intended scope. Prefer project scope for project-specific parts; use global scope only for deliberately shared libraries. Verify registration with the corresponding list/search tools.

## Validation and handoff

1. Re-open the generated symbol/footprint with `get_symbol_info` or `get_footprint_info`.
2. Compare every pin/pad and critical dimension against the datasheet.
3. Confirm symbol pin numbers map exactly to footprint pad numbers.
4. Place a test instance and run ERC/DRC in a representative project.
5. Inspect courtyard, mask, paste, silkscreen, fab outline, 3D model alignment, and pin-1/polarity visually in KiCAD.
6. Record the source document and revision used.

For schematic sourcing fields, load `sch_components` and `design_review`; use `check_bom_health` as a missing-field prompt. It does not verify lifecycle or live stock.
