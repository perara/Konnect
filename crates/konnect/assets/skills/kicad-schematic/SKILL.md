---
name: kicad-schematic
description: |
  Workflow skill for KiCAD schematic design via MCP tools. Triggers on: "design a circuit",
  "add a component", "wire up", "connect pins", "build schematic", "place resistor",
  "place cap", "place IC", "schematic", "add symbol", "net label", "power rail".
argument-hint: "[circuit description or task]"
---

# KiCAD Schematic Design Workflow

Use Konnect tools for every `.kicad_sch` modification. Do not patch KiCAD source files with generic text-writing tools.

## Load the right toolsets

Start with `get_active_toolsets`, then load only what the task needs:

```text
load_toolset("sch_components")  # create/place/edit/query symbols
load_toolset("sch_wiring")      # wires, labels, power symbols, pin connections
load_toolset("sch_analysis")    # connectivity and structural inspection
load_toolset("sch_batch")       # bulk edits plus connection validation
load_toolset("sch_export")      # SVG/PDF/netlist/ERC
load_toolset("library")         # symbol/footprint search and metadata
load_toolset("sch_hierarchy")   # hierarchical sheets and sheet pins
```

There are no `sch_library` or `sch_query` toolsets. Library lookup is in `library`; schematic queries are split across `sch_components` and `sch_analysis`.

## Safe build sequence

1. Use `search_symbols` and `get_symbol_info`; never guess a library ID or pin number.
2. Place with `add_schematic_component`, or use the batch tools for repetitive work.
3. Verify references and exact pin endpoints with `list_schematic_components`, `get_schematic_component`, or `get_schematic_pin_locations`.
4. Wire using the method that matches the information available.
5. Run structural validation and formal ERC.
6. Re-query the edited design to confirm the intended connectivity.

## Connection methods

- `connect_pins` accepts a schematic path plus `ref1`, `pin1`, `ref2`, and `pin2`. It looks up both endpoints and creates an orthogonal connection.
- `connect_to_net` accepts a schematic path, exact `pin_x`/`pin_y`, and `net`, with optional direction, stub length, and label type. Obtain the endpoint first; it does not accept a component reference and pin number.
- `batch_connect_to_net` accepts a schematic path, `net_name`, and pin objects containing `reference` and `pin_number`.
- `add_power_symbol` accepts a schematic path, `power_net`, and placement `x`/`y`. Place it on a known pin endpoint; it does not accept a component reference and pin number.
- `add_wire` is for an explicit horizontal or vertical segment. Use `connect_pins` when references are available.
- `add_schematic_net_label` adds net, global, or hierarchical labels at an explicit location.
- `add_no_connect` marks an intentionally unused endpoint.

Use direct pin-to-pin wires for nearby point-to-point signals and named labels for shared or distant nets. Do not infer electrical correctness from matching names alone; confirm with `get_net_connections`, `get_pin_connections`, or `get_component_nets`.

## Verification

Run the checks that match the change:

```text
find_orphan_items
find_shorted_nets
find_single_pin_nets
validate_wire_connections
validate_component_connections
run_erc
```

`run_erc` requires `kicad-cli`. Structural checks use the schematic file directly. `annotate_schematic` atomically assigns sequential designators and does not require `kicad-cli`.

Schematic write tools persist their changes atomically as part of each call. Do not call `save_project` for schematic edits: that tool saves the currently open PCB through KiCAD IPC.

## Practical rules

1. Never edit `.kicad_sch` directly.
2. Search before placing and inspect pins before connecting.
3. Use the 1.27 mm schematic grid unless the project requires otherwise.
4. Prefer batch tools for repetitive work so a logical change uses one atomic write.
5. Mark intentional unused pins explicitly.
6. Run structural checks after edits and ERC before declaring a design complete.
7. Report heuristic findings as heuristics; do not claim electrical behavior that the tools did not verify.
