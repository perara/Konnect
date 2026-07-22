# Common Wiring Patterns

## Pattern 1: Decoupling Capacitor
```
        +3V3 (power symbol)
         |
    ┌────┤
    │    C1 100nF
    │    │
    │    GND (power symbol)
    │
    U1 VCC pin
```
**Tools**: `add_schematic_component` (cap) → inspect exact endpoints → `connect_pins` (cap pin 1 to IC VCC) → `add_power_symbol` at the capacitor's other endpoint for GND. Add the rail symbol/connection required by the specific circuit topology.

## Pattern 2: Pull-up Resistor
```
    +3V3
     |
     R1 4.7k
     |
     ├──── net label "SDA"
     |
    IC pin
```
**Tools**: `add_schematic_component` (R, datasheet-appropriate value) → inspect endpoints → `add_power_symbol` at the supply-side endpoint → `connect_to_net` using the signal-side `pin_x`/`pin_y` and net `SDA`.

## Pattern 3: Voltage Divider
```
    VIN ──── R1 ──┬── R2 ──── GND
                  |
              net label "FB"
```
**Tools**: Place R1 and R2 → `connect_pins` (R1 pin 2 to R2 pin 1) → `add_schematic_net_label` at the junction → `connect_to_net` using R1 pin 1's coordinates → `add_power_symbol` at R2 pin 2's coordinates.

## Pattern 4: LED with Current Limiting Resistor
```
    GPIO_OUT ──── R1 330Ω ──── D1 LED ──── GND
```
**Tools**: Place R1 (calculate its value from voltage, LED drop, and target current) and D1 → `connect_pins` → `connect_to_net` using R1 pin 1's coordinates → `add_power_symbol` at the LED cathode endpoint.

## Pattern 5: Crystal Oscillator
```
         ┌── C1 ──┐
    OSC_IN ──┤     ├── GND
         │  XTAL  │
    OSC_OUT ─┤     ├── GND
         └── C2 ──┘
```
**Tools**: Use the MCU/crystal reference design and calculated load values. Place the crystal and capacitors → inspect endpoints → `connect_pins` for the crystal/capacitor branches → `add_power_symbol` at capacitor ground endpoints → `connect_to_net` using the crystal endpoint coordinates for `OSC_IN` and `OSC_OUT`.

## Pattern 6: USB Type-C Power Sink (5V only)
```
    VBUS ────────── +5V
    CC1 ──── R 5.1k ──── GND
    CC2 ──── R 5.1k ──── GND
    GND ─────────── GND
    D+ ──────────── USB_DP
    D- ──────────── USB_DM
```
**Tools**: Use `search_templates("usb_c_5v_sink")` first — the templates toolset has this pre-built.

## Wiring Decision Guide

| Scenario | Tool | Why |
|----------|------|-----|
| Two specific pins on two components | `connect_pins` | Auto-routes, knows pin coordinates |
| Known endpoint coordinates to a named net | `connect_to_net` | Adds a stub + label |
| Known endpoint coordinate to a power rail | `add_power_symbol` | Places a global power symbol |
| Multiple pins to same net | `batch_connect_to_net` | Single atomic write |
| Two points already known by coordinates | `add_schematic_connection` | Auto H+V routing |
| Simple horizontal/vertical wire | `add_wire` | Manual, use sparingly |

## Net Label Types

| Type | Scope | When to use |
|------|-------|-------------|
| Net label (`net_label`) | Single sheet | Local signals within one schematic sheet |
| Global label (`global_label`) | All sheets | Signals shared across hierarchical sheets |
| Hierarchical label (`hierarchical_label`) | Sheet boundary | Interface pins on hierarchical sheet symbols |
| Power symbol | Global | Power rails (+3V3, GND, VCC) |

## Spacing Guidelines

- Components: minimum 5.08mm (4 grid units) between component bodies
- Labels: place at wire endpoints, not floating in space
- Power symbols: directly on component power pins when possible
- Junctions: added automatically by Konnect at T-intersections
