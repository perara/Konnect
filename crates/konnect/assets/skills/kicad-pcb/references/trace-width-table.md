# Trace and Via Sizing Reference

Konnect can apply geometry and design rules, but it does not calculate certified current capacity or controlled impedance.

## Current-carrying traces

Size traces from a recognized calculation method and the real construction: internal/external layer, finished copper thickness, allowable temperature rise, ambient and enclosure conditions, trace length, nearby copper, and manufacturing tolerance. Validate high-current paths and connectors as a system. Use planes/pours and via arrays only after checking current distribution and thermal behavior.

## Controlled impedance

Obtain the manufacturer's stackup and use its field solver/calculator for the chosen process. Width and pair gap alone do not determine impedance; dielectric height/permittivity, copper thickness, solder mask, reference planes, etch tolerance, and coupling all matter.

`route_differential_pair` creates parallel geometry at a requested gap. It does not match lengths, avoid obstacles, or certify impedance. Inspect and tune the result in KiCAD, then run DRC and follow the manufacturer's impedance workflow.

## Vias and clearances

Choose drill, pad, annular ring, and antipad from the selected fabrication rules. Check finished-hole versus tool size, plating, aspect ratio, via-in-pad processing, current capacity, and thermal needs. Encode the selected limits with `set_design_rules`/`create_netclass`, verify with `get_design_rules`, and run `run_drc`.
