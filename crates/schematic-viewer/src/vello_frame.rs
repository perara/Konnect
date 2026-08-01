//! Vello frame overlay composition for selections and schematic artwork.

use super::*;

pub(super) fn append_sheet(frame: &mut Scene, sheet: &NativeSheet, transform: Affine) {
    if !sheet.semantic.coverage.is_complete() {
        if let Some(compatibility) = &sheet.compatibility {
            frame.append(compatibility, Some(transform));
            return;
        }
    }
    frame.append(&sheet.rendered, Some(transform));
}

pub(super) fn append_object_artwork(
    frame: &mut Scene,
    sheet: &NativeSheet,
    object: &crate::native_scene::SceneObject,
    transform: Affine,
    palette: Palette,
) {
    let Some(primitives) = sheet
        .semantic
        .primitives
        .get(object.primitive_range.clone())
    else {
        return;
    };
    frame.append(&encode_primitives(primitives, palette), Some(transform));
}

pub(super) fn append_selection(
    frame: &mut Scene,
    sheet: &NativeSheet,
    object: &crate::native_scene::SceneObject,
    transform: Affine,
    scale: f64,
    palette: Palette,
) {
    let Some(primitives) = sheet
        .semantic
        .primitives
        .get(object.primitive_range.clone())
    else {
        return;
    };
    let halo_mm = 3.0 / scale.max(0.001);
    let color = palette.selection.with_alpha(0.76);
    let mut outline = Scene::new();

    for primitive in primitives {
        match primitive {
            Primitive::Line { from, to, style } => outline.stroke(
                &Stroke::new(style.width_mm.max(0.05) + halo_mm),
                Affine::IDENTITY,
                color,
                None,
                &Line::new((from.x, from.y), (to.x, to.y)),
            ),
            Primitive::Polyline {
                points,
                closed,
                style,
                ..
            } => outline.stroke(
                &Stroke::new(style.width_mm.max(0.05) + halo_mm),
                Affine::IDENTITY,
                color,
                None,
                &polyline_path(points, *closed),
            ),
            Primitive::Rect { bounds, style, .. } => outline.stroke(
                &Stroke::new(style.width_mm.max(0.05) + halo_mm),
                Affine::IDENTITY,
                color,
                None,
                &Rect::new(bounds.min_x, bounds.min_y, bounds.max_x, bounds.max_y),
            ),
            Primitive::Circle {
                center,
                radius,
                style,
                ..
            } => outline.stroke(
                &Stroke::new(style.width_mm.max(0.05) + halo_mm),
                Affine::IDENTITY,
                color,
                None,
                &Circle::new((center.x, center.y), *radius),
            ),
            Primitive::Arc {
                start,
                mid,
                end,
                style,
            } => {
                if let Some(arc) = arc_shape(*start, *mid, *end) {
                    outline.stroke(
                        &round_stroke(style.width_mm.max(0.05) + halo_mm),
                        Affine::IDENTITY,
                        color,
                        None,
                        &arc,
                    );
                }
            }
            Primitive::Bezier { points, style } => {
                let mut path = BezPath::new();
                if let Some(first) = points.first() {
                    path.move_to((first.x, first.y));
                    for controls in points[1..].chunks_exact(3) {
                        path.curve_to(
                            (controls[0].x, controls[0].y),
                            (controls[1].x, controls[1].y),
                            (controls[2].x, controls[2].y),
                        );
                    }
                    outline.stroke(
                        &Stroke::new(style.width_mm.max(0.05) + halo_mm),
                        Affine::IDENTITY,
                        color,
                        None,
                        &path,
                    );
                }
            }
            Primitive::Text { .. } => {}
        }
    }
    frame.append(&outline, Some(transform));
}

#[cfg(test)]
pub(super) fn arc_points(
    start: SchPoint,
    mid: SchPoint,
    end: SchPoint,
    segments: usize,
) -> Vec<SchPoint> {
    let Some(arc) = arc_shape(start, mid, end) else {
        return vec![start, mid, end];
    };
    (0..=segments)
        .map(|index| {
            let angle = arc.start_angle + arc.sweep_angle * index as f64 / segments as f64;
            SchPoint {
                x: arc.center.x + arc.radii.x * angle.cos(),
                y: arc.center.y + arc.radii.y * angle.sin(),
            }
        })
        .collect()
}

pub(super) fn arc_shape(start: SchPoint, mid: SchPoint, end: SchPoint) -> Option<KurboArc> {
    let determinant =
        2.0 * (start.x * (mid.y - end.y) + mid.x * (end.y - start.y) + end.x * (start.y - mid.y));
    if determinant.abs() < 1e-9 {
        return None;
    }
    let start_sq = start.x * start.x + start.y * start.y;
    let mid_sq = mid.x * mid.x + mid.y * mid.y;
    let end_sq = end.x * end.x + end.y * end.y;
    let center_x =
        (start_sq * (mid.y - end.y) + mid_sq * (end.y - start.y) + end_sq * (start.y - mid.y))
            / determinant;
    let center_y =
        (start_sq * (end.x - mid.x) + mid_sq * (start.x - end.x) + end_sq * (mid.x - start.x))
            / determinant;
    let radius = (start.x - center_x).hypot(start.y - center_y);
    let start_angle = (start.y - center_y).atan2(start.x - center_x);
    let mid_angle = (mid.y - center_y).atan2(mid.x - center_x);
    let end_angle = (end.y - center_y).atan2(end.x - center_x);
    let ccw = positive_angle(end_angle - start_angle);
    let mid_ccw = positive_angle(mid_angle - start_angle);
    let sweep = if mid_ccw <= ccw + 1e-9 {
        ccw
    } else {
        ccw - std::f64::consts::TAU
    };
    Some(KurboArc::new(
        (center_x, center_y),
        (radius, radius),
        start_angle,
        sweep,
        0.0,
    ))
}

pub(super) fn positive_angle(angle: f64) -> f64 {
    angle.rem_euclid(std::f64::consts::TAU)
}
