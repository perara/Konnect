//! Reload workers, compatibility fallback, headless rendering, and benchmark CLI handling.

use super::*;

pub(super) fn spawn_reload_worker(
    receiver: mpsc::Receiver<ReloadRequest>,
    proxy: EventLoopProxy<UserEvent>,
    latest_generation: Arc<AtomicU64>,
) -> Result<()> {
    std::thread::Builder::new()
        .name("schematic-reload".to_owned())
        .spawn(move || {
            let mut dirty = HashSet::new();
            while let Ok(mut request) = receiver.recv() {
                let mut external = request.external;
                dirty.extend(request.changed.iter().cloned());
                while let Ok(next) = receiver.try_recv() {
                    external |= next.external;
                    dirty.extend(next.changed.iter().cloned());
                    request = next;
                }
                request.changed.clone_from(&dirty);
                request.external = external;

                let Some(batch) = build_reload_batch(&request, &latest_generation) else {
                    continue;
                };
                if proxy.send_event(UserEvent::Reloaded(batch)).is_err() {
                    break;
                }
                dirty.clear();
            }
        })
        .context("failed to start schematic reload worker")?;
    Ok(())
}

pub(super) fn build_reload_batch(
    request: &ReloadRequest,
    latest_generation: &AtomicU64,
) -> Option<ReloadBatch> {
    let entries = match discover_hierarchy(&request.root) {
        Ok(entries) => entries,
        Err(error) => {
            return (latest_generation.load(Ordering::Acquire) == request.generation).then(|| {
                ReloadBatch {
                    generation: request.generation,
                    entries: Err(format!("{error:#}")),
                    loaded: HashMap::new(),
                    external: request.external,
                }
            });
        }
    };
    if latest_generation.load(Ordering::Acquire) != request.generation {
        return None;
    }

    let mut loaded = HashMap::new();
    for entry in &entries {
        let key = path_key(&entry.file);
        if !request.changed.contains(&key) && request.known.contains(&key) {
            continue;
        }
        if latest_generation.load(Ordering::Acquire) != request.generation {
            return None;
        }
        loaded.insert(
            key,
            load_scene_with_fallback(&request.root, &entries, &entry.file),
        );
    }
    (latest_generation.load(Ordering::Acquire) == request.generation).then_some(ReloadBatch {
        generation: request.generation,
        entries: Ok(entries),
        loaded,
        external: request.external,
    })
}

pub(super) fn load_scene_with_fallback(
    root: &Path,
    entries: &[HierarchyEntry],
    file: &Path,
) -> std::result::Result<LoadedScene, String> {
    let semantic = SchematicScene::load(file).map_err(|error| format!("{error:#}"))?;
    if semantic.coverage.is_complete() {
        return Ok(LoadedScene {
            semantic,
            compatibility: None,
            compatibility_error: None,
        });
    }
    match crate::compat_svg::load_or_export(root, entries, file) {
        Ok(svg) => match compatibility_scene(&svg) {
            Ok(compatibility) => Ok(LoadedScene {
                semantic,
                compatibility: Some(compatibility),
                compatibility_error: None,
            }),
            Err(error) => Ok(LoadedScene {
                semantic,
                compatibility: None,
                compatibility_error: Some(format!("KiCad SVG parse failed: {error:#}")),
            }),
        },
        Err(error) => Ok(LoadedScene {
            semantic,
            compatibility: None,
            compatibility_error: Some(error),
        }),
    }
}

pub(super) fn run_native() -> Result<()> {
    #[cfg(feature = "golden-svg-reference")]
    if let Some((output, width, height, svg)) = render_svg_png_argument()? {
        return render_svg_png(&svg, &output, width, height);
    }
    let root = schematic_argument()?;
    let journal_root = root
        .parent()
        .ok_or_else(|| anyhow!("the root schematic has no project directory"))?;
    let recovered = recover_file_transactions(journal_root).with_context(|| {
        format!(
            "failed to recover transactions in {}",
            journal_root.display()
        )
    })?;
    if let Some(iterations) = benchmark_iterations_argument()? {
        return benchmark_scene_pipeline(&root, iterations);
    }
    if let Some(output) = render_png_argument() {
        return render_png(&root, &output, 10.0);
    }
    let font = NativeFont::load()?;
    let hierarchy = load_hierarchy(&root)?;
    let palette = Theme::Light.palette();
    let sheets = hierarchy
        .into_iter()
        .map(|entry| NativeSheet::from_hierarchy(entry, palette))
        .collect::<Vec<_>>();
    if sheets.is_empty() {
        return Err(anyhow!("the schematic hierarchy is empty"));
    }

    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .context("failed to create native event loop")?;
    let proxy = event_loop.create_proxy();
    let reload_generation = Arc::new(AtomicU64::new(0));
    let (reload_tx, reload_rx) = mpsc::channel();
    spawn_reload_worker(reload_rx, proxy.clone(), Arc::clone(&reload_generation))?;
    let callback_proxy = proxy.clone();
    let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            return;
        };
        if !matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        ) {
            return;
        }
        let paths = event
            .paths
            .into_iter()
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "kicad_sch")
            })
            .collect::<Vec<_>>();
        if !paths.is_empty() {
            let _ = callback_proxy.send_event(UserEvent::FilesChanged(paths));
        }
    })
    .context("failed to create schematic watcher")?;

    let mut viewer = VelloViewer::new(root, font, sheets, watcher, reload_tx, reload_generation);
    if !recovered.is_empty() {
        let files = recovered
            .iter()
            .map(|outcome| outcome.completed_files)
            .sum::<usize>();
        viewer.status = format!(
            "Recovered {} interrupted transaction(s), completing {files} file(s)",
            recovered.len()
        );
    }
    let initial_files = viewer
        .sheets
        .iter()
        .map(|sheet| sheet.file.clone())
        .collect::<Vec<_>>();
    viewer.schedule_reload(&initial_files);
    event_loop
        .run_app(&mut viewer)
        .context("native event loop failed")
}

#[cfg(feature = "golden-svg-reference")]
pub(super) fn render_svg_png(
    svg_path: &Path,
    output: &Path,
    width: u32,
    height: u32,
) -> Result<()> {
    const SVG_PX_TO_MM: f64 = 25.4 / 96.0;

    let source = std::fs::read_to_string(svg_path)
        .with_context(|| format!("failed to read {}", svg_path.display()))?;
    let options = vello_svg::usvg::Options::default();
    let tree = vello_svg::usvg::Tree::from_str(&source, &options)
        .with_context(|| format!("failed to parse {}", svg_path.display()))?;
    let mut reference = Scene::new();
    append_svg_group_flat(&mut reference, tree.root());
    let size = tree.size();
    let scale_x = f64::from(width) / (f64::from(size.width()) * SVG_PX_TO_MM);
    let scale_y = f64::from(height) / (f64::from(size.height()) * SVG_PX_TO_MM);
    // The native golden renderer uses a fixed physical scale and rounds the
    // output extent to whole pixels. Do the same here instead of stretching
    // the slightly-over-nominal KiCad paper dimensions independently in X/Y.
    let pixels_per_mm = ((scale_x + scale_y) / 2.0).round().max(1.0);
    let mut scene = Scene::new();
    scene.append(&reference, Some(Affine::scale(pixels_per_mm)));
    pollster::block_on(render_scene_png(
        &scene,
        output,
        width,
        height,
        Theme::Light.palette().page,
    ))
}

pub(super) fn compatibility_scene(source: &str) -> Result<Scene> {
    let options = vello_svg::usvg::Options::default();
    let tree = vello_svg::usvg::Tree::from_str(source, &options)
        .context("failed to parse KiCad compatibility SVG")?;
    let mut scene = Scene::new();
    append_svg_group_flat(&mut scene, tree.root());
    Ok(scene)
}

pub(super) fn append_svg_group_flat(scene: &mut Scene, group: &vello_svg::usvg::Group) {
    const SVG_PX_TO_MM: f64 = 25.4 / 96.0;
    static PATH_INDEX: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    for node in group.children() {
        match node {
            vello_svg::usvg::Node::Group(group) => append_svg_group_flat(scene, group),
            vello_svg::usvg::Node::Path(path) if path.is_visible() => {
                let geometry = vello_svg::util::to_bez_path(path);
                let transform = canonical_svg_transform(
                    Affine::scale(SVG_PX_TO_MM) * vello_svg::util::to_affine(&path.abs_transform()),
                );
                let path_index = PATH_INDEX.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if std::env::var_os("KONNECT_SVG_STATS").is_some() && path_index < 12 {
                    eprintln!(
                        "svg path {path_index}: bbox={:?} transform={transform:?} stroke={:?}",
                        path.bounding_box(),
                        path.stroke().map(|stroke| stroke.width())
                    );
                }
                match path.paint_order() {
                    vello_svg::usvg::PaintOrder::FillAndStroke => {
                        append_svg_fill(scene, path, &geometry, transform);
                        append_svg_stroke(scene, path, &geometry, transform);
                    }
                    vello_svg::usvg::PaintOrder::StrokeAndFill => {
                        append_svg_stroke(scene, path, &geometry, transform);
                        append_svg_fill(scene, path, &geometry, transform);
                    }
                }
            }
            // KiCad emits invisible SVG text for search/accessibility and
            // follows it with the authoritative visible Newstroke paths.
            // Flattening the invisible text loses its group opacity and
            // renders a duplicate, sometimes at an untransformed origin.
            vello_svg::usvg::Node::Text(_)
            | vello_svg::usvg::Node::Path(_)
            | vello_svg::usvg::Node::Image(_) => {}
        }
    }
}

pub(super) fn canonical_svg_transform(transform: Affine) -> Affine {
    let mut coefficients = transform.as_coeffs();
    for (index, coefficient) in coefficients.iter_mut().enumerate() {
        let identity = matches!(index, 0 | 3);
        let target = if identity { 1.0 } else { 0.0 };
        if (*coefficient - target).abs() < 1e-5 {
            *coefficient = target;
        }
    }
    Affine::new(coefficients)
}

pub(super) fn append_svg_fill(
    scene: &mut Scene,
    path: &vello_svg::usvg::Path,
    geometry: &BezPath,
    transform: Affine,
) {
    let Some(fill) = path.fill() else {
        return;
    };
    let Some((brush, brush_transform)) = vello_svg::util::to_brush(fill.paint(), fill.opacity())
    else {
        return;
    };
    scene.fill(
        match fill.rule() {
            vello_svg::usvg::FillRule::NonZero => Fill::NonZero,
            vello_svg::usvg::FillRule::EvenOdd => Fill::EvenOdd,
        },
        transform,
        &brush,
        Some(brush_transform),
        geometry,
    );
}

pub(super) fn append_svg_stroke(
    scene: &mut Scene,
    path: &vello_svg::usvg::Path,
    geometry: &BezPath,
    transform: Affine,
) {
    let Some(stroke) = path.stroke() else {
        return;
    };
    let Some((brush, brush_transform)) =
        vello_svg::util::to_brush(stroke.paint(), stroke.opacity())
    else {
        return;
    };
    scene.stroke(
        &vello_svg::util::to_stroke(stroke),
        transform,
        &brush,
        Some(brush_transform),
        geometry,
    );
}

pub(super) fn render_png(schematic: &Path, output: &Path, pixels_per_mm: f64) -> Result<()> {
    let palette = Theme::Light.palette();
    let semantic = SchematicScene::load(schematic)?;
    if std::env::var_os("KONNECT_SCENE_STATS").is_some() {
        let geometry = semantic
            .primitives
            .iter()
            .filter(|primitive| !matches!(primitive, Primitive::Text { .. }))
            .count();
        eprintln!("scene geometry: {geometry} primitives");
        for role in [
            ColorRole::Border,
            ColorRole::Bus,
            ColorRole::GraphicText,
            ColorRole::Junction,
            ColorRole::Label,
            ColorRole::NoConnect,
            ColorRole::Page,
            ColorRole::Pin,
            ColorRole::PinName,
            ColorRole::PinNumber,
            ColorRole::SheetFile,
            ColorRole::Symbol,
            ColorRole::Text,
            ColorRole::Wire,
        ] {
            let texts = semantic
                .primitives
                .iter()
                .filter_map(|primitive| match primitive {
                    Primitive::Text {
                        role: primitive_role,
                        text,
                        ..
                    } if *primitive_role == role => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            eprintln!(
                "scene {role:?}: {} runs, {} characters",
                texts.len(),
                texts.iter().map(|text| text.chars().count()).sum::<usize>()
            );
        }
    }
    if std::env::var_os("KONNECT_SCENE_TEXTS").is_some() {
        for primitive in &semantic.primitives {
            if let Primitive::Text {
                position,
                rotation_deg,
                size_mm,
                stroke_width_mm,
                align,
                italic,
                role,
                text,
            } = primitive
            {
                eprintln!(
                    "scene text {role:?}: {text:?} at=({:.4},{:.4}) rotation={rotation_deg:.1} size={size_mm:.4} stroke={stroke_width_mm:.4} align={align:?} italic={italic}",
                    position.x, position.y
                );
            }
        }
    }
    if std::env::var_os("KONNECT_SCENE_OBJECTS").is_some() {
        for object in &semantic.objects {
            eprintln!(
                "scene object {:?} {} {} {:.4},{:.4}..{:.4},{:.4} index={:.4},{:.4}..{:.4},{:.4} initial={:.4},{:.4}..{:.4},{:.4}",
                object.kind,
                object.label,
                object.uuid,
                object.bounds.min_x,
                object.bounds.min_y,
                object.bounds.max_x,
                object.bounds.max_y,
                object.index_bounds.min_x,
                object.index_bounds.min_y,
                object.index_bounds.max_x,
                object.index_bounds.max_y,
                object.initial_index_bounds.min_x,
                object.initial_index_bounds.min_y,
                object.initial_index_bounds.max_x,
                object.initial_index_bounds.max_y
            );
        }
    }
    let width = (semantic.width_mm * pixels_per_mm).round() as u32;
    let height = (semantic.height_mm * pixels_per_mm).round() as u32;
    let sheet = NativeSheet {
        name: schematic
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("schematic")
            .to_owned(),
        depth: 0,
        file: schematic.to_path_buf(),
        rendered: encode_scene(&semantic, palette),
        semantic,
        compatibility: None,
        compatibility_error: None,
    };
    let mut scene = Scene::new();
    append_sheet(&mut scene, &sheet, Affine::scale(pixels_per_mm));

    pollster::block_on(render_scene_png(
        &scene,
        output,
        width,
        height,
        palette.page,
    ))
}

async fn render_scene_png(
    scene: &Scene,
    output: &Path,
    width: u32,
    height: u32,
    background: Color,
) -> Result<()> {
    let mut context = RenderContext::new();
    let device_id = context
        .device(None)
        .await
        .ok_or_else(|| anyhow!("no compatible GPU adapter is available"))?;
    let handle = &context.devices[device_id];
    let texture = handle.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Konnect schematic golden image"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut renderer = Renderer::new(
        &handle.device,
        RendererOptions {
            // Headless goldens must be byte-stable.  The GPU path can vary a
            // single 8-bit channel at a primitive overlap across otherwise
            // identical runs; Vello's CPU path removes that race.  The live
            // viewer above remains GPU-accelerated.
            use_cpu: true,
            antialiasing_support: [AaConfig::Area].into_iter().collect(),
            num_init_threads: None,
            pipeline_cache: None,
        },
    )?;
    renderer.render_to_texture(
        &handle.device,
        &handle.queue,
        scene,
        &view,
        &vello::RenderParams {
            base_color: background,
            width,
            height,
            antialiasing_method: AaConfig::Area,
        },
    )?;

    let unpadded_bytes_per_row = width * 4;
    let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(256) * 256;
    let buffer = handle.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Konnect schematic PNG readback"),
        size: u64::from(padded_bytes_per_row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = handle
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Konnect schematic PNG copy"),
        });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bytes_per_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    handle.queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    handle.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: None,
    })?;
    receiver
        .recv()
        .context("GPU readback callback was dropped")?
        .context("failed to map the rendered image")?;
    let mapped = slice.get_mapped_range();
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for row in mapped.chunks_exact(padded_bytes_per_row as usize) {
        pixels.extend_from_slice(&row[..unpadded_bytes_per_row as usize]);
    }

    let file = std::fs::File::create(output)
        .with_context(|| format!("failed to create {}", output.display()))?;
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()?
        .write_image_data(&pixels)
        .with_context(|| format!("failed to write {}", output.display()))?;
    Ok(())
}

pub(super) fn schematic_argument() -> Result<PathBuf> {
    let path = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .find(|argument| {
            argument
                .extension()
                .is_some_and(|extension| extension == "kicad_sch")
        })
        .ok_or_else(|| anyhow!("usage: schematic-viewer <project.kicad_sch>"))?;
    if !path.is_file() {
        return Err(anyhow!("schematic not found: {}", path.display()));
    }
    Ok(path.canonicalize().unwrap_or(path))
}

pub(super) fn render_png_argument() -> Option<PathBuf> {
    let mut arguments = std::env::args_os();
    while let Some(argument) = arguments.next() {
        if argument == "--render-png" {
            return arguments.next().map(PathBuf::from);
        }
    }
    None
}

pub(super) fn benchmark_iterations_argument() -> Result<Option<usize>> {
    let mut arguments = std::env::args_os();
    while let Some(argument) = arguments.next() {
        if argument == "--benchmark-load" {
            let iterations = arguments
                .next()
                .and_then(|value| value.to_str().and_then(|value| value.parse::<usize>().ok()))
                .ok_or_else(|| anyhow!("--benchmark-load requires a positive iteration count"))?;
            if iterations == 0 {
                return Err(anyhow!("--benchmark-load iteration count must be non-zero"));
            }
            return Ok(Some(iterations));
        }
    }
    Ok(None)
}

pub(super) fn benchmark_scene_pipeline(root: &Path, iterations: usize) -> Result<()> {
    let palette = Theme::Light.palette();
    // Warm filesystem metadata, font tables, and allocator paths before the
    // measured samples. The benchmark intentionally excludes GPU submission:
    // it measures the parse/semantic/scene-encoding work that gates reloads.
    let warm = load_hierarchy(root)?;
    std::hint::black_box(
        warm.into_iter()
            .map(|entry| NativeSheet::from_hierarchy(entry, palette))
            .collect::<Vec<_>>(),
    );

    let mut hierarchy_ms = Vec::with_capacity(iterations);
    let mut active_sheet_ms = Vec::with_capacity(iterations);
    let mut pages = 0usize;
    for _ in 0..iterations {
        let started = std::time::Instant::now();
        let hierarchy = load_hierarchy(root)?;
        pages = hierarchy.len();
        let encoded = hierarchy
            .into_iter()
            .map(|entry| NativeSheet::from_hierarchy(entry, palette))
            .collect::<Vec<_>>();
        std::hint::black_box(encoded);
        hierarchy_ms.push(started.elapsed().as_secs_f64() * 1_000.0);

        let started = std::time::Instant::now();
        let scene = SchematicScene::load(root)?;
        let encoded = encode_scene(&scene, palette);
        std::hint::black_box(encoded);
        active_sheet_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let hierarchy = latency_summary(&mut hierarchy_ms);
    let active = latency_summary(&mut active_sheet_ms);
    println!(
        "KONNECT_BENCH pages={pages} iterations={iterations} hierarchy_mean_ms={:.3} hierarchy_p95_ms={:.3} hierarchy_max_ms={:.3} active_mean_ms={:.3} active_p95_ms={:.3} active_max_ms={:.3}",
        hierarchy.mean_ms,
        hierarchy.p95_ms,
        hierarchy.max_ms,
        active.mean_ms,
        active.p95_ms,
        active.max_ms,
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct LatencySummary {
    pub(super) mean_ms: f64,
    pub(super) p95_ms: f64,
    pub(super) max_ms: f64,
}

pub(super) fn latency_summary(samples: &mut [f64]) -> LatencySummary {
    samples.sort_by(f64::total_cmp);
    let mean_ms = samples.iter().sum::<f64>() / samples.len() as f64;
    let p95_index = ((samples.len() as f64 * 0.95).ceil() as usize)
        .saturating_sub(1)
        .min(samples.len().saturating_sub(1));
    LatencySummary {
        mean_ms,
        p95_ms: samples[p95_index],
        max_ms: *samples.last().unwrap_or(&0.0),
    }
}

#[cfg(feature = "golden-svg-reference")]
pub(super) fn render_svg_png_argument() -> Result<Option<(PathBuf, u32, u32, PathBuf)>> {
    let mut arguments = std::env::args_os();
    while let Some(argument) = arguments.next() {
        if argument == "--render-svg-png" {
            let output = arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| anyhow!("--render-svg-png requires OUTPUT WIDTH HEIGHT SVG"))?;
            let width = arguments
                .next()
                .and_then(|value| value.to_str().and_then(|value| value.parse().ok()))
                .ok_or_else(|| anyhow!("--render-svg-png WIDTH must be a positive integer"))?;
            let height = arguments
                .next()
                .and_then(|value| value.to_str().and_then(|value| value.parse().ok()))
                .ok_or_else(|| anyhow!("--render-svg-png HEIGHT must be a positive integer"))?;
            let svg = arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| anyhow!("--render-svg-png requires an SVG path"))?;
            if width == 0 || height == 0 {
                return Err(anyhow!("--render-svg-png dimensions must be non-zero"));
            }
            if !svg.is_file() {
                return Err(anyhow!("SVG not found: {}", svg.display()));
            }
            return Ok(Some((output, width, height, svg)));
        }
    }
    Ok(None)
}

pub(super) fn window_attributes() -> WindowAttributes {
    Window::default_attributes()
        .with_inner_size(LogicalSize::new(1280.0, 900.0))
        .with_min_inner_size(LogicalSize::new(720.0, 480.0))
        .with_resizable(true)
        .with_window_icon(window_icon())
        .with_title("Konnect — Schematic Studio")
}

pub(super) fn window_icon() -> Option<Icon> {
    let decoder = png::Decoder::new(Cursor::new(include_bytes!(
        "../../../packaging/resources/icon.png"
    )));
    let mut reader = decoder.read_info().ok()?;
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut pixels).ok()?;
    Icon::from_rgba(
        pixels[..info.buffer_size()].to_vec(),
        info.width,
        info.height,
    )
    .ok()
}

pub(super) fn path_key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

pub(super) fn sheet_rectangle(source: &str, uuid: &str) -> Option<(f64, f64, f64, f64)> {
    konnect_sexp::writer::find_direct_child_blocks(source, "kicad_sch")
        .into_iter()
        .find_map(|(start, end)| {
            let node = parse_sexp(&source[start..end]).ok()?;
            if node.head() != Some("sheet") || node.find_str("uuid") != Some(uuid) {
                return None;
            }
            let (x, y, _) = konnect_sexp::schematic::parse_at(&node)?;
            let size = node.find("size")?;
            Some((x, y, x + size.get_f64(1)?, y + size.get_f64(2)?))
        })
}

pub(super) fn nearest_sheet_edge(
    rectangle: (f64, f64, f64, f64),
    cursor: SchPoint,
    grid_mm: f64,
    snap_enabled: bool,
) -> (SchPoint, f64) {
    let (left, top, right, bottom) = rectangle;
    let snapped = snap_point(cursor, snap_enabled, grid_mm);
    let distances = [
        ((cursor.x - right).abs(), 0.0),
        ((cursor.x - left).abs(), 180.0),
        ((cursor.y - top).abs(), 90.0),
        ((cursor.y - bottom).abs(), 270.0),
    ];
    let rotation = distances
        .into_iter()
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .map_or(0.0, |(_, rotation)| rotation);
    let point = match rotation as u16 {
        0 => SchPoint {
            x: right,
            y: snapped.y.clamp(top, bottom),
        },
        180 => SchPoint {
            x: left,
            y: snapped.y.clamp(top, bottom),
        },
        90 => SchPoint {
            x: snapped.x.clamp(left, right),
            y: top,
        },
        _ => SchPoint {
            x: snapped.x.clamp(left, right),
            y: bottom,
        },
    };
    (point, rotation)
}

pub(super) fn next_sheet_instance(source: &str) -> Result<(String, String)> {
    let root = parse_sexp(source).context("parent schematic is not valid S-expression")?;
    let root_uuid = root
        .find("uuid")
        .and_then(|uuid| uuid.get(1))
        .and_then(konnect_sexp::SexpNode::as_str)
        .filter(|uuid| !uuid.is_empty())
        .context("parent schematic has no root UUID")?;
    let mut max_page = 1_u32;
    for sheet in root.find_all("sheet") {
        let Some(instances) = sheet.find("instances") else {
            continue;
        };
        for project in instances.find_all("project") {
            for path in project.find_all("path") {
                if let Some(page) = path
                    .find("page")
                    .and_then(|page| page.get(1))
                    .and_then(konnect_sexp::SexpNode::as_str)
                    .and_then(|page| page.parse::<u32>().ok())
                {
                    max_page = max_page.max(page);
                }
            }
        }
    }
    Ok((format!("/{root_uuid}"), (max_page + 1).to_string()))
}

pub(super) fn truncate_ui(value: &str, max_chars: usize) -> String {
    let mut characters = value.chars();
    let head = characters.by_ref().take(max_chars).collect::<String>();
    if characters.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

pub(super) fn display_sheet_name(value: &str) -> String {
    let value = value
        .split_once('_')
        .filter(|(prefix, _)| prefix.chars().all(|character| character.is_ascii_digit()))
        .map_or(value, |(_, rest)| rest);
    let words = value
        .split(['_', '-'])
        .filter(|word| !word.is_empty())
        .map(|word| match word.to_ascii_uppercase().as_str() {
            "BMS" | "AFE" | "MCU" | "USB" | "I2C" | "SPI" | "CAN" => word.to_ascii_uppercase(),
            _ if word.chars().any(|character| character.is_ascii_digit()) => {
                word.to_ascii_uppercase()
            }
            _ => {
                let lowercase = word.to_ascii_lowercase();
                let mut characters = lowercase.chars();
                characters
                    .next()
                    .map(|first| first.to_ascii_uppercase().to_string() + characters.as_str())
                    .unwrap_or_default()
            }
        })
        .collect::<Vec<_>>();
    if words.is_empty() {
        value.to_owned()
    } else {
        words.join(" ")
    }
}

pub(super) fn union_bounds(left: Bounds, right: Bounds) -> Bounds {
    Bounds {
        min_x: left.min_x.min(right.min_x),
        min_y: left.min_y.min(right.min_y),
        max_x: left.max_x.max(right.max_x),
        max_y: left.max_y.max(right.max_y),
    }
}
