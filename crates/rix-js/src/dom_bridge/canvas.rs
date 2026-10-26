
use super::*;
use boa_engine::object::builtins::JsUint8ClampedArray;
use rix_css::CssColor;
use rix_dom::CanvasBitmap;

const MAX_CANVAS_BITMAP_BYTES: usize = 16 * 1024 * 1024;
const MAX_CANVAS_BYTES_PER_DOCUMENT: usize = 16 * 1024 * 1024;

pub struct CanvasContextState {
    context: JsObject,
    width: u32,
    height: u32,
    fill_style: CssColor,
    alpha: bool,
}

#[derive(Clone)]
struct CanvasApiIntrinsics {
    context_prototype: JsObject,
    image_data_constructor: JsObject,
}

fn canvas_context_intrinsics(context: &mut Context) -> JsResult<CanvasApiIntrinsics> {
    context
        .get_data::<CanvasApiIntrinsics>()
        .cloned()
        .ok_or_else(|| {
            JsNativeError::error()
                .with_message("Canvas API is not initialized")
                .into()
        })
}

fn define_native_method(
    context: &mut Context,
    interface: &str,
    name: &str,
    length: usize,
    function: fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
) -> JsResult<()> {
    let constructor = context
        .global_object()
        .get(js_string!(interface), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message(format!("{interface} is unavailable"))
        })?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message(format!("{interface} prototype is unavailable"))
        })?;
    let function =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(function))
            .name(js_string!(name))
            .length(length)
            .build();
    prototype.define_property_or_throw(
        js_string!(name),
        PropertyDescriptor::builder()
            .value(function)
            .writable(true)
            .configurable(true)
            .build(),
        context,
    )?;
    Ok(())
}

pub(super) fn register_canvas_api(context: &mut Context) -> JsResult<()> {
    context.eval(boa_engine::Source::from_bytes(
        r#"
        (function() {
            if (!globalThis.CanvasRenderingContext2D) {
                function CanvasRenderingContext2D() {
                    throw new TypeError('Illegal constructor: CanvasRenderingContext2D');
                }
                Object.defineProperty(CanvasRenderingContext2D, 'name', {value: 'CanvasRenderingContext2D'});
                CanvasRenderingContext2D.prototype = Object.create(Object.prototype, {
                    constructor: {value: CanvasRenderingContext2D, writable: true, configurable: true},
                    [Symbol.toStringTag]: {value: 'CanvasRenderingContext2D', configurable: true},
                });
                globalThis.CanvasRenderingContext2D = CanvasRenderingContext2D;
            }

            if (!globalThis.ImageData) {
                const slots = new WeakMap();
                function imageDataState(value) {
                    const state = slots.get(value);
                    if (!state) throw new TypeError('Illegal ImageData receiver');
                    return state;
                }
                function dimension(value) {
                    const number = Number(value);
                    if (!Number.isFinite(number) || number < 0 || number > 0xffffffff) {
                        throw new TypeError('ImageData dimensions must be unsigned long values');
                    }
                    return Math.trunc(number);
                }
                function indexSizeError() {
                    throw new DOMException('The requested ImageData dimensions are invalid', 'IndexSizeError');
                }
                function checkBudget(width, height) {
                    if (width * height > 4194304) {
                        throw new RangeError('ImageData exceeds the configured bitmap budget');
                    }
                }
                class ImageData {
                    constructor(dataOrWidth, widthOrHeight, optionalHeight) {
                        if (!new.target) throw new TypeError('ImageData requires new');
                        let data;
                        let width;
                        let height;
                        if (typeof dataOrWidth === 'number') {
                            width = dimension(dataOrWidth);
                            height = widthOrHeight === undefined ? 1 : dimension(widthOrHeight);
                            if (width === 0 || height === 0) indexSizeError();
                            checkBudget(width, height);
                            data = new Uint8ClampedArray(width * height * 4);
                        } else if (dataOrWidth instanceof Uint8ClampedArray) {
                            data = dataOrWidth;
                            width = dimension(widthOrHeight);
                            if (width === 0) indexSizeError();
                            const rowBytes = width * 4;
                            if (optionalHeight === undefined) {
                                if (data.length % rowBytes !== 0) indexSizeError();
                                height = data.length / rowBytes;
                            } else {
                                height = dimension(optionalHeight);
                            }
                            if (height === 0 || data.length !== width * height * 4) indexSizeError();
                            checkBudget(width, height);
                        } else {
                            throw new TypeError('ImageData requires dimensions or a Uint8ClampedArray');
                        }
                        slots.set(this, {data, width, height});
                    }
                    get data() { return imageDataState(this).data; }
                    get width() { return imageDataState(this).width; }
                    get height() { return imageDataState(this).height; }
                    get colorSpace() { imageDataState(this); return 'srgb'; }
                }
                Object.defineProperty(ImageData.prototype, Symbol.toStringTag, {value: 'ImageData', configurable: true});
                globalThis.ImageData = ImageData;
            }

        })();
        "#,
    ))?;

    define_native_method(
        context,
        "HTMLCanvasElement",
        "getContext",
        1,
        canvas_get_context,
    )?;
    define_accessor(
        context,
        "HTMLCanvasElement",
        "width",
        Some(("get width", 0, canvas_get_width)),
        Some(("set width", 1, canvas_set_width)),
    )?;
    define_accessor(
        context,
        "HTMLCanvasElement",
        "height",
        Some(("get height", 0, canvas_get_height)),
        Some(("set height", 1, canvas_set_height)),
    )?;
    define_accessor(
        context,
        "CanvasRenderingContext2D",
        "canvas",
        Some(("get canvas", 0, canvas_context_get_canvas)),
        None,
    )?;
    define_accessor(
        context,
        "CanvasRenderingContext2D",
        "fillStyle",
        Some(("get fillStyle", 0, canvas_context_get_fill_style)),
        Some(("set fillStyle", 1, canvas_context_set_fill_style)),
    )?;
    let context_methods: [(
        &str,
        fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
    ); 3] = [
        ("fillRect", canvas_context_fill_rect),
        ("clearRect", canvas_context_clear_rect),
        ("getImageData", canvas_context_get_image_data),
    ];
    for (name, function) in context_methods {
        let length = match name {
            "fillRect" | "clearRect" => 4,
            "getImageData" => 4,
            _ => 0,
        };
        define_native_method(context, "CanvasRenderingContext2D", name, length, function)?;
    }

    let context_constructor = context
        .global_object()
        .get(js_string!("CanvasRenderingContext2D"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("CanvasRenderingContext2D is unavailable")
        })?;
    let context_prototype = context_constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("CanvasRenderingContext2D prototype is unavailable")
        })?;
    let image_data_constructor = context
        .global_object()
        .get(js_string!("ImageData"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("ImageData is unavailable"))?;
    context.insert_data(CanvasApiIntrinsics {
        context_prototype,
        image_data_constructor,
    });
    Ok(())
}

fn define_accessor(
    context: &mut Context,
    interface: &str,
    property: &str,
    getter: Option<(
        &'static str,
        usize,
        fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
    )>,
    setter: Option<(
        &'static str,
        usize,
        fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
    )>,
) -> JsResult<()> {
    let constructor = context
        .global_object()
        .get(js_string!(interface), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message(format!("{interface} is unavailable"))
        })?;
    let prototype = constructor
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message(format!("{interface} prototype is unavailable"))
        })?;
    let make_function = |context: &mut Context,
                         (name, length, function): (
        &'static str,
        usize,
        fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
    )| {
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(function))
            .name(js_string!(name))
            .length(length)
            .build()
    };
    let mut descriptor = PropertyDescriptor::builder()
        .enumerable(true)
        .configurable(true);
    if let Some(getter) = getter {
        descriptor = descriptor.get(make_function(context, getter));
    }
    if let Some(setter) = setter {
        descriptor = descriptor.set(make_function(context, setter));
    }
    prototype.define_property_or_throw(js_string!(property), descriptor.build(), context)?;
    Ok(())
}

fn is_html_canvas(state: &DomBridgeState, node_id: NodeId) -> bool {
    matches!(
        state.document.borrow().get(node_id),
        Some(NodeData::Element(element))
            if &*element.name.ns == "http://www.w3.org/1999/xhtml" && element.name.local.as_str() == "canvas"
    )
}

fn unsigned_dimension(value: Option<&str>, default: u32) -> u32 {
    value
        .and_then(rix_dom::node::parse_non_negative_integer)
        .unwrap_or(default)
}

fn canvas_dimensions(state: &DomBridgeState, node_id: NodeId) -> JsResult<(u32, u32)> {
    let document = state.document.borrow();
    let Some(NodeData::Element(element)) = document.get(node_id) else {
        return Err(JsNativeError::typ()
            .with_message("Canvas methods require an HTMLCanvasElement receiver")
            .into());
    };
    if &*element.name.ns != "http://www.w3.org/1999/xhtml"
        || element.name.local.as_str() != "canvas"
    {
        return Err(JsNativeError::typ()
            .with_message("Canvas methods require an HTMLCanvasElement receiver")
            .into());
    }
    Ok((
        unsigned_dimension(element.get_attribute("width"), 300),
        unsigned_dimension(element.get_attribute("height"), 150),
    ))
}

fn blank_bitmap(width: u32, height: u32, alpha: bool) -> CanvasBitmap {
    let pixels = rgba_buffer_len(width, height)
        .filter(|bytes| *bytes <= MAX_CANVAS_BITMAP_BYTES)
        .map(|bytes| {
            let mut rgba = vec![0; bytes];
            if !alpha {
                for pixel in rgba.chunks_exact_mut(4) {
                    pixel[3] = 255;
                }
            }
            rgba
        });
    CanvasBitmap {
        width,
        height,
        rgba: pixels,
    }
}

fn rgba_buffer_len(width: u32, height: u32) -> Option<usize> {
    usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
}

fn bitmap_byte_len(bitmap: Option<&CanvasBitmap>) -> usize {
    bitmap
        .and_then(|bitmap| bitmap.rgba.as_ref())
        .map_or(0, Vec::len)
}

fn set_canvas_bitmap(
    state: &Rc<RefCell<DomBridgeState>>,
    node_id: NodeId,
    mut bitmap: CanvasBitmap,
    allow_allocation_failure: bool,
) -> bool {
    let (document, current_bytes, old_bytes) = {
        let state_ref = state.borrow();
        let document = state_ref.document.clone();
        let document_ref = document.borrow();
        (
            document.clone(),
            state_ref.canvas_bitmap_bytes,
            bitmap_byte_len(document_ref.canvas_bitmap(node_id)),
        )
    };
    let new_bytes = bitmap_byte_len(Some(&bitmap));
    let available = current_bytes.saturating_sub(old_bytes);
    if new_bytes > MAX_CANVAS_BYTES_PER_DOCUMENT.saturating_sub(available) {
        if !allow_allocation_failure {
            return false;
        }
        bitmap.rgba = None;
    }
    let new_bytes = bitmap_byte_len(Some(&bitmap));
    let changed = document
        .borrow_mut()
        .set_canvas_bitmap(node_id, Some(bitmap));
    if changed {
        state.borrow_mut().canvas_bitmap_bytes = available.saturating_add(new_bytes);
    }
    true
}

fn reset_context_for_node(
    state: &Rc<RefCell<DomBridgeState>>,
    node_id: NodeId,
    context: &mut Context,
) -> JsResult<()> {
    let (width, height) = canvas_dimensions(&state.borrow(), node_id)?;
    let alpha = state
        .borrow()
        .canvas_contexts
        .get(&node_id)
        .map_or(true, |canvas| canvas.alpha);
    if let Some(canvas) = state.borrow_mut().canvas_contexts.get_mut(&node_id) {
        canvas.width = width;
        canvas.height = height;
        canvas.fill_style = CssColor::BLACK;
    }
    set_canvas_bitmap(state, node_id, blank_bitmap(width, height, alpha), true);
    let _ = context;
    Ok(())
}

pub(super) fn canvas_dimension_attribute_mutated(
    context: &mut Context,
    node_id: NodeId,
    namespace: &str,
    local_name: &str,
) -> JsResult<()> {
    if !namespace.is_empty() || !matches!(local_name, "width" | "height") {
        return Ok(());
    }
    let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() else {
        return Ok(());
    };
    if !is_html_canvas(&state.borrow(), node_id)
        || !state.borrow().canvas_contexts.contains_key(&node_id)
    {
        return Ok(());
    }
    reset_context_for_node(&state, node_id, context)
}

fn canvas_get_width(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("Canvas DOM state is unavailable"))?;
    let (width, _) = canvas_dimensions(&state.borrow(), node_id)?;
    Ok(JsValue::new(width))
}

fn canvas_get_height(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("Canvas DOM state is unavailable"))?;
    let (_, height) = canvas_dimensions(&state.borrow(), node_id)?;
    Ok(JsValue::new(height))
}

fn webidl_unsigned_long(value: &JsValue, context: &mut Context) -> JsResult<u32> {
    let number = value.to_number(context)?;
    if !number.is_finite() || number == 0.0 {
        return Ok(0);
    }
    let modulo = number.trunc().rem_euclid(4_294_967_296.0);
    Ok(modulo as u32)
}

fn canvas_set_dimension(
    this: &JsValue,
    value: &JsValue,
    name: &'static str,
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("Canvas DOM state is unavailable"))?;
    canvas_dimensions(&state.borrow(), node_id)?;
    let dimension = webidl_unsigned_long(value, context)?;
    super::element::element_set_attribute(
        this,
        &[
            js_string!(name).into(),
            js_string!(dimension.to_string()).into(),
        ],
        context,
    )
}

fn canvas_set_width(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    canvas_set_dimension(this, args.get_or_undefined(0), "width", context)
}

fn canvas_set_height(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    canvas_set_dimension(this, args.get_or_undefined(0), "height", context)
}

fn canvas_get_context(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let context_id = dom_string(args.get_or_undefined(0), context)?;
    if context_id != "2d" {
        return Ok(JsValue::null());
    }
    let node_id = get_node_id_from_this(this, context)?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("Canvas DOM state is unavailable"))?;
    if !is_html_canvas(&state.borrow(), node_id) {
        return Err(JsNativeError::typ()
            .with_message("getContext requires an HTMLCanvasElement receiver")
            .into());
    }
    if let Some(existing) = state
        .borrow()
        .canvas_contexts
        .get(&node_id)
        .map(|context| context.context.clone())
    {
        ensure_current_context_size(&state, node_id, context)?;
        return Ok(existing.into());
    }

    let (width, height) = canvas_dimensions(&state.borrow(), node_id)?;
    let options = args.get_or_undefined(1);
    let alpha = options
        .as_object()
        .map(|options| options.get(js_string!("alpha"), context))
        .transpose()?
        .map_or(true, |value| value.to_boolean());
    let bitmap = blank_bitmap(width, height, alpha);
    if bitmap.rgba.is_none() {
        return Ok(JsValue::null());
    }
    if !set_canvas_bitmap(&state, node_id, bitmap, false) {
        return Ok(JsValue::null());
    }
    let intrinsics = canvas_context_intrinsics(context)?;
    let mut context_builder = ObjectInitializer::new(context);
    let canvas_context = context_builder.build();
    if !canvas_context.set_prototype(Some(intrinsics.context_prototype)) {
        return Err(JsNativeError::error()
            .with_message("Could not initialize CanvasRenderingContext2D prototype")
            .into());
    }
    state.borrow_mut().canvas_contexts.insert(
        node_id,
        CanvasContextState {
            context: canvas_context.clone(),
            width,
            height,
            fill_style: CssColor::BLACK,
            alpha,
        },
    );
    Ok(canvas_context.into())
}

fn context_canvas_node(
    this: &JsValue,
    context: &mut Context,
) -> JsResult<(Rc<RefCell<DomBridgeState>>, NodeId)> {
    let object = this.as_object().ok_or_else(|| {
        JsNativeError::typ()
            .with_message("CanvasRenderingContext2D method requires a context receiver")
    })?;
    let state = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("Canvas DOM state is unavailable"))?;
    let node_id = state
        .borrow()
        .canvas_contexts
        .iter()
        .find(|(_, entry)| JsObject::equals(&entry.context, &object))
        .map(|(node_id, _)| *node_id)
        .ok_or_else(|| {
            JsNativeError::typ().with_message("Illegal CanvasRenderingContext2D receiver")
        })?;
    ensure_current_context_size(&state, node_id, context)?;
    Ok((state, node_id))
}

fn ensure_current_context_size(
    state: &Rc<RefCell<DomBridgeState>>,
    node_id: NodeId,
    context: &mut Context,
) -> JsResult<()> {
    let (width, height) = canvas_dimensions(&state.borrow(), node_id)?;
    let differs = state
        .borrow()
        .canvas_contexts
        .get(&node_id)
        .is_some_and(|canvas| canvas.width != width || canvas.height != height);
    if differs {
        reset_context_for_node(state, node_id, context)?;
    }
    Ok(())
}

fn canvas_context_get_canvas(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (_state, node_id) = context_canvas_node(this, context)?;
    Ok(wrap_element(node_id, context).into())
}

fn canvas_context_get_fill_style(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (state, node_id) = context_canvas_node(this, context)?;
    let fill_style = state
        .borrow()
        .canvas_contexts
        .get(&node_id)
        .map(|canvas| canvas.fill_style)
        .ok_or_else(|| {
            JsNativeError::typ().with_message("Illegal CanvasRenderingContext2D receiver")
        })?;
    Ok(js_string!(serialize_canvas_color(fill_style)).into())
}

fn canvas_context_set_fill_style(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (state, node_id) = context_canvas_node(this, context)?;
    let value = dom_string(args.get_or_undefined(0), context)?;
    if let Some(color) = CssColor::parse(&value) {
        if let Some(canvas) = state.borrow_mut().canvas_contexts.get_mut(&node_id) {
            canvas.fill_style = color;
        }
    }
    Ok(JsValue::undefined())
}

fn serialize_canvas_color(color: CssColor) -> String {
    let (red, green, blue, _) = color.to_rgba_u8();
    if color.a >= 1.0 {
        format!("rgb({red}, {green}, {blue})")
    } else {
        let mut alpha = format!("{:.6}", color.a.clamp(0.0, 1.0));
        while alpha.ends_with('0') {
            alpha.pop();
        }
        if alpha.ends_with('.') {
            alpha.pop();
        }
        format!("rgba({red}, {green}, {blue}, {alpha})")
    }
}

fn dom_string(value: &JsValue, context: &mut Context) -> JsResult<String> {
    if value.as_symbol().is_some() {
        return Err(JsNativeError::typ()
            .with_message("Cannot convert a Symbol value to a string")
            .into());
    }
    Ok(value.to_string(context)?.to_std_string_escaped())
}

fn context_fill_style(
    state: &Rc<RefCell<DomBridgeState>>,
    node_id: NodeId,
) -> JsResult<(CssColor, bool)> {
    state
        .borrow()
        .canvas_contexts
        .get(&node_id)
        .map(|canvas| (canvas.fill_style, canvas.alpha))
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message("Illegal CanvasRenderingContext2D receiver")
                .into()
        })
}

fn rect_arguments(
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<Option<(f64, f64, f64, f64)>> {
    let x = args.get_or_undefined(0).to_number(context)?;
    let y = args.get_or_undefined(1).to_number(context)?;
    let width = args.get_or_undefined(2).to_number(context)?;
    let height = args.get_or_undefined(3).to_number(context)?;
    let right = x + width;
    let bottom = y + height;
    if !x.is_finite() || !y.is_finite() || !right.is_finite() || !bottom.is_finite() {
        return Ok(None);
    }
    if width == 0.0 || height == 0.0 {
        return Ok(None);
    }
    let left = x.min(right);
    let top = y.min(bottom);
    let rect_width = x.max(right) - left;
    let rect_height = y.max(bottom) - top;
    if rect_width <= 0.0 || rect_height <= 0.0 {
        return Ok(None);
    }
    Ok(Some((left, top, rect_width, rect_height)))
}

fn canvas_context_fill_rect(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (state, node_id) = context_canvas_node(this, context)?;
    let Some((left, top, width, height)) = rect_arguments(args, context)? else {
        return Ok(JsValue::undefined());
    };
    let (color, alpha) = context_fill_style(&state, node_id)?;
    let color = color.to_rgba_u8();
    let document = state.borrow().document.clone();
    document
        .borrow_mut()
        .mutate_canvas_bitmap(node_id, |bitmap| {
            let Some(pixels) = bitmap.rgba.as_mut() else {
                return false;
            };
            paint_rect(
                bitmap.width,
                bitmap.height,
                pixels,
                (left, top, width, height),
                color,
                alpha,
            )
        });
    Ok(JsValue::undefined())
}

fn canvas_context_clear_rect(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (state, node_id) = context_canvas_node(this, context)?;
    let Some((left, top, width, height)) = rect_arguments(args, context)? else {
        return Ok(JsValue::undefined());
    };
    let (_, alpha) = context_fill_style(&state, node_id)?;
    let document = state.borrow().document.clone();
    document
        .borrow_mut()
        .mutate_canvas_bitmap(node_id, |bitmap| {
            let Some(pixels) = bitmap.rgba.as_mut() else {
                return false;
            };
            clear_rect(
                bitmap.width,
                bitmap.height,
                pixels,
                (left, top, width, height),
                alpha,
            )
        });
    Ok(JsValue::undefined())
}

fn pixel_bounds(
    bitmap_width: u32,
    bitmap_height: u32,
    rect: (f64, f64, f64, f64),
) -> (u32, u32, u32, u32) {
    let (x, y, width, height) = rect;
    let right = x + width;
    let bottom = y + height;
    let x0 = x.floor().max(0.0).min(f64::from(bitmap_width)) as u32;
    let y0 = y.floor().max(0.0).min(f64::from(bitmap_height)) as u32;
    let x1 = right.ceil().max(0.0).min(f64::from(bitmap_width)) as u32;
    let y1 = bottom.ceil().max(0.0).min(f64::from(bitmap_height)) as u32;
    (x0, y0, x1, y1)
}

fn paint_rect(
    bitmap_width: u32,
    bitmap_height: u32,
    pixels: &mut [u8],
    rect: (f64, f64, f64, f64),
    source: (u8, u8, u8, u8),
    _alpha: bool,
) -> bool {
    if rgba_buffer_len(bitmap_width, bitmap_height) != Some(pixels.len()) {
        return false;
    }
    let (left, top, width, height) = rect;
    let right = left + width;
    let bottom = top + height;
    let (x0, y0, x1, y1) = pixel_bounds(bitmap_width, bitmap_height, rect);
    let mut changed = false;
    for y in y0..y1 {
        let coverage_y = (bottom.min(f64::from(y + 1)) - top.max(f64::from(y))).clamp(0.0, 1.0);
        for x in x0..x1 {
            let coverage_x = (right.min(f64::from(x + 1)) - left.max(f64::from(x))).clamp(0.0, 1.0);
            let coverage = coverage_x * coverage_y;
            if coverage <= 0.0 {
                continue;
            }
            let offset = ((y as usize * bitmap_width as usize) + x as usize) * 4;
            let previous = [
                pixels[offset],
                pixels[offset + 1],
                pixels[offset + 2],
                pixels[offset + 3],
            ];
            let source_alpha = f64::from(source.3) / 255.0 * coverage;
            let dest_alpha = f64::from(previous[3]) / 255.0;
            let out_alpha = source_alpha + dest_alpha * (1.0 - source_alpha);
            let mut next = [0; 4];
            if out_alpha > 0.0 {
                for channel in 0..3 {
                    let source_channel = f64::from([source.0, source.1, source.2][channel]) / 255.0;
                    let dest_channel = f64::from(previous[channel]) / 255.0;
                    let out = (source_channel * source_alpha
                        + dest_channel * dest_alpha * (1.0 - source_alpha))
                        / out_alpha;
                    next[channel] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
                }
                next[3] = (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8;
            }
            if previous != next {
                pixels[offset..offset + 4].copy_from_slice(&next);
                changed = true;
            }
        }
    }
    changed
}

fn clear_rect(
    bitmap_width: u32,
    bitmap_height: u32,
    pixels: &mut [u8],
    rect: (f64, f64, f64, f64),
    alpha: bool,
) -> bool {
    if rgba_buffer_len(bitmap_width, bitmap_height) != Some(pixels.len()) {
        return false;
    }
    let (left, top, width, height) = rect;
    let right = left + width;
    let bottom = top + height;
    let (x0, y0, x1, y1) = pixel_bounds(bitmap_width, bitmap_height, rect);
    let mut changed = false;
    for y in y0..y1 {
        let coverage_y = (bottom.min(f64::from(y + 1)) - top.max(f64::from(y))).clamp(0.0, 1.0);
        for x in x0..x1 {
            let coverage_x = (right.min(f64::from(x + 1)) - left.max(f64::from(x))).clamp(0.0, 1.0);
            let coverage = coverage_x * coverage_y;
            if coverage <= 0.0 {
                continue;
            }
            let offset = ((y as usize * bitmap_width as usize) + x as usize) * 4;
            let previous = [
                pixels[offset],
                pixels[offset + 1],
                pixels[offset + 2],
                pixels[offset + 3],
            ];
            let mut next = [0; 4];
            if alpha {
                next[..3].copy_from_slice(&previous[..3]);
                next[3] = (f64::from(previous[3]) * (1.0 - coverage))
                    .round()
                    .clamp(0.0, 255.0) as u8;
                if next[3] == 0 {
                    next[..3].fill(0);
                }
            } else {
                let remain = 1.0 - coverage;
                for channel in 0..3 {
                    next[channel] = (f64::from(previous[channel]) * remain)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
                next[3] = 255;
            }
            if previous != next {
                pixels[offset..offset + 4].copy_from_slice(&next);
                changed = true;
            }
        }
    }
    changed
}

fn webidl_long(value: &JsValue, context: &mut Context) -> JsResult<i32> {
    let number = value.to_number(context)?;
    if !number.is_finite() || number == 0.0 {
        return Ok(0);
    }
    let modulo = number.trunc().rem_euclid(4_294_967_296.0) as u32;
    Ok(modulo as i32)
}

fn canvas_context_get_image_data(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let (state, node_id) = context_canvas_node(this, context)?;
    if let Some(options) = args.get(4).and_then(JsValue::as_object) {
        for (name, expected) in [("colorSpace", "srgb"), ("pixelFormat", "rgba-unorm8")] {
            let value = options.get(js_string!(name), context)?;
            if !value.is_undefined() && dom_string(&value, context)? != expected {
                return Err(dom_exception(
                    "NotSupportedError",
                    "Unsupported ImageData format",
                    context,
                )?);
            }
        }
    }
    let sx = webidl_long(args.get_or_undefined(0), context)?;
    let sy = webidl_long(args.get_or_undefined(1), context)?;
    let sw = webidl_long(args.get_or_undefined(2), context)?;
    let sh = webidl_long(args.get_or_undefined(3), context)?;
    if sw == 0 || sh == 0 {
        return Err(dom_exception(
            "IndexSizeError",
            "ImageData width and height must be non-zero",
            context,
        )?);
    }
    let x = if sw < 0 {
        i64::from(sx) + i64::from(sw)
    } else {
        i64::from(sx)
    };
    let y = if sh < 0 {
        i64::from(sy) + i64::from(sh)
    } else {
        i64::from(sy)
    };
    let width = sw.unsigned_abs();
    let height = sh.unsigned_abs();
    let length = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes <= MAX_CANVAS_BITMAP_BYTES)
        .ok_or_else(|| {
            JsNativeError::range().with_message("ImageData exceeds the configured bitmap budget")
        })?;
    let mut output = vec![0; length];
    let document = state.borrow().document.clone();
    if let Some(bitmap) = document.borrow().canvas_bitmap(node_id) {
        if let Some(source) = bitmap.rgba.as_ref() {
            if rgba_buffer_len(bitmap.width, bitmap.height) == Some(source.len()) {
                for output_y in 0..height {
                    let source_y = y + i64::from(output_y);
                    if source_y < 0 || source_y >= i64::from(bitmap.height) {
                        continue;
                    }
                    for output_x in 0..width {
                        let source_x = x + i64::from(output_x);
                        if source_x < 0 || source_x >= i64::from(bitmap.width) {
                            continue;
                        }
                        let source_offset =
                            (source_y as usize * bitmap.width as usize + source_x as usize) * 4;
                        let output_offset =
                            (output_y as usize * width as usize + output_x as usize) * 4;
                        output[output_offset..output_offset + 4]
                            .copy_from_slice(&source[source_offset..source_offset + 4]);
                    }
                }
            }
        }
    }
    let data: JsValue = JsUint8ClampedArray::from_iter(output, context)?.into();
    let constructor = canvas_context_intrinsics(context)?.image_data_constructor;
    let image_data = constructor.construct(
        &[data, JsValue::new(width), JsValue::new(height)],
        None,
        context,
    )?;
    Ok(image_data.into())
}

fn dom_exception(name: &str, message: &str, context: &mut Context) -> JsResult<JsError> {
    let constructor = context
        .global_object()
        .get(js_string!("DOMException"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::error().with_message("DOMException constructor is unavailable")
        })?;
    let exception = constructor.construct(
        &[js_string!(message).into(), js_string!(name).into()],
        None,
        context,
    )?;
    Ok(JsError::from_opaque(exception.into()))
}
