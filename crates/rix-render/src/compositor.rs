
use crate::display_list::DisplayList;
use crate::vello_backend::VelloSceneBuilder;
use rix_layout::box_model::Rect;
use rix_text::font_loader::FontDatabase;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;
use vello::kurbo::Affine;
use vello::Scene;

pub const FPS_120_BUDGET_MS: f32 = 8.333;

pub const TELEMETRY_HISTORY_CAP: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameStats {
    pub frame_number: u64,
    pub frame_duration_ms: f32,
    pub fps: f32,
    pub is_120_fps_capable: bool,
    pub layers_composited: usize,
    pub damage_area_px2: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameTelemetry {
    pub frames_in_window: usize,
    pub total_frames: u64,
    pub avg_frame_ms: f32,
    pub avg_fps: f32,
    pub min_frame_ms: f32,
    pub max_frame_ms: f32,
    pub missed_120_budget: usize,
    pub healthy_120fps: bool,
}

impl Default for FrameTelemetry {
    fn default() -> Self {
        Self {
            frames_in_window: 0,
            total_frames: 0,
            avg_frame_ms: 0.0,
            avg_fps: 0.0,
            min_frame_ms: 0.0,
            max_frame_ms: 0.0,
            missed_120_budget: 0,
            healthy_120fps: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompositorLayer {
    pub id: usize,
    pub display_list: DisplayList,
    pub scroll_x: f32,
    pub scroll_y: f32,
    pub opacity: f32,
    pub z_index: i32,
    pub is_dirty: bool,
    pub damage: Vec<Rect>,
}

impl CompositorLayer {
    pub fn new(id: usize, z_index: i32) -> Self {
        Self {
            id,
            display_list: DisplayList::new(),
            scroll_x: 0.0,
            scroll_y: 0.0,
            opacity: 1.0,
            z_index,
            is_dirty: true,
            damage: Vec::new(),
        }
    }

    pub fn mark_damage(&mut self, rect: Rect) {
        if rect.width > 0.0 && rect.height > 0.0 {
            self.damage.push(rect);
            self.is_dirty = true;
        }
    }

    pub fn clear_damage(&mut self) {
        self.damage.clear();
        self.is_dirty = false;
    }
}

pub struct Compositor {
    layers: Vec<CompositorLayer>,
    cached_layer_scenes: Vec<Option<Arc<Scene>>>,
    font_resources: Vec<(u64, u32)>,
    font_fallbacks: [Option<rix_text::FontId>; 4],
    frame_counter: u64,
    last_frame_instant: Option<Instant>,
    frame_history: VecDeque<FrameStats>,
    pending_damage: Vec<Rect>,
    viewport: Rect,
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

impl Compositor {
    pub fn new() -> Self {
        Self {
            layers: Vec::new(),
            cached_layer_scenes: Vec::new(),
            font_resources: Vec::new(),
            font_fallbacks: [None; 4],
            frame_counter: 0,
            last_frame_instant: None,
            frame_history: VecDeque::with_capacity(TELEMETRY_HISTORY_CAP),
            pending_damage: Vec::new(),
            viewport: Rect::new(0.0, 0.0, 1280.0, 800.0),
        }
    }

    pub fn set_viewport(&mut self, width: f32, height: f32) {
        self.viewport = Rect::new(0.0, 0.0, width.max(1.0), height.max(1.0));
        self.mark_dirty_region(self.viewport);
    }

    pub fn viewport(&self) -> Rect {
        self.viewport
    }

    pub fn create_layer(&mut self, z_index: i32) -> usize {
        let id = self.layers.len();
        self.layers.push(CompositorLayer::new(id, z_index));
        self.cached_layer_scenes.push(None);
        self.mark_dirty_region(self.viewport);
        id
    }

    pub fn set_layer_display_list(&mut self, layer_id: usize, list: DisplayList) {
        if let Some(layer) = self.layers.get_mut(layer_id) {
            layer.display_list = list;
            self.cached_layer_scenes[layer_id] = None;
            layer.damage.push(self.viewport);
            layer.is_dirty = true;
            self.pending_damage.push(self.viewport);
        }
    }

    pub fn set_layer_opacity(&mut self, layer_id: usize, opacity: f32) {
        if let Some(layer) = self.layers.get_mut(layer_id) {
            layer.opacity = opacity.clamp(0.0, 1.0);
            layer.is_dirty = true;
        }
    }

    pub fn set_scroll_offset(&mut self, layer_id: usize, scroll_x: f32, scroll_y: f32) {
        if let Some(layer) = self.layers.get_mut(layer_id) {
            let dx = (scroll_x - layer.scroll_x).abs();
            let dy = (scroll_y - layer.scroll_y).abs();
            layer.scroll_x = scroll_x;
            layer.scroll_y = scroll_y;
            layer.is_dirty = true;
            if dx > 0.0 || dy > 0.0 {
                layer.damage.push(self.viewport);
                self.pending_damage.push(self.viewport);
            }
        }
    }

    pub fn mark_dirty_region(&mut self, rect: Rect) {
        if rect.width > 0.0 && rect.height > 0.0 {
            self.pending_damage.push(rect);
        }
    }

    pub fn damage_rects(&self) -> &[Rect] {
        &self.pending_damage
    }

    pub fn damage_area(&self) -> f32 {
        self.pending_damage
            .iter()
            .map(|r| (r.width.max(0.0) * r.height.max(0.0)) as f32)
            .sum()
    }

    pub fn take_damage(&mut self) -> Vec<Rect> {
        std::mem::take(&mut self.pending_damage)
    }

    pub fn render_frame(&mut self) -> (Scene, FrameStats) {
        let stats = self.render_frame_inner(None);
        stats
    }

    pub fn render_frame_with_fonts(&mut self, font_db: &FontDatabase) -> (Scene, FrameStats) {
        self.render_frame_inner(Some(font_db))
    }

    fn render_frame_inner(&mut self, font_db: Option<&FontDatabase>) -> (Scene, FrameStats) {
        let resolved_db: &FontDatabase = match font_db {
            Some(db) => db,
            None => crate::vello_backend::default_font_db(),
        };
        let start = Instant::now();
        self.frame_counter += 1;

        let font_fallbacks = [
            resolved_db.default_sans_serif,
            resolved_db.default_serif,
            resolved_db.default_monospace,
            resolved_db.default_emoji,
        ];
        let fonts_changed = self.font_fallbacks != font_fallbacks
            || self.font_resources.len() != resolved_db.len()
            || self
                .font_resources
                .iter()
                .enumerate()
                .any(|(index, resource)| {
                    let font = resolved_db.get_font(rix_text::FontId(index)).unwrap();
                    *resource != (font.blob().id(), font.face_index)
                });
        if fonts_changed {
            self.font_fallbacks = font_fallbacks;
            self.font_resources = (0..resolved_db.len())
                .map(|index| {
                    let font = resolved_db.get_font(rix_text::FontId(index)).unwrap();
                    (font.blob().id(), font.face_index)
                })
                .collect();
            self.cached_layer_scenes.fill(None);
        }

        let mut final_scene = Scene::new();

        let mut sorted_indices: Vec<usize> = (0..self.layers.len()).collect();
        sorted_indices.sort_by_key(|&idx| self.layers[idx].z_index);

        let damage_area = self.damage_area();
        for idx in sorted_indices {
            let layer = &mut self.layers[idx];
            let layer_scene = self.cached_layer_scenes[idx].get_or_insert_with(|| {
                Arc::new(VelloSceneBuilder::build_scene_with_fonts(
                    &layer.display_list,
                    resolved_db,
                ))
            });
            let scroll_transform =
                Affine::translate((-layer.scroll_x as f64, -layer.scroll_y as f64));
            final_scene.append(layer_scene, Some(scroll_transform));
            let _ = layer.opacity;
            layer.clear_damage();
        }
        self.pending_damage.clear();

        let elapsed = start.elapsed();
        let duration_ms = elapsed.as_secs_f32() * 1000.0;
        let fps = if duration_ms > 0.001 {
            (1000.0 / duration_ms).min(1000.0)
        } else {
            1000.0
        };

        let is_120_fps_capable = duration_ms <= FPS_120_BUDGET_MS;

        let stats = FrameStats {
            frame_number: self.frame_counter,
            frame_duration_ms: duration_ms,
            fps,
            is_120_fps_capable,
            layers_composited: self.layers.len(),
            damage_area_px2: damage_area,
        };

        self.frame_history.push_back(stats);
        while self.frame_history.len() > TELEMETRY_HISTORY_CAP {
            self.frame_history.pop_front();
        }

        self.last_frame_instant = Some(Instant::now());
        (final_scene, stats)
    }

    pub fn telemetry(&self) -> FrameTelemetry {
        if self.frame_history.is_empty() {
            return FrameTelemetry {
                total_frames: self.frame_counter,
                ..FrameTelemetry::default()
            };
        }
        let n = self.frame_history.len() as f32;
        let sum_ms: f32 = self.frame_history.iter().map(|s| s.frame_duration_ms).sum();
        let avg_ms = sum_ms / n;
        let avg_fps = if avg_ms > 0.001 {
            1000.0 / avg_ms
        } else {
            1000.0
        };
        let min_ms = self
            .frame_history
            .iter()
            .map(|s| s.frame_duration_ms)
            .fold(f32::INFINITY, f32::min);
        let max_ms = self
            .frame_history
            .iter()
            .map(|s| s.frame_duration_ms)
            .fold(0.0_f32, f32::max);
        let missed = self
            .frame_history
            .iter()
            .filter(|s| !s.is_120_fps_capable)
            .count();
        FrameTelemetry {
            frames_in_window: self.frame_history.len(),
            total_frames: self.frame_counter,
            avg_frame_ms: avg_ms,
            avg_fps,
            min_frame_ms: min_ms,
            max_frame_ms: max_ms,
            missed_120_budget: missed,
            healthy_120fps: avg_ms <= FPS_120_BUDGET_MS,
        }
    }

    pub fn frame_history(&self) -> &VecDeque<FrameStats> {
        &self.frame_history
    }

    pub fn average_fps(&self) -> f32 {
        self.telemetry().avg_fps
    }

    pub fn average_frame_ms(&self) -> f32 {
        self.telemetry().avg_frame_ms
    }

    pub fn layers(&self) -> &[CompositorLayer] {
        &self.layers
    }

    pub fn frame_count(&self) -> u64 {
        self.frame_counter
    }
}
