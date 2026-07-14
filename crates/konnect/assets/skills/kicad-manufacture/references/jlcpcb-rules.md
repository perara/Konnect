# JLCPCB Manufacturing Reference

JLCPCB capabilities, part categories, fees, file headers, and assembly constraints change over time and by selected service. Treat the order portal and current official documentation as authoritative; do not encode remembered limits into a release decision.

## Konnect workflow

1. Run formal schematic ERC and PCB DRC.
2. Run `validate_for_manufacturing` as a supplemental file-level heuristic, not as certification.
3. Use `export_manufacturing_package` with the board, schematic, output directory, and selected fab house.
4. Inspect every generated Gerber/drill/BOM/position artifact.
5. Use `download_jlcpcb_database` before local sourcing searches when an updated cache is needed.
6. Use `search_jlcpcb_parts`, `get_jlcpcb_part`, and `suggest_jlcpcb_alternatives` for planning, then confirm current availability and category in the order portal.
7. Treat `estimate_cost` as a rough estimate, not a quote.
8. Upload the files and review JLCPCB's Gerber and placement previews before ordering.

## BOM and placement review

Confirm the portal's current required column names and formatting. At minimum, verify references/designators, values/comments, footprints/packages, sourcing identifiers, DNP handling, component side, units, origin, coordinates, rotations, and polarity. Do not apply package-wide “rotation offsets”; inspect each assembled orientation against pin 1/polarity and the current preview.

## Design rules

Copy the limits for the exact selected stackup, copper weight, finish, via process, and assembly service into KiCAD's design rules. Re-run DRC after applying them. A value that is valid for one JLCPCB service may be invalid or unnecessarily expensive for another.
