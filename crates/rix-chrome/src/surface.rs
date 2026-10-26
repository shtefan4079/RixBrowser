
#![deny(unsafe_code)]

use rix_dom::node::NodeData;
use rix_dom::tree::Document;
use rix_dom::NodeId;
use rix_ipc::protocol::IpcInputEvent;
use rix_ipc::shmem::SharedTripleBuffer;
use rix_layout::box_model::Rect;
use rix_layout::tree::LayoutTree;
use rix_render::compositor::Compositor;
use rix_render::display_list::{DisplayList, DrawCommand};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SurfaceError {
    #[error("WebSurface element not found in DOM or LayoutTree")]
    NotFound,

    #[error("Shared memory error: {0}")]
    Shmem(#[from] rix_ipc::shmem::ShmemError),

    #[error("Frame buffer size mismatch: expected at least {expected} bytes, got {actual}")]
    BufferMismatch { expected: usize, actual: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub struct WebSurface {
    pub bounds: Rect,
    pub node_id: Option<NodeId>,
    pub device_pixel_ratio: f32,
    pub last_projected_frame: u64,
}

impl Default for WebSurface {
    fn default() -> Self {
        Self {
            bounds: Rect::default(),
            node_id: None,
            device_pixel_ratio: 1.0,
            last_projected_frame: 0,
        }
    }
}

impl WebSurface {
    pub fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            node_id: None,
            device_pixel_ratio: 1.0,
            last_projected_frame: 0,
        }
    }

    pub fn with_scale(bounds: Rect, device_pixel_ratio: f32) -> Self {
        Self {
            bounds,
            node_id: None,
            device_pixel_ratio: device_pixel_ratio.max(0.5),
            last_projected_frame: 0,
        }
    }

    pub fn update_from_layout(
        &mut self,
        layout_tree: &LayoutTree,
        doc: &Document,
    ) -> Result<(), SurfaceError> {
        let mut fallback: Option<(Rect, NodeId)> = None;
        for box_data in layout_tree.iter() {
            let Some(node_id) = box_data.dom_node else {
                continue;
            };
            let Some(NodeData::Element(elem)) = doc.get(node_id) else {
                continue;
            };
            let is_tag = &*elem.name.local == "web-surface";
            let is_id = elem.id() == Some("active-web-surface");
            if is_tag {
                self.bounds = box_data.dimensions.border_box();
                self.node_id = Some(node_id);
                return Ok(());
            } else if is_id && fallback.is_none() {
                fallback = Some((box_data.dimensions.border_box(), node_id));
            }
        }
        if let Some((rect, node)) = fallback {
            self.bounds = rect;
            self.node_id = Some(node);
            return Ok(());
        }
        Err(SurfaceError::NotFound)
    }

    #[inline]
    pub fn viewport_rect(&self) -> Rect {
        self.bounds
    }

    #[inline]
    pub fn contains(&self, wx: f32, wy: f32) -> bool {
        self.bounds.contains(wx, wy)
    }

    pub fn translate_coordinates(&self, wx: f32, wy: f32) -> Option<(f32, f32)> {
        if self.contains(wx, wy) {
            Some((
                (wx - self.bounds.x) * self.device_pixel_ratio,
                (wy - self.bounds.y) * self.device_pixel_ratio,
            ))
        } else {
            None
        }
    }

    pub fn to_global_coordinates(&self, cx: f32, cy: f32) -> (f32, f32) {
        (
            cx / self.device_pixel_ratio + self.bounds.x,
            cy / self.device_pixel_ratio + self.bounds.y,
        )
    }

    pub fn map_input_event(&self, event: &IpcInputEvent) -> Option<IpcInputEvent> {
        match event {
            IpcInputEvent::MouseMove { x, y } => self
                .translate_coordinates(*x, *y)
                .map(|(cx, cy)| IpcInputEvent::MouseMove { x: cx, y: cy }),
            IpcInputEvent::MouseDown { x, y, button } => {
                self.translate_coordinates(*x, *y)
                    .map(|(cx, cy)| IpcInputEvent::MouseDown {
                        x: cx,
                        y: cy,
                        button: *button,
                    })
            }
            IpcInputEvent::MouseUp { x, y, button } => {
                self.translate_coordinates(*x, *y)
                    .map(|(cx, cy)| IpcInputEvent::MouseUp {
                        x: cx,
                        y: cy,
                        button: *button,
                    })
            }
            IpcInputEvent::Scroll { dx, dy } => Some(IpcInputEvent::Scroll { dx: *dx, dy: *dy }),
            IpcInputEvent::KeyDown { key, code } => Some(IpcInputEvent::KeyDown {
                key: key.clone(),
                code: code.clone(),
            }),
            IpcInputEvent::KeyUp { key, code } => Some(IpcInputEvent::KeyUp {
                key: key.clone(),
                code: code.clone(),
            }),
        }
    }

    pub fn read_active_frame<'a>(
        &self,
        shmem: &'a SharedTripleBuffer,
    ) -> Result<&'a [u8], SurfaceError> {
        let active_index = shmem.active_front_index() as usize;
        let slice = shmem.get_buffer(active_index)?;
        Ok(slice)
    }

    #[inline]
    pub fn active_frame_number(&self, shmem: &SharedTripleBuffer) -> u64 {
        shmem.frame_number()
    }

    pub fn has_new_frame(&self, shmem: &SharedTripleBuffer) -> bool {
        shmem.frame_number() != self.last_projected_frame
    }

    pub fn project_active_frame_to_rgba(
        &mut self,
        shmem: &SharedTripleBuffer,
        dst: &mut [u8],
        dst_width: usize,
        dst_height: usize,
    ) -> Result<bool, SurfaceError> {
        let header = shmem.header();
        let src_w = header.width as usize;
        let src_h = header.height as usize;
        if src_w == 0 || src_h == 0 {
            return Ok(false);
        }
        let expected_dst = dst_width
            .checked_mul(dst_height)
            .and_then(|n| n.checked_mul(4))
            .unwrap_or(usize::MAX);
        if dst.len() < expected_dst {
            return Err(SurfaceError::BufferMismatch {
                expected: expected_dst,
                actual: dst.len(),
            });
        }

        let frame = self.read_active_frame(shmem)?;
        let frame_no = shmem.frame_number();
        if frame.is_empty() {
            return Ok(false);
        }

        let dx0 = self.bounds.x.max(0.0) as usize;
        let dy0 = self.bounds.y.max(0.0) as usize;
        let dx1 = ((self.bounds.x + self.bounds.width) as usize).min(dst_width);
        let dy1 = ((self.bounds.y + self.bounds.height) as usize).min(dst_height);
        if dx1 <= dx0 || dy1 <= dy0 {
            return Ok(false);
        }
        let dw = dx1 - dx0;
        let dh = dy1 - dy0;

        for dy in 0..dh {
            let sy = (dy * src_h / dh).min(src_h.saturating_sub(1));
            for dx in 0..dw {
                let sx = (dx * src_w / dw).min(src_w.saturating_sub(1));
                let s_idx = (sy * src_w + sx) * 4;
                let d_idx = ((dy0 + dy) * dst_width + (dx0 + dx)) * 4;
                if s_idx + 4 <= frame.len() && d_idx + 4 <= dst.len() {
                    dst[d_idx..d_idx + 4].copy_from_slice(&frame[s_idx..s_idx + 4]);
                }
            }
        }

        self.last_projected_frame = frame_no;
        Ok(true)
    }

    pub fn build_surface_display_list(&self) -> DisplayList {
        let mut list = DisplayList::new();
        list.push(DrawCommand::PushClip { rect: self.bounds }, i32::MIN);
        list.push(
            DrawCommand::DrawRect {
                rect: self.bounds,
                color: rix_css::color::CssColor {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                },
                border_radius: 0.0,
            },
            i32::MIN + 1,
        );
        list.push(DrawCommand::PopClip, i32::MAX);
        list
    }

    pub fn present_to_compositor(&self, compositor: &mut Compositor, layer_id: usize) {
        let list = self.build_surface_display_list();
        compositor.set_layer_display_list(layer_id, list);
    }
}
