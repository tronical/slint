// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use std::{cell::RefCell, rc::Rc, sync::Arc};

use i_slint_core::api::PhysicalSize as PhysicalWindowSize;

use crate::{FemtoVGRenderer, GraphicsBackend, WindowSurface};

pub struct WGPUBackend {
    device: RefCell<Option<Arc<wgpu::Device>>>,
    surface_config: RefCell<Option<wgpu::SurfaceConfiguration>>,
    surface: RefCell<Option<wgpu::Surface<'static>>>,
}

pub struct WGPUWindowSurface {
    surface_texture: wgpu::SurfaceTexture,
}

impl WindowSurface<femtovg::renderer::WGPURenderer> for WGPUWindowSurface {
    fn render_surface(&self) -> &wgpu::Texture {
        &self.surface_texture.texture
    }
}

impl GraphicsBackend for WGPUBackend {
    type Renderer = femtovg::renderer::WGPURenderer;
    type WindowSurface = WGPUWindowSurface;

    fn new_suspended() -> Self {
        Self {
            device: Default::default(),
            surface_config: Default::default(),
            surface: Default::default(),
        }
    }

    fn clear_graphics_context(&self) {
        self.surface.borrow_mut().take();
        self.device.borrow_mut().take();
    }

    fn begin_surface_rendering(
        &self,
    ) -> Result<Self::WindowSurface, Box<dyn std::error::Error + Send + Sync>> {
        let frame = self
            .surface
            .borrow()
            .as_ref()
            .unwrap()
            .get_current_texture()
            .expect("unable to get next texture from swapchain");
        Ok(WGPUWindowSurface { surface_texture: frame })
    }

    fn present_surface(
        &self,
        surface: Self::WindowSurface,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        surface.surface_texture.present();
        Ok(())
    }

    fn with_graphics_api<R>(
        &self,
        callback: impl FnOnce(Option<i_slint_core::api::GraphicsAPI<'_>>) -> R,
    ) -> Result<R, i_slint_core::platform::PlatformError> {
        Ok(callback(None))
    }

    fn resize(
        &self,
        width: std::num::NonZeroU32,
        height: std::num::NonZeroU32,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut surface_config = self.surface_config.borrow_mut();
        let surface_config = surface_config.as_mut().unwrap();

        surface_config.width = width.get();
        surface_config.height = height.get();

        let mut device = self.device.borrow_mut();
        let device = device.as_mut().unwrap();

        self.surface.borrow_mut().as_mut().unwrap().configure(device, surface_config);
        Ok(())
    }
}

impl WGPUBackend {
    pub fn set_window_handle(
        &self,
        renderer: &FemtoVGRenderer<Self>,
        window_handle: Box<dyn wgpu::WindowHandle>,
        size: PhysicalWindowSize,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let backends = wgpu::util::backend_bits_from_env().unwrap_or_default();
        let dx12_shader_compiler = wgpu::util::dx12_shader_compiler_from_env().unwrap_or_default();
        let gles_minor_version = wgpu::util::gles_minor_version_from_env().unwrap_or_default();

        let instance = spin_on::spin_on(async {
            wgpu::util::new_instance_with_webgpu_detection(wgpu::InstanceDescriptor {
                backends,
                flags: wgpu::InstanceFlags::from_build_config().with_env(),
                dx12_shader_compiler,
                gles_minor_version,
            })
            .await
        });

        let surface = instance.create_surface(window_handle).unwrap();

        let adapter = spin_on::spin_on(async {
            wgpu::util::initialize_adapter_from_env_or_default(&instance, Some(&surface))
                .await
                .expect("Failed to find an appropriate adapter")
        });

        let (device, queue) = spin_on::spin_on(async {
            adapter
                .request_device(
                    &wgpu::DeviceDescriptor {
                        label: None,
                        required_features: wgpu::Features::empty(),
                        // Make sure we use the texture resolution limits from the adapter, so we can support images the size of the swapchain.
                        required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                            .using_resolution(adapter.limits()),
                        memory_hints: wgpu::MemoryHints::MemoryUsage,
                    },
                    None,
                )
                .await
                .expect("Failed to create device")
        });

        let mut surface_config =
            surface.get_default_config(&adapter, size.width, size.height).unwrap();

        let swapchain_capabilities = surface.get_capabilities(&adapter);
        let swapchain_format = swapchain_capabilities
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or_else(|| swapchain_capabilities.formats[0]);
        surface_config.format = swapchain_format;
        surface.configure(&device, &surface_config);

        let device = Arc::new(device);

        *self.device.borrow_mut() = Some(device.clone());
        *self.surface_config.borrow_mut() = Some(surface_config);
        *self.surface.borrow_mut() = Some(surface);

        let wgpu_renderer = femtovg::renderer::WGPURenderer::new(device, Arc::new(queue));
        let femtovg_canvas = femtovg::Canvas::new_with_text_context(
            wgpu_renderer,
            crate::fonts::FONT_CACHE.with(|cache| cache.borrow().text_context.clone()),
        )
        .unwrap();

        let canvas = Rc::new(RefCell::new(femtovg_canvas));
        renderer.reset_canvas(canvas);
        Ok(())
    }
}
