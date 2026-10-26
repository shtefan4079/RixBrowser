
use super::*;
use boa_engine::object::builtins::JsPromise;
use rix_layout::box_model::Rect;
use rix_layout::tree::{LayoutBoxId, LayoutTree};

pub struct LayoutSnapshot {
    tree: LayoutTree,
    boxes_by_node: HashMap<NodeId, Vec<LayoutBoxId>>,
    styles: Rc<HashMap<NodeId, rix_css::ComputedStyle>>,
}

impl LayoutSnapshot {
    pub fn with_styles(tree: LayoutTree, styles: Rc<HashMap<NodeId, rix_css::ComputedStyle>>) -> Self {
        let mut snapshot = Self::new(tree);
        snapshot.styles = styles;
        snapshot
    }

    pub fn style(&self, node: NodeId) -> Option<&rix_css::ComputedStyle> {
        self.styles
            .get(&node)
            .or_else(|| self.boxes(node).next().map(|item| &item.style))
    }

    pub fn new(tree: LayoutTree) -> Self {
        let mut boxes_by_node: HashMap<NodeId, Vec<LayoutBoxId>> = HashMap::new();
        for item in tree.iter() {
            if let Some(node) = item.dom_node {
                boxes_by_node.entry(node).or_default().push(item.id);
            }
        }
        Self {
            tree,
            boxes_by_node,
            styles: Rc::new(HashMap::new()),
        }
    }

    pub fn empty() -> Self {
        Self::new(LayoutTree::new())
    }

    pub fn tree(&self) -> &LayoutTree {
        &self.tree
    }

    pub(super) fn boxes(
        &self,
        node: NodeId,
    ) -> impl Iterator<Item = &rix_layout::tree::LayoutBoxData> {
        self.boxes_by_node
            .get(&node)
            .into_iter()
            .flatten()
            .filter_map(|id| self.tree.get(*id))
    }

    pub fn border_boxes(&self, node: NodeId) -> Vec<Rect> {
        self.boxes(node)
            .map(|item| item.dimensions.border_box())
            .collect()
    }

    pub fn border_boxes_with_anchoring(&self, node: NodeId) -> Vec<(Rect, bool)> {
        self.boxes(node)
            .map(|item| {
                (
                    item.dimensions.border_box(),
                    self.tree.is_viewport_anchored(item.id),
                )
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageLoadState {
    Pending,
    Loaded { width: f32, height: f32 },
    Broken,
}

pub trait LayoutHost {
    fn layout(&self, document: &Document) -> Option<Rc<LayoutSnapshot>>;
    fn computed_style(
        &self,
        _document: &Document,
        snapshot: &LayoutSnapshot,
        node: NodeId,
    ) -> Option<rix_css::ComputedStyle> {
        snapshot.style(node).cloned()
    }
    fn computed_style_for_node(
        &self,
        _document: &Document,
        _node: NodeId,
    ) -> Option<rix_css::ComputedStyle> {
        None
    }
    fn viewport_size(&self) -> (f32, f32);
    fn scroll_position(&self) -> (f32, f32);
    fn request_scroll_position(&self, x: f32, y: f32);
    fn rendering_revision(&self) -> (u64, u64) {
        (0, 0)
    }
    fn image_load_state(&self, _url: &str) -> ImageLoadState {
        ImageLoadState::Pending
    }
}

struct LayoutHostData(Rc<dyn LayoutHost>);

pub(super) fn host(context: &Context) -> JsResult<Rc<dyn LayoutHost>> {
    context
        .get_data::<LayoutHostData>()
        .map(|data| data.0.clone())
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("layout is unavailable")
                .into()
        })
}

pub(super) fn image_load_state(context: &Context, url: &str) -> ImageLoadState {
    context
        .get_data::<LayoutHostData>()
        .map(|data| data.0.image_load_state(url))
        .unwrap_or(ImageLoadState::Pending)
}

pub(super) fn snapshot_for(
    context: &Context,
    _reason: &'static str,
) -> JsResult<Option<(Rc<LayoutSnapshot>, Rc<RefCell<Document>>)>> {
    let host = host(context)?;
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return Ok(None);
    };
    let document = state.borrow().document.clone();
    let snapshot = host.layout(&document.borrow());
    Ok(snapshot.map(|snapshot| (snapshot, document)))
}

fn union(rects: &[Rect]) -> Option<Rect> {
    let first = *rects.first()?;
    Some(rects[1..].iter().fold(first, |acc, rect| {
        let left = acc.x.min(rect.x);
        let top = acc.y.min(rect.y);
        let right = (acc.x + acc.width).max(rect.x + rect.width);
        let bottom = (acc.y + acc.height).max(rect.y + rect.height);
        Rect::new(left, top, right - left, bottom - top)
    }))
}

fn element_name(document: &Document, node: NodeId) -> Option<String> {
    match document.get(node)? {
        NodeData::Element(element) => Some(element.name.local.to_string()),
        _ => None,
    }
}

fn make_rect(rect: Rect, context: &mut Context) -> JsResult<JsValue> {
    let constructor = context
        .global_object()
        .get(js_string!("DOMRect"), context)?;
    let constructor = constructor
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("DOMRect is not available"))?;
    constructor
        .construct(
            &[
                JsValue::new(rect.x as f64),
                JsValue::new(rect.y as f64),
                JsValue::new(rect.width as f64),
                JsValue::new(rect.height as f64),
            ],
            None,
            context,
        )
        .map(Into::into)
}

fn is_visually_hidden(doc: &Document, node: NodeId) -> bool {
    let mut curr = Some(node);
    let mut depth = 0;
    while let Some(n) = curr {
        if depth > 50 {
            break;
        }
        depth += 1;
        if let Some(NodeData::Element(element)) = doc.get(n) {
            if element.has_attribute("hidden") {
                return true;
            }
            if let Some(style) = element.get_attribute("style") {
                if style.contains("display:none") || style.contains("display: none") {
                    return true;
                }
            }
        }
        curr = doc.parent(n);
    }
    false
}

fn client_rects(node: NodeId, context: &mut Context) -> JsResult<Vec<Rect>> {
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return Ok(Vec::new());
    };
    let doc = state.borrow().document.clone();
    let doc_ref = doc.borrow();
    if !shadow_dom::node_is_connected(node, &doc_ref) || is_visually_hidden(&doc_ref, node) {
        return Ok(Vec::new());
    }
    drop(doc_ref);
    drop(doc);
    let layout_host = host(context)?;
    let Some((snapshot, _)) = snapshot_for(context, "client_rects")? else {
        return Ok(Vec::new());
    };
    let (scroll_x, scroll_y) = layout_host.scroll_position();
    Ok(snapshot
        .border_boxes_with_anchoring(node)
        .into_iter()
        .map(|(rect, anchored)| {
            let (scroll_x, scroll_y) = if anchored {
                (0.0, 0.0)
            } else {
                (scroll_x, scroll_y)
            };
            Rect::new(
                rect.x - scroll_x,
                rect.y - scroll_y,
                rect.width,
                rect.height,
            )
        })
        .collect())
}

fn get_bounding_client_rect(
    this: &JsValue,
    _: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    let rects = client_rects(node, context)?;
    make_rect(union(&rects).unwrap_or_default(), context)
}

fn get_client_rects(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    let rects = client_rects(node, context)?;
    let mut values = Vec::with_capacity(rects.len());
    for rect in rects {
        values.push(make_rect(rect, context)?);
    }
    let list = context
        .global_object()
        .get(js_string!("DOMRectList"), context)?;
    let list = list
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("DOMRectList is not available"))?;
    let array = boa_engine::object::builtins::JsArray::from_iter(values, context);
    list.call(&JsValue::undefined(), &[array.into()], context)
}

fn offset_parent(snapshot: &LayoutSnapshot, document: &Document, node: NodeId) -> Option<NodeId> {
    let item = snapshot.boxes(node).next()?;
    let name = element_name(document, node)?;
    if name == "body" || name == "html" || item.style.position == "fixed" {
        return None;
    }
    let positioned = item.style.position != "static";
    let mut ancestor = document.parent(node);
    while let Some(candidate) = ancestor {
        if let Some(candidate_name) = element_name(document, candidate) {
            if let Some(candidate_box) = snapshot.boxes(candidate).next() {
                if candidate_box.style.position != "static" || candidate_name == "body" {
                    return Some(candidate);
                }
                if !positioned && matches!(candidate_name.as_str(), "td" | "th" | "table") {
                    return Some(candidate);
                }
            }
        }
        ancestor = document.parent(candidate);
    }
    None
}

fn offset_value(this: &JsValue, context: &mut Context, which: OffsetKind) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return Ok(if which == OffsetKind::Parent {
            JsValue::null()
        } else {
            JsValue::new(0)
        });
    };
    let doc = state.borrow().document.clone();
    let doc_ref = doc.borrow();
    if !shadow_dom::node_is_connected(node, &doc_ref) || is_visually_hidden(&doc_ref, node) {
        return Ok(if which == OffsetKind::Parent {
            JsValue::null()
        } else {
            JsValue::new(0)
        });
    }
    drop(doc_ref);
    drop(doc);
    let Some((snapshot, document)) = snapshot_for(context, "offset_value")? else {
        return Ok(if which == OffsetKind::Parent {
            JsValue::null()
        } else {
            JsValue::new(0)
        });
    };
    let document = document.borrow();
    let Some(border) = snapshot
        .boxes(node)
        .next()
        .map(|item| item.dimensions.border_box())
    else {
        return Ok(if which == OffsetKind::Parent {
            JsValue::null()
        } else {
            JsValue::new(0)
        });
    };
    let parent = offset_parent(&snapshot, &document, node);
    let origin = parent
        .filter(|parent| element_name(&document, *parent).as_deref() != Some("body"))
        .and_then(|parent| snapshot.boxes(parent).next())
        .map(|item| item.dimensions.padding_box())
        .map_or((0.0, 0.0), |rect| (rect.x, rect.y));
    let value = match which {
        OffsetKind::Parent => {
            drop(document);
            return Ok(parent.map_or(JsValue::null(), |parent| {
                wrap_element(parent, context).into()
            }));
        }
        OffsetKind::Top => border.y - origin.1,
        OffsetKind::Left => border.x - origin.0,
        OffsetKind::Width => border.width,
        OffsetKind::Height => border.height,
    };
    Ok(JsValue::new(value.round() as i32))
}

#[derive(PartialEq, Clone, Copy)]
enum OffsetKind {
    Parent,
    Top,
    Left,
    Width,
    Height,
}

#[derive(Clone, Copy)]
enum ClientKind {
    Width,
    Height,
    Top,
    Left,
    ScrollWidth,
    ScrollHeight,
}

fn client_value(this: &JsValue, context: &mut Context, which: ClientKind) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return Ok(JsValue::new(0));
    };
    let doc = state.borrow().document.clone();
    let doc_ref = doc.borrow();
    if !shadow_dom::node_is_connected(node, &doc_ref) || is_visually_hidden(&doc_ref, node) {
        return Ok(JsValue::new(0));
    }
    drop(doc_ref);
    drop(doc);
    let layout_host = host(context)?;
    let Some((snapshot, document)) = snapshot_for(context, "client_value")? else {
        return Ok(JsValue::new(0));
    };
    let document = document.borrow();
    let is_root = document.parent(node) == Some(document.root());
    let Some(item) = snapshot.boxes(node).next() else {
        return Ok(JsValue::new(0));
    };
    let (viewport_width, viewport_height) = layout_host.viewport_size();
    let padding = item.dimensions.padding_box();
    let inline = item.style.display == rix_css::Display::Inline && !item.is_replaced;
    let overflow = || {
        let mut rect = padding;
        for descendant in document.descendants(node).skip(1) {
            for border in snapshot.border_boxes(descendant) {
                if border.width > 0.0 || border.height > 0.0 {
                    rect.width = rect.width.max(border.x + border.width - rect.x);
                    rect.height = rect.height.max(border.y + border.height - rect.y);
                }
            }
        }
        rect
    };
    let value = match which {
        _ if inline => 0.0,
        ClientKind::Width if is_root => viewport_width,
        ClientKind::Height if is_root => viewport_height,
        ClientKind::Width => padding.width,
        ClientKind::Height => padding.height,
        ClientKind::Top => item.dimensions.border.top,
        ClientKind::Left => item.dimensions.border.left,
        ClientKind::ScrollWidth if is_root => overflow().width.max(viewport_width),
        ClientKind::ScrollHeight if is_root => overflow().height.max(viewport_height),
        ClientKind::ScrollWidth => overflow().width,
        ClientKind::ScrollHeight => overflow().height,
    };
    Ok(JsValue::new(value.round() as i32))
}

macro_rules! getter {
    ($name:ident, $body:expr) => {
        fn $name(this: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
            #[allow(clippy::redundant_closure_call)]
            ($body)(this, context)
        }
    };
}

getter!(offset_parent_get, |this, context| offset_value(
    this,
    context,
    OffsetKind::Parent
));
getter!(offset_top_get, |this, context| offset_value(
    this,
    context,
    OffsetKind::Top
));
getter!(offset_left_get, |this, context| offset_value(
    this,
    context,
    OffsetKind::Left
));
getter!(offset_width_get, |this, context| offset_value(
    this,
    context,
    OffsetKind::Width
));
getter!(offset_height_get, |this, context| offset_value(
    this,
    context,
    OffsetKind::Height
));
getter!(client_width_get, |this, context| client_value(
    this,
    context,
    ClientKind::Width
));
getter!(client_height_get, |this, context| client_value(
    this,
    context,
    ClientKind::Height
));
getter!(client_top_get, |this, context| client_value(
    this,
    context,
    ClientKind::Top
));
getter!(client_left_get, |this, context| client_value(
    this,
    context,
    ClientKind::Left
));
getter!(scroll_width_get, |this, context| client_value(
    this,
    context,
    ClientKind::ScrollWidth
));
getter!(scroll_height_get, |this, context| client_value(
    this,
    context,
    ClientKind::ScrollHeight
));

fn element_scroll_value(
    this: &JsValue,
    context: &mut Context,
    is_y: bool,
) -> JsResult<f64> {
    let node_id = match get_node_id_from_this(this, context) {
        Ok(id) => id,
        Err(_) => return Ok(0.0),
    };
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM is unavailable"))?;
    let doc = state.borrow().document.clone();
    let doc = doc.borrow();
    let is_root = matches!(doc.get(node_id), Some(NodeData::Element(el)) if el.name.local.as_str() == "html");
    drop(doc);

    if is_root {
        let (scroll_x, scroll_y) = host(context)?.scroll_position();
        Ok(if is_y { scroll_y as f64 } else { scroll_x as f64 })
    } else {
        let bridge = state.borrow();
        let pos = bridge.element_scroll_positions.get(&node_id).copied().unwrap_or((0.0, 0.0));
        Ok(if is_y { pos.1 as f64 } else { pos.0 as f64 })
    }
}

fn element_set_scroll_value(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
    is_y: bool,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let val = normalize_scroll_value(args.get_or_undefined(0).to_number(context)?);
    let val_f32 = val.max(0.0) as f32;

    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM is unavailable"))?;
    let doc = state.borrow().document.clone();
    let doc = doc.borrow();
    let is_root = matches!(doc.get(node_id), Some(NodeData::Element(el)) if el.name.local.as_str() == "html");
    drop(doc);

    if is_root {
        let host = host(context)?;
        let (cur_x, cur_y) = host.scroll_position();
        if is_y {
            host.request_scroll_position(cur_x, val_f32);
        } else {
            host.request_scroll_position(val_f32, cur_y);
        }
    } else {
        let mut bridge = state.borrow_mut();
        let entry = bridge.element_scroll_positions.entry(node_id).or_insert((0.0, 0.0));
        if is_y {
            entry.1 = val_f32;
        } else {
            entry.0 = val_f32;
        }
    }
    Ok(JsValue::undefined())
}

getter!(scroll_top_get, |this, context| {
    let val = element_scroll_value(this, context, true)?;
    Ok(JsValue::new(val))
});

fn scroll_top_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    element_set_scroll_value(this, args, context, true)
}

getter!(scroll_left_get, |this, context| {
    let val = element_scroll_value(this, context, false)?;
    Ok(JsValue::new(val))
});

fn scroll_left_set(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    element_set_scroll_value(this, args, context, false)
}

fn parse_element_scroll_coords(
    args: &[JsValue],
    current_x: f32,
    current_y: f32,
    relative: bool,
    context: &mut Context,
) -> JsResult<(f32, f32)> {
    let current_x = normalize_scroll_value(f64::from(current_x));
    let current_y = normalize_scroll_value(f64::from(current_y));
    let (mut x, mut y);
    if args.len() >= 2 {
        x = normalize_scroll_value(args[0].to_number(context)?);
        y = normalize_scroll_value(args[1].to_number(context)?);
        if relative {
            x += current_x;
            y += current_y;
        }
    } else if let Some(options) = args.first().and_then(|v| v.as_object()) {
        let left = options.get(js_string!("left"), context)?;
        let top = options.get(js_string!("top"), context)?;
        let left = if left.is_undefined() { None } else { Some(normalize_scroll_value(left.to_number(context)?)) };
        let top = if top.is_undefined() { None } else { Some(normalize_scroll_value(top.to_number(context)?)) };
        if relative {
            x = current_x + left.unwrap_or(0.0);
            y = current_y + top.unwrap_or(0.0);
        } else {
            x = left.unwrap_or(current_x);
            y = top.unwrap_or(current_y);
        }
    } else {
        x = current_x;
        y = current_y;
    }
    Ok((x.max(0.0) as f32, y.max(0.0) as f32))
}

fn element_scroll_to(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM is unavailable"))?;
    let doc = state.borrow().document.clone();
    let doc = doc.borrow();
    let is_root = matches!(doc.get(node_id), Some(NodeData::Element(el)) if el.name.local.as_str() == "html");
    drop(doc);

    if is_root {
        scroll_to(this, args, context)
    } else {
        let (current_x, current_y) = state
            .borrow()
            .element_scroll_positions
            .get(&node_id)
            .copied()
            .unwrap_or((0.0, 0.0));
        let (new_x, new_y) = parse_element_scroll_coords(args, current_x, current_y, false, context)?;
        state
            .borrow_mut()
            .element_scroll_positions
            .insert(node_id, (new_x, new_y));
        Ok(JsValue::undefined())
    }
}

fn element_scroll_by(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM is unavailable"))?;
    let doc = state.borrow().document.clone();
    let doc = doc.borrow();
    let is_root = matches!(doc.get(node_id), Some(NodeData::Element(el)) if el.name.local.as_str() == "html");
    drop(doc);

    if is_root {
        scroll_by(this, args, context)
    } else {
        let (current_x, current_y) = state
            .borrow()
            .element_scroll_positions
            .get(&node_id)
            .copied()
            .unwrap_or((0.0, 0.0));
        let (new_x, new_y) = parse_element_scroll_coords(args, current_x, current_y, true, context)?;
        state
            .borrow_mut()
            .element_scroll_positions
            .insert(node_id, (new_x, new_y));
        Ok(JsValue::undefined())
    }
}

fn viewport_get(which: u8, context: &mut Context) -> JsResult<JsValue> {
    let host = host(context)?;
    let (width, height) = host.viewport_size();
    let (scroll_x, scroll_y) = host.scroll_position();
    Ok(match which {
        0 => JsValue::new(width.round() as i32),
        1 => JsValue::new(height.round() as i32),
        2 => JsValue::new(scroll_x as f64),
        _ => JsValue::new(scroll_y as f64),
    })
}

getter!(inner_width_get, |_, context| viewport_get(0, context));
getter!(inner_height_get, |_, context| viewport_get(1, context));
getter!(device_pixel_ratio_get, |_, _context| Ok(JsValue::new(1.0)));
getter!(scroll_x_get, |_, context| viewport_get(2, context));
getter!(scroll_y_get, |_, context| viewport_get(3, context));

fn normalize_scroll_value(value: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

fn scroll_coordinates(
    args: &[JsValue],
    relative: bool,
    context: &mut Context,
) -> JsResult<(f32, f32)> {
    let host = host(context)?;
    let (current_x, current_y) = host.scroll_position();
    let current_x = normalize_scroll_value(f64::from(current_x));
    let current_y = normalize_scroll_value(f64::from(current_y));
    let (mut x, mut y);

    if args.len() >= 2 {
        x = normalize_scroll_value(args[0].to_number(context)?);
        y = normalize_scroll_value(args[1].to_number(context)?);
        if relative {
            x += current_x;
            y += current_y;
        }
    } else {
        let options = match args.first() {
            None => None,
            Some(value) if value.is_null() || value.is_undefined() => None,
            Some(value) => Some(value.as_object().ok_or_else(|| {
                JsNativeError::typ().with_message("Scroll options must be an object")
            })?),
        };

        if let Some(options) = options {
            let behavior = options.get(js_string!("behavior"), context)?;
            if !behavior.is_undefined() {
                let behavior = behavior.to_string(context)?.to_std_string_escaped();
                if !matches!(behavior.as_str(), "auto" | "instant" | "smooth") {
                    return Err(JsNativeError::typ()
                        .with_message("Invalid ScrollBehavior value")
                        .into());
                }
            }

            let left = options.get(js_string!("left"), context)?;
            let left = if left.is_undefined() {
                None
            } else {
                Some(normalize_scroll_value(left.to_number(context)?))
            };
            let top = options.get(js_string!("top"), context)?;
            let top = if top.is_undefined() {
                None
            } else {
                Some(normalize_scroll_value(top.to_number(context)?))
            };

            if relative {
                x = current_x + left.unwrap_or(0.0);
                y = current_y + top.unwrap_or(0.0);
            } else {
                x = left.unwrap_or(current_x);
                y = top.unwrap_or(current_y);
            }
        } else if relative {
            x = current_x;
            y = current_y;
        } else {
            x = current_x;
            y = current_y;
        }
    }

    if let Some((snapshot, _)) = snapshot_for(context, "scroll_into_view")? {
        let (width, height) = host.viewport_size();
        let range = rix_layout::viewport_scroll_range(snapshot.tree(), (width, height));
        x = x.clamp(f64::from(range.min_x), f64::from(range.max_x));
        y = y.clamp(f64::from(range.min_y), f64::from(range.max_y));
    } else {
        x = x.max(0.0);
        y = y.max(0.0);
    }
    let to_layout_coordinate = |value: f64| value.min(f64::from(f32::MAX)) as f32;
    Ok((to_layout_coordinate(x), to_layout_coordinate(y)))
}

fn scroll_to(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (x, y) = scroll_coordinates(args, false, context)?;
    host(context)?.request_scroll_position(x, y);
    Ok(JsValue::undefined())
}

fn scroll_by(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let (x, y) = scroll_coordinates(args, true, context)?;
    host(context)?.request_scroll_position(x, y);
    Ok(JsValue::undefined())
}

#[derive(Clone, Copy)]
enum ScrollAlignment {
    Start,
    Center,
    End,
    Nearest,
}

struct ScrollIntoViewOptions {
    block: ScrollAlignment,
    inline: ScrollAlignment,
}

fn scroll_alignment(
    value: &JsValue,
    member: &str,
    context: &mut Context,
) -> JsResult<ScrollAlignment> {
    let value = value.to_string(context)?.to_std_string_escaped();
    match value.as_str() {
        "start" => Ok(ScrollAlignment::Start),
        "center" => Ok(ScrollAlignment::Center),
        "end" => Ok(ScrollAlignment::End),
        "nearest" => Ok(ScrollAlignment::Nearest),
        _ => Err(JsNativeError::typ()
            .with_message(format!("Invalid ScrollLogicalPosition for {member}"))
            .into()),
    }
}

fn scroll_into_view_options(
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<ScrollIntoViewOptions> {
    let mut options = ScrollIntoViewOptions {
        block: ScrollAlignment::Start,
        inline: ScrollAlignment::Nearest,
    };
    let Some(argument) = args.first() else {
        return Ok(options);
    };
    if argument.is_boolean() {
        if !argument.to_boolean() {
            options.block = ScrollAlignment::End;
        }
        return Ok(options);
    }
    if argument.is_null() || argument.is_undefined() {
        return Ok(options);
    }
    let Some(dictionary) = argument.as_object() else {
        return Err(JsNativeError::typ()
            .with_message("scrollIntoView expects a boolean or options object")
            .into());
    };

    let behavior = dictionary.get(js_string!("behavior"), context)?;
    if !behavior.is_undefined()
        && !matches!(
            behavior
                .to_string(context)?
                .to_std_string_escaped()
                .as_str(),
            "auto" | "instant" | "smooth"
        )
    {
        return Err(JsNativeError::typ()
            .with_message("Invalid ScrollBehavior value")
            .into());
    }

    let block = dictionary.get(js_string!("block"), context)?;
    if !block.is_undefined() {
        options.block = scroll_alignment(&block, "block", context)?;
    }
    let inline = dictionary.get(js_string!("inline"), context)?;
    if !inline.is_undefined() {
        options.inline = scroll_alignment(&inline, "inline", context)?;
    }
    let container = dictionary.get(js_string!("container"), context)?;
    if !container.is_undefined()
        && !matches!(
            container
                .to_string(context)?
                .to_std_string_escaped()
                .as_str(),
            "all" | "nearest"
        )
    {
        return Err(JsNativeError::typ()
            .with_message("Invalid ScrollIntoViewContainer value")
            .into());
    }
    Ok(options)
}

fn aligned_scroll_position(
    current: f32,
    start: f32,
    end: f32,
    viewport: f32,
    alignment: ScrollAlignment,
) -> f32 {
    let current = f64::from(current);
    let start = f64::from(start);
    let end = f64::from(end);
    let viewport = f64::from(viewport);
    let target = match alignment {
        ScrollAlignment::Start => start,
        ScrollAlignment::Center => (start + end - viewport) / 2.0,
        ScrollAlignment::End => end - viewport,
        ScrollAlignment::Nearest => {
            let viewport_end = current + viewport;
            if (start >= current && end <= viewport_end) || (start < current && end > viewport_end)
            {
                current
            } else if start < current {
                start
            } else if end > viewport_end {
                end - viewport
            } else {
                current
            }
        }
    };
    target.clamp(f64::from(f32::MIN), f64::from(f32::MAX)) as f32
}

fn resolved_scroll_result(context: &mut Context) -> JsResult<JsValue> {
    let result = ObjectInitializer::new(context)
        .property(
            js_string!("interrupted"),
            JsValue::new(false),
            Attribute::WRITABLE | Attribute::ENUMERABLE | Attribute::CONFIGURABLE,
        )
        .build();
    Ok(JsPromise::resolve(result, context)?.into())
}

fn scroll_into_view(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM context unavailable"))?;
    let document = state.borrow().document.clone();
    if !matches!(document.borrow().get(node), Some(NodeData::Element(_))) {
        return Err(JsNativeError::typ()
            .with_message("scrollIntoView must be called on an Element")
            .into());
    }
    let options = scroll_into_view_options(args, context)?;
    let Some((snapshot, _)) = snapshot_for(context, "scroll_into_view_element")? else {
        return resolved_scroll_result(context);
    };
    let Some(rect) = union(&snapshot.border_boxes(node)) else {
        return resolved_scroll_result(context);
    };

    let host = host(context)?;
    let (viewport_width, viewport_height) = host.viewport_size();
    let (current_x, current_y) = host.scroll_position();
    let x = aligned_scroll_position(
        current_x,
        rect.x,
        rect.x + rect.width,
        viewport_width,
        options.inline,
    );
    let y = aligned_scroll_position(
        current_y,
        rect.y,
        rect.y + rect.height,
        viewport_height,
        options.block,
    );
    let range =
        rix_layout::viewport_scroll_range(snapshot.tree(), (viewport_width, viewport_height));
    host.request_scroll_position(
        x.clamp(range.min_x, range.max_x),
        y.clamp(range.min_y, range.max_y),
    );
    resolved_scroll_result(context)
}

fn element_from_point(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let receiver = get_node_id_from_this(this, context)?;
    let x = args.get_or_undefined(0).to_number(context)?;
    let y = args.get_or_undefined(1).to_number(context)?;
    if !x.is_finite() || !y.is_finite() {
        return Err(JsNativeError::typ()
            .with_message("elementFromPoint coordinates must be finite")
            .into());
    }

    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return Err(JsNativeError::typ()
            .with_message("elementFromPoint receiver is not a Document")
            .into());
    };
    let receiver_document = state.borrow().document.clone();
    if !matches!(
        receiver_document.borrow().get(receiver),
        Some(NodeData::Document)
    ) {
        return Err(JsNativeError::typ()
            .with_message("elementFromPoint receiver is not a Document")
            .into());
    }
    let Some((snapshot, document)) = snapshot_for(context, "elements_from_point")? else {
        return Ok(JsValue::null());
    };
    let host = host(context)?;
    let (width, height) = host.viewport_size();
    if x < 0.0 || y < 0.0 || x > width as f64 || y > height as f64 {
        return Ok(JsValue::null());
    }

    let document = document.borrow();
    let (scroll_x, scroll_y) = host.scroll_position();
    let target = rix_layout::hit_test::hit_test_tree_with_scroll(
        snapshot.tree(),
        Some(&document),
        x as f32,
        y as f32,
        scroll_x,
        scroll_y,
    )
    .and_then(|node| {
        let mut current = Some(node);
        while let Some(candidate) = current {
            if matches!(document.get(candidate), Some(NodeData::Element(_))) {
                return Some(candidate);
            }
            current = document.parent(candidate);
        }
        None
    })
    .or_else(|| {
        document
            .children(document.root())
            .find(|node| matches!(document.get(*node), Some(NodeData::Element(_))))
    });
    drop(document);
    Ok(target.map_or(JsValue::null(), |node| wrap_element(node, context).into()))
}

type NativeFn = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

fn define_on(
    context: &mut Context,
    target: &JsObject,
    name: &str,
    function: NativeFn,
    accessor: bool,
) -> JsResult<()> {
    let function =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(function))
            .name(js_string!(if accessor {
                format!("get {name}")
            } else {
                name.to_string()
            }))
            .length(0)
            .build();
    let descriptor = if accessor {
        PropertyDescriptor::builder()
            .get(function)
            .enumerable(true)
            .configurable(true)
            .build()
    } else {
        PropertyDescriptor::builder()
            .value(function)
            .writable(true)
            .configurable(true)
            .build()
    };
    target.define_property_or_throw(js_string!(name), descriptor, context)?;
    Ok(())
}

fn define_accessor_on(
    context: &mut Context,
    target: &JsObject,
    name: &str,
    getter: NativeFn,
    setter: Option<NativeFn>,
) -> JsResult<()> {
    let getter_fn =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(getter))
            .name(js_string!(format!("get {name}")))
            .length(0)
            .build();
    let mut builder = PropertyDescriptor::builder()
        .get(getter_fn)
        .enumerable(true)
        .configurable(true);
    if let Some(setter) = setter {
        let setter_fn =
            FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(setter))
                .name(js_string!(format!("set {name}")))
                .length(1)
                .build();
        builder = builder.set(setter_fn);
    }
    target.define_property_or_throw(js_string!(name), builder.build(), context)?;
    Ok(())
}

fn prototype_of(context: &mut Context, interface: &str) -> JsResult<JsObject> {
    let constructor = context
        .global_object()
        .get(js_string!(interface), context)?;
    constructor
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message(format!("{interface} missing")))?
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message(format!("{interface}.prototype missing"))
                .into()
        })
}

pub(super) fn register_geometry_interfaces(context: &mut Context) -> JsResult<()> {
    context.eval(boa_engine::Source::from_bytes(
        r#"
        (function() {
            const internals = new WeakMap();
            const slots = self => {
                const value = internals.get(self);
                if (!value) throw new TypeError('Illegal invocation');
                return value;
            };
            const number = value => value === undefined ? 0 : Number(value);
            class DOMRectReadOnly {
                constructor(x, y, width, height) {
                    internals.set(this, {x: number(x), y: number(y), width: number(width), height: number(height)});
                }
                static fromRect(other) {
                    other = other || {};
                    return new this(other.x, other.y, other.width, other.height);
                }
                get x() { return slots(this).x; }
                get y() { return slots(this).y; }
                get width() { return slots(this).width; }
                get height() { return slots(this).height; }
                get top() { const s = slots(this); return Math.min(s.y, s.y + s.height); }
                get bottom() { const s = slots(this); return Math.max(s.y, s.y + s.height); }
                get left() { const s = slots(this); return Math.min(s.x, s.x + s.width); }
                get right() { const s = slots(this); return Math.max(s.x, s.x + s.width); }
                toJSON() {
                    return {x: this.x, y: this.y, width: this.width, height: this.height,
                            top: this.top, right: this.right, bottom: this.bottom, left: this.left};
                }
            }
            class DOMRect extends DOMRectReadOnly {
                get x() { return slots(this).x; }
                set x(value) { slots(this).x = Number(value); }
                get y() { return slots(this).y; }
                set y(value) { slots(this).y = Number(value); }
                get width() { return slots(this).width; }
                set width(value) { slots(this).width = Number(value); }
                get height() { return slots(this).height; }
                set height(value) { slots(this).height = Number(value); }
            }
            const lengths = new WeakMap();
            function DOMRectList(rects) {
                if (new.target) throw new TypeError('Illegal constructor');
                const list = Object.create(DOMRectList.prototype);
                rects.forEach((rect, index) => Object.defineProperty(list, index, {value: rect, enumerable: true}));
                lengths.set(list, rects.length);
                return Object.freeze(list);
            }
            DOMRectList.prototype.item = function(index) {
                if (!lengths.has(this)) throw new TypeError('Illegal invocation');
                return this[index >>> 0] || null;
            };
            Object.defineProperty(DOMRectList.prototype, 'length', {get() {
                if (!lengths.has(this)) throw new TypeError('Illegal invocation');
                return lengths.get(this);
            }, configurable: true});
            DOMRectList.prototype[Symbol.iterator] = function* () {
                for (let i = 0; i < this.length; i++) yield this[i];
            };
            for (const [name, value] of [['DOMRectReadOnly', DOMRectReadOnly], ['DOMRect', DOMRect], ['DOMRectList', DOMRectList]]) {
                Object.defineProperty(value.prototype, Symbol.toStringTag, {value: name, configurable: true});
                Object.defineProperty(globalThis, name, {value, writable: true, configurable: true});
            }
        })();
        "#,
    ))?;
    Ok(())
}

pub fn register_layout_api(context: &mut Context, host: Rc<dyn LayoutHost>) -> JsResult<()> {
    context.insert_data(LayoutHostData(host));
    context.register_global_callable(
        js_string!("__rix_layout_revision"),
        0,
        NativeFunction::from_fn_ptr(layout_revision),
    )?;
    register_match_media(context)?;
    super::resize_observer::register(context)?;
    super::intersection_observer::register(context)?;
    context.eval(boa_engine::Source::from_bytes(
        "delete globalThis.__rix_layout_revision;",
    ))?;
    let element = prototype_of(context, "Element")?;
    define_on(
        context,
        &element,
        "getBoundingClientRect",
        get_bounding_client_rect,
        false,
    )?;
    define_on(context, &element, "getClientRects", get_client_rects, false)?;
    define_on(context, &element, "scrollIntoView", scroll_into_view, false)?;
    define_on(context, &element, "scroll", element_scroll_to, false)?;
    define_on(context, &element, "scrollTo", element_scroll_to, false)?;
    define_on(context, &element, "scrollBy", element_scroll_by, false)?;
    define_accessor_on(context, &element, "scrollLeft", scroll_left_get, Some(scroll_left_set))?;
    define_accessor_on(context, &element, "scrollTop", scroll_top_get, Some(scroll_top_set))?;
    for (name, function) in [
        ("clientWidth", client_width_get as NativeFn),
        ("clientHeight", client_height_get),
        ("clientTop", client_top_get),
        ("clientLeft", client_left_get),
        ("scrollWidth", scroll_width_get),
        ("scrollHeight", scroll_height_get),
    ] {
        define_on(context, &element, name, function, true)?;
    }
    let html_element = prototype_of(context, "HTMLElement")?;
    for (name, function) in [
        ("offsetParent", offset_parent_get as NativeFn),
        ("offsetTop", offset_top_get),
        ("offsetLeft", offset_left_get),
        ("offsetWidth", offset_width_get),
        ("offsetHeight", offset_height_get),
    ] {
        define_on(context, &html_element, name, function, true)?;
    }
    let document = prototype_of(context, "Document")?;
    define_on(
        context,
        &document,
        "elementFromPoint",
        element_from_point,
        false,
    )?;
    let window = prototype_of(context, "Window")?;
    for (name, function) in [
        ("innerWidth", inner_width_get as NativeFn),
        ("innerHeight", inner_height_get),
        ("devicePixelRatio", device_pixel_ratio_get),
        ("scrollX", scroll_x_get),
        ("scrollY", scroll_y_get),
        ("pageXOffset", scroll_x_get),
        ("pageYOffset", scroll_y_get),
    ] {
        define_on(context, &window, name, function, true)?;
    }
    define_on(context, &window, "scroll", scroll_to, false)?;
    define_on(context, &window, "scrollTo", scroll_to, false)?;
    define_on(context, &window, "scrollBy", scroll_by, false)?;
    Ok(())
}

fn layout_revision(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("DOM is unavailable"))?;
    let document = state.borrow().document.clone();
    let document = document.borrow();
    let (inputs_revision, scroll_revision) = host(context)?.rendering_revision();
    let mut revision = format!(
        "{}:{}:{}",
        document.revision(),
        document.style_revision(),
        inputs_revision
    );
    if args.get_or_undefined(0).as_boolean() == Some(true) {
        revision = format!("{revision}:{scroll_revision}");
    }
    Ok(js_string!(revision).into())
}

pub fn run_layout_observers(context: &mut Context) -> JsResult<()> {
    super::resize_observer::deliver(context)?;
    super::intersection_observer::deliver(context)
}

fn register_match_media(context: &mut Context) -> JsResult<()> {
    context.register_global_callable(
        js_string!("__rix_match_media_matches"),
        1,
        NativeFunction::from_fn_ptr(match_media_matches),
    )?;
    context.eval(boa_engine::Source::from_bytes(
        r#"
        (function(matchesQuery) {
            const slots = new WeakMap();
            const constructorToken = Symbol('MediaQueryList constructor token');
            function slot(value) {
                const state = slots.get(value);
                if (!state) throw new TypeError('Illegal MediaQueryList receiver');
                return state;
            }
            class MediaQueryList extends EventTarget {
                constructor(media, token) {
                    super();
                    if (token !== constructorToken) throw new TypeError('Illegal constructor: MediaQueryList');
                    const state = {media, onchange: null};
                    slots.set(this, state);
                }
                get media() { return slot(this).media; }
                get matches() { const media = slot(this).media; return media !== '' && matchesQuery(media); }
                get onchange() { return slot(this).onchange; }
                set onchange(callback) {
                    if (callback !== null && callback !== undefined && typeof callback !== 'function') {
                        throw new TypeError('MediaQueryList.onchange must be a function or null');
                    }
                    slot(this).onchange = callback == null ? null : callback;
                }
                addListener(listener) { this.addEventListener('change', listener); }
                removeListener(listener) { this.removeEventListener('change', listener); }
            }
            class MediaQueryListEvent extends Event {
                constructor(type, init = {}) {
                    super(type, init);
                    this.media = init.media === undefined ? '' : String(init.media);
                    this.matches = !!init.matches;
                }
            }
            Object.defineProperty(MediaQueryList.prototype, Symbol.toStringTag, {value: 'MediaQueryList', configurable: true});
            Object.defineProperty(MediaQueryListEvent.prototype, Symbol.toStringTag, {value: 'MediaQueryListEvent', configurable: true});
            Object.defineProperty(Window.prototype, 'matchMedia', {
                value: function(query) {
                    if (!(this instanceof Window)) throw new TypeError('Window.matchMedia called on an incompatible receiver');
                    const media = String(query).trim();
                    return new MediaQueryList(media, constructorToken);
                }, writable: true, configurable: true,
            });
            Object.defineProperty(globalThis, 'MediaQueryList', {value: MediaQueryList, configurable: true, writable: true});
            Object.defineProperty(globalThis, 'MediaQueryListEvent', {value: MediaQueryListEvent, configurable: true, writable: true});
        })(__rix_match_media_matches);
        delete globalThis.__rix_match_media_matches;
        "#,
    ))?;
    Ok(())
}

fn match_media_matches(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let query = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    if query.trim().is_empty() {
        return Ok(JsValue::new(false));
    }
    let (width, height) = host(context)?.viewport_size();
    Ok(JsValue::new(rix_css::matches_media_query(
        &query, width, height,
    )))
}
