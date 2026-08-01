//! Reload reconciliation, text selection, and UI panel/frame composition.

use super::*;

impl VelloViewer {
    pub(super) fn reconcile_watch_dirs(&mut self) {
        let wanted = self
            .sheets
            .iter()
            .filter_map(|sheet| sheet.file.parent().map(Path::to_path_buf))
            .collect::<HashSet<_>>();
        for directory in wanted.difference(&self.watched_dirs) {
            if let Err(error) = self.watcher.watch(directory, RecursiveMode::NonRecursive) {
                self.status = format!("Watch error: {error}");
            }
        }
        for directory in self.watched_dirs.difference(&wanted) {
            let _ = self.watcher.unwatch(directory);
        }
        self.watched_dirs = wanted;
    }

    pub(super) fn schedule_reload(&mut self, changed: &[PathBuf]) {
        self.schedule_reload_with_origin(changed, false);
    }

    pub(super) fn schedule_external_reload(&mut self, changed: &[PathBuf]) {
        self.schedule_reload_with_origin(changed, true);
    }

    pub(super) fn schedule_reload_with_origin(&mut self, changed: &[PathBuf], external: bool) {
        let generation = self.reload_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let request = ReloadRequest {
            generation,
            root: self.root.clone(),
            changed: changed
                .iter()
                .map(|path| path_key(path))
                .collect::<HashSet<_>>(),
            known: self
                .sheets
                .iter()
                .map(|sheet| path_key(&sheet.file))
                .collect(),
            external,
        };
        if self.reload_tx.send(request).is_err() {
            self.status = "Background reload worker stopped unexpectedly".to_owned();
        }
    }

    pub(super) fn apply_reload_batch(&mut self, batch: ReloadBatch) {
        if batch.generation != self.reload_generation.load(Ordering::Acquire) {
            return;
        }
        let external = batch.external;
        let entries = match batch.entries {
            Ok(entries) => entries,
            Err(error) => {
                self.status = format!("Hierarchy reload failed: {error}");
                return;
            }
        };
        let active_key = self
            .sheets
            .get(self.active)
            .map(|sheet| path_key(&sheet.file));
        let selected = std::mem::take(&mut self.selected_uuids);
        let palette = self.palette();
        let mut old = self
            .sheets
            .drain(..)
            .map(|sheet| (path_key(&sheet.file), sheet))
            .collect::<HashMap<_, _>>();
        let mut loaded = batch.loaded;
        let mut next = Vec::with_capacity(entries.len());
        let mut errors = Vec::new();
        let mut external_lines = Vec::new();
        let mut external_changes = Vec::new();

        for entry in entries {
            let key = path_key(&entry.file);
            match loaded.remove(&key) {
                Some(Ok(loaded_scene)) => {
                    if external {
                        if let Some(previous) = old.get(&key) {
                            if let Some(summary) = summarize_external_change(
                                &entry.name,
                                &previous.semantic.source,
                                &loaded_scene.semantic.source,
                            ) {
                                external_lines.push(summary.description.clone());
                                external_changes.push((
                                    summary.description,
                                    entry.file.clone(),
                                    summary.changed_uuids,
                                ));
                            }
                        }
                    }
                    let rendered = encode_scene(&loaded_scene.semantic, palette);
                    next.push(NativeSheet {
                        name: entry.name,
                        depth: entry.depth,
                        file: entry.file,
                        semantic: loaded_scene.semantic,
                        rendered,
                        compatibility: loaded_scene.compatibility,
                        compatibility_error: loaded_scene.compatibility_error,
                    });
                }
                Some(Err(error)) => {
                    errors.push(format!("{}: {error}", entry.name));
                    if let Some(mut sheet) = old.remove(&key) {
                        sheet.name = entry.name;
                        sheet.depth = entry.depth;
                        sheet.file = entry.file;
                        next.push(sheet);
                    }
                }
                None => {
                    if let Some(mut sheet) = old.remove(&key) {
                        sheet.name = entry.name;
                        sheet.depth = entry.depth;
                        sheet.file = entry.file;
                        next.push(sheet);
                    } else {
                        errors.push(format!(
                            "{}: background reload returned no scene",
                            entry.name
                        ));
                    }
                }
            }
        }

        self.sheets = next;
        self.active = active_key
            .and_then(|key| {
                self.sheets
                    .iter()
                    .position(|sheet| path_key(&sheet.file) == key)
            })
            .unwrap_or(0)
            .min(self.sheets.len().saturating_sub(1));
        self.selected_uuids = selected
            .into_iter()
            .filter(|uuid| {
                self.sheets.get(self.active).is_some_and(|sheet| {
                    sheet
                        .semantic
                        .objects
                        .iter()
                        .any(|object| &object.uuid == uuid)
                })
            })
            .collect();
        self.status = if errors.is_empty() {
            self.live_status()
        } else {
            format!("Render error: {}", errors.join("; "))
        };
        if external && !external_lines.is_empty() {
            self.external_preview = Some(ExternalChangePreview {
                lines: external_lines,
            });
            if errors.is_empty() {
                self.status =
                    "External schematic update loaded · review summary · Esc dismisses".to_owned();
            }
        }
        let mut latest_external = None;
        for (label, file, uuids) in external_changes {
            latest_external = Some(self.record_change(ChangeOrigin::External, label, file, uuids));
        }
        if self.settings.follow_changes {
            self.pending_follow = latest_external;
        }
        if self.edit_session.is_conflicted() {
            self.status = format!(
                "Commit blocked · {} staged file(s) changed externally; Discard to load them",
                self.edit_session.conflicted_count()
            );
        }
        self.reconcile_watch_dirs();
    }

    pub(super) fn start_text_selection(&mut self, cursor: PhysicalPosition<f64>) -> bool {
        let Some(target) = self
            .text_targets
            .iter()
            .rev()
            .find(|target| target.rect.contains(cursor.x, cursor.y))
            .cloned()
        else {
            self.text_drag = None;
            self.text_selection = None;
            self.status = "Text selection · drag over text to select".to_owned();
            return false;
        };
        let anchor = if target.select_whole {
            0
        } else {
            target.character_at(cursor.x)
        };
        let current = if target.select_whole {
            target.character_count()
        } else {
            anchor
        };
        self.text_drag = Some(TextDrag {
            target: target.clone(),
            anchor,
            current,
        });
        self.text_selection = Some(TextSelection {
            target,
            start: anchor,
            end: current,
        });
        true
    }

    pub(super) fn update_text_selection(&mut self, cursor: PhysicalPosition<f64>) {
        let Some(drag) = &mut self.text_drag else {
            return;
        };
        drag.current = if drag.target.select_whole {
            drag.target.character_count()
        } else {
            drag.target.character_at(cursor.x)
        };
        self.text_selection = Some(TextSelection {
            target: drag.target.clone(),
            start: drag.anchor,
            end: drag.current,
        });
    }

    pub(super) fn finish_text_selection(&mut self) {
        let Some(drag) = self.text_drag.take() else {
            return;
        };
        let (start, end) = if drag.target.select_whole || drag.anchor == drag.current {
            (0, drag.target.character_count())
        } else {
            (drag.anchor, drag.current)
        };
        let selected = drag.target.selected_text(start, end);
        self.text_selection = Some(TextSelection {
            target: drag.target,
            start,
            end,
        });
        self.status = format!(
            "Selected {} character(s) · Ctrl/Command C to copy",
            selected.chars().count()
        );
    }

    pub(super) fn copy_selected_text(&mut self) {
        let Some(selection) = &self.text_selection else {
            self.status = "Nothing selected · enable text selection and drag text first".to_owned();
            return;
        };
        let text = selection
            .target
            .selected_text(selection.start, selection.end);
        if text.is_empty() {
            self.status = "The text selection is empty".to_owned();
            return;
        }
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        match self
            .clipboard
            .as_mut()
            .ok_or_else(|| anyhow!("platform clipboard is unavailable"))
            .and_then(|clipboard| {
                clipboard
                    .set_text(text.clone())
                    .map_err(anyhow::Error::from)
            }) {
            Ok(()) => self.status = format!("Copied {} character(s)", text.chars().count()),
            Err(error) => self.status = format!("Could not copy text: {error:#}"),
        }
    }

    pub(super) fn register_schematic_text(&mut self, transform: Affine) {
        if !self.text_select_mode {
            return;
        }
        let Some(sheet) = self.sheets.get(self.active) else {
            return;
        };
        self.text_targets
            .extend(sheet.semantic.primitives.iter().filter_map(|primitive| {
                let Primitive::Text { text, .. } = primitive else {
                    return None;
                };
                if text.is_empty() {
                    return None;
                }
                let bounds = primitive.bounds()?;
                let first = transform * KurboPoint::new(bounds.min_x, bounds.min_y);
                let second = transform * KurboPoint::new(bounds.max_x, bounds.max_y);
                Some(SelectableText::whole(
                    text,
                    ScreenRect {
                        x0: first.x.min(second.x) - 3.0,
                        y0: first.y.min(second.y) - 3.0,
                        x1: first.x.max(second.x) + 3.0,
                        y1: first.y.max(second.y) + 3.0,
                    },
                ))
            }));
    }

    pub(super) fn draw_text_selection(&mut self, palette: Palette) {
        let Some(selection) = &mut self.text_selection else {
            return;
        };
        let previous_center = selection.target.rect.center();
        if let Some(current) = self
            .text_targets
            .iter()
            .filter(|target| target.text == selection.target.text)
            .min_by(|left, right| {
                let left_center = left.rect.center();
                let right_center = right.rect.center();
                let left_distance = (left_center.0 - previous_center.0).powi(2)
                    + (left_center.1 - previous_center.1).powi(2);
                let right_distance = (right_center.0 - previous_center.0).powi(2)
                    + (right_center.1 - previous_center.1).powi(2);
                left_distance.total_cmp(&right_distance)
            })
        {
            selection.target.clone_from(current);
        }
        let rect = selection
            .target
            .selection_rect(selection.start, selection.end);
        self.frame.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.accent.with_alpha(0.3),
            None,
            &RoundedRect::new(rect.x0, rect.y0, rect.x1, rect.y1, 2.0),
        );
        self.frame.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            palette.accent,
            None,
            &RoundedRect::new(rect.x0, rect.y0, rect.x1, rect.y1, 2.0),
        );
    }

    pub(super) fn draw_frame(&mut self, width: u32, height: u32) {
        let width = f64::from(width);
        let height = f64::from(height);
        self.apply_pending_follow(width, height);
        let palette = self.palette();
        self.frame.reset();
        self.text_targets.clear();
        let drag_delta = self
            .item_drag
            .as_ref()
            .and_then(|drag| self.drag_delta(width, height, drag));
        let highlighted = (self.item_drag.is_none() && self.settings.highlight_changes)
            .then_some(self.highlighted_change)
            .flatten()
            .and_then(|id| self.timeline.event(id))
            .map(|event| (path_key(&event.file), event.origin, event.uuids.clone()));

        if let Some(sheet) = self.sheets.get(self.active) {
            if let Some((transform, scale, _, _)) = self.page_transform(width, height) {
                if let Some(drag) = &self.item_drag {
                    self.frame.append(&drag.base_scene, Some(transform));
                } else {
                    append_sheet(&mut self.frame, sheet, transform);
                }
                if let Some((file, origin, uuids)) = &highlighted {
                    if path_key(&sheet.file) == *file {
                        let mut highlight_palette = palette;
                        highlight_palette.selection = match origin {
                            ChangeOrigin::External => palette.bus,
                            ChangeOrigin::Local => palette.accent,
                            ChangeOrigin::Undo => palette.selection,
                            ChangeOrigin::Redo => palette.junction,
                        };
                        for object in sheet
                            .semantic
                            .objects
                            .iter()
                            .filter(|object| uuids.contains(&object.uuid))
                        {
                            append_selection(
                                &mut self.frame,
                                sheet,
                                object,
                                transform,
                                scale,
                                highlight_palette,
                            );
                        }
                    }
                }
                for diagnostic in &sheet.semantic.diagnostics {
                    let radius = (6.0 / scale).clamp(0.45, 1.75);
                    let color = match diagnostic.kind {
                        ConnectivityDiagnosticKind::ConnectedNoConnect => palette.no_connect,
                        ConnectivityDiagnosticKind::DanglingWire => palette.selection,
                        ConnectivityDiagnosticKind::DuplicateReference => palette.accent,
                        ConnectivityDiagnosticKind::DuplicateSheetName => palette.accent,
                        ConnectivityDiagnosticKind::DuplicateSheetPin => palette.accent,
                        ConnectivityDiagnosticKind::MissingJunction => palette.accent,
                        ConnectivityDiagnosticKind::UnconnectedBusEntry => palette.bus,
                        ConnectivityDiagnosticKind::UnpositionedSheetField => palette.sheet_file,
                    };
                    self.frame.stroke(
                        &Stroke::new((2.0 / scale).max(0.1)),
                        transform,
                        color,
                        None,
                        &Circle::new((diagnostic.point.x, diagnostic.point.y), radius),
                    );
                    if diagnostic.kind == ConnectivityDiagnosticKind::MissingJunction {
                        let diagonal = radius * 0.65;
                        for (dx0, dy0, dx1, dy1) in [
                            (-diagonal, -diagonal, diagonal, diagonal),
                            (-diagonal, diagonal, diagonal, -diagonal),
                        ] {
                            self.frame.stroke(
                                &Stroke::new((1.5 / scale).max(0.08)),
                                transform,
                                color,
                                None,
                                &Line::new(
                                    (diagnostic.point.x + dx0, diagnostic.point.y + dy0),
                                    (diagnostic.point.x + dx1, diagnostic.point.y + dy1),
                                ),
                            );
                        }
                    }
                }
                if let Some(wire) = self.wire_draft {
                    let line = Line::new(
                        (wire.start.x, wire.start.y),
                        (wire.current.x, wire.current.y),
                    );
                    let color = if wire.is_bus {
                        palette.bus
                    } else {
                        palette.wire
                    };
                    self.frame
                        .stroke(&round_stroke(0.254), transform, color, None, &line);
                    let endpoint_radius = (4.0 / scale).clamp(0.35, 1.25);
                    for point in [wire.start, wire.current] {
                        self.frame.stroke(
                            &Stroke::new((1.5 / scale).max(0.08)),
                            transform,
                            palette.selection,
                            None,
                            &Circle::new((point.x, point.y), endpoint_radius),
                        );
                    }
                }
                if let (Some(drag), Some((dx, dy))) = (&self.item_drag, drag_delta) {
                    for wire in &drag.connected_wires {
                        let start = if wire.move_start {
                            (wire.start.x + dx, wire.start.y + dy)
                        } else {
                            (wire.start.x, wire.start.y)
                        };
                        let end = if wire.move_end {
                            (wire.end.x + dx, wire.end.y + dy)
                        } else {
                            (wire.end.x, wire.end.y)
                        };
                        self.frame.stroke(
                            &round_stroke(0.254),
                            transform,
                            palette.wire,
                            None,
                            &Line::new(start, end),
                        );
                    }
                }
                let selection_transform = drag_delta.map_or(transform, |(dx, dy)| {
                    transform * Affine::translate((dx, dy))
                });
                for uuid in &self.selected_uuids {
                    if let Some(object) = sheet
                        .semantic
                        .objects
                        .iter()
                        .find(|object| &object.uuid == uuid)
                    {
                        if drag_delta.is_some() {
                            append_object_artwork(
                                &mut self.frame,
                                sheet,
                                object,
                                selection_transform,
                                palette,
                            );
                        }
                        append_selection(
                            &mut self.frame,
                            sheet,
                            object,
                            selection_transform,
                            scale,
                            palette,
                        );
                    }
                }
            }
        }

        if let Some((transform, _, _, _)) = self.page_transform(width, height) {
            self.register_schematic_text(transform);
        }

        if let Some(selection) = self.selection_box {
            let x0 = selection.start.x.min(selection.current.x);
            let x1 = selection.start.x.max(selection.current.x);
            let y0 = selection.start.y.min(selection.current.y);
            let y1 = selection.start.y.max(selection.current.y);
            let rect = Rect::new(x0, y0, x1, y1);
            self.frame.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.selection.with_alpha(0.12),
                None,
                &rect,
            );
            self.frame.stroke(
                &Stroke::new(1.25),
                Affine::IDENTITY,
                palette.selection,
                None,
                &rect,
            );
        }

        self.draw_toolbar(width, height, palette);
        self.draw_filmstrip(width, height, palette);
        self.draw_external_preview(width, palette);
        self.draw_diagnostics(width, height, palette);
        self.draw_inspector(width, height, palette);
        self.draw_search(width, palette);
        self.draw_property_edit(width, palette);
        self.draw_label_edit(width, palette);
        self.draw_sheet_edit(width, palette);
        self.draw_sheet_pin_edit(width, palette);
        self.draw_text_selection(palette);
    }

    pub(super) fn draw_sheet_pin_edit(&mut self, width: f64, palette: Palette) {
        let Some(edit) = &self.sheet_pin_edit else {
            return;
        };
        let x0 = (width / 2.0 - 270.0).max(12.0);
        let x1 = (width / 2.0 + 270.0).min(width - 12.0);
        let y0 = STATUS_HEIGHT + 90.0;
        let rect = RoundedRect::new(x0, y0, x1, y0 + 104.0, 7.0);
        self.frame
            .fill(Fill::NonZero, Affine::IDENTITY, palette.card, None, &rect);
        self.frame.stroke(
            &Stroke::new(1.5),
            Affine::IDENTITY,
            palette.accent,
            None,
            &rect,
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            &format!(
                "Sheet pin at {:.3}, {:.3} mm · type {}",
                edit.point.x,
                edit.point.y,
                edit.pin_type.keyword()
            ),
            TextRun {
                size: 13.0,
                position: (x0 + 14.0, y0 + 23.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.accent,
            },
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            &format!("Name: {}▏", truncate_ui(&edit.name, 54)),
            TextRun {
                size: 14.0,
                position: (x0 + 14.0, y0 + 52.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.text,
            },
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            "Tab · cycle type    Enter · place    Esc · cancel",
            TextRun {
                size: 10.0,
                position: (x0 + 14.0, y0 + 86.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.text,
            },
        );
    }

    pub(super) fn draw_sheet_edit(&mut self, width: f64, palette: Palette) {
        let Some(edit) = &self.sheet_edit else {
            return;
        };
        let x0 = (width / 2.0 - 280.0).max(12.0);
        let x1 = (width / 2.0 + 280.0).min(width - 12.0);
        let y0 = STATUS_HEIGHT + 82.0;
        let rect = RoundedRect::new(x0, y0, x1, y0 + 132.0, 7.0);
        self.frame
            .fill(Fill::NonZero, Affine::IDENTITY, palette.card, None, &rect);
        self.frame.stroke(
            &Stroke::new(1.5),
            Affine::IDENTITY,
            palette.accent,
            None,
            &rect,
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            &format!(
                "Hierarchical sheet at {:.3}, {:.3} mm",
                edit.point.x, edit.point.y
            ),
            TextRun {
                size: 13.0,
                position: (x0 + 14.0, y0 + 21.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.accent,
            },
        );
        for (index, (field, value)) in [
            (SheetEditField::Name, edit.name.as_str()),
            (SheetEditField::File, edit.file.as_str()),
        ]
        .into_iter()
        .enumerate()
        {
            let active = edit.field == field;
            let label = match field {
                SheetEditField::Name => "Name",
                SheetEditField::File => "Child file",
            };
            draw_selectable_text(
                &self.font,
                &mut self.frame,
                &mut self.text_targets,
                &format!(
                    "{label}: {}{}",
                    truncate_ui(value, 54),
                    if active { "▏" } else { "" }
                ),
                TextRun {
                    size: 14.0,
                    position: (x0 + 14.0, y0 + 50.0 + index as f64 * 29.0),
                    rotation_deg: 0.0,
                    align: TextAlign::Left,
                    color: if active { palette.accent } else { palette.text },
                },
            );
        }
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            "Tab · switch field    Enter · create/link    Esc · cancel",
            TextRun {
                size: 10.0,
                position: (x0 + 14.0, y0 + 116.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.text,
            },
        );
    }

    pub(super) fn draw_label_edit(&mut self, width: f64, palette: Palette) {
        let Some(edit) = &self.label_edit else {
            return;
        };
        let x0 = (width / 2.0 - 250.0).max(12.0);
        let x1 = (width / 2.0 + 250.0).min(width - 12.0);
        let y0 = STATUS_HEIGHT + 90.0;
        let rect = RoundedRect::new(x0, y0, x1, y0 + 82.0, 7.0);
        self.frame
            .fill(Fill::NonZero, Affine::IDENTITY, palette.card, None, &rect);
        self.frame.stroke(
            &Stroke::new(1.5),
            Affine::IDENTITY,
            palette.accent,
            None,
            &rect,
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            &format!("Local label at {:.3}, {:.3} mm", edit.point.x, edit.point.y),
            TextRun {
                size: 12.0,
                position: (x0 + 14.0, y0 + 20.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.accent,
            },
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            &truncate_ui(&format!("{}▏", edit.value), 58),
            TextRun {
                size: 16.0,
                position: (x0 + 14.0, y0 + 48.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.text,
            },
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            "Enter · atomic place    Esc · cancel",
            TextRun {
                size: 10.0,
                position: (x0 + 14.0, y0 + 68.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.text,
            },
        );
    }

    pub(super) fn draw_external_preview(&mut self, width: f64, palette: Palette) {
        let Some(preview) = &self.external_preview else {
            return;
        };
        let visible = preview.lines.len().min(6);
        let x0 = (width - 560.0).max(SIDEBAR_WIDTH + 14.0);
        let y0 = STATUS_HEIGHT + 8.0;
        let x1 = (x0 + 546.0).min(width - 14.0);
        let y1 = y0 + 58.0 + visible as f64 * 31.0;
        let panel = RoundedRect::new(x0, y0, x1, y1, 7.0);
        self.frame.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.card.with_alpha(0.97),
            None,
            &panel,
        );
        self.frame.stroke(
            &Stroke::new(1.5),
            Affine::IDENTITY,
            palette.accent,
            None,
            &panel,
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            "External schematic update · loaded safely",
            TextRun {
                size: 15.0,
                position: (x0 + 14.0, y0 + 23.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.accent,
            },
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            "Item preconditions remain active · Esc dismisses",
            TextRun {
                size: 11.0,
                position: (x0 + 14.0, y0 + 44.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.text,
            },
        );
        for (index, line) in preview.lines.iter().take(visible).enumerate() {
            draw_selectable_text(
                &self.font,
                &mut self.frame,
                &mut self.text_targets,
                &truncate_ui(line, 72),
                TextRun {
                    size: 12.0,
                    position: (x0 + 14.0, y0 + 70.0 + index as f64 * 31.0),
                    rotation_deg: 0.0,
                    align: TextAlign::Left,
                    color: palette.text,
                },
            );
        }
    }

    pub(super) fn draw_property_edit(&mut self, width: f64, palette: Palette) {
        let Some(edit) = &self.property_edit else {
            return;
        };
        let x0 = (width / 2.0 - 250.0).max(12.0);
        let x1 = (width / 2.0 + 250.0).min(width - 12.0);
        let y0 = STATUS_HEIGHT + 90.0;
        let rect = RoundedRect::new(x0, y0, x1, y0 + 82.0, 7.0);
        self.frame
            .fill(Fill::NonZero, Affine::IDENTITY, palette.card, None, &rect);
        self.frame.stroke(
            &Stroke::new(1.5),
            Affine::IDENTITY,
            palette.selection,
            None,
            &rect,
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            &format!("Edit {}", edit.name),
            TextRun {
                size: 13.0,
                position: (x0 + 14.0, y0 + 20.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.accent,
            },
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            &truncate_ui(&format!("{}▏", edit.value), 58),
            TextRun {
                size: 16.0,
                position: (x0 + 14.0, y0 + 48.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.text,
            },
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            "Enter · atomic save    Esc · cancel",
            TextRun {
                size: 10.0,
                position: (x0 + 14.0, y0 + 68.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.text,
            },
        );
    }

    pub(super) fn draw_inspector(&mut self, width: f64, height: f64, palette: Palette) {
        if self.selected_uuids.len() != 1 {
            return;
        }
        let Some(uuid) = self.selected_uuids.iter().next() else {
            return;
        };
        let Some(object) = self.sheets.get(self.active).and_then(|sheet| {
            sheet
                .semantic
                .objects
                .iter()
                .find(|object| &object.uuid == uuid)
        }) else {
            return;
        };
        let x0 = (width - 310.0).max(0.0);
        let y0 = STATUS_HEIGHT + 8.0;
        let y1 = (height - FILMSTRIP_HEIGHT - 8.0).max(y0 + 80.0);
        let panel = RoundedRect::new(x0, y0, width - 8.0, y1, 7.0);
        self.frame.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.card.with_alpha(0.96),
            None,
            &panel,
        );
        self.frame.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            palette.card_border,
            None,
            &panel,
        );
        let mut lines = vec![
            ("Properties".to_owned(), 15.0_f32, palette.accent),
            (
                truncate_ui(&format!("{:?} · {}", object.kind, object.label), 42),
                12.0,
                palette.text,
            ),
            (
                truncate_ui(&format!("UUID · {}", object.uuid), 42),
                10.0,
                palette.text,
            ),
            ("E · edit primary property".to_owned(), 10.0, palette.accent),
        ];
        lines.extend(object.properties.iter().map(|property| {
            (
                truncate_ui(&format!("{} · {}", property.name, property.value), 44),
                11.0,
                palette.text,
            )
        }));
        let mut line_y = y0 + 21.0;
        for (text, size, color) in lines {
            if line_y > y1 - 14.0 {
                draw_selectable_text(
                    &self.font,
                    &mut self.frame,
                    &mut self.text_targets,
                    "…",
                    TextRun {
                        size: 12.0,
                        position: (x0 + 12.0, line_y),
                        rotation_deg: 0.0,
                        align: TextAlign::Left,
                        color: palette.text,
                    },
                );
                break;
            }
            draw_selectable_text(
                &self.font,
                &mut self.frame,
                &mut self.text_targets,
                &text,
                TextRun {
                    size,
                    position: (x0 + 12.0, line_y),
                    rotation_deg: 0.0,
                    align: TextAlign::Left,
                    color,
                },
            );
            line_y += f64::from(size) + 7.0;
        }
    }

    pub(super) fn draw_diagnostics(&mut self, width: f64, height: f64, palette: Palette) {
        let diagnostic_count = self
            .sheets
            .get(self.active)
            .map_or(0, |sheet| sheet.semantic.diagnostics.len());
        if diagnostic_count == 0 {
            return;
        }
        let visible = diagnostic_count.min(4);
        let panel_rect = self.diagnostics_panel_rect(width, height, diagnostic_count);
        let ScreenRect { x0, y0, x1, y1 } = panel_rect;
        let line_height = 36.0 * f64::from(self.settings.ui_scale).min(1.35);
        let panel = RoundedRect::new(x0, y0, x1, y1, 9.0);
        self.frame.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.card.with_alpha(0.96),
            None,
            &panel,
        );
        self.frame.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            palette.selection,
            None,
            &panel,
        );
        for row in 0..3 {
            for column in 0..2 {
                self.frame.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    palette.selection,
                    None,
                    &Circle::new(
                        (
                            x0 + 13.0 + column as f64 * 5.0,
                            y0 + 14.0 + row as f64 * 5.0,
                        ),
                        1.2,
                    ),
                );
            }
        }
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            &if self.settings.diagnostics_collapsed {
                format!("{diagnostic_count} design warnings")
            } else {
                format!("Design checks · {diagnostic_count} warning(s)")
            },
            TextRun {
                size: if self.settings.diagnostics_collapsed {
                    12.0
                } else {
                    16.0
                },
                position: (x0 + 31.0, y0 + DIAGNOSTICS_HEADER_HEIGHT / 2.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.selection,
            },
        );
        let collapse_rect = Self::diagnostics_collapse_rect(panel_rect);
        let collapse_button = RoundedRect::new(
            collapse_rect.x0,
            collapse_rect.y0,
            collapse_rect.x1,
            collapse_rect.y1,
            5.0,
        );
        self.frame.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.fill.with_alpha(0.42),
            None,
            &collapse_button,
        );
        self.frame.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            palette.card_border,
            None,
            &collapse_button,
        );
        let (collapse_x, collapse_y) = collapse_rect.center();
        self.frame.stroke(
            &Stroke::new(1.8),
            Affine::IDENTITY,
            palette.text,
            None,
            &Line::new(
                (collapse_x - 5.0, collapse_y),
                (collapse_x + 5.0, collapse_y),
            ),
        );
        if self.settings.diagnostics_collapsed {
            self.frame.stroke(
                &Stroke::new(1.8),
                Affine::IDENTITY,
                palette.text,
                None,
                &Line::new(
                    (collapse_x, collapse_y - 5.0),
                    (collapse_x, collapse_y + 5.0),
                ),
            );
            return;
        }
        let diagnostics = self.sheets[self.active]
            .semantic
            .diagnostics
            .iter()
            .take(visible)
            .map(|diagnostic| diagnostic.message.clone())
            .collect::<Vec<_>>();
        for (index, diagnostic) in diagnostics.iter().enumerate() {
            draw_selectable_text(
                &self.font,
                &mut self.frame,
                &mut self.text_targets,
                &truncate_ui(diagnostic, 72),
                TextRun {
                    size: 13.0,
                    position: (
                        x0 + 16.0,
                        y0 + DIAGNOSTICS_HEADER_HEIGHT + 19.0 + index as f64 * line_height,
                    ),
                    rotation_deg: 0.0,
                    align: TextAlign::Left,
                    color: palette.text,
                },
            );
        }
        if diagnostic_count > visible {
            draw_selectable_text(
                &self.font,
                &mut self.frame,
                &mut self.text_targets,
                &format!("+{} more checks", diagnostic_count - visible),
                TextRun {
                    size: 11.0,
                    position: (x1 - 152.0, y0 + 24.0),
                    rotation_deg: 0.0,
                    align: TextAlign::Left,
                    color: palette.text,
                },
            );
        }
    }

    pub(super) fn draw_search(&mut self, width: f64, palette: Palette) {
        let Some(search) = &self.search else {
            return;
        };
        let x0 = (width / 2.0 - 260.0).max(12.0);
        let y0 = STATUS_HEIGHT + 12.0;
        let rect = RoundedRect::new(
            x0,
            y0,
            (width / 2.0 + 260.0).min(width - 12.0),
            STATUS_HEIGHT + 72.0,
            7.0,
        );
        self.frame
            .fill(Fill::NonZero, Affine::IDENTITY, palette.card, None, &rect);
        self.frame.stroke(
            &Stroke::new(1.5),
            Affine::IDENTITY,
            palette.accent,
            None,
            &rect,
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            &format!("Find: {}▏", search.query),
            TextRun {
                size: 15.0,
                position: (x0 + 14.0, y0 + 20.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.text,
            },
        );
        let detail = search
            .hits
            .get(search.current.min(search.hits.len().saturating_sub(1)))
            .map_or_else(
                || {
                    format!(
                        "{} result(s) · Enter to navigate · Esc to close",
                        search.hits.len()
                    )
                },
                |hit| {
                    format!(
                        "{} result(s) · next: {} · Enter to navigate",
                        search.hits.len(),
                        hit.description
                    )
                },
            );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            &detail,
            TextRun {
                size: 11.0,
                position: (x0 + 14.0, y0 + 44.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.text,
            },
        );
    }

    pub(super) fn draw_toolbar(&mut self, width: f64, height: f64, palette: Palette) {
        self.frame.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.toolbar,
            None,
            &Rect::new(0.0, 0.0, SIDEBAR_WIDTH, height),
        );
        self.frame.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.app.with_alpha(0.94),
            None,
            &Rect::new(SIDEBAR_WIDTH, 0.0, width, STATUS_HEIGHT),
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            "KONNECT",
            TextRun {
                size: 10.5,
                position: (SIDEBAR_WIDTH / 2.0, STATUS_HEIGHT / 2.0),
                rotation_deg: 0.0,
                align: TextAlign::Center,
                color: palette.accent,
            },
        );
        let filename = self
            .sheets
            .get(self.active)
            .and_then(|sheet| sheet.file.file_name())
            .and_then(|name| name.to_str())
            .unwrap_or("schematic");
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            filename,
            TextRun {
                size: 12.0,
                position: (SIDEBAR_WIDTH + 14.0, STATUS_HEIGHT / 2.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.accent,
            },
        );
        let status_x = SIDEBAR_WIDTH + 24.0 + filename.chars().count() as f64 * 7.8;
        let controls_x = Self::edit_controls(width)[0].1.x0;
        let status_chars = ((controls_x - status_x - 12.0) / 6.3).max(0.0) as usize;
        let status = truncate_ui(&self.status, status_chars);
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            &status,
            TextRun {
                size: 10.5,
                position: (status_x, STATUS_HEIGHT / 2.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.text,
            },
        );
        self.draw_edit_controls(width, palette);

        let hovered = self.cursor.and_then(|cursor| {
            Self::toolbar_buttons(width)
                .into_iter()
                .find(|(_, rect)| rect.contains(cursor.x, cursor.y))
                .map(|(action, _)| action)
        });
        for (action, rect) in Self::toolbar_buttons(width) {
            let active = self.toolbar_action_is_active(action);
            let is_hovered = hovered == Some(action);
            self.frame.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                if is_hovered {
                    palette.accent.with_alpha(0.16)
                } else if active {
                    palette.accent.with_alpha(0.1)
                } else {
                    palette.card
                },
                None,
                &RoundedRect::new(rect.x0, rect.y0, rect.x1, rect.y1, 7.0),
            );
            self.frame.stroke(
                &Stroke::new(if active || is_hovered { 1.5 } else { 1.0 }),
                Affine::IDENTITY,
                if active || is_hovered {
                    palette.accent
                } else {
                    palette.card_border
                },
                None,
                &RoundedRect::new(rect.x0, rect.y0, rect.x1, rect.y1, 7.0),
            );
            draw_ui_icon(
                &mut self.frame,
                Self::toolbar_icon(action),
                ScreenRect {
                    x0: rect.x0 + 7.0,
                    y0: rect.y0 + 7.0,
                    x1: rect.x1 - 7.0,
                    y1: rect.y1 - 7.0,
                },
                if active || is_hovered {
                    palette.accent
                } else {
                    palette.text
                },
            );
        }
        if let Some(action) = hovered {
            let label = self.toolbar_label(action);
            let (_, text_width) = self
                .font
                .measure_and_glyphs(10.0 * self.font.ui_scale, &label);
            let tooltip = ScreenRect {
                x0: SIDEBAR_WIDTH + 8.0,
                y0: self
                    .cursor
                    .map_or(48.0, |cursor| (cursor.y - 17.0).max(42.0)),
                x1: (SIDEBAR_WIDTH + 34.0 + f64::from(text_width)).min(width - 8.0),
                y1: self
                    .cursor
                    .map_or(82.0, |cursor| (cursor.y + 17.0).max(76.0)),
            };
            self.frame.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.card.with_alpha(0.98),
                None,
                &RoundedRect::new(tooltip.x0, tooltip.y0, tooltip.x1, tooltip.y1, 6.0),
            );
            self.frame.stroke(
                &Stroke::new(1.0),
                Affine::IDENTITY,
                palette.card_border,
                None,
                &RoundedRect::new(tooltip.x0, tooltip.y0, tooltip.x1, tooltip.y1, 6.0),
            );
            draw_selectable_text(
                &self.font,
                &mut self.frame,
                &mut self.text_targets,
                &label,
                TextRun {
                    size: 10.0,
                    position: (tooltip.x0 + 10.0, tooltip.center().1),
                    rotation_deg: 0.0,
                    align: TextAlign::Left,
                    color: palette.text,
                },
            );
        }
    }

    pub(super) fn draw_filmstrip(&mut self, width: f64, height: f64, palette: Palette) {
        let film = self.filmstrip_rect(width, height);
        self.frame.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.filmstrip,
            None,
            &film.as_kurbo(),
        );

        for (index, sheet) in self.sheets.iter().enumerate() {
            let card = self.thumbnail_rect(index, height);
            if card.x1 < 0.0 || card.x0 > width {
                continue;
            }
            self.frame.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.card,
                None,
                &RoundedRect::new(card.x0, card.y0, card.x1, card.y1, 6.0),
            );
            self.frame.stroke(
                &Stroke::new(if index == self.active { 3.0 } else { 1.0 }),
                Affine::IDENTITY,
                if index == self.active {
                    palette.accent
                } else {
                    palette.card_border
                },
                None,
                &RoundedRect::new(card.x0, card.y0, card.x1, card.y1, 6.0),
            );

            let preview = ScreenRect {
                x0: card.x0 + 7.0,
                y0: card.y0 + 7.0,
                x1: card.x1 - 7.0,
                y1: card.y1 - 29.0,
            };
            let scale = (preview.width() / sheet.semantic.width_mm)
                .min(preview.height() / sheet.semantic.height_mm);
            let tx = preview.center().0 - sheet.semantic.width_mm * scale / 2.0;
            let ty = preview.center().1 - sheet.semantic.height_mm * scale / 2.0;
            append_sheet(
                &mut self.frame,
                sheet,
                Affine::new([scale, 0.0, 0.0, scale, tx, ty]),
            );

            let hierarchy = "› ".repeat(sheet.depth);
            let thumbnail_label = truncate_ui(
                &format!(
                    "{:02}  {hierarchy}{}",
                    index + 1,
                    display_sheet_name(&sheet.name)
                ),
                22,
            );
            draw_selectable_text(
                &self.font,
                &mut self.frame,
                &mut self.text_targets,
                &thumbnail_label,
                TextRun {
                    size: 10.5,
                    position: (card.x0 + 7.0, card.y1 - 14.0),
                    rotation_deg: 0.0,
                    align: TextAlign::Left,
                    color: palette.text,
                },
            );
            if !sheet.semantic.coverage.is_complete() {
                draw_selectable_text(
                    &self.font,
                    &mut self.frame,
                    &mut self.text_targets,
                    "!",
                    TextRun {
                        size: 16.0,
                        position: (card.x1 - 14.0, card.y0 + 15.0),
                        rotation_deg: 0.0,
                        align: TextAlign::Center,
                        color: palette.selection,
                    },
                );
            }
        }
        self.draw_timeline(width, height, palette);
    }
}
