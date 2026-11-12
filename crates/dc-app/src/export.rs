//! Headless raster export.
//!
//! Rendering a document to a PNG needs no window, so this path also serves as
//! the editor's visual regression harness: a document in, an image out, with
//! the same renderer the canvas uses.

use dc_model::BoundingBox;
use dc_render::{Camera, Overlay, Renderer};
use kurbo::Vec2;
use std::num::NonZeroUsize;
use std::path::Path;
use vello::util::RenderContext;
use vello::{AaConfig, AaSupport, RenderParams, RendererOptions, Scene};

/// Margin left around the content, in document units.
const MARGIN: f64 = 24.0;
/// `copy_texture_to_buffer` requires rows aligned to this many bytes.
const ROW_ALIGNMENT: u32 = 256;
/// Guard against a document asking for an image too large to allocate.
const MAX_DIMENSION: u32 = 16384;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no suitable GPU device was found")]
    NoDevice,

    #[error("render failed: {0}")]
    Render(String),

    #[error("could not read the rendered image back from the GPU")]
    Readback,

    #[error("the requested image is {width}x{height}, beyond the {MAX_DIMENSION} pixel limit")]
    TooLarge { width: u32, height: u32 },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("png encoding failed: {0}")]
    Png(#[from] png::EncodingError),
}

/// Renders one page to a PNG file.
///
/// `scale` multiplies document units to pixels, so 2.0 gives a double-density
/// image. The framed region is the page's content, or the page sheet when the
/// page is empty.
pub fn render_png(
    package: &dc_lucid::LucidPackage,
    page_index: usize,
    scale: f64,
    path: impl AsRef<Path>,
) -> Result<(u32, u32), Error> {
    let document = &package.document;
    let page = document
        .pages
        .get(page_index)
        .or_else(|| document.pages.first())
        .ok_or(Error::NoDevice)?;

    let units = document.units();
    // Framing to content is the common case; an empty page falls back to the
    // sheet so the export is not zero-sized.
    let framed_to_content = page.content_bounds().is_some();
    let region = page
        .content_bounds()
        .map(|b| b.inflated(MARGIN))
        .or_else(|| {
            page.resolved_settings()
                .bounds_points(units)
                .map(|(w, h)| BoundingBox::new(0.0, 0.0, w, h))
        })
        .unwrap_or(BoundingBox::new(0.0, 0.0, 612.0, 792.0));

    let width = ((region.w * scale).round() as u32).max(1);
    let height = ((region.h * scale).round() as u32).max(1);
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(Error::TooLarge { width, height });
    }

    // The camera frames exactly the region, with no interface chrome.
    let mut camera = Camera::new(Vec2::new(width as f64, height as f64));
    camera.zoom = scale;
    camera.origin = dc_model::Point::new(region.x, region.y);

    let mut renderer = Renderer::new();
    // Bundled images must be decoded or every image shape exports as a
    // placeholder, which is exactly the bug this line prevents.
    renderer.images.load(
        package
            .images
            .iter()
            .map(|(name, bytes)| (name.clone(), bytes.clone())),
    );
    // Grid lines and selection handles are interface aids, not content.
    renderer.options.show_grid = false;
    renderer.options.show_selection = false;
    // When framing to content, the sheet edge would cut across the image
    // wherever the content and the page disagree about their extent.
    renderer.options.show_page_bounds = !framed_to_content;

    let mut scene = Scene::new();
    renderer.draw(&mut scene, document, page_index, &camera, &Overlay::default());

    let pixels = rasterize(&scene, width, height, renderer.theme.page)?;
    write_png(path, width, height, &pixels)?;
    Ok((width, height))
}

/// Renders the full editor, chrome included, to a PNG without opening a
/// window.
///
/// This is how documentation screenshots are produced, and it means the
/// interface can be inspected in review without a display attached.
#[allow(clippy::too_many_arguments)]
pub fn render_editor_png(
    editor: &mut crate::editor::Editor,
    window: Vec2,
    scale: f64,
    dark: bool,
    library: dc_model::ShapeLibrary,
    open_menu: Option<usize>,
    settings_open: bool,
    rulers: bool,
    presenting: bool,
    path: impl AsRef<Path>,
) -> Result<(u32, u32), Error> {
    use crate::menu::ViewState;
    use crate::ui::Chrome;
    use dc_render::theme::{Metrics, Theme};

    let metrics = Metrics::default();
    let view = ViewState {
        settings: crate::settings::Settings {
            dark_theme: dark,
            show_rulers: rulers,
            ..Default::default()
        },
        library,
        settings_open,
        presenting,
        ..Default::default()
    };
    let chrome = Chrome::build_for(window, editor, &metrics, &view, open_menu, None, None);
    let canvas = chrome.layout.canvas;
    editor.resize_viewport(canvas.width(), canvas.height());
    // Rebuild so the chrome reflects the camera that was just sized.
    let chrome = Chrome::build_for(window, editor, &metrics, &view, open_menu, None, None);
    // Presenting frames the page only once the canvas is the whole window.
    if presenting {
        editor.zoom_to_fit();
    }

    let mut renderer = Renderer::new();
    renderer.theme = if dark { Theme::dark() } else { Theme::light() };
    // A slide is the drawing and nothing else, so the graph paper and the
    // sheet outline go away — matching what the editor does when it starts
    // presenting, so this preview shows what is really on screen.
    if presenting {
        renderer.options.show_grid = false;
        renderer.options.show_page_bounds = false;
    }
    renderer.images.load(
        editor
            .package
            .images
            .iter()
            .map(|(name, bytes)| (name.clone(), bytes.clone())),
    );

    let mut doc_scene = Scene::new();
    renderer.draw(
        &mut doc_scene,
        editor.document(),
        editor.page_index,
        &editor.camera,
        &editor.overlay(),
    );

    // An open label is part of what the editor looks like, so the preview
    // draws it the same way the window does.
    let theme = renderer.theme.clone();
    let editing_tab = editor.editing_page_title().is_some();
    if !editing_tab {
        let camera = editor.camera.transform();
        let (fonts, layouts) = renderer.text_engine().contexts();
        let mut cx = crate::textedit::TextCtx::new(fonts, layouts);
        if let Some(edit) = editor.text_edit.as_mut() {
            crate::textedit::draw_session(&mut doc_scene, edit, &theme, camera, &mut cx);
        }
    }

    let mut ui_scene = Scene::new();
    ui_scene.append(
        &doc_scene,
        Some(kurbo::Affine::translate((canvas.x0, canvas.y0))),
    );
    chrome.draw(
        &mut ui_scene,
        &theme,
        &metrics,
        renderer.text_engine(),
        None,
        &editor.status,
        &editor.camera,
    );

    if editing_tab {
        let (fonts, layouts) = renderer.text_engine().contexts();
        let mut cx = crate::textedit::TextCtx::new(fonts, layouts);
        if let Some(edit) = editor.text_edit.as_mut() {
            crate::textedit::draw_session(
                &mut ui_scene,
                edit,
                &theme,
                kurbo::Affine::IDENTITY,
                &mut cx,
            );
        }
    }

    let mut scene = Scene::new();
    scene.append(&ui_scene, Some(kurbo::Affine::scale(scale)));

    let width = ((window.x * scale).round() as u32).max(1);
    let height = ((window.y * scale).round() as u32).max(1);
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(Error::TooLarge { width, height });
    }

    let pixels = rasterize(&scene, width, height, theme.canvas)?;
    write_png(path, width, height, &pixels)?;
    Ok((width, height))
}

/// Renders the application icon to a PNG at the given size.
///
/// The icon is drawn by the same renderer the canvas uses, so regenerating an
/// icon set is a command rather than a trip through an image editor.
pub fn render_icon_png(size: u32, path: impl AsRef<Path>) -> Result<(u32, u32), Error> {
    let size = size.clamp(1, MAX_DIMENSION);
    let mut scene = Scene::new();
    crate::icon::draw(&mut scene, size as f64);

    // Transparent outside the rounded square: an icon's corners must let the
    // desktop through, not sit on a colour of their own.
    let pixels = rasterize(&scene, size, size, dc_model::Color::TRANSPARENT)?;
    write_png(path, size, size, &pixels)?;
    Ok((size, size))
}

/// Runs a scene through Vello on a headless device and returns RGBA8 pixels.
fn rasterize(
    scene: &Scene,
    width: u32,
    height: u32,
    base: dc_model::Color,
) -> Result<Vec<u8>, Error> {
    let mut context = RenderContext::new();
    let device_id =
        pollster::block_on(context.device(None)).ok_or(Error::NoDevice)?;
    let handle = &context.devices[device_id];
    let device = &handle.device;
    let queue = &handle.queue;

    let mut vello_renderer = vello::Renderer::new(
        device,
        RendererOptions {
            use_cpu: false,
            antialiasing_support: AaSupport::area_only(),
            num_init_threads: NonZeroUsize::new(1),
            ..Default::default()
        },
    )
    .map_err(|e| Error::Render(e.to_string()))?;

    let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("diagramcraft.export.target"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        // Vello writes through a storage binding; the copy needs COPY_SRC.
        usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    vello_renderer
        .render_to_texture(
            device,
            queue,
            scene,
            &view,
            &RenderParams {
                base_color: peniko::Color::from_rgba8(base.r, base.g, base.b, base.a),
                width,
                height,
                antialiasing_method: AaConfig::Area,
            },
        )
        .map_err(|e| Error::Render(e.to_string()))?;

    // Buffer rows must be aligned, so the readback is padded and trimmed.
    let unpadded_row = width * 4;
    let padded_row = unpadded_row.div_ceil(ROW_ALIGNMENT) * ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("diagramcraft.export.readback"),
        size: (padded_row * height) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("diagramcraft.export.copy"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(height),
            },
        },
        size,
    );
    queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|_| Error::Readback)?;
    receiver
        .recv()
        .map_err(|_| Error::Readback)?
        .map_err(|_| Error::Readback)?;

    let padded = slice.get_mapped_range().map_err(|_| Error::Readback)?;
    let mut pixels = Vec::with_capacity((unpadded_row * height) as usize);
    for row in 0..height {
        let start = (row * padded_row) as usize;
        let end = start + unpadded_row as usize;
        pixels.extend_from_slice(&padded[start..end]);
    }
    drop(padded);
    buffer.unmap();

    Ok(pixels)
}

fn write_png(
    path: impl AsRef<Path>,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(), Error> {
    let file = std::fs::File::create(path)?;
    let writer = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_model::{
        BoundingBox, Document, Endpoint, EndpointStyle, Line, LineType, Shape, ShapeType, Units,
    };

    fn flowchart() -> Document {
        let mut doc = Document::new();
        doc.set_units(Units::Pt);
        let page = &mut doc.pages[0];
        page.add_shape(
            Shape::new("start", ShapeType::Terminator, BoundingBox::new(80.0, 40.0, 150.0, 54.0))
                .with_text("Start"),
        );
        page.add_shape(
            Shape::new("check", ShapeType::Decision, BoundingBox::new(80.0, 150.0, 150.0, 110.0))
                .with_text("Approved?"),
        );
        page.add_line(
            Line::new(
                "e1",
                Endpoint::at_shape("start", EndpointStyle::None),
                Endpoint::at_shape("check", EndpointStyle::Arrow),
            )
            .with_line_type(LineType::Elbow),
        );
        doc
    }

    /// GPU-backed, so it is skipped where no adapter is available.
    fn gpu_available() -> bool {
        let mut cx = RenderContext::new();
        pollster::block_on(cx.device(None)).is_some()
    }

    #[test]
    fn exports_a_png_of_the_expected_size() {
        if !gpu_available() {
            eprintln!("skipping: no GPU adapter");
            return;
        }
        let dir = std::env::temp_dir().join("dc-export-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("flowchart.png");

        let doc = flowchart();
        let package = dc_lucid::LucidPackage::new(doc.clone());
        let (w, h) = render_png(&package, 0, 2.0, &path).unwrap();

        // Content spans 150 wide and 220 tall, plus a margin on each side.
        let content = doc.pages[0].content_bounds().unwrap().inflated(MARGIN);
        assert_eq!(w, (content.w * 2.0).round() as u32);
        assert_eq!(h, (content.h * 2.0).round() as u32);

        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.len() > 1000, "the png is suspiciously small");
        assert_eq!(&bytes[1..4], b"PNG", "not a png file");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_empty_page_exports_the_sheet() {
        if !gpu_available() {
            eprintln!("skipping: no GPU adapter");
            return;
        }
        let dir = std::env::temp_dir().join("dc-export-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("empty.png");

        let package = dc_lucid::LucidPackage::new(dc_model::Document::new());
        let (w, h) = render_png(&package, 0, 1.0, &path).unwrap();
        // US Letter at 72 points per inch.
        assert_eq!((w, h), (612, 792));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_excessive_scale_is_refused_rather_than_allocated() {
        let package = dc_lucid::LucidPackage::new(flowchart());
        let err = render_png(&package, 0, 10_000.0, "/tmp/never-written.png").unwrap_err();
        assert!(matches!(err, Error::TooLarge { .. }), "got {err:?}");
    }

    #[test]
    fn rendered_pixels_are_not_uniform() {
        if !gpu_available() {
            eprintln!("skipping: no GPU adapter");
            return;
        }
        // A blank image would mean the scene never reached the target.
        let doc = flowchart();
        let page = &doc.pages[0];
        let region = page.content_bounds().unwrap().inflated(MARGIN);
        let width = region.w.round() as u32;
        let height = region.h.round() as u32;

        let mut camera = Camera::new(Vec2::new(width as f64, height as f64));
        camera.origin = dc_model::Point::new(region.x, region.y);
        let mut renderer = Renderer::new();
        renderer.options.show_grid = false;
        let mut scene = Scene::new();
        renderer.draw(&mut scene, &doc, 0, &camera, &Overlay::default());

        let pixels = rasterize(&scene, width, height, renderer.theme.page).unwrap();
        assert_eq!(pixels.len(), (width * height * 4) as usize);

        let first = &pixels[0..4];
        let varied = pixels.chunks_exact(4).any(|p| p != first);
        assert!(varied, "every pixel is identical; nothing was drawn");
    }
}
