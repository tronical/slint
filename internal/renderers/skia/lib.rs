// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-1.0 OR LicenseRef-Slint-commercial

#![doc = include_str!("README.md")]
#![doc(html_logo_url = "https://slint.dev/logo/slint-logo-square-light.svg")]

use std::cell::RefCell;
use std::rc::Rc;

use i_slint_common::sharedfontdb;
use i_slint_core::api::{
    GraphicsAPI, PhysicalSize as PhysicalWindowSize, RenderingNotifier, RenderingState,
    SetRenderingNotifierError,
};
use i_slint_core::graphics::euclid::{self, Vector2D};
use i_slint_core::graphics::rendering_metrics_collector::RenderingMetricsCollector;
use i_slint_core::graphics::FontRequest;
use i_slint_core::item_rendering::ItemCache;
use i_slint_core::lengths::{
    LogicalLength, LogicalPoint, LogicalRect, LogicalSize, PhysicalPx, ScaleFactor,
};
use i_slint_core::platform::PlatformError;
use i_slint_core::window::WindowInner;
use i_slint_core::Brush;
use lyon_path::geom::euclid::num::Round;
use unicode_segmentation::UnicodeSegmentation;

type PhysicalLength = euclid::Length<f32, PhysicalPx>;
type PhysicalRect = euclid::Rect<f32, PhysicalPx>;
type PhysicalSize = euclid::Size2D<f32, PhysicalPx>;
type PhysicalPoint = euclid::Point2D<f32, PhysicalPx>;

mod cached_image;
mod itemrenderer;
mod textlayout;

#[cfg(target_os = "macos")]
mod metal_surface;

#[cfg(target_family = "windows")]
mod d3d_surface;

cfg_if::cfg_if! {
    if #[cfg(skia_backend_vulkan)] {
        mod vulkan_surface;
        type DefaultSurface = vulkan_surface::VulkanSurface;
    } else if #[cfg(skia_backend_opengl)] {
        mod opengl_surface;
        type DefaultSurface = opengl_surface::OpenGLSurface;
    } else if #[cfg(skia_backend_metal)] {
        type DefaultSurface = metal_surface::MetalSurface;
    } else if #[cfg(skia_backend_d3d)] {
        type DefaultSurface = d3d_surface::D3DSurface;
    }
}

/// Use the SkiaRenderer when implementing a custom Slint platform where you deliver events to
/// Slint and want the scene to be rendered using Skia as underlying graphics library.
pub struct SkiaRenderer {
    rendering_notifier: RefCell<Option<Box<dyn RenderingNotifier>>>,
    image_cache: ItemCache<Option<skia_safe::Image>>,
    path_cache: ItemCache<Option<(Vector2D<f32, PhysicalPx>, skia_safe::Path)>>,
    rendering_metrics_collector: RefCell<Option<Rc<RenderingMetricsCollector>>>,
    surface: DefaultSurface,
}

impl SkiaRenderer {
    /// Creates a new renderer is associated with the provided window adapter.
    pub fn new(
        window_handle: raw_window_handle::WindowHandle<'_>,
        display_handle: raw_window_handle::DisplayHandle<'_>,
        size: PhysicalWindowSize,
    ) -> Result<Self, PlatformError> {
        let surface = DefaultSurface::new(window_handle, display_handle, size)?;

        Ok(Self {
            rendering_notifier: Default::default(),
            image_cache: Default::default(),
            path_cache: Default::default(),
            rendering_metrics_collector: Default::default(),
            surface,
        })
    }

    /// Notifiers the renderer that the underlying window is becoming visible.
    pub fn show(&self) -> Result<(), PlatformError> {
        *self.rendering_metrics_collector.borrow_mut() = RenderingMetricsCollector::new(&format!(
            "Skia renderer (skia backend {}; surface: {} bpp)",
            self.surface.name(),
            self.surface.bits_per_pixel()?
        ));

        if let Some(callback) = self.rendering_notifier.borrow_mut().as_mut() {
            self.surface
                .with_graphics_api(|api| callback.notify(RenderingState::RenderingSetup, &api))
        }

        Ok(())
    }

    /// Notifiers the renderer that the underlying window will be hidden soon.
    pub fn hide(&self) -> Result<(), i_slint_core::platform::PlatformError> {
        self.surface.with_active_surface(|| {
            if let Some(callback) = self.rendering_notifier.borrow_mut().as_mut() {
                self.surface.with_graphics_api(|api| {
                    callback.notify(RenderingState::RenderingTeardown, &api)
                })
            }
        })?;
        Ok(())
    }

    /// Render the scene in the previously associated window. The size parameter must match the size of the window.
    pub fn render(
        &self,
        window: &i_slint_core::api::Window,
    ) -> Result<(), i_slint_core::platform::PlatformError> {
        let size = window.size();
        let window_inner = WindowInner::from_pub(window);

        self.surface.render(size, |skia_canvas, gr_context| {
            window_inner.draw_contents(|components| {
                let window_background_brush =
                    window_inner.window_item().map(|w| w.as_pin_ref().background());

                // Clear with window background if it is a solid color otherwise it will drawn as gradient
                if let Some(Brush::SolidColor(clear_color)) = window_background_brush {
                    skia_canvas.clear(itemrenderer::to_skia_color(&clear_color));
                }

                if let Some(callback) = self.rendering_notifier.borrow_mut().as_mut() {
                    // For the BeforeRendering rendering notifier callback it's important that this happens *after* clearing
                    // the back buffer, in order to allow the callback to provide its own rendering of the background.
                    // Skia's clear() will merely schedule a clear call, so flush right away to make it immediate.
                    gr_context.flush(None);

                    self.surface.with_graphics_api(|api| {
                        callback.notify(RenderingState::BeforeRendering, &api)
                    })
                }

                let mut box_shadow_cache = Default::default();

                let mut item_renderer = itemrenderer::SkiaRenderer::new(
                    skia_canvas,
                    window,
                    &self.image_cache,
                    &self.path_cache,
                    &mut box_shadow_cache,
                );

                // Draws the window background as gradient
                match window_background_brush {
                    Some(Brush::SolidColor(..)) | None => {}
                    Some(brush @ _) => {
                        item_renderer.draw_rect(
                            i_slint_core::lengths::logical_size_from_api(
                                size.to_logical(window_inner.scale_factor()),
                            ),
                            brush,
                        );
                    }
                }

                for (component, origin) in components {
                    i_slint_core::item_rendering::render_component_items(
                        component,
                        &mut item_renderer,
                        *origin,
                    );
                }

                if let Some(collector) = &self.rendering_metrics_collector.borrow_mut().as_ref() {
                    collector.measure_frame_rendered(&mut item_renderer);
                }

                drop(item_renderer);
                gr_context.flush(None);
            });

            if let Some(callback) = self.rendering_notifier.borrow_mut().as_mut() {
                self.surface
                    .with_graphics_api(|api| callback.notify(RenderingState::AfterRendering, &api))
            }
        })
    }

    /// Call this when you receive a notification from the windowing system that the size of the window has changed.
    pub fn resize_event(
        &self,
        size: PhysicalWindowSize,
    ) -> Result<(), i_slint_core::platform::PlatformError> {
        self.surface.resize_event(size)
    }
}

impl i_slint_core::renderer::RendererSealed for SkiaRenderer {
    fn text_size(
        &self,
        font_request: i_slint_core::graphics::FontRequest,
        text: &str,
        max_width: Option<LogicalLength>,
        scale_factor: ScaleFactor,
    ) -> LogicalSize {
        let (width, height) = sharedfontdb::FONT_DB.with(|db| {
            let mut db = db.borrow_mut();
            let mut font_system = &mut db.font_system;

            // TODO:
            // text alignment (horizontal and vertical)
            // overflow handling
            // wrap / no-wrap

            let pixel_size: PhysicalLength =
                font_request.pixel_size.unwrap_or(textlayout::DEFAULT_FONT_SIZE) * scale_factor;

            let mut buffer = cosmic_text::Buffer::new(
                &mut font_system,
                cosmic_text::Metrics { font_size: pixel_size.get(), line_height: pixel_size.get() },
            );
            buffer.set_text(
                &mut font_system,
                text,
                cosmic_text::Attrs::new(),
                cosmic_text::Shaping::Advanced,
            );
            buffer.shape_until(&mut font_system, i32::max_value());
            buffer.set_size(
                font_system,
                max_width.map(|w| w * scale_factor).unwrap_or_default().get(),
                f32::MAX,
            );

            let mut width: f32 = 0.0;
            for line in buffer.lines.iter() {
                match line.layout_opt() {
                    Some(layout) => {
                        for line in layout {
                            width = width.max(line.w);
                        }
                    }
                    None => (),
                }
            }

            let height = buffer.lines.len() as f32 * buffer.metrics().line_height;

            (width, height)
        });

        PhysicalSize::new(width.ceil(), height.ceil()) / scale_factor
    }

    fn text_input_byte_offset_for_position(
        &self,
        text_input: std::pin::Pin<&i_slint_core::items::TextInput>,
        pos: LogicalPoint,
        font_request: FontRequest,
        scale_factor: ScaleFactor,
    ) -> usize {
        let max_width = text_input.width() * scale_factor;
        let max_height = text_input.height() * scale_factor;
        let pos = pos * scale_factor;

        if max_width.get() <= 0. || max_height.get() <= 0. {
            return 0;
        }

        let visual_representation = text_input.visual_representation(None);

        let string = text_input.text();
        let string = string.as_str();

        let byte_offset = sharedfontdb::FONT_DB.with(|db| {
            let mut db = db.borrow_mut();
            let mut font_system = &mut db.font_system;

            // TODO:
            // text alignment (horizontal and vertical)
            // overflow handling
            // wrap / no-wrap

            let pixel_size: PhysicalLength =
                font_request.pixel_size.unwrap_or(textlayout::DEFAULT_FONT_SIZE) * scale_factor;

            let mut buffer = cosmic_text::Buffer::new(
                &mut font_system,
                cosmic_text::Metrics { font_size: pixel_size.get(), line_height: pixel_size.get() },
            );
            buffer.set_text(
                &mut font_system,
                string,
                cosmic_text::Attrs::new(),
                cosmic_text::Shaping::Advanced,
            );
            buffer.shape_until(&mut font_system, i32::max_value());
            buffer.set_size(font_system, max_width.get(), max_height.get());

            if let Some(cursor) = buffer.hit(pos.x, pos.y) {
                cursor.index
            } else {
                0
            }
        });

        visual_representation.map_byte_offset_from_byte_offset_in_visual_text(byte_offset)
    }

    fn text_input_cursor_rect_for_byte_offset(
        &self,
        text_input: std::pin::Pin<&i_slint_core::items::TextInput>,
        byte_offset: usize,
        font_request: FontRequest,
        scale_factor: ScaleFactor,
    ) -> LogicalRect {
        let max_width = text_input.width() * scale_factor;
        let max_height = text_input.height() * scale_factor;

        if max_width.get() <= 0. || max_height.get() <= 0. {
            return Default::default();
        }

        let string = text_input.text();
        let string = string.as_str();
        let mut cursor_x = 0.;
        let mut cursor_y = 0.;

        let cursor_pos = sharedfontdb::FONT_DB.with(|db| {
            let mut db = db.borrow_mut();
            let mut font_system = &mut db.font_system;

            // TODO:
            // text alignment (horizontal and vertical)
            // overflow handling
            // wrap / no-wrap

            let pixel_size: PhysicalLength =
                font_request.pixel_size.unwrap_or(textlayout::DEFAULT_FONT_SIZE) * scale_factor;

            let mut buffer = cosmic_text::Buffer::new(
                &mut font_system,
                cosmic_text::Metrics { font_size: pixel_size.get(), line_height: pixel_size.get() },
            );
            buffer.set_text(
                &mut font_system,
                string,
                cosmic_text::Attrs::new(),
                cosmic_text::Shaping::Advanced,
            );
            buffer.shape_until(&mut font_system, i32::max_value());
            buffer.set_size(font_system, max_width.get(), max_height.get());

            for run in buffer.layout_runs() {
                let line_i = run.line_i;
                let line_y = run.line_y;

                let cursor_glyph_opt = |cursor: &cosmic_text::Cursor| -> Option<(usize, f32)> {
                    if cursor.line == line_i {
                        for (glyph_i, glyph) in run.glyphs.iter().enumerate() {
                            if cursor.index == glyph.start {
                                return Some((glyph_i, 0.0));
                            } else if cursor.index > glyph.start && cursor.index < glyph.end {
                                // Guess x offset based on characters
                                let mut before = 0;
                                let mut total = 0;

                                let cluster = &run.text[glyph.start..glyph.end];
                                for (i, _) in cluster.grapheme_indices(true) {
                                    if glyph.start + i < cursor.index {
                                        before += 1;
                                    }
                                    total += 1;
                                }

                                let offset = glyph.w * (before as f32) / (total as f32);
                                return Some((glyph_i, offset));
                            }
                        }
                        match run.glyphs.last() {
                            Some(glyph) => {
                                if cursor.index == glyph.end {
                                    return Some((run.glyphs.len(), 0.0));
                                }
                            }
                            None => {
                                return Some((0, 0.0));
                            }
                        }
                    }
                    None
                };

                if let Some((cursor_glyph, cursor_glyph_offset)) =
                    cursor_glyph_opt(&cosmic_text::Cursor::new(
                        0,
                        text_input.cursor_position_byte_offset().round() as usize,
                    ))
                {
                    let x = match run.glyphs.get(cursor_glyph) {
                        Some(glyph) => {
                            // Start of detected glyph
                            if glyph.level.is_rtl() {
                                (glyph.x + glyph.w - cursor_glyph_offset) as i32
                            } else {
                                (glyph.x + cursor_glyph_offset) as i32
                            }
                        }
                        None => match run.glyphs.last() {
                            Some(glyph) => {
                                // End of last glyph
                                if glyph.level.is_rtl() {
                                    glyph.x as i32
                                } else {
                                    (glyph.x + glyph.w) as i32
                                }
                            }
                            None => {
                                // Start of empty line
                                0
                            }
                        },
                    };

                    cursor_x = x as f32;
                    cursor_y = line_y - pixel_size.get();
                }
            }
        });

        println!("x: {}, y: {}", cursor_x / scale_factor.get(), cursor_y);

        return (PhysicalRect::new(
            PhysicalPoint::new(cursor_x, cursor_y),
            PhysicalSize::from_lengths(
                (text_input.text_cursor_width().cast() * scale_factor).cast(),
                font_request.pixel_size.unwrap_or(textlayout::DEFAULT_FONT_SIZE) * scale_factor,
            ),
        )
        .cast()
            / scale_factor)
            .cast();
    }

    fn register_font_from_memory(
        &self,
        data: &'static [u8],
    ) -> Result<(), Box<dyn std::error::Error>> {
        // TODO(cosmic): remove
        textlayout::register_font_from_memory(data).unwrap();
        sharedfontdb::register_font_from_memory(data)
    }

    fn register_font_from_path(
        &self,
        path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // TODO(cosmic): remove
        textlayout::register_font_from_path(path).unwrap();
        sharedfontdb::register_font_from_path(path)
    }

    fn set_rendering_notifier(
        &self,
        callback: Box<dyn RenderingNotifier>,
    ) -> std::result::Result<(), SetRenderingNotifierError> {
        if !DefaultSurface::SUPPORTS_GRAPHICS_API {
            return Err(SetRenderingNotifierError::Unsupported);
        }
        let mut notifier = self.rendering_notifier.borrow_mut();
        if notifier.replace(callback).is_some() {
            Err(SetRenderingNotifierError::AlreadySet)
        } else {
            Ok(())
        }
    }

    fn default_font_size(&self) -> LogicalLength {
        self::textlayout::DEFAULT_FONT_SIZE
    }

    fn free_graphics_resources(
        &self,
        component: i_slint_core::component::ComponentRef,
        _items: &mut dyn Iterator<Item = std::pin::Pin<i_slint_core::items::ItemRef<'_>>>,
    ) -> Result<(), i_slint_core::platform::PlatformError> {
        self.image_cache.component_destroyed(component);
        self.path_cache.component_destroyed(component);
        Ok(())
    }
}

trait Surface {
    const SUPPORTS_GRAPHICS_API: bool;
    fn new(
        window_handle: raw_window_handle::WindowHandle<'_>,
        display_handle: raw_window_handle::DisplayHandle<'_>,
        size: PhysicalWindowSize,
    ) -> Result<Self, PlatformError>
    where
        Self: Sized;
    fn name(&self) -> &'static str;
    fn with_graphics_api(&self, callback: impl FnOnce(GraphicsAPI<'_>));
    fn with_active_surface(
        &self,
        callback: impl FnOnce(),
    ) -> Result<(), i_slint_core::platform::PlatformError> {
        callback();
        Ok(())
    }
    fn render(
        &self,
        size: PhysicalWindowSize,
        callback: impl FnOnce(&mut skia_safe::Canvas, &mut skia_safe::gpu::DirectContext),
    ) -> Result<(), i_slint_core::platform::PlatformError>;
    fn resize_event(
        &self,
        size: PhysicalWindowSize,
    ) -> Result<(), i_slint_core::platform::PlatformError>;
    fn bits_per_pixel(&self) -> Result<u8, PlatformError>;
}
