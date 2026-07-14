---
name: kicad-pcb
description: |
  Workflow skill for KiCAD PCB layout and routing via MCP tools. Triggers on: "layout the board",
  "route traces", "PCB", "place footprints", "copper pour", "board outline", "differential pair",
  "board setup", "track width", "via", "zone", "design rules", "stackup", "silkscreen".
argument-hint: "[layout task]"
---

# KiCAD PCB Layout Workflow

Use Konnect tools for every `.kicad_pcb` modification. Do not patch board files with generic text-writing tools.

## Runtime model

Footprint placement and most trace operations use KiCAD's live PCB IPC API. Open the intended board in PCB Editor with IPC enabled before using those tools. File-based board setup, netclass, net, and copper-pour operations do not all require the GUI; read each tool result and never assume a live edit succeeded.

If IPC discovery fails, confirm that the board is open in PCB Editor and use Konnect's `register-kicad` command when the Linux socket is outside the default discovery path.

## Toolsets

Start with `get_active_toolsets`, then load:

```text
load_toolset("pcb_board")       # outline, layers, holes, text, zones
load_toolset("pcb_components")  # live footprint placement/query
load_toolset("pcb_routing")     # nets, traces, vias, pours, netclasses
load_toolset("verification")    # DRC, rules, clearance, UI helpers
load_toolset("pcb_export")      # production and interchange exports
```

There are no `pcb_layout`, `pcb_zones`, `pcb_query`, `pcb_batch`, or `pcb_design_rules` toolsets.

## Layout sequence

1. Define and verify the closed `Edge.Cuts` outline with `set_board_size` or `add_board_outline`.
2. Confirm footprints and nets already exist. Konnect does not expose a KiCad 10 `update_pcb_from_schematic` command; perform that synchronization in KiCAD when needed.
3. Place footprints with `place_component`, `move_component`, `rotate_component`, `align_components`, or `place_component_array`.
4. Inspect pads and nets with `get_component_pads`, `get_pad_position`, and `get_nets_list`.
5. Define rules with `create_netclass`/`assign_net_to_class` or `set_design_rules`.
6. Route, then add/refill copper zones.
7. Run `run_drc`, review every violation, save the live PCB with `save_project`, and re-run DRC after fixes.

## Routing capabilities and limits

- `route_trace` creates one trace between explicit start/end coordinates on one copper layer.
- `route_pad_to_pad` looks up two pads and creates a same-layer straight or L-shaped route. It does not insert vias or perform obstacle avoidance.
- `add_via` explicitly creates a through-hole via. Add and connect layer-specific traces yourself when changing layers.
- `route_differential_pair` creates two parallel traces at the requested gap. It does not tune length, avoid obstacles, or prove impedance.
- `query_traces`, `modify_trace`, and `delete_trace` support inspection and iteration.

Do not describe these tools as an autorouter. For a full autoroute flow, use the `integration` toolset's `autoroute` only after `check_freerouting` succeeds, and validate its imported result with DRC.

## Copper zones

Use `pcb_board.add_zone` or `pcb_routing.add_copper_pour` to create a zone. Use `pcb_export.refill_zones` to refill through a running KiCAD PCB Editor IPC session. Refill after geometry or rule changes and before final DRC/export.

## Verification rules

1. Never edit `.kicad_pcb` directly.
2. Verify the open PCB before live IPC writes.
3. Inspect pad coordinates and net assignments before routing.
4. Treat differential-pair spacing as geometry generation, not impedance or length certification.
5. Run DRC after routing, zone refill, or rule changes.
6. `save_project` saves the live PCB only; file-based tools already persist their own atomic edits.
7. Use correct KiCAD layer names such as `F.SilkS`, `B.SilkS`, and `Edge.Cuts`.
