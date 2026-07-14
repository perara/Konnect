# Pre-Fabrication Design Checklist

Adapt this checklist to the circuit requirements, component datasheets, selected stackup, manufacturer, enclosure, and assembly process. A checked box should have evidence; generic rules of thumb are not substitutes for device requirements.

## Schematic

- [ ] Power rails, voltage/current margins, sequencing, reverse/back-power paths, and protection were reviewed.
- [ ] Decoupling and bulk capacitance values/topology follow each device's datasheet and load-transient needs.
- [ ] Regulators, references, crystals, analog networks, terminations, and bias components follow authoritative designs/calculations.
- [ ] External interfaces have requirement-appropriate ESD/EMC and fault protection.
- [ ] Inputs have defined states when required; intentional unused pins are handled per the datasheet and marked no-connect where appropriate.
- [ ] References, values, footprints, pin mappings, polarity, ratings, and sourcing fields were checked.
- [ ] `find_orphan_items`, `find_shorted_nets`, `find_single_pin_nets`, `validate_wire_connections`, and `validate_component_connections` findings were reviewed.
- [ ] Formal `run_erc` results are resolved or explicitly waived.

Single-pin nets and unconnected pins are review prompts, not automatically defects; intentional test points, NC pins, and hierarchical intent require context.

## PCB

- [ ] Board outline, cutouts, dimensions, mounting, connectors, keepouts, and enclosure fit were checked.
- [ ] Footprints and pad numbering match datasheets and physical parts.
- [ ] Placement satisfies electrical, thermal, mechanical, courtyard, rework, and assembly needs.
- [ ] Routing is complete and current/impedance/return-path requirements were checked against the real stackup.
- [ ] Differential pairs were inspected for topology, reference plane, spacing, skew/length, and discontinuities; tool-generated parallel geometry alone is not proof.
- [ ] Zones are refilled and plane continuity/thermal relief/copper islands were reviewed.
- [ ] Drill, annular ring, clearances, mask, paste, silkscreen, edge, and special-process geometry meet the selected manufacturer's current rules.
- [ ] Polarity/pin-1 markings, designators, fiducials, tooling needs, and test access suit the assembly plan.
- [ ] Formal `run_drc` results are resolved or explicitly waived.

## Konnect coverage

| Check | Tool or evidence |
|-------|------------------|
| Structural schematic issues | `sch_analysis` and `sch_batch` validation tools |
| Formal electrical rules | `sch_export.run_erc` |
| Formal board rules | `verification.run_drc` |
| Heuristic circuit/DFM prompts | `design_review.run_design_review` |
| File-level manufacturing pre-flight | `manufacturing.validate_for_manufacturing` |
| ESD, SI/PI, thermal, mechanical, and datasheet compliance | Manual/independent engineering evidence |

`run_design_review` does not include formal ERC/DRC and does not verify the manual engineering items in the last row.
