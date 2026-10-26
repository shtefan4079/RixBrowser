
use std::path::Path;
use std::time::Duration;
use thiserror::Error;
use vello::peniko::Color;
use vello::util::{RenderContext, RenderSurface};
use vello::{AaConfig, RenderParams, Renderer, RendererOptions, Scene};

#[derive(Error, Debug)]
pub enum RenderError {
    #[error("No GPU surface attached")]
    NoSurface,

    #[error("Surface error: {0}")]
    Surface(String),

    #[error("Vello render error: {0}")]
    Vello(String),

    #[error("No compatible GPU device")]
    NoDevice,

    #[error("Invalid surface size {0}x{1}")]
    BadSize(u32, u32),

    #[error("GPU readback error: {0}")]
    Readback(String),

    #[error("PNG save error: {0}")]
    Image(#[from] image::ImageError),
}

pub struct VelloRenderer {
    context: RenderContext,
    renderer: Option<Renderer>,
    renderer_device_id: Option<usize>,
    surface: Option<RenderSurface<'static>>,
    width: u32,
    height: u32,
}

impl Default for VelloRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl VelloRenderer {
    pub fn new() -> Self {
        Self {
            context: RenderContext::new(),
            renderer: None,
            renderer_device_id: None,
            surface: None,
            width: 0,
            height: 0,
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.surface.is_some() && self.renderer.is_some()
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub async fn ensure_surface(
        &mut self,
        target: impl Into<vello::wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<(), RenderError> {
        let width = width.max(1);
        let height = height.max(1);
        if self.surface.is_some() && self.width == width && self.height == height {
            return Ok(());
        }
        if self.surface.is_some() && (self.width != width || self.height != height) {
            if let Some(surface) = self.surface.as_mut() {
                self.context.resize_surface(surface, width, height);
                self.width = width;
                self.height = height;
                return Ok(());
            }
        }
        let mut surface = self
            .context
            .create_surface(target, width, height, vello::wgpu::PresentMode::AutoNoVsync)
            .await
            .map_err(|e| RenderError::Surface(format!("{e:?}")))?;
        if self.renderer_device_id != Some(surface.dev_id) {
            let device_handle = self
                .context
                .devices
                .get(surface.dev_id)
                .ok_or(RenderError::NoDevice)?;
            let renderer = Renderer::new(&device_handle.device, RendererOptions::default())
                .map_err(|e| RenderError::Vello(format!("{e:?}")))?;
            self.renderer = Some(renderer);
            self.renderer_device_id = Some(surface.dev_id);
        }
        self.context.resize_surface(&mut surface, width, height);
        self.width = width;
        self.height = height;
        self.surface = Some(surface);
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let width = width.max(1);
        let height = height.max(1);
        if let Some(surface) = self.surface.as_mut() {
            if self.width != width || self.height != height {
                self.context.resize_surface(surface, width, height);
                self.width = width;
                self.height = height;
            }
        } else {
            self.width = width;
            self.height = height;
        }
    }

    pub fn render(&mut self, scene: &Scene) -> Result<(), RenderError> {
        let (width, height) = (self.width.max(1), self.height.max(1));
        let surface = self.surface.as_mut().ok_or(RenderError::NoSurface)?;
        let renderer = self.renderer.as_mut().ok_or(RenderError::NoSurface)?;
        let device_handle = self
            .context
            .devices
            .get(surface.dev_id)
            .ok_or(RenderError::NoDevice)?;
        let device = &device_handle.device;
        let queue = &device_handle.queue;

        let frame = match surface.surface.get_current_texture() {
            vello::wgpu::CurrentSurfaceTexture::Success(frame)
            | vello::wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            vello::wgpu::CurrentSurfaceTexture::Timeout
            | vello::wgpu::CurrentSurfaceTexture::Occluded => {
                return Err(RenderError::Surface("frame timeout/occluded".to_string()));
            }
            vello::wgpu::CurrentSurfaceTexture::Outdated
            | vello::wgpu::CurrentSurfaceTexture::Lost => {
                return Err(RenderError::Surface("surface lost/outdated".to_string()));
            }
            vello::wgpu::CurrentSurfaceTexture::Validation => {
                return Err(RenderError::Surface("surface validation error".to_string()));
            }
        };
        let frame_view = frame
            .texture
            .create_view(&vello::wgpu::TextureViewDescriptor::default());

        let params = render_params(width, height);
        renderer
            .render_to_texture(device, queue, scene, &surface.target_view, &params)
            .map_err(|e| RenderError::Vello(format!("{e:?}")))?;

        let mut encoder = device.create_command_encoder(&vello::wgpu::CommandEncoderDescriptor {
            label: Some("rix-vello-blit"),
        });
        surface
            .blitter
            .copy(device, &mut encoder, &surface.target_view, &frame_view);
        queue.submit([encoder.finish()]);
        frame.present();
        Ok(())
    }

    pub async fn render_to_rgba(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, RenderError> {
        if width == 0 || height == 0 {
            return Err(RenderError::BadSize(width, height));
        }
        let device_id = match &self.surface {
            Some(surface) => surface.dev_id,
            None => self
                .context
                .device(None)
                .await
                .ok_or(RenderError::NoDevice)?,
        };
        let handle = &self.context.devices[device_id];
        let limits = handle.device.limits();
        if width > limits.max_texture_dimension_2d || height > limits.max_texture_dimension_2d {
            return Err(RenderError::BadSize(width, height));
        }
        let row_bytes = width
            .checked_mul(4)
            .ok_or(RenderError::BadSize(width, height))?;
        let padded_row_bytes = row_bytes
            .checked_add(255)
            .map(|bytes| bytes / 256 * 256)
            .ok_or(RenderError::BadSize(width, height))?;
        let buffer_size = padded_row_bytes as u64 * height as u64;
        if buffer_size > limits.max_buffer_size {
            return Err(RenderError::BadSize(width, height));
        }
        if self.renderer_device_id != Some(device_id) {
            self.renderer = Some(
                Renderer::new(&handle.device, RendererOptions::default())
                    .map_err(|error| RenderError::Vello(format!("{error:?}")))?,
            );
            self.renderer_device_id = Some(device_id);
        }
        let device = &handle.device;
        let queue = &handle.queue;
        let size = vello::wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&vello::wgpu::TextureDescriptor {
            label: Some("rix-offscreen-frame"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: vello::wgpu::TextureDimension::D2,
            format: vello::wgpu::TextureFormat::Rgba8Unorm,
            usage: vello::wgpu::TextureUsages::STORAGE_BINDING
                | vello::wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        self.renderer
            .as_mut()
            .ok_or(RenderError::NoDevice)?
            .render_to_texture(device, queue, scene, &view, &render_params(width, height))
            .map_err(|error| RenderError::Vello(format!("{error:?}")))?;
        let buffer = device.create_buffer(&vello::wgpu::BufferDescriptor {
            label: Some("rix-offscreen-readback"),
            size: buffer_size,
            usage: vello::wgpu::BufferUsages::COPY_DST | vello::wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            vello::wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: vello::wgpu::Origin3d::ZERO,
                aspect: vello::wgpu::TextureAspect::All,
            },
            vello::wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: vello::wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row_bytes),
                    rows_per_image: Some(height),
                },
            },
            size,
        );
        let submission = queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(vello::wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let timeout = Duration::from_secs(30);
        device
            .poll(vello::wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(timeout),
            })
            .map_err(|error| RenderError::Readback(error.to_string()))?;
        receiver
            .recv_timeout(timeout)
            .map_err(|error| RenderError::Readback(error.to_string()))?
            .map_err(|error| RenderError::Readback(error.to_string()))?;
        let mapped = slice.get_mapped_range();
        let mut pixels = Vec::with_capacity(row_bytes as usize * height as usize);
        for row in mapped.chunks_exact(padded_row_bytes as usize) {
            pixels.extend_from_slice(&row[..row_bytes as usize]);
        }
        drop(mapped);
        buffer.unmap();
        Ok(pixels)
    }

    pub async fn render_scene_to_png(
        scene: &Scene,
        width: u32,
        height: u32,
        path: impl AsRef<Path>,
    ) -> Result<(), RenderError> {
        let mut renderer = Self::new();
        let pixels = renderer.render_to_rgba(scene, width, height).await?;
        image::save_buffer_with_format(
            path,
            &pixels,
            width,
            height,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )?;
        Ok(())
    }
}

fn render_params(width: u32, height: u32) -> RenderParams {
    RenderParams {
        base_color: Color::from_rgba8(12, 14, 20, 255),
        width,
        height,
        antialiasing_method: AaConfig::Msaa16,
    }
}
