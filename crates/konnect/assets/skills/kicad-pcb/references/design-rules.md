# PCB Design Rules

Fabrication limits are process-specific and change over time. Obtain the current rules for the selected manufacturer, layer stack, copper weight, finish, via technology, impedance service, and assembly option before setting the board constraints.

## Konnect setup

- Use `set_design_rules` for board-level clearance, trace width, and via-size constraints.
- Use `get_design_rules` to verify what is stored in the board.
- Use `create_netclass` to define named routing rules and `assign_net_to_class` to map nets to them.
- Use `set_layer_constraints` when a layer requires a distinct constraint.
- Run formal `run_drc` after any rule or routing change.

## Rule-selection checklist

Record the source and selected process for every constraint:

- copper-to-copper clearance and minimum track width;
- copper-to-board-edge clearance;
- finished hole, drill, via diameter, and annular ring;
- slot, castellated-hole, and plated-edge requirements;
- solder-mask dam/sliver and paste rules;
- silkscreen line/text capability;
- courtyard, component-to-edge, and assembly clearances;
- differential impedance geometry supplied by the actual stackup calculator.

Do not copy generic USB, RF, power, or “high-speed” widths into production. Current capacity and impedance depend on stackup, copper thickness, temperature rise, reference planes, dielectric properties, and fabrication tolerances.
