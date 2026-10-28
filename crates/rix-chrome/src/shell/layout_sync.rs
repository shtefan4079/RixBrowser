
use super::*;
use crate::page_worker::LayoutChange;

impl BrowserShell {
    pub(super) fn sync_page_layout_inputs(&mut self) {
        let active_id = self.active_tab_id();
        let is_active_internal = self.is_active_internal();
        for (&tab_id, page) in &mut self.pages {
            let is_internal = if tab_id == active_id {
                is_active_internal
            } else {
                let u = page.url.trim();
                u.is_empty() || u.starts_with("rix://") || u == "about:blank"
            };
            if !is_internal {
                let scroll_range =
                    rix_layout::viewport_scroll_range(&page.layout_tree, page.layout_viewport);
                page.scroll_x = scroll_range.clamp_x(page.scroll_x);
                page.scroll_y = scroll_range.clamp_y(page.scroll_y);
            }
            let Some(worker) = page.page_worker.clone() else {
                continue;
            };
            let worker = worker.borrow();
            let sync = &mut page.worker_layout_sync;
            if sync.viewport != Some(page.layout_viewport) {
                let (width, height) = page.layout_viewport;
                worker.layout_change(LayoutChange::Viewport { width, height });
                sync.viewport = Some(page.layout_viewport);
            }
            if sync.scroll_x != Some(page.scroll_x) || sync.scroll_y != Some(page.scroll_y) {
                worker.layout_change(LayoutChange::Scroll {
                    x: page.scroll_x,
                    y: page.scroll_y,
                });
                sync.scroll_x = Some(page.scroll_x);
                sync.scroll_y = Some(page.scroll_y);
            }
            for (url, image) in &page.images {
                if sync.images.insert(url.clone()) {
                    worker.layout_change(LayoutChange::ImageSize {
                        url: url.clone(),
                        width: image.data.width as f32,
                        height: image.data.height as f32,
                    });
                }
            }
        }
    }
}
