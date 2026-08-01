//! Selection, drag, search, wiring, properties, modal editing, and history controllers.

use super::*;

impl VelloViewer {
    pub(super) fn live_status(&self) -> String {
        let mode = if self.edit_session.enabled {
            let pending = self.edit_session.dirty_document_count();
            if pending == 0 {
                "Edit mode".to_owned()
            } else {
                format!("Edit mode · {pending} staged file(s)")
            }
        } else {
            "Read-only".to_owned()
        };
        let mut base = format!("{mode} · Live · {} page(s)", self.sheets.len());
        let Some(sheet) = self.sheets.get(self.active) else {
            return base;
        };
        if !sheet.semantic.diagnostics.is_empty() {
            base.push_str(&format!(
                " · {} design warning(s)",
                sheet.semantic.diagnostics.len()
            ));
        }
        if sheet.semantic.coverage.is_complete() {
            return base;
        }
        if sheet.compatibility.is_some() {
            return format!("{base} · KiCad SVG fallback active for this sheet");
        }
        let kinds = sheet
            .semantic
            .coverage
            .unsupported
            .iter()
            .map(|construct| format!("{}×{}", construct.kind, construct.count))
            .collect::<Vec<_>>()
            .join(", ");
        match &sheet.compatibility_error {
            Some(error) => format!("{base} · native fallback ({kinds}) · fallback error: {error}"),
            None => format!("{base} · preparing KiCad SVG fallback: {kinds}"),
        }
    }

    pub(super) fn request_redraw(&self) {
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }

    pub(super) fn remember_local_revision(&mut self, file: &Path, revision: DocumentRevision) {
        self.local_revisions.insert(path_key(file), revision);
    }

    pub(super) fn apply_staged_source(&mut self, file: &Path, source: String) -> bool {
        let Ok(semantic) = SchematicScene::from_source(file, source) else {
            return false;
        };
        let palette = self.palette();
        let rendered = encode_scene(&semantic, palette);
        let Some(sheet) = self
            .sheets
            .iter_mut()
            .find(|sheet| path_key(&sheet.file) == path_key(file))
        else {
            return false;
        };
        sheet.semantic = semantic;
        sheet.rendered = rendered;
        // A KiCad SVG fallback reflects the durable file, not the staged
        // source. Keep the semantic preview authoritative until Commit.
        sheet.compatibility = None;
        sheet.compatibility_error = None;
        true
    }

    pub(super) fn stage_command(
        &mut self,
        file: &Path,
        command: &SchematicCommand,
    ) -> std::result::Result<konnect_sexp::TransactionOutcome, SexpError> {
        let source = self
            .sheets
            .iter()
            .find(|sheet| path_key(&sheet.file) == path_key(file))
            .map(|sheet| sheet.semantic.source.to_string())
            .ok_or_else(|| SexpError::InvalidValue("staged sheet is not loaded".to_owned()))?;
        let key = path_key(file);
        let mut candidate = self.edit_session.clone();
        let (replacement, outcome) = candidate.stage_command(key, file, &source, command)?;
        if !self.apply_staged_source(file, replacement) {
            return Err(SexpError::InvalidValue(
                "staged schematic could not be rendered".to_owned(),
            ));
        }
        self.edit_session = candidate;
        Ok(outcome)
    }

    pub(super) fn require_edit_mode(&mut self) -> bool {
        if self.edit_session.enabled {
            true
        } else {
            self.status = "Read-only · enable Edit mode before changing the schematic".to_owned();
            false
        }
    }

    pub(super) fn toggle_edit_mode(&mut self) {
        if self.edit_session.enabled && self.edit_session.has_pending() {
            self.status = "Commit or Discard staged changes before leaving Edit mode".to_owned();
            return;
        }
        self.edit_session.enabled = !self.edit_session.enabled;
        self.status = if self.edit_session.enabled {
            "Edit mode · changes stay in memory until Commit".to_owned()
        } else {
            "Read-only mode · schematic editing is disabled".to_owned()
        };
    }

    pub(super) fn commit_edit_session(&mut self) {
        if !self.edit_session.enabled {
            self.status = "Enable Edit mode before committing".to_owned();
            return;
        }
        if !self.edit_session.has_pending() {
            self.status = "Nothing staged to commit".to_owned();
            return;
        }
        if self.edit_session.is_conflicted() {
            self.status = format!(
                "Commit blocked · {} staged file(s) changed externally; Discard and review the new source",
                self.edit_session.conflicted_count()
            );
            return;
        }
        if self
            .edit_session
            .dirty_documents()
            .any(|document| kicad_lock_path(&document.file).exists())
        {
            self.status = "Commit blocked · close staged sheets in KiCad first".to_owned();
            return;
        }
        let transitions = self.edit_session.transitions();
        let files = self
            .edit_session
            .dirty_documents()
            .map(|document| document.file.clone())
            .collect::<Vec<_>>();
        let Some(journal_root) = self.root.parent().map(Path::to_path_buf) else {
            self.status = "Commit blocked · project has no transaction directory".to_owned();
            return;
        };
        match commit_file_transaction(&journal_root, transitions) {
            Ok(_) => {
                let revisions = self
                    .edit_session
                    .dirty_documents()
                    .map(|document| {
                        (
                            document.file.clone(),
                            DocumentRevision::of(&document.staged),
                        )
                    })
                    .collect::<Vec<_>>();
                for (file, revision) in &revisions {
                    self.remember_local_revision(file, *revision);
                }
                let edit_count = self.undo_stack.len();
                self.edit_session.clear();
                self.undo_stack.clear();
                self.redo_stack.clear();
                self.record_change(
                    ChangeOrigin::Local,
                    format!("Committed {edit_count} staged edit(s)"),
                    self.root.clone(),
                    Vec::new(),
                );
                self.schedule_reload(&files);
                self.status = format!(
                    "Committed {edit_count} staged edit(s) across {} file(s) atomically",
                    files.len()
                );
            }
            Err(error) => {
                self.status =
                    format!("Commit stopped safely: {error} · staged changes remain available");
            }
        }
    }

    pub(super) fn discard_edit_session(&mut self) {
        if !self.edit_session.has_pending() {
            self.status = "Nothing staged to discard".to_owned();
            return;
        }
        let files = self
            .edit_session
            .dirty_documents()
            .map(|document| document.file.clone())
            .collect::<Vec<_>>();
        let load_as_external = self.edit_session.is_conflicted();
        self.edit_session.clear();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.selected_uuids.clear();
        if load_as_external {
            self.schedule_external_reload(&files);
        } else {
            self.schedule_reload(&files);
        }
        self.status = "Discarded all staged changes · durable files were untouched".to_owned();
    }

    pub(super) fn record_change(
        &mut self,
        origin: ChangeOrigin,
        label: impl Into<String>,
        file: PathBuf,
        uuids: Vec<String>,
    ) -> u64 {
        let id = self.timeline.push(origin, label, file, uuids);
        self.timeline_scroll = 0;
        if self.settings.highlight_changes {
            self.highlighted_change = Some(id);
        }
        id
    }

    pub(super) fn record_command_change(
        &mut self,
        origin: ChangeOrigin,
        file: &Path,
        command: &SchematicCommand,
    ) -> u64 {
        self.record_change(
            origin,
            command.label.clone(),
            file.to_path_buf(),
            command
                .changes
                .iter()
                .map(|change| change.id.as_str().to_owned())
                .collect(),
        )
    }

    pub(super) fn navigate_to_change(&mut self, id: u64) {
        if self.timeline.event(id).is_none() {
            return;
        }
        self.highlighted_change = self.settings.highlight_changes.then_some(id);
        self.pending_follow = Some(id);
        self.status = "Navigating to timeline change".to_owned();
    }

    pub(super) fn apply_pending_follow(&mut self, width: f64, height: f64) {
        let Some(id) = self.pending_follow.take() else {
            return;
        };
        let Some(event) = self.timeline.event(id) else {
            return;
        };
        let file = path_key(&event.file);
        let uuids = event.uuids.clone();
        let Some(index) = self
            .sheets
            .iter()
            .position(|sheet| path_key(&sheet.file) == file)
        else {
            return;
        };
        self.active = index;
        self.selected_uuids.clear();
        let bounds = self.sheets[index]
            .semantic
            .objects
            .iter()
            .filter(|object| uuids.contains(&object.uuid))
            .map(|object| object.bounds)
            .reduce(union_bounds);
        let Some(bounds) = bounds else {
            self.fit();
            return;
        };
        self.zoom = 3.0;
        let sheet = &self.sheets[index];
        let area = self.main_rect(width, height);
        let fit = ((area.width() - PAGE_PADDING * 2.0) / sheet.semantic.width_mm)
            .min((area.height() - PAGE_PADDING * 2.0) / sheet.semantic.height_mm)
            .max(0.001);
        let scale = fit * self.zoom;
        let target_x = (bounds.min_x + bounds.max_x) / 2.0;
        let target_y = (bounds.min_y + bounds.max_y) / 2.0;
        self.pan = (
            (sheet.semantic.width_mm / 2.0 - target_x) * scale,
            (sheet.semantic.height_mm / 2.0 - target_y) * scale,
        );
    }

    pub(super) fn focus_point(&mut self, width: f64, height: f64, point: SchPoint) {
        let Some(sheet) = self.sheets.get(self.active) else {
            return;
        };
        self.zoom = 4.0;
        let area = self.main_rect(width, height);
        let fit = ((area.width() - PAGE_PADDING * 2.0) / sheet.semantic.width_mm)
            .min((area.height() - PAGE_PADDING * 2.0) / sheet.semantic.height_mm)
            .max(0.001);
        let scale = fit * self.zoom;
        self.pan = (
            (sheet.semantic.width_mm / 2.0 - point.x) * scale,
            (sheet.semantic.height_mm / 2.0 - point.y) * scale,
        );
    }

    pub(super) fn diagnostics_panel_rect(
        &self,
        width: f64,
        height: f64,
        diagnostic_count: usize,
    ) -> ScreenRect {
        let x_min = SIDEBAR_WIDTH + 14.0;
        let y_min = STATUS_HEIGHT + 8.0;
        let available_right = (width - 14.0).max(x_min + 1.0);
        let requested_width = if self.settings.diagnostics_collapsed {
            244.0
        } else {
            DIAGNOSTICS_PANEL_WIDTH
        };
        let panel_width = requested_width.min(available_right - x_min);
        let visible = diagnostic_count.min(4);
        let line_height = 36.0 * f64::from(self.settings.ui_scale).min(1.35);
        let requested_height = if self.settings.diagnostics_collapsed {
            DIAGNOSTICS_HEADER_HEIGHT
        } else {
            46.0 + visible as f64 * line_height
        };
        let available_bottom =
            (height - FILMSTRIP_HEIGHT - 8.0).max(y_min + DIAGNOSTICS_HEADER_HEIGHT);
        let panel_height = requested_height.min(available_bottom - y_min);
        let x_max = (available_right - panel_width).max(x_min);
        let y_max = (available_bottom - panel_height).max(y_min);
        let (x_fraction, y_fraction) = if self.settings.diagnostics_collapsed {
            (
                self.settings.diagnostics_collapsed_x.unwrap_or(1.0),
                self.settings.diagnostics_collapsed_y.unwrap_or(0.0),
            )
        } else {
            (
                self.settings.diagnostics_panel_x.unwrap_or(0.0),
                self.settings.diagnostics_panel_y.unwrap_or(0.0),
            )
        };
        let x0 = x_min + (x_max - x_min) * f64::from(x_fraction);
        let y0 = y_min + (y_max - y_min) * f64::from(y_fraction);
        ScreenRect {
            x0,
            y0,
            x1: x0 + panel_width,
            y1: y0 + panel_height,
        }
    }

    pub(super) fn diagnostics_collapse_rect(panel: ScreenRect) -> ScreenRect {
        ScreenRect {
            x0: panel.x1 - 32.0,
            y0: panel.y0 + 7.0,
            x1: panel.x1 - 8.0,
            y1: panel.y0 + 31.0,
        }
    }

    pub(super) fn move_diagnostics_panel(
        &mut self,
        width: f64,
        height: f64,
        position: PhysicalPosition<f64>,
        drag: DiagnosticsPanelDrag,
    ) {
        let diagnostic_count = self
            .sheets
            .get(self.active)
            .map_or(0, |sheet| sheet.semantic.diagnostics.len());
        let panel = self.diagnostics_panel_rect(width, height, diagnostic_count);
        let x_min = SIDEBAR_WIDTH + 14.0;
        let y_min = STATUS_HEIGHT + 8.0;
        let x_max = (width - 14.0 - panel.width()).max(x_min);
        let available_bottom =
            (height - FILMSTRIP_HEIGHT - 8.0).max(y_min + DIAGNOSTICS_HEADER_HEIGHT);
        let y_max = (available_bottom - panel.height()).max(y_min);
        let x0 = (position.x - drag.pointer_offset.0).clamp(x_min, x_max);
        let y0 = (position.y - drag.pointer_offset.1).clamp(y_min, y_max);
        let x_fraction = if x_max > x_min {
            ((x0 - x_min) / (x_max - x_min)) as f32
        } else {
            0.0
        };
        let y_fraction = if y_max > y_min {
            ((y0 - y_min) / (y_max - y_min)) as f32
        } else {
            0.0
        };
        if self.settings.diagnostics_collapsed {
            self.settings.diagnostics_collapsed_x = Some(x_fraction);
            self.settings.diagnostics_collapsed_y = Some(y_fraction);
        } else {
            self.settings.diagnostics_panel_x = Some(x_fraction);
            self.settings.diagnostics_panel_y = Some(y_fraction);
        }
        self.status = "Moving design checks · release to keep position".to_owned();
    }

    pub(super) fn finish_diagnostics_drag(&mut self) {
        if self.diagnostics_drag.take().is_some() && self.persist_settings() {
            self.status = "Design checks · panel position saved".to_owned();
        }
    }

    pub(super) fn handle_diagnostics(&mut self, width: f64, height: f64, x: f64, y: f64) -> bool {
        let diagnostic_count = self
            .sheets
            .get(self.active)
            .map_or(0, |sheet| sheet.semantic.diagnostics.len());
        if diagnostic_count == 0 {
            return false;
        }
        let panel = self.diagnostics_panel_rect(width, height, diagnostic_count);
        if !panel.contains(x, y) {
            return false;
        }
        if Self::diagnostics_collapse_rect(panel).contains(x, y) {
            self.settings.diagnostics_collapsed = !self.settings.diagnostics_collapsed;
            if self.persist_settings() {
                self.status = if self.settings.diagnostics_collapsed {
                    "Design checks · collapsed".to_owned()
                } else {
                    "Design checks · expanded".to_owned()
                };
            }
            return true;
        }
        if y <= panel.y0 + DIAGNOSTICS_HEADER_HEIGHT {
            self.diagnostics_drag = Some(DiagnosticsPanelDrag {
                pointer_offset: (x - panel.x0, y - panel.y0),
            });
            self.status = "Moving design checks · drag anywhere by the header".to_owned();
            return true;
        }
        if self.settings.diagnostics_collapsed {
            return true;
        }
        let line_height = 36.0 * f64::from(self.settings.ui_scale).min(1.35);
        let row = ((y - (panel.y0 + DIAGNOSTICS_HEADER_HEIGHT)) / line_height).floor() as usize;
        if let Some(diagnostic) = self
            .sheets
            .get(self.active)
            .and_then(|sheet| sheet.semantic.diagnostics.get(row))
            .cloned()
        {
            self.focus_point(width, height, diagnostic.point);
            self.status = format!("Design check · {}", diagnostic.message);
        }
        true
    }

    pub(super) fn external_paths(&mut self, paths: Vec<PathBuf>) -> Vec<PathBuf> {
        paths
            .into_iter()
            .filter(|path| {
                let key = path_key(path);
                let Some(expected) = self.local_revisions.get(&key).copied() else {
                    return true;
                };
                let matches_local = notification_matches_revision(path, expected);
                if !matches_local {
                    self.local_revisions.remove(&key);
                }
                !matches_local
            })
            .collect()
    }

    pub(super) fn palette(&self) -> Palette {
        self.theme.palette()
    }

    pub(super) fn main_rect(&self, width: f64, height: f64) -> ScreenRect {
        ScreenRect {
            x0: SIDEBAR_WIDTH,
            y0: STATUS_HEIGHT,
            x1: width,
            y1: (height - FILMSTRIP_HEIGHT).max(STATUS_HEIGHT + 1.0),
        }
    }

    pub(super) fn filmstrip_rect(&self, width: f64, height: f64) -> ScreenRect {
        ScreenRect {
            x0: SIDEBAR_WIDTH,
            y0: (height - FILMSTRIP_HEIGHT).max(STATUS_HEIGHT),
            x1: width,
            y1: height,
        }
    }

    pub(super) fn page_transform(
        &self,
        width: f64,
        height: f64,
    ) -> Option<(Affine, f64, f64, f64)> {
        let sheet = self.sheets.get(self.active)?;
        let area = self.main_rect(width, height);
        let fit = ((area.width() - PAGE_PADDING * 2.0) / sheet.semantic.width_mm)
            .min((area.height() - PAGE_PADDING * 2.0) / sheet.semantic.height_mm)
            .max(0.001);
        let scale = fit * self.zoom;
        let (center_x, center_y) = area.center();
        let x = center_x + self.pan.0 - sheet.semantic.width_mm * scale / 2.0;
        let y = center_y + self.pan.1 - sheet.semantic.height_mm * scale / 2.0;
        Some((Affine::new([scale, 0.0, 0.0, scale, x, y]), scale, x, y))
    }

    pub(super) fn schematic_point(
        &self,
        width: f64,
        height: f64,
        x: f64,
        y: f64,
    ) -> Option<SchPoint> {
        let (_, scale, tx, ty) = self.page_transform(width, height)?;
        Some(SchPoint {
            x: (x - tx) / scale,
            y: (y - ty) / scale,
        })
    }

    pub(super) fn set_zoom_about(
        &mut self,
        width: f64,
        height: f64,
        new_zoom: f64,
        x: f64,
        y: f64,
    ) {
        let Some(world) = self.schematic_point(width, height, x, y) else {
            return;
        };
        self.zoom = new_zoom.clamp(0.1, 40.0);
        let Some(sheet) = self.sheets.get(self.active) else {
            return;
        };
        let area = self.main_rect(width, height);
        let fit = ((area.width() - PAGE_PADDING * 2.0) / sheet.semantic.width_mm)
            .min((area.height() - PAGE_PADDING * 2.0) / sheet.semantic.height_mm)
            .max(0.001);
        let scale = fit * self.zoom;
        let (center_x, center_y) = area.center();
        self.pan.0 = x - center_x - (world.x - sheet.semantic.width_mm / 2.0) * scale;
        self.pan.1 = y - center_y - (world.y - sheet.semantic.height_mm / 2.0) * scale;
    }

    pub(super) fn fit(&mut self) {
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
    }

    pub(super) fn switch_sheet(&mut self, index: usize) {
        if index >= self.sheets.len() || index == self.active {
            return;
        }
        self.active = index;
        self.selected_uuids.clear();
        self.fit();
        self.status = self.live_status();
    }

    pub(super) fn toggle_theme(&mut self) {
        self.theme = match self.theme {
            Theme::Dark => Theme::Light,
            Theme::Light => Theme::Dark,
        };
        self.settings.dark_theme = self.theme == Theme::Dark;
        let palette = self.palette();
        for sheet in &mut self.sheets {
            sheet.rebuild(palette);
        }
        if self.persist_settings() {
            self.status = format!(
                "Theme · {}",
                if self.settings.dark_theme {
                    "high-contrast dark"
                } else {
                    "KiCad light"
                }
            );
        }
    }

    pub(super) fn edit_controls(width: f64) -> [(EditControl, ScreenRect); 3] {
        const GAP: f64 = 6.0;
        const EDIT_WIDTH: f64 = 112.0;
        const ACTION_WIDTH: f64 = 82.0;
        let right = width - 10.0;
        let discard_x = right - ACTION_WIDTH;
        let commit_x = discard_x - GAP - ACTION_WIDTH;
        let edit_x = commit_x - GAP - EDIT_WIDTH;
        [
            (
                EditControl::EditMode,
                ScreenRect {
                    x0: edit_x,
                    y0: 6.0,
                    x1: edit_x + EDIT_WIDTH,
                    y1: 34.0,
                },
            ),
            (
                EditControl::Commit,
                ScreenRect {
                    x0: commit_x,
                    y0: 6.0,
                    x1: commit_x + ACTION_WIDTH,
                    y1: 34.0,
                },
            ),
            (
                EditControl::Discard,
                ScreenRect {
                    x0: discard_x,
                    y0: 6.0,
                    x1: discard_x + ACTION_WIDTH,
                    y1: 34.0,
                },
            ),
        ]
    }

    pub(super) fn handle_edit_controls(&mut self, width: f64, x: f64, y: f64) -> bool {
        let Some((control, _)) = Self::edit_controls(width)
            .into_iter()
            .find(|(_, rect)| rect.contains(x, y))
        else {
            return false;
        };
        match control {
            EditControl::EditMode => self.toggle_edit_mode(),
            EditControl::Commit => self.commit_edit_session(),
            EditControl::Discard => self.discard_edit_session(),
        }
        true
    }

    pub(super) fn draw_edit_controls(&mut self, width: f64, palette: Palette) {
        let hovered = self.cursor.and_then(|cursor| {
            Self::edit_controls(width)
                .into_iter()
                .find(|(_, rect)| rect.contains(cursor.x, cursor.y))
                .map(|(control, _)| control)
        });
        let pending = self.edit_session.dirty_document_count();
        for (control, rect) in Self::edit_controls(width) {
            let active = control == EditControl::EditMode && self.edit_session.enabled;
            let actionable = control == EditControl::EditMode || pending > 0;
            let border = if self.edit_session.is_conflicted()
                && matches!(control, EditControl::Commit | EditControl::Discard)
            {
                palette.selection
            } else if active || hovered == Some(control) {
                palette.accent
            } else {
                palette.card_border
            };
            self.frame.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                if active {
                    palette.accent.with_alpha(0.16)
                } else {
                    palette.card.with_alpha(if actionable { 0.96 } else { 0.5 })
                },
                None,
                &RoundedRect::new(rect.x0, rect.y0, rect.x1, rect.y1, 6.0),
            );
            self.frame.stroke(
                &Stroke::new(if active || hovered == Some(control) {
                    1.5
                } else {
                    1.0
                }),
                Affine::IDENTITY,
                border,
                None,
                &RoundedRect::new(rect.x0, rect.y0, rect.x1, rect.y1, 6.0),
            );
            let text_color = if actionable {
                palette.text
            } else {
                palette.text.with_alpha(0.48)
            };
            let (label, text_x) = match control {
                EditControl::EditMode => {
                    let checkbox = ScreenRect {
                        x0: rect.x0 + 8.0,
                        y0: rect.y0 + 7.0,
                        x1: rect.x0 + 22.0,
                        y1: rect.y0 + 21.0,
                    };
                    self.frame.stroke(
                        &Stroke::new(1.4),
                        Affine::IDENTITY,
                        if active {
                            palette.accent
                        } else {
                            palette.card_border
                        },
                        None,
                        &RoundedRect::new(checkbox.x0, checkbox.y0, checkbox.x1, checkbox.y1, 2.5),
                    );
                    if active {
                        draw_ui_icon(
                            &mut self.frame,
                            UiIcon::Commit,
                            ScreenRect {
                                x0: checkbox.x0 + 2.0,
                                y0: checkbox.y0 + 2.0,
                                x1: checkbox.x1 - 2.0,
                                y1: checkbox.y1 - 2.0,
                            },
                            palette.accent,
                        );
                    }
                    ("Edit mode".to_owned(), rect.x0 + 29.0)
                }
                EditControl::Commit => {
                    draw_ui_icon(
                        &mut self.frame,
                        UiIcon::Commit,
                        ScreenRect {
                            x0: rect.x0 + 7.0,
                            y0: rect.y0 + 7.0,
                            x1: rect.x0 + 21.0,
                            y1: rect.y0 + 21.0,
                        },
                        text_color,
                    );
                    (
                        if pending == 0 {
                            "Commit".to_owned()
                        } else {
                            format!("Commit {pending}")
                        },
                        rect.x0 + 27.0,
                    )
                }
                EditControl::Discard => {
                    draw_ui_icon(
                        &mut self.frame,
                        UiIcon::Discard,
                        ScreenRect {
                            x0: rect.x0 + 7.0,
                            y0: rect.y0 + 7.0,
                            x1: rect.x0 + 21.0,
                            y1: rect.y0 + 21.0,
                        },
                        text_color,
                    );
                    ("Discard".to_owned(), rect.x0 + 27.0)
                }
            };
            draw_selectable_text(
                &self.font,
                &mut self.frame,
                &mut self.text_targets,
                &label,
                TextRun {
                    size: 10.5,
                    position: (text_x, rect.center().1),
                    rotation_deg: 0.0,
                    align: TextAlign::Left,
                    color: text_color,
                },
            );
        }
    }

    pub(super) fn toolbar_buttons(_width: f64) -> [(ToolbarAction, ScreenRect); 12] {
        std::array::from_fn(|index| {
            const BUTTON_SIZE: f64 = 36.0;
            const COLUMN_GAP: f64 = 8.0;
            const ROW_GAP: f64 = 7.0;
            let column = index % 2;
            let row = index / 2;
            let x0 = 8.0 + column as f64 * (BUTTON_SIZE + COLUMN_GAP);
            let y0 = 52.0 + row as f64 * (BUTTON_SIZE + ROW_GAP);
            let rect = ScreenRect {
                x0,
                y0,
                x1: x0 + BUTTON_SIZE,
                y1: y0 + BUTTON_SIZE,
            };
            (
                [
                    ToolbarAction::Undo,
                    ToolbarAction::Redo,
                    ToolbarAction::ZoomIn,
                    ToolbarAction::ZoomOut,
                    ToolbarAction::Fit,
                    ToolbarAction::Grid,
                    ToolbarAction::Snap,
                    ToolbarAction::UiScale,
                    ToolbarAction::HighlightChanges,
                    ToolbarAction::FollowChanges,
                    ToolbarAction::TextSelect,
                    ToolbarAction::Theme,
                ][index],
                rect,
            )
        })
    }

    pub(super) fn toolbar_icon(action: ToolbarAction) -> UiIcon {
        match action {
            ToolbarAction::Undo => UiIcon::Undo,
            ToolbarAction::Redo => UiIcon::Redo,
            ToolbarAction::ZoomIn => UiIcon::ZoomIn,
            ToolbarAction::ZoomOut => UiIcon::ZoomOut,
            ToolbarAction::Fit => UiIcon::Fit,
            ToolbarAction::Grid => UiIcon::Grid,
            ToolbarAction::Snap => UiIcon::Snap,
            ToolbarAction::UiScale => UiIcon::Scale,
            ToolbarAction::HighlightChanges => UiIcon::Highlight,
            ToolbarAction::FollowChanges => UiIcon::Follow,
            ToolbarAction::TextSelect => UiIcon::TextSelect,
            ToolbarAction::Theme => UiIcon::Theme,
        }
    }

    pub(super) fn toolbar_label(&self, action: ToolbarAction) -> String {
        match action {
            ToolbarAction::Undo => "Undo · Ctrl/Command Z".to_owned(),
            ToolbarAction::Redo => "Redo · Ctrl/Command Shift Z".to_owned(),
            ToolbarAction::ZoomIn => "Zoom in".to_owned(),
            ToolbarAction::ZoomOut => "Zoom out".to_owned(),
            ToolbarAction::Fit => "Fit active sheet · 0".to_owned(),
            ToolbarAction::Grid => format!("Grid · {:.3} mm · G", self.grid_mm),
            ToolbarAction::Snap => format!(
                "Snap · {} · S",
                if self.snap_enabled { "on" } else { "off" }
            ),
            ToolbarAction::UiScale => {
                format!("Interface scale · {:.0}%", self.settings.ui_scale * 100.0)
            }
            ToolbarAction::HighlightChanges => format!(
                "Highlight changes · {}",
                if self.settings.highlight_changes {
                    "on"
                } else {
                    "off"
                }
            ),
            ToolbarAction::FollowChanges => format!(
                "Follow external changes · {}",
                if self.settings.follow_changes {
                    "on"
                } else {
                    "off"
                }
            ),
            ToolbarAction::TextSelect => format!(
                "Select text · {} · Ctrl/Command C to copy",
                if self.text_select_mode { "on" } else { "off" }
            ),
            ToolbarAction::Theme => format!(
                "Theme · {}",
                if self.settings.dark_theme {
                    "high-contrast dark"
                } else {
                    "KiCad light"
                }
            ),
        }
    }

    pub(super) fn toolbar_action_is_active(&self, action: ToolbarAction) -> bool {
        match action {
            ToolbarAction::Snap => self.snap_enabled,
            ToolbarAction::HighlightChanges => self.settings.highlight_changes,
            ToolbarAction::FollowChanges => self.settings.follow_changes,
            ToolbarAction::TextSelect => self.text_select_mode,
            ToolbarAction::Theme => self.settings.dark_theme,
            _ => false,
        }
    }

    pub(super) fn handle_toolbar(&mut self, width: f64, height: f64, x: f64, y: f64) -> bool {
        let Some((action, _)) = Self::toolbar_buttons(width)
            .into_iter()
            .find(|(_, rect)| rect.contains(x, y))
        else {
            return false;
        };
        match action {
            ToolbarAction::Undo => self.undo(),
            ToolbarAction::Redo => self.redo(),
            ToolbarAction::ZoomIn => {
                let (cx, cy) = self.main_rect(width, height).center();
                self.set_zoom_about(width, height, self.zoom * 1.25, cx, cy);
            }
            ToolbarAction::ZoomOut => {
                let (cx, cy) = self.main_rect(width, height).center();
                self.set_zoom_about(width, height, self.zoom / 1.25, cx, cy);
            }
            ToolbarAction::Fit => self.fit(),
            ToolbarAction::Grid => self.cycle_grid(),
            ToolbarAction::Snap => self.toggle_snap(),
            ToolbarAction::UiScale => {
                self.settings.cycle_ui_scale();
                self.font.ui_scale = self.settings.ui_scale;
                if self.persist_settings() {
                    self.status =
                        format!("Interface text · {:.0}%", self.settings.ui_scale * 100.0);
                }
            }
            ToolbarAction::HighlightChanges => {
                self.settings.highlight_changes = !self.settings.highlight_changes;
                if !self.settings.highlight_changes {
                    self.highlighted_change = None;
                }
                if self.persist_settings() {
                    self.status = format!(
                        "Highlight changes · {}",
                        if self.settings.highlight_changes {
                            "on"
                        } else {
                            "off"
                        }
                    );
                }
            }
            ToolbarAction::FollowChanges => {
                self.settings.follow_changes = !self.settings.follow_changes;
                if self.persist_settings() {
                    self.status = format!(
                        "Follow external changes · {}",
                        if self.settings.follow_changes {
                            "on"
                        } else {
                            "off"
                        }
                    );
                }
            }
            ToolbarAction::TextSelect => {
                self.text_select_mode = !self.text_select_mode;
                self.text_drag = None;
                if !self.text_select_mode {
                    self.text_selection = None;
                }
                self.status = if self.text_select_mode {
                    "Text selection · drag text, then Ctrl/Command C to copy".to_owned()
                } else {
                    self.live_status()
                };
            }
            ToolbarAction::Theme => self.toggle_theme(),
        }
        true
    }

    pub(super) fn persist_settings(&mut self) -> bool {
        if let Err(error) = self.settings.save() {
            self.status = format!("Could not save viewer settings: {error:#}");
            false
        } else {
            true
        }
    }

    pub(super) fn cycle_grid(&mut self) {
        const GRID_STEPS: &[f64] = &[0.254, 0.508, 1.27, 2.54, 5.08];
        let index = GRID_STEPS
            .iter()
            .position(|step| (*step - self.grid_mm).abs() < 1e-9)
            .unwrap_or(2);
        self.grid_mm = GRID_STEPS[(index + 1) % GRID_STEPS.len()];
        self.status = format!("Grid · {:.3} mm", self.grid_mm);
    }

    pub(super) fn toggle_snap(&mut self) {
        self.snap_enabled = !self.snap_enabled;
        self.status = format!("Snap · {}", if self.snap_enabled { "on" } else { "off" });
    }

    pub(super) fn start_search(&mut self) {
        self.search = Some(SearchState::default());
        self.status = "Search · type a reference, value, net, UUID, or sheet".to_owned();
    }

    pub(super) fn refresh_search(&mut self) {
        let Some(search) = &mut self.search else {
            return;
        };
        search.hits.clear();
        search.current = 0;
        let query = search.query.trim().to_lowercase();
        if query.is_empty() {
            return;
        }
        for (sheet_index, sheet) in self.sheets.iter().enumerate() {
            let sheet_text = format!("{} {}", sheet.name, sheet.file.display()).to_lowercase();
            if sheet_text.contains(&query) {
                search.hits.push(SearchHit {
                    sheet: sheet_index,
                    uuid: None,
                    description: format!("Sheet · {}", sheet.name),
                });
            }
            for object in &sheet.semantic.objects {
                if object.search_text.to_lowercase().contains(&query) {
                    search.hits.push(SearchHit {
                        sheet: sheet_index,
                        uuid: Some(object.uuid.clone()),
                        description: format!("{:?} · {}", object.kind, object.label),
                    });
                }
            }
        }
        self.status = format!("Search · {} result(s)", search.hits.len());
    }

    pub(super) fn activate_search_hit(&mut self) {
        let (hit, index, hit_count) = {
            let Some(search) = &mut self.search else {
                return;
            };
            if search.hits.is_empty() {
                self.status = format!("Search · no results for ‘{}’", search.query);
                return;
            }
            let index = search.current.min(search.hits.len() - 1);
            let hit = search.hits[index].clone();
            let hit_count = search.hits.len();
            search.current = (index + 1) % hit_count;
            (hit, index, hit_count)
        };
        self.active = hit.sheet;
        self.fit();
        self.selected_uuids.clear();
        if let Some(uuid) = hit.uuid {
            self.selected_uuids.insert(uuid);
        }
        self.status = format!(
            "Search result {}/{} · {}",
            index + 1,
            hit_count,
            hit.description
        );
    }

    pub(super) fn handle_search_key(&mut self, key: Key<&str>) -> bool {
        if self.search.is_none() {
            return false;
        }
        match key {
            Key::Named(NamedKey::Escape) => {
                self.search = None;
                self.status = self.live_status();
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some(search) = &mut self.search {
                    search.query.pop();
                }
                self.refresh_search();
            }
            Key::Named(NamedKey::Enter) => self.activate_search_hit(),
            Key::Character(value)
                if !self.modifiers.control_key() && !self.modifiers.super_key() =>
            {
                if let Some(search) = &mut self.search {
                    search.query.push_str(value);
                }
                self.refresh_search();
            }
            _ => {}
        }
        true
    }

    pub(super) fn start_property_edit(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        if self.selected_uuids.len() != 1 {
            self.status = "Select exactly one symbol to edit its properties".to_owned();
            return;
        }
        let Some(uuid) = self.selected_uuids.iter().next() else {
            return;
        };
        let Some(sheet) = self.sheets.get(self.active) else {
            return;
        };
        let Some(object) = sheet
            .semantic
            .objects
            .iter()
            .find(|object| &object.uuid == uuid)
        else {
            return;
        };
        let property = object
            .properties
            .iter()
            .find(|property| property.name == "Value")
            .or_else(|| {
                object.properties.iter().find(|property| {
                    !matches!(
                        property.name.as_str(),
                        "Library" | "Position" | "Reference" | "Text"
                    )
                })
            });
        let Some(property) = property else {
            self.status = "This item has no directly editable property yet".to_owned();
            return;
        };
        self.property_edit = Some(PropertyEdit {
            file: sheet.file.clone(),
            uuid: uuid.clone(),
            name: property.name.clone(),
            value: property.value.clone(),
        });
        self.status = format!("Editing {} · Enter to stage · Esc to cancel", property.name);
    }

    pub(super) fn commit_property_edit(&mut self) {
        let Some(edit) = self.property_edit.take() else {
            return;
        };
        let Some(sheet) = self
            .sheets
            .iter()
            .find(|sheet| path_key(&sheet.file) == path_key(&edit.file))
        else {
            self.status = "Property edit rejected: sheet is no longer loaded".to_owned();
            return;
        };
        if sheet
            .semantic
            .objects
            .iter()
            .find(|object| object.uuid == edit.uuid)
            .and_then(|object| {
                object
                    .properties
                    .iter()
                    .find(|property| property.name == edit.name)
            })
            .is_some_and(|property| property.value == edit.value)
        {
            self.status = format!("{} is unchanged", edit.name);
            return;
        }
        let id = match ItemId::new(edit.uuid.clone()) {
            Ok(id) => id,
            Err(error) => {
                self.status = format!("Property edit rejected: {error}");
                return;
            }
        };
        let command = match SchematicCommand::set_property(
            &sheet.semantic.source,
            id,
            &edit.name,
            &edit.value,
            format!("Edit {}", edit.name),
        ) {
            Ok(command) => command,
            Err(error) => {
                self.status = format!("Property edit rejected: {error}");
                return;
            }
        };
        match self.stage_command(&edit.file, &command) {
            Ok(outcome) => {
                self.record_command_change(ChangeOrigin::Local, &edit.file, &command);
                push_history(
                    &mut self.undo_stack,
                    HistoryEntry::single(edit.file.clone(), outcome.inverse),
                );
                self.redo_stack.clear();
                self.status = format!("Staged {} · press Commit to write", edit.name);
            }
            Err(error @ (SexpError::Conflict { .. } | SexpError::ItemConflict { .. })) => {
                self.status = format!("Property staging conflict: {error}");
            }
            Err(error) => self.status = format!("Could not stage property: {error}"),
        }
    }

    pub(super) fn handle_property_key(&mut self, key: Key<&str>) -> bool {
        if self.property_edit.is_none() {
            return false;
        }
        match key {
            Key::Named(NamedKey::Escape) => {
                self.property_edit = None;
                self.status = self.live_status();
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some(edit) = &mut self.property_edit {
                    edit.value.pop();
                }
            }
            Key::Named(NamedKey::Enter) => self.commit_property_edit(),
            Key::Character(value)
                if !self.modifiers.control_key() && !self.modifiers.super_key() =>
            {
                if let Some(edit) = &mut self.property_edit {
                    edit.value.push_str(value);
                }
            }
            _ => {}
        }
        true
    }

    pub(super) fn thumbnail_rect(&self, index: usize, height: f64) -> ScreenRect {
        let x0 = SIDEBAR_WIDTH + THUMBNAIL_GAP + index as f64 * (THUMBNAIL_WIDTH + THUMBNAIL_GAP)
            - self.film_scroll;
        ScreenRect {
            x0,
            y0: height - FILMSTRIP_HEIGHT + 9.0,
            x1: x0 + THUMBNAIL_WIDTH,
            y1: height - TIMELINE_HEIGHT - 9.0,
        }
    }

    pub(super) fn handle_filmstrip(&mut self, width: f64, height: f64, x: f64, y: f64) -> bool {
        if !self.filmstrip_rect(width, height).contains(x, y) {
            return false;
        }
        if y >= height - TIMELINE_HEIGHT {
            if let Some((id, _)) = self
                .timeline_event_rects(width, height)
                .into_iter()
                .find(|(_, rect)| rect.contains(x, y))
            {
                self.navigate_to_change(id);
            }
            return true;
        }
        if let Some(index) =
            (0..self.sheets.len()).find(|index| self.thumbnail_rect(*index, height).contains(x, y))
        {
            self.switch_sheet(index);
        }
        true
    }

    pub(super) fn timeline_event_rects(&self, width: f64, height: f64) -> Vec<(u64, ScreenRect)> {
        const LABEL_WIDTH: f64 = 96.0;
        const CARD_WIDTH: f64 = 138.0;
        const CARD_GAP: f64 = 6.0;

        let available = (width - SIDEBAR_WIDTH - LABEL_WIDTH - 20.0).max(CARD_WIDTH);
        let visible = ((available + CARD_GAP) / (CARD_WIDTH + CARD_GAP))
            .floor()
            .max(1.0) as usize;
        let mut ids = self
            .timeline
            .events()
            .rev()
            .skip(self.timeline_scroll)
            .take(visible)
            .map(|event| event.id)
            .collect::<Vec<_>>();
        ids.reverse();
        ids.into_iter()
            .enumerate()
            .map(|(index, id)| {
                let x0 = SIDEBAR_WIDTH + LABEL_WIDTH + index as f64 * (CARD_WIDTH + CARD_GAP);
                (
                    id,
                    ScreenRect {
                        x0,
                        y0: height - TIMELINE_HEIGHT + 7.0,
                        x1: x0 + CARD_WIDTH,
                        y1: height - 7.0,
                    },
                )
            })
            .collect()
    }

    pub(super) fn draw_timeline(&mut self, width: f64, height: f64, palette: Palette) {
        let y0 = height - TIMELINE_HEIGHT;
        self.frame.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.toolbar.with_alpha(0.98),
            None,
            &Rect::new(SIDEBAR_WIDTH, y0, width, height),
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            &format!("CHANGES · {}", self.timeline.len()),
            TextRun {
                size: 9.5,
                position: (SIDEBAR_WIDTH + 10.0, y0 + 17.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.accent,
            },
        );
        draw_selectable_text(
            &self.font,
            &mut self.frame,
            &mut self.text_targets,
            if self.timeline.len() == 0 {
                "Session history"
            } else if self.timeline_scroll == 0 {
                "Newest · scroll"
            } else {
                "Earlier · scroll"
            },
            TextRun {
                size: 8.0,
                position: (SIDEBAR_WIDTH + 10.0, y0 + 34.0),
                rotation_deg: 0.0,
                align: TextAlign::Left,
                color: palette.text.with_alpha(0.75),
            },
        );
        if self.timeline.len() == 0 {
            draw_selectable_text(
                &self.font,
                &mut self.frame,
                &mut self.text_targets,
                "No changes yet — edits and external updates appear here",
                TextRun {
                    size: 9.5,
                    position: (SIDEBAR_WIDTH + 106.0, y0 + 25.0),
                    rotation_deg: 0.0,
                    align: TextAlign::Left,
                    color: palette.text.with_alpha(0.62),
                },
            );
        }
        let now = Instant::now();
        for (id, rect) in self.timeline_event_rects(width, height) {
            let Some(event) = self.timeline.event(id) else {
                continue;
            };
            let progress = event.intro_progress(now);
            let eased = 1.0 - (1.0 - progress).powi(3);
            let dy = (1.0 - eased) * 12.0;
            let card = RoundedRect::new(rect.x0, rect.y0 + dy, rect.x1, rect.y1 + dy, 6.0);
            let origin_color = match event.origin {
                ChangeOrigin::External => palette.bus,
                ChangeOrigin::Local => palette.accent,
                ChangeOrigin::Undo => palette.selection,
                ChangeOrigin::Redo => palette.junction,
            };
            self.frame.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.card.with_alpha((0.72 + eased * 0.28) as f32),
                None,
                &card,
            );
            self.frame.stroke(
                &Stroke::new(if self.highlighted_change == Some(id) {
                    2.5
                } else {
                    1.0
                }),
                Affine::IDENTITY,
                if self.highlighted_change == Some(id) {
                    origin_color
                } else {
                    palette.card_border
                },
                None,
                &card,
            );
            let icon_rect = ScreenRect {
                x0: rect.x0 + 7.0,
                y0: rect.y0 + 7.0 + dy,
                x1: rect.x0 + 29.0,
                y1: rect.y1 - 7.0 + dy,
            };
            self.frame.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                origin_color.with_alpha(0.14),
                None,
                &RoundedRect::new(icon_rect.x0, icon_rect.y0, icon_rect.x1, icon_rect.y1, 5.0),
            );
            draw_ui_icon(
                &mut self.frame,
                change_icon(event.kind),
                icon_rect,
                origin_color,
            );
            draw_selectable_text(
                &self.font,
                &mut self.frame,
                &mut self.text_targets,
                &truncate_ui(&event.label, 19),
                TextRun {
                    size: 9.0,
                    position: (rect.x0 + 35.0, rect.y0 + 12.0 + dy),
                    rotation_deg: 0.0,
                    align: TextAlign::Left,
                    color: palette.text,
                },
            );
            draw_selectable_text(
                &self.font,
                &mut self.frame,
                &mut self.text_targets,
                &format!(
                    "{} · {}",
                    event.origin.label(),
                    relative_time(event.recorded_at)
                ),
                TextRun {
                    size: 7.5,
                    position: (rect.x0 + 35.0, rect.y0 + 27.0 + dy),
                    rotation_deg: 0.0,
                    align: TextAlign::Left,
                    color: origin_color,
                },
            );
        }
    }

    pub(super) fn select_at(
        &mut self,
        width: f64,
        height: f64,
        x: f64,
        y: f64,
        additive: bool,
    ) -> bool {
        let Some(point) = self.schematic_point(width, height, x, y) else {
            return false;
        };
        let Some(sheet) = self.sheets.get(self.active) else {
            return false;
        };
        let tolerance = 5.0
            / self
                .page_transform(width, height)
                .map(|(_, scale, _, _)| scale)
                .unwrap_or(1.0);
        if let Some(object) = sheet.semantic.hit_test(point, tolerance) {
            if additive && !self.selected_uuids.insert(object.uuid.clone()) {
                self.selected_uuids.remove(&object.uuid);
            } else if !additive {
                self.selected_uuids.clear();
                self.selected_uuids.insert(object.uuid.clone());
            }
            self.status = format!(
                "Selected {} item(s) · {:?} · {}",
                self.selected_uuids.len(),
                object.kind,
                object.label
            );
            true
        } else {
            if !additive {
                self.selected_uuids.clear();
            }
            false
        }
    }

    pub(super) fn finish_box_selection(
        &mut self,
        width: f64,
        height: f64,
        selection: SelectionBox,
    ) {
        let Some(start) = self.schematic_point(width, height, selection.start.x, selection.start.y)
        else {
            return;
        };
        let Some(end) =
            self.schematic_point(width, height, selection.current.x, selection.current.y)
        else {
            return;
        };
        if !selection.additive {
            self.selected_uuids.clear();
        }
        let Some(sheet) = self.sheets.get(self.active) else {
            return;
        };
        for object in &sheet.semantic.objects {
            if box_selects_bounds(object.bounds, start, end) {
                self.selected_uuids.insert(object.uuid.clone());
            }
        }
        self.status = format!("Selected {} item(s)", self.selected_uuids.len());
    }

    pub(super) fn selected_items_are_movable(&self) -> bool {
        !self.selected_uuids.is_empty()
            && self.sheets.get(self.active).is_some_and(|sheet| {
                self.selected_uuids.iter().all(|uuid| {
                    sheet
                        .semantic
                        .objects
                        .iter()
                        .any(|object| &object.uuid == uuid)
                })
            })
    }

    pub(super) fn drag_delta(
        &self,
        width: f64,
        height: f64,
        drag: &ItemDrag,
    ) -> Option<(f64, f64)> {
        let (_, scale, _, _) = self.page_transform(width, height)?;
        Some(drag_delta_mm(
            drag.start,
            drag.current,
            scale,
            self.snap_enabled,
            self.grid_mm,
        ))
    }

    pub(super) fn finish_item_drag(&mut self, width: f64, height: f64, drag: ItemDrag) {
        let distance = (drag.current.x - drag.start.x).hypot(drag.current.y - drag.start.y);
        if distance < 2.0 {
            return;
        }
        let Some((dx, dy)) = self.drag_delta(width, height, &drag) else {
            return;
        };
        if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
            self.status = "Drag snapped back to the original position".to_owned();
            return;
        }
        self.nudge_selected(dx, dy);
    }

    pub(super) fn start_item_drag(&mut self, cursor: PhysicalPosition<f64>) {
        let Some(sheet) = self.sheets.get(self.active) else {
            return;
        };
        let connected_wires =
            match connected_wire_moves(&sheet.semantic.source, &self.selected_uuids) {
                Ok(wires) => wires,
                Err(error) => {
                    self.status = format!("Could not prepare connected drag: {error:#}");
                    return;
                }
            };
        let mut excluded_uuids = self.selected_uuids.clone();
        excluded_uuids.extend(connected_wires.iter().map(|wire| wire.uuid.clone()));
        let excluded = sheet
            .semantic
            .objects
            .iter()
            .filter(|object| excluded_uuids.contains(&object.uuid))
            .map(|object| object.primitive_range.clone())
            .collect::<Vec<_>>();
        let base_scene = encode_scene_without_ranges(&sheet.semantic, &excluded, self.palette());
        self.item_drag = Some(ItemDrag {
            start: cursor,
            current: cursor,
            base_scene,
            connected_wires,
        });
    }

    pub(super) fn nudge_selected(&mut self, dx: f64, dy: f64) {
        if !self.require_edit_mode() {
            return;
        }
        if self.selected_uuids.is_empty() {
            self.status = "Select a symbol before using Ctrl/Command + Arrow".to_owned();
            return;
        }
        let Some(sheet) = self.sheets.get(self.active) else {
            return;
        };
        let mut uuids = self.selected_uuids.iter().cloned().collect::<Vec<_>>();
        uuids.sort();
        let selected_objects = uuids
            .iter()
            .filter_map(|uuid| {
                sheet
                    .semantic
                    .objects
                    .iter()
                    .find(|object| &object.uuid == uuid)
            })
            .collect::<Vec<_>>();
        if selected_objects.len() != uuids.len() {
            self.status = "Part of the selection is no longer present; reloading".to_owned();
            return;
        }
        let file = sheet.semantic.file.clone();
        let expected = sheet.semantic.source.clone();
        let (edited, changed_uuids) =
            match move_items_with_connected_wires(&expected, &self.selected_uuids, dx, dy) {
                Ok(result) => result,
                Err(error) => {
                    self.status = format!("Edit rejected: {error:#}");
                    return;
                }
            };
        let item_ids = match changed_uuids
            .iter()
            .cloned()
            .map(ItemId::new)
            .collect::<std::result::Result<Vec<_>, _>>()
        {
            Ok(item_ids) => item_ids,
            Err(error) => {
                self.status = format!("Edit rejected: {error}");
                return;
            }
        };
        let command = match SchematicCommand::replace_items_from_document(
            &expected,
            &edited,
            item_ids,
            if uuids.len() == 1 {
                "Move item"
            } else {
                "Move items"
            },
        ) {
            Ok(command) => command,
            Err(error) => {
                self.status = format!("Edit rejected: {error}");
                return;
            }
        };
        match self.stage_command(&file, &command) {
            Ok(outcome) => {
                push_history(
                    &mut self.undo_stack,
                    HistoryEntry::single(file.clone(), outcome.inverse),
                );
                self.redo_stack.clear();
                self.record_change(
                    ChangeOrigin::Local,
                    format!("Moved {} item(s)", uuids.len()),
                    file.clone(),
                    changed_uuids,
                );
                let rebased = if outcome.rebased {
                    " · safely rebased"
                } else {
                    ""
                };
                self.status = format!(
                    "Staged{rebased} · moved {} item(s) by ({dx:.2}, {dy:.2}) mm · press Commit",
                    uuids.len()
                );
            }
            Err(SexpError::Conflict { .. } | SexpError::ItemConflict { .. }) => {
                self.status =
                    "Staging conflict: the selected item changed; no durable file was written"
                        .to_owned();
            }
            Err(error) => {
                self.status = format!("Could not stage edit: {error}");
            }
        }
    }

    pub(super) fn delete_selected(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        if self.selected_uuids.is_empty() {
            self.status = "Select one or more items to stage for deletion".to_owned();
            return;
        }
        let Some(sheet) = self.sheets.get(self.active) else {
            return;
        };
        let file = sheet.file.clone();
        let ids = match self
            .selected_uuids
            .iter()
            .cloned()
            .map(ItemId::new)
            .collect::<std::result::Result<Vec<_>, _>>()
        {
            Ok(ids) => ids,
            Err(error) => {
                self.status = format!("Delete rejected: {error}");
                return;
            }
        };
        let count = ids.len();
        let command = match SchematicCommand::delete_items(
            &sheet.semantic.source,
            ids,
            if count == 1 {
                "Delete item"
            } else {
                "Delete items"
            },
        ) {
            Ok(command) => command,
            Err(error) => {
                self.status = format!("Delete rejected: {error}");
                return;
            }
        };
        match self.stage_command(&file, &command) {
            Ok(outcome) => {
                self.record_command_change(ChangeOrigin::Local, &file, &command);
                push_history(
                    &mut self.undo_stack,
                    HistoryEntry::single(file.clone(), outcome.inverse),
                );
                self.redo_stack.clear();
                self.selected_uuids.clear();
                self.status = format!(
                    "Staged deletion of {count} item(s) · Undo restores them · press Commit"
                );
            }
            Err(error @ (SexpError::Conflict { .. } | SexpError::ItemConflict { .. })) => {
                self.status = format!("Delete staging conflict: {error}");
            }
            Err(error) => self.status = format!("Could not stage deletion: {error}"),
        }
    }

    pub(super) fn transform_selected_items(
        &mut self,
        operation: &str,
        past_tense: &str,
        item_name: &str,
        supports: impl Fn(ObjectKind) -> bool,
        transform: impl Fn(&str, &str) -> Result<String>,
    ) {
        if !self.require_edit_mode() {
            return;
        }
        if self.selected_uuids.is_empty() {
            self.status = format!("Select one or more items to {}", operation.to_lowercase());
            return;
        }
        let Some(sheet) = self.sheets.get(self.active) else {
            return;
        };
        let mut uuids = self.selected_uuids.iter().cloned().collect::<Vec<_>>();
        uuids.sort();
        if !uuids.iter().all(|uuid| {
            sheet
                .semantic
                .objects
                .iter()
                .any(|object| &object.uuid == uuid && supports(object.kind))
        }) {
            self.status = format!("{operation} requires {item_name}");
            return;
        }
        let file = sheet.file.clone();
        let expected = Arc::clone(&sheet.semantic.source);
        let mut edited = expected.to_string();
        for uuid in &uuids {
            edited = match transform(&edited, uuid) {
                Ok(edited) => edited,
                Err(error) => {
                    self.status = format!("{operation} rejected: {error:#}");
                    return;
                }
            };
        }
        let ids = match uuids
            .iter()
            .cloned()
            .map(ItemId::new)
            .collect::<std::result::Result<Vec<_>, _>>()
        {
            Ok(ids) => ids,
            Err(error) => {
                self.status = format!("{operation} rejected: {error}");
                return;
            }
        };
        let command = match SchematicCommand::replace_items_from_document(
            &expected,
            &edited,
            ids,
            format!("{operation} {item_name}"),
        ) {
            Ok(command) => command,
            Err(error) => {
                self.status = format!("{operation} rejected: {error}");
                return;
            }
        };
        match self.stage_command(&file, &command) {
            Ok(outcome) => {
                self.record_command_change(ChangeOrigin::Local, &file, &command);
                push_history(
                    &mut self.undo_stack,
                    HistoryEntry::single(file.clone(), outcome.inverse),
                );
                self.redo_stack.clear();
                self.status = format!(
                    "Staged · {past_tense} {} {item_name} · press Commit",
                    uuids.len()
                );
            }
            Err(error @ (SexpError::Conflict { .. } | SexpError::ItemConflict { .. })) => {
                self.status = format!("{operation} staging conflict: {error}");
            }
            Err(error) => self.status = format!("Could not stage {operation}: {error}"),
        }
    }

    pub(super) fn rotate_selected(&mut self) {
        self.transform_selected_items(
            "Rotate",
            "Rotated",
            "symbol(s) or bus entry/entries",
            |kind| matches!(kind, ObjectKind::Symbol | ObjectKind::BusEntry),
            rotate_item_source,
        );
    }

    pub(super) fn mirror_selected(&mut self, axis: &'static str) {
        self.transform_selected_items(
            "Mirror",
            "Mirrored",
            "symbol(s)",
            |kind| kind == ObjectKind::Symbol,
            move |source, uuid| crate::native_scene::mirror_symbol_source(source, uuid, axis),
        );
    }

    pub(super) fn duplicate_selected(&mut self) {
        if !self.require_edit_mode() {
            return;
        }
        if self.selected_uuids.len() != 1 {
            self.status = "Select exactly one symbol to duplicate".to_owned();
            return;
        }
        let Some(uuid) = self.selected_uuids.iter().next().cloned() else {
            return;
        };
        let Some(sheet) = self.sheets.get(self.active) else {
            return;
        };
        if !sheet
            .semantic
            .objects
            .iter()
            .any(|object| object.uuid == uuid && object.kind == ObjectKind::Symbol)
        {
            self.status = "Duplicate currently supports placed symbols".to_owned();
            return;
        }
        let file = sheet.file.clone();
        let (duplicate, duplicate_uuid) =
            match duplicate_symbol_block(&sheet.semantic.source, &uuid, self.grid_mm, self.grid_mm)
            {
                Ok(duplicate) => duplicate,
                Err(error) => {
                    self.status = format!("Duplicate rejected: {error:#}");
                    return;
                }
            };
        let command = match SchematicCommand::insert_item(
            &sheet.semantic.source,
            duplicate,
            ItemAnchor::BeforeFooter,
            "Duplicate symbol",
        ) {
            Ok(command) => command,
            Err(error) => {
                self.status = format!("Duplicate rejected: {error}");
                return;
            }
        };
        match self.stage_command(&file, &command) {
            Ok(outcome) => {
                self.record_command_change(ChangeOrigin::Local, &file, &command);
                push_history(
                    &mut self.undo_stack,
                    HistoryEntry::single(file.clone(), outcome.inverse),
                );
                self.redo_stack.clear();
                self.selected_uuids.clear();
                self.selected_uuids.insert(duplicate_uuid);
                self.status =
                    "Staged duplicate · reference left unannotated · press Commit".to_owned();
            }
            Err(error @ (SexpError::Conflict { .. } | SexpError::ItemConflict { .. })) => {
                self.status = format!("Duplicate staging conflict: {error}");
            }
            Err(error) => self.status = format!("Could not stage duplicate: {error}"),
        }
    }

    pub(super) fn snap_schematic_point(&self, point: SchPoint) -> SchPoint {
        snap_point(point, self.snap_enabled, self.grid_mm)
    }

    pub(super) fn start_wire(&mut self, width: f64, height: f64, is_bus: bool) {
        if !self.require_edit_mode() {
            return;
        }
        let Some(cursor) = self.cursor else {
            self.status = "Move the pointer onto the sheet before starting a wire".to_owned();
            return;
        };
        if !self.main_rect(width, height).contains(cursor.x, cursor.y) {
            self.status = "Start wires inside the schematic page".to_owned();
            return;
        }
        let Some(point) = self.schematic_point(width, height, cursor.x, cursor.y) else {
            return;
        };
        let point = self.snap_schematic_point(point);
        self.wire_draft = Some(WireDraft {
            start: point,
            current: point,
            is_bus,
        });
        let kind = if is_bus { "Bus" } else { "Wire" };
        self.status = format!("{kind} · click endpoint to stage · Esc to cancel");
    }

    pub(super) fn update_wire_draft(
        &mut self,
        width: f64,
        height: f64,
        cursor: PhysicalPosition<f64>,
    ) {
        let Some(point) = self.schematic_point(width, height, cursor.x, cursor.y) else {
            return;
        };
        let point = self.snap_schematic_point(point);
        if let Some(wire) = &mut self.wire_draft {
            wire.current = point;
        }
    }

    pub(super) fn commit_wire(&mut self) {
        let Some(wire) = self.wire_draft.take() else {
            return;
        };
        if (wire.current.x - wire.start.x).hypot(wire.current.y - wire.start.y) < 1e-9 {
            let kind = if wire.is_bus { "Bus" } else { "Wire" };
            self.status = format!("{kind} needs two distinct endpoints");
            return;
        }
        let Some(sheet) = self.sheets.get(self.active) else {
            return;
        };
        let file = sheet.file.clone();
        let source = Arc::clone(&sheet.semantic.source);
        let (block, kind, display_kind) = if wire.is_bus {
            (
                konnect_sexp::schematic::format_bus(
                    wire.start.x,
                    wire.start.y,
                    wire.current.x,
                    wire.current.y,
                ),
                "bus",
                "Bus",
            )
        } else {
            (
                konnect_sexp::schematic::format_wire(
                    wire.start.x,
                    wire.start.y,
                    wire.current.x,
                    wire.current.y,
                ),
                "wire",
                "Wire",
            )
        };
        let command = match SchematicCommand::insert_item(
            &source,
            block,
            ItemAnchor::BeforeFooter,
            format!("Add {kind}"),
        ) {
            Ok(command) => command,
            Err(error) => {
                self.status = format!("{display_kind} rejected: {error}");
                return;
            }
        };
        match self.stage_command(&file, &command) {
            Ok(outcome) => {
                self.record_command_change(ChangeOrigin::Local, &file, &command);
                push_history(
                    &mut self.undo_stack,
                    HistoryEntry::single(file.clone(), outcome.inverse),
                );
                self.redo_stack.clear();
                self.status = format!("Staged {kind} · press Commit to write");
            }
            Err(error @ (SexpError::Conflict { .. } | SexpError::ItemConflict { .. })) => {
                self.status = format!("{display_kind} staging conflict: {error}");
            }
            Err(error) => self.status = format!("Could not stage {kind}: {error}"),
        }
    }

    pub(super) fn cursor_schematic_point(&self, width: f64, height: f64) -> Option<SchPoint> {
        let cursor = self.cursor?;
        if !self.main_rect(width, height).contains(cursor.x, cursor.y) {
            return None;
        }
        self.schematic_point(width, height, cursor.x, cursor.y)
            .map(|point| self.snap_schematic_point(point))
    }

    pub(super) fn insert_at_cursor(
        &mut self,
        width: f64,
        height: f64,
        description: &'static str,
        format: impl FnOnce(SchPoint) -> String,
    ) {
        let Some(point) = self.cursor_schematic_point(width, height) else {
            self.status = format!("Place {description} inside the schematic page");
            return;
        };
        self.commit_insert(format(point), description);
    }

    pub(super) fn commit_insert(&mut self, block: String, description: &str) {
        if !self.require_edit_mode() {
            return;
        }
        let Some((file, source)) = self
            .sheets
            .get(self.active)
            .map(|sheet| (sheet.file.clone(), Arc::clone(&sheet.semantic.source)))
        else {
            return;
        };
        let command = match SchematicCommand::insert_item(
            &source,
            block,
            ItemAnchor::BeforeFooter,
            format!("Add {description}"),
        ) {
            Ok(command) => command,
            Err(error) => {
                self.status = format!("{description} rejected: {error}");
                return;
            }
        };
        match self.stage_command(&file, &command) {
            Ok(outcome) => {
                self.record_command_change(ChangeOrigin::Local, &file, &command);
                push_history(
                    &mut self.undo_stack,
                    HistoryEntry::single(file.clone(), outcome.inverse),
                );
                self.redo_stack.clear();
                self.status = format!("Staged {description} · press Commit to write");
            }
            Err(error @ (SexpError::Conflict { .. } | SexpError::ItemConflict { .. })) => {
                self.status = format!("{description} staging conflict: {error}");
            }
            Err(error) => self.status = format!("Could not stage {description}: {error}"),
        }
    }

    pub(super) fn start_label_edit(&mut self, width: f64, height: f64) {
        if !self.require_edit_mode() {
            return;
        }
        let Some(point) = self.cursor_schematic_point(width, height) else {
            self.status = "Place labels inside the schematic page".to_owned();
            return;
        };
        self.label_edit = Some(LabelEdit {
            point,
            value: String::new(),
        });
        self.status = "New local label · enter a net name · Enter to place".to_owned();
    }

    pub(super) fn commit_label_edit(&mut self) {
        let Some(edit) = self.label_edit.take() else {
            return;
        };
        let value = edit.value.trim();
        if value.is_empty() {
            self.label_edit = Some(edit);
            self.status = "A label needs a non-empty net name".to_owned();
            return;
        }
        self.commit_insert(
            konnect_sexp::schematic::format_net_label(value, edit.point.x, edit.point.y, 0.0),
            "local label",
        );
    }

    pub(super) fn handle_label_key(&mut self, key: Key<&str>) -> bool {
        if self.label_edit.is_none() {
            return false;
        }
        match key {
            Key::Named(NamedKey::Escape) => {
                self.label_edit = None;
                self.status = self.live_status();
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some(edit) = &mut self.label_edit {
                    edit.value.pop();
                }
            }
            Key::Named(NamedKey::Enter) => self.commit_label_edit(),
            Key::Character(value)
                if !self.modifiers.control_key() && !self.modifiers.super_key() =>
            {
                if let Some(edit) = &mut self.label_edit {
                    edit.value.push_str(value);
                }
            }
            _ => {}
        }
        true
    }

    pub(super) fn start_sheet_edit(&mut self, width: f64, height: f64) {
        if !self.require_edit_mode() {
            return;
        }
        let Some(mut point) = self.cursor_schematic_point(width, height) else {
            self.status = "Place hierarchical sheets inside the schematic page".to_owned();
            return;
        };
        if let Some(sheet) = self.sheets.get(self.active) {
            point.x = point
                .x
                .clamp(2.54, (sheet.semantic.width_mm - 82.54).max(2.54));
            point.y = point
                .y
                .clamp(2.54, (sheet.semantic.height_mm - 52.54).max(2.54));
        }
        self.sheet_edit = Some(SheetEdit {
            point,
            name: String::new(),
            file: String::new(),
            field: SheetEditField::Name,
        });
        self.status = "New hierarchical sheet · Tab switches name/file · Enter creates".to_owned();
    }

    pub(super) fn handle_sheet_key(&mut self, key: Key<&str>) -> bool {
        if self.sheet_edit.is_none() {
            return false;
        }
        match key {
            Key::Named(NamedKey::Escape) => {
                self.sheet_edit = None;
                self.status = self.live_status();
            }
            Key::Named(NamedKey::Tab) => {
                if let Some(edit) = &mut self.sheet_edit {
                    edit.field = match edit.field {
                        SheetEditField::Name => SheetEditField::File,
                        SheetEditField::File => SheetEditField::Name,
                    };
                }
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some(edit) = &mut self.sheet_edit {
                    match edit.field {
                        SheetEditField::Name => {
                            edit.name.pop();
                        }
                        SheetEditField::File => {
                            edit.file.pop();
                        }
                    }
                }
            }
            Key::Named(NamedKey::Enter) => self.commit_sheet_edit(),
            Key::Character(value)
                if !self.modifiers.control_key() && !self.modifiers.super_key() =>
            {
                if let Some(edit) = &mut self.sheet_edit {
                    match edit.field {
                        SheetEditField::Name => edit.name.push_str(value),
                        SheetEditField::File => edit.file.push_str(value),
                    }
                }
            }
            _ => {}
        }
        true
    }

    pub(super) fn start_sheet_pin_edit(&mut self, width: f64, height: f64) {
        if !self.require_edit_mode() {
            return;
        }
        let Some(sheet_uuid) = self
            .selected_uuids
            .iter()
            .next()
            .filter(|_| self.selected_uuids.len() == 1)
            .cloned()
        else {
            self.status = "Select exactly one hierarchical sheet before adding a pin".to_owned();
            return;
        };
        let Some(sheet) = self.sheets.get(self.active) else {
            return;
        };
        if !sheet
            .semantic
            .objects
            .iter()
            .any(|object| object.uuid == sheet_uuid && object.kind == ObjectKind::Sheet)
        {
            self.status = "The selected item is not a hierarchical sheet".to_owned();
            return;
        }
        let Some(rectangle) = sheet_rectangle(&sheet.semantic.source, &sheet_uuid) else {
            self.status = "Could not read the selected sheet geometry".to_owned();
            return;
        };
        let cursor = self
            .cursor_schematic_point(width, height)
            .unwrap_or(SchPoint {
                x: rectangle.2,
                y: (rectangle.1 + rectangle.3) / 2.0,
            });
        let (point, rotation) =
            nearest_sheet_edge(rectangle, cursor, self.grid_mm, self.snap_enabled);
        self.sheet_pin_edit = Some(SheetPinEdit {
            sheet_uuid,
            point,
            rotation,
            name: String::new(),
            pin_type: SheetPinType::Passive,
        });
        self.status = "New sheet pin · Tab cycles electrical type · Enter places".to_owned();
    }

    pub(super) fn handle_sheet_pin_key(&mut self, key: Key<&str>) -> bool {
        if self.sheet_pin_edit.is_none() {
            return false;
        }
        match key {
            Key::Named(NamedKey::Escape) => {
                self.sheet_pin_edit = None;
                self.status = self.live_status();
            }
            Key::Named(NamedKey::Tab) => {
                if let Some(edit) = &mut self.sheet_pin_edit {
                    edit.pin_type = edit.pin_type.next();
                }
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some(edit) = &mut self.sheet_pin_edit {
                    edit.name.pop();
                }
            }
            Key::Named(NamedKey::Enter) => self.commit_sheet_pin_edit(),
            Key::Character(value)
                if !self.modifiers.control_key() && !self.modifiers.super_key() =>
            {
                if let Some(edit) = &mut self.sheet_pin_edit {
                    edit.name.push_str(value);
                }
            }
            _ => {}
        }
        true
    }

    pub(super) fn commit_sheet_pin_edit(&mut self) {
        let Some(edit) = self.sheet_pin_edit.take() else {
            return;
        };
        let name = edit.name.trim().to_owned();
        if name.is_empty() {
            self.sheet_pin_edit = Some(edit);
            self.status = "A hierarchical sheet pin needs a non-empty name".to_owned();
            return;
        }
        let Some(sheet) = self.sheets.get(self.active) else {
            return;
        };
        let file = sheet.file.clone();
        let source = Arc::clone(&sheet.semantic.source);
        let pin = konnect_sexp::schematic::format_sheet_pin(
            &name,
            edit.pin_type,
            edit.point.x,
            edit.point.y,
            edit.rotation,
        );
        let command = match SchematicCommand::insert_sheet_pin(
            &source,
            match ItemId::new(edit.sheet_uuid.clone()) {
                Ok(id) => id,
                Err(error) => {
                    self.status = format!("Sheet pin rejected: {error}");
                    return;
                }
            },
            &pin,
            "Add hierarchical sheet pin",
        ) {
            Ok(command) => command,
            Err(error) => {
                self.sheet_pin_edit = Some(edit);
                self.status = format!("Sheet pin rejected: {error}");
                return;
            }
        };
        match self.stage_command(&file, &command) {
            Ok(outcome) => {
                self.record_command_change(ChangeOrigin::Local, &file, &command);
                push_history(
                    &mut self.undo_stack,
                    HistoryEntry::single(file.clone(), outcome.inverse),
                );
                self.redo_stack.clear();
                self.status = format!(
                    "Staged {name} {} sheet pin · press Commit",
                    edit.pin_type.keyword()
                );
            }
            Err(error @ (SexpError::Conflict { .. } | SexpError::ItemConflict { .. })) => {
                self.sheet_pin_edit = Some(edit);
                self.status = format!("Sheet-pin staging conflict: {error}");
            }
            Err(error) => {
                self.sheet_pin_edit = Some(edit);
                self.status = format!("Could not stage sheet pin: {error}");
            }
        }
    }

    pub(super) fn commit_sheet_edit(&mut self) {
        let Some(edit) = self.sheet_edit.take() else {
            return;
        };
        let name = edit.name.trim().to_owned();
        let file_name = edit.file.trim().to_owned();
        if name.is_empty() || file_name.is_empty() {
            self.sheet_edit = Some(edit);
            self.status = "Hierarchical sheet needs both a name and child filename".to_owned();
            return;
        }
        let relative = Path::new(&file_name);
        let valid_relative = !relative.is_absolute()
            && relative
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_)))
            && relative
                .extension()
                .is_some_and(|extension| extension == "kicad_sch");
        if !valid_relative {
            self.sheet_edit = Some(edit);
            self.status =
                "Child file must be a relative .kicad_sch path without parent traversal".to_owned();
            return;
        }
        let Some(sheet) = self.sheets.get(self.active) else {
            return;
        };
        if sheet.semantic.objects.iter().any(|object| {
            object.kind == ObjectKind::Sheet
                && object
                    .properties
                    .iter()
                    .any(|property| property.name == "Sheetname" && property.value == name)
        }) {
            self.sheet_edit = Some(edit);
            self.status = format!("A hierarchical sheet named ‘{name}’ already exists");
            return;
        }
        let parent_file = sheet.file.clone();
        let source = Arc::clone(&sheet.semantic.source);
        let Some(parent_dir) = parent_file.parent().map(Path::to_path_buf) else {
            self.status = "Parent schematic has no directory".to_owned();
            return;
        };
        let child_file = parent_dir.join(relative);
        if !child_file.parent().is_some_and(Path::is_dir) {
            self.sheet_edit = Some(edit);
            self.status = "The child sheet directory does not exist".to_owned();
            return;
        }
        if path_key(&child_file) == path_key(&parent_file) {
            self.sheet_edit = Some(edit);
            self.status = "A sheet cannot reference itself".to_owned();
            return;
        }
        let child_key = path_key(&child_file);
        let existing_child_source = if let Some(staged) =
            self.edit_session.staged_source(&child_key)
        {
            Some(staged.to_owned())
        } else if child_file.is_file() {
            let child_source = match read_consistent(&child_file) {
                Ok(source) => source,
                Err(error) => {
                    self.sheet_edit = Some(edit);
                    self.status = format!("Could not inspect child sheet: {error}");
                    return;
                }
            };
            Some(child_source)
        } else if child_file.exists() {
            self.sheet_edit = Some(edit);
            self.status = "The child schematic path exists but is not a regular file".to_owned();
            return;
        } else {
            None
        };
        let (parent_instance_path, page) = match next_sheet_instance(&source) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.sheet_edit = Some(edit);
                self.status = format!("Sheet metadata rejected: {error:#}");
                return;
            }
        };
        let project_name = self
            .root
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("project")
            .to_owned();
        let block = konnect_sexp::schematic::format_hierarchical_sheet(HierarchicalSheetSpec {
            name: &name,
            file: &file_name,
            x: edit.point.x,
            y: edit.point.y,
            width: 80.0,
            height: 50.0,
            project_name: &project_name,
            parent_instance_path: &parent_instance_path,
            page: &page,
        });
        let command = match SchematicCommand::insert_item(
            &source,
            block,
            ItemAnchor::BeforeFooter,
            "Add hierarchical sheet",
        ) {
            Ok(command) => command.requiring_unchanged_document(),
            Err(error) => {
                self.sheet_edit = Some(edit);
                self.status = format!("Sheet creation rejected: {error}");
                return;
            }
        };
        let Some(sheet_uuid) = command.changes.first().map(|change| change.id.to_string()) else {
            self.sheet_edit = Some(edit);
            self.status = "Sheet insertion unexpectedly produced no item change".to_owned();
            return;
        };
        let hierarchy_path = format!(
            "{}/{}",
            parent_instance_path.trim_end_matches('/'),
            sheet_uuid
        );
        let child_patch = match &existing_child_source {
            Some(child_source) => match SchematicCommand::ensure_symbol_instance_path(
                child_source,
                &project_name,
                &hierarchy_path,
                "Link hierarchical child symbols",
            ) {
                Ok(Some(child_command)) => {
                    match prepare_command(&child_file, child_source, &child_command) {
                        Ok((replacement, outcome)) => {
                            Some((replacement, outcome, child_command.changes.len()))
                        }
                        Err(error) => {
                            self.sheet_edit = Some(edit);
                            self.status = format!("Child instance patch conflict: {error}");
                            return;
                        }
                    }
                }
                Ok(None) => None,
                Err(error) => {
                    self.sheet_edit = Some(edit);
                    self.status = format!("Child instance patch rejected: {error}");
                    return;
                }
            },
            None => None,
        };
        let (parent_after, parent_outcome) = match prepare_command(&parent_file, &source, &command)
        {
            Ok(prepared) => prepared,
            Err(error) => {
                self.sheet_edit = Some(edit);
                self.status = format!("Sheet creation conflict: {error}");
                return;
            }
        };
        let child_creation = existing_child_source
            .is_none()
            .then(konnect_sexp::schematic::format_blank_schematic);
        let mut staged_session = self.edit_session.clone();
        let staging_result = staged_session
            .stage_replacement(
                path_key(&parent_file),
                &parent_file,
                &source,
                parent_after.clone(),
            )
            .and_then(|()| {
                if let Some(child_source) = &child_creation {
                    staged_session.stage_creation(
                        child_key.clone(),
                        &child_file,
                        child_source.clone(),
                    )?;
                }
                if let (Some(child_source), Some((replacement, _, _))) =
                    (&existing_child_source, &child_patch)
                {
                    staged_session.stage_replacement(
                        child_key.clone(),
                        &child_file,
                        child_source,
                        replacement.clone(),
                    )?;
                }
                Ok(())
            });
        match staging_result {
            Ok(()) => {
                self.edit_session = staged_session;
                self.apply_staged_source(&parent_file, parent_after);
                self.record_command_change(ChangeOrigin::Local, &parent_file, &command);
                let history = if let Some((_, child_outcome, _)) = &child_patch {
                    HistoryEntry::group(
                        parent_dir.clone(),
                        vec![
                            HistoryCommand {
                                file: parent_file.clone(),
                                command: parent_outcome.inverse,
                            },
                            HistoryCommand {
                                file: child_file.clone(),
                                command: child_outcome.inverse.clone(),
                            },
                        ],
                    )
                } else if let Some(child_source) = &child_creation {
                    HistoryEntry::group_with_creations(
                        parent_dir.clone(),
                        vec![HistoryCommand {
                            file: parent_file.clone(),
                            command: parent_outcome.inverse,
                        }],
                        vec![HistoryCreation {
                            file: child_file.clone(),
                            source: child_source.clone(),
                        }],
                    )
                } else {
                    HistoryEntry::single(parent_file.clone(), parent_outcome.inverse)
                };
                push_history(&mut self.undo_stack, history);
                self.redo_stack.clear();
                self.status = if let Some((_, _, count)) = child_patch {
                    format!(
                        "Staged {file_name} link and {count} child symbol patch(es) · press Commit"
                    )
                } else if child_creation.is_some() {
                    format!("Staged new child {file_name} and link · press Commit")
                } else {
                    format!("Staged existing child link {file_name} · press Commit")
                };
            }
            Err(error) => {
                self.sheet_edit = Some(edit);
                self.status = format!("Sheet staging stopped safely: {error}");
            }
        }
    }

    pub(super) fn undo(&mut self) {
        self.apply_history(true);
    }

    pub(super) fn redo(&mut self) {
        self.apply_history(false);
    }

    pub(super) fn apply_history(&mut self, undoing: bool) {
        if !self.require_edit_mode() {
            return;
        }
        let entry = if undoing {
            self.undo_stack.pop()
        } else {
            self.redo_stack.pop()
        };
        let Some(entry) = entry else {
            self.status = if undoing {
                "Nothing to undo".to_owned()
            } else {
                "Nothing to redo".to_owned()
            };
            return;
        };

        if entry.journal_root.is_some() || entry.commands.len() > 1 || !entry.creations.is_empty() {
            self.apply_grouped_history(undoing, entry);
            return;
        }
        let Some(part) = entry.commands.into_iter().next() else {
            self.status = "History entry had no commands".to_owned();
            return;
        };
        match self.stage_command(&part.file, &part.command) {
            Ok(outcome) => {
                self.record_command_change(
                    if undoing {
                        ChangeOrigin::Undo
                    } else {
                        ChangeOrigin::Redo
                    },
                    &part.file,
                    &part.command,
                );
                let inverse = HistoryEntry::single(part.file.clone(), outcome.inverse);
                if undoing {
                    push_history(&mut self.redo_stack, inverse);
                } else {
                    push_history(&mut self.undo_stack, inverse);
                }
                let rebased = if outcome.rebased {
                    " · safely rebased"
                } else {
                    ""
                };
                self.status = format!("Staged {}{rebased} · press Commit", part.command.label);
            }
            Err(error @ (SexpError::Conflict { .. } | SexpError::ItemConflict { .. })) => {
                self.restore_history_entry(undoing, HistoryEntry::single(part.file, part.command));
                self.status = format!("Staged history conflict: {error}");
            }
            Err(error) => {
                self.restore_history_entry(undoing, HistoryEntry::single(part.file, part.command));
                self.status = format!("Could not stage history operation: {error}");
            }
        }
    }

    pub(super) fn apply_grouped_history(&mut self, undoing: bool, entry: HistoryEntry) {
        let Some(journal_root) = entry.journal_root.clone() else {
            self.restore_history_entry(undoing, entry);
            self.status = "Grouped history has no project transaction directory".to_owned();
            return;
        };
        let label = entry
            .commands
            .first()
            .map(|part| part.command.label.clone())
            .unwrap_or_else(|| "grouped edit".to_owned());
        let mut candidate = self.edit_session.clone();
        let mut inverses = Vec::with_capacity(entry.commands.len());
        let mut rendered_sources = Vec::with_capacity(entry.commands.len());
        for part in &entry.commands {
            let key = path_key(&part.file);
            let current = candidate
                .staged_source(&key)
                .map(str::to_owned)
                .or_else(|| {
                    self.sheets
                        .iter()
                        .find(|sheet| path_key(&sheet.file) == key)
                        .map(|sheet| sheet.semantic.source.to_string())
                })
                .or_else(|| read_consistent(&part.file).ok());
            let Some(current) = current else {
                self.restore_history_entry(undoing, entry.clone());
                self.status = format!("Could not read {} for staged history", part.file.display());
                return;
            };
            let (replacement, outcome) = match prepare_command(&part.file, &current, &part.command)
            {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.restore_history_entry(undoing, entry.clone());
                    self.status = format!("Grouped staging conflict: {error}");
                    return;
                }
            };
            if let Err(error) =
                candidate.stage_replacement(key, &part.file, &current, replacement.clone())
            {
                self.restore_history_entry(undoing, entry.clone());
                self.status = format!("Grouped staging conflict: {error}");
                return;
            }
            inverses.push(HistoryCommand {
                file: part.file.clone(),
                command: outcome.inverse,
            });
            rendered_sources.push((part.file.clone(), replacement));
        }
        for creation in &entry.creations {
            let key = path_key(&creation.file);
            let result = if undoing {
                candidate.cancel_creation(&key, &creation.file, &creation.source)
            } else {
                candidate.stage_creation(key, &creation.file, creation.source.clone())
            };
            if let Err(error) = result {
                self.restore_history_entry(undoing, entry.clone());
                self.status = format!("Grouped creation history conflict: {error}");
                return;
            }
        }
        self.edit_session = candidate;
        for (file, source) in rendered_sources {
            self.apply_staged_source(&file, source);
        }
        let inverse = HistoryEntry::group_with_creations(
            journal_root.clone(),
            inverses,
            entry.creations.clone(),
        );
        let timeline_uuids = inverse
            .commands
            .iter()
            .flat_map(|part| part.command.changes.iter())
            .map(|change| change.id.as_str().to_owned())
            .collect();
        self.record_change(
            if undoing {
                ChangeOrigin::Undo
            } else {
                ChangeOrigin::Redo
            },
            label.clone(),
            journal_root,
            timeline_uuids,
        );
        if undoing {
            push_history(&mut self.redo_stack, inverse);
        } else {
            push_history(&mut self.undo_stack, inverse);
        }
        self.status = format!("Staged {label} across multiple files · press Commit");
    }

    pub(super) fn restore_history_entry(&mut self, undoing: bool, entry: HistoryEntry) {
        if undoing {
            push_history(&mut self.undo_stack, entry);
        } else {
            push_history(&mut self.redo_stack, entry);
        }
    }
}
