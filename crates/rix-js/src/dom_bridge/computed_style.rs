
use super::layout::{host, snapshot_for, LayoutSnapshot};
use super::*;
use rix_css::color::CssColor;
use rix_css::{ComputedStyle, Display};

const PROPERTIES: &[&str] = &[
    "align-content",
    "background-color",
    "border-bottom-color",
    "border-bottom-width",
    "border-left-color",
    "border-left-width",
    "border-right-color",
    "border-right-width",
    "border-top-color",
    "border-top-width",
    "bottom",
    "box-sizing",
    "color",
    "column-gap",
    "display",
    "flex-basis",
    "flex-direction",
    "flex-grow",
    "flex-shrink",
    "flex-wrap",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "height",
    "left",
    "line-height",
    "margin-bottom",
    "margin-left",
    "margin-right",
    "margin-top",
    "max-height",
    "max-width",
    "min-height",
    "min-width",
    "opacity",
    "order",
    "overflow-x",
    "overflow-y",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "padding-top",
    "pointer-events",
    "position",
    "right",
    "row-gap",
    "text-align",
    "text-decoration-line",
    "text-overflow",
    "top",
    "vertical-align",
    "visibility",
    "white-space",
    "width",
    "z-index",
    "border-radius",
    "margin",
    "overflow",
    "padding",
];

fn number(value: f32) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded == 0.0 {
        return "0".into();
    }
    format!("{rounded}")
}

fn px(value: f32) -> String {
    format!("{}px", number(value))
}

fn color(value: CssColor) -> String {
    let (r, g, b, a) = value.to_rgba_u8();
    if a == 255 {
        format!("rgb({r}, {g}, {b})")
    } else {
        format!("rgba({r}, {g}, {b}, {})", alpha(a))
    }
}

fn alpha(byte: u8) -> String {
    let exact = f32::from(byte) / 255.0;
    let two = (exact * 100.0).round() / 100.0;
    if (two * 255.0).round() as u8 == byte {
        return number(two);
    }
    number((exact * 1000.0).round() / 1000.0)
}

fn length_or(value: Option<f32>, percent: Option<f32>, fallback: &str) -> String {
    match (value, percent) {
        (Some(value), _) => px(value),
        (None, Some(percent)) => format!("{}%", number(percent)),
        (None, None) => fallback.into(),
    }
}

fn display(value: Display) -> &'static str {
    match value {
        Display::None => "none",
        Display::Block => "block",
        Display::Inline => "inline",
        Display::InlineBlock => "inline-block",
        Display::Flex => "flex",
        Display::InlineFlex => "inline-flex",
        Display::Grid => "grid",
        Display::InlineGrid => "inline-grid",
        Display::Table => "table",
        Display::TableRow => "table-row",
        Display::TableCell => "table-cell",
        Display::ListItem => "list-item",
    }
}

fn serialize_shorthand_four(values: &[String]) -> String {
    if values.len() != 4 {
        return values.join(" ");
    }
    let t = &values[0];
    let r = &values[1];
    let b = &values[2];
    let l = &values[3];
    if t == b && r == l {
        if t == r {
            t.clone()
        } else {
            format!("{t} {r}")
        }
    } else if r == l {
        format!("{t} {r} {b}")
    } else {
        format!("{t} {r} {b} {l}")
    }
}

fn resolved_value(
    snapshot: &LayoutSnapshot,
    node: NodeId,
    style: &ComputedStyle,
    parent_style: Option<&ComputedStyle>,
    property: &str,
) -> Option<String> {
    if let Some(custom) = property.strip_prefix("--") {
        return Some(
            style
                .variables
                .get(&format!("--{custom}"))
                .cloned()
                .unwrap_or_default(),
        );
    }
    let rendered = snapshot.boxes(node).next();
    let used = |pick: fn(&rix_layout::tree::LayoutBoxData) -> f32| rendered.map(pick);
    let border_box = style.box_sizing == "border-box";
    Some(match property {
        "display" => display(style.display).into(),
        "position" => style.position.clone(),
        "visibility" => style.visibility.clone(),
        "overflow-x" => style.overflow_x.clone(),
        "overflow-y" => style.overflow_y.clone(),
        "white-space" => style.white_space.clone(),
        "text-align" => style.text_align.clone(),
        "vertical-align" => style.vertical_align.clone(),
        "align-content" => style.align_content.clone(),
        "text-overflow" => style.text_overflow.clone(),
        "text-decoration-line" => style.text_decoration_line.clone(),
        "box-sizing" => style.box_sizing.clone(),
        "flex-direction" => style.flex_direction.clone(),
        "flex-wrap" => style.flex_wrap.clone(),
        "pointer-events" => style.pointer_events.clone(),
        "font-style" => style.font_style.clone(),
        "font-family" => style.font_family.clone(),
        "font-size" => px(style.font_size),
        "font-weight" => style.font_weight.to_string(),
        "line-height" if style.line_height_normal => "normal".into(),
        "line-height" => px(style.line_height),
        "color" => color(style.color),
        "background-color" => color(style.background_color),
        "border-top-color" => color(style.border_top_color),
        "border-right-color" => color(style.border_right_color),
        "border-bottom-color" => color(style.border_bottom_color),
        "border-left-color" => color(style.border_left_color),
        "border-top-width" => px(style.border_top_width),
        "border-right-width" => px(style.border_right_width),
        "border-bottom-width" => px(style.border_bottom_width),
        "border-left-width" => px(style.border_left_width),
        "margin-top" => px(used(|b| b.dimensions.margin.top).unwrap_or(style.margin_top)),
        "margin-right" => px(used(|b| b.dimensions.margin.right).unwrap_or(style.margin_right)),
        "margin-bottom" => px(used(|b| b.dimensions.margin.bottom).unwrap_or(style.margin_bottom)),
        "margin-left" => px(used(|b| b.dimensions.margin.left).unwrap_or(style.margin_left)),
        "padding-top" => px(used(|b| b.dimensions.padding.top).unwrap_or(style.padding_top)),
        "padding-right" => px(used(|b| b.dimensions.padding.right).unwrap_or(style.padding_right)),
        "padding-bottom" => {
            px(used(|b| b.dimensions.padding.bottom).unwrap_or(style.padding_bottom))
        }
        "padding-left" => px(used(|b| b.dimensions.padding.left).unwrap_or(style.padding_left)),
        "width" | "height" => {
            let horizontal = property == "width";
            let inline =
                style.display == Display::Inline && !rendered.is_some_and(|b| b.is_replaced);
            match rendered.filter(|_| !inline) {
                Some(item) => {
                    let rect = if border_box {
                        item.dimensions.border_box()
                    } else {
                        item.dimensions.content
                    };
                    px(if horizontal { rect.width } else { rect.height })
                }
                None if horizontal => length_or(style.width, style.width_percent, "auto"),
                None => length_or(style.height, style.height_percent, "auto"),
            }
        }
        "min-width" => length_or(style.min_width, style.min_width_percent, "auto"),
        "min-height" => {
            if style.min_height.is_none() && style.min_height_percent.is_none() {
                let flex_or_grid_item = parent_style.is_some_and(|parent| {
                    matches!(
                        parent.display,
                        Display::Flex | Display::InlineFlex | Display::Grid | Display::InlineGrid
                    )
                });
                if flex_or_grid_item {
                    "auto".into()
                } else {
                    "0px".into()
                }
            } else {
                length_or(style.min_height, style.min_height_percent, "auto")
            }
        }
        "max-width" => length_or(style.max_width, style.max_width_percent, "none"),
        "max-height" => length_or(style.max_height, style.max_height_percent, "none"),
        "top" => {
            if style.top.is_none() && style.top_percent.is_none() {
                if style.position == "relative" {
                    "0px".into()
                } else {
                    "auto".into()
                }
            } else {
                length_or(style.top, style.top_percent, "auto")
            }
        }
        "right" => {
            if style.right.is_none() && style.right_percent.is_none() {
                if style.position == "relative" {
                    "0px".into()
                } else {
                    "auto".into()
                }
            } else {
                length_or(style.right, style.right_percent, "auto")
            }
        }
        "bottom" => {
            if style.bottom.is_none() && style.bottom_percent.is_none() {
                if style.position == "relative" {
                    "0px".into()
                } else {
                    "auto".into()
                }
            } else {
                length_or(style.bottom, style.bottom_percent, "auto")
            }
        }
        "left" => {
            if style.left.is_none() && style.left_percent.is_none() {
                if style.position == "relative" {
                    "0px".into()
                } else {
                    "auto".into()
                }
            } else {
                length_or(style.left, style.left_percent, "auto")
            }
        }
        "opacity" => number(style.opacity),
        "order" => style.order.to_string(),
        "z-index" if style.position == "static" && style.z_index == 0 => "auto".into(),
        "z-index" => style.z_index.to_string(),
        "flex-grow" => number(style.flex_grow),
        "flex-shrink" => number(style.flex_shrink),
        "flex-basis" => length_or(style.flex_basis, style.flex_basis_percent, "auto"),
        "row-gap" => style.row_gap.map_or_else(|| "normal".into(), px),
        "column-gap" => style.column_gap.map_or_else(|| "normal".into(), px),
        "margin" => {{
            let top = px(rendered.map(|b| b.dimensions.margin.top).unwrap_or(style.margin_top));
            let right = px(rendered.map(|b| b.dimensions.margin.right).unwrap_or(style.margin_right));
            let bottom = px(rendered.map(|b| b.dimensions.margin.bottom).unwrap_or(style.margin_bottom));
            let left = px(rendered.map(|b| b.dimensions.margin.left).unwrap_or(style.margin_left));
            serialize_shorthand_four(&[top, right, bottom, left])
        }}
        "padding" => {{
            let top = px(rendered.map(|b| b.dimensions.padding.top).unwrap_or(style.padding_top));
            let right = px(rendered.map(|b| b.dimensions.padding.right).unwrap_or(style.padding_right));
            let bottom = px(rendered.map(|b| b.dimensions.padding.bottom).unwrap_or(style.padding_bottom));
            let left = px(rendered.map(|b| b.dimensions.padding.left).unwrap_or(style.padding_left));
            serialize_shorthand_four(&[top, right, bottom, left])
        }}
        "overflow" => {{
            if style.overflow_x == style.overflow_y {{
                style.overflow_x.clone()
            }} else {{
                format!("{} {}", style.overflow_x, style.overflow_y)
            }}
        }}
        "border-radius" => px(style.border_radius),
        _ => return None,
    })
}

fn is_layout_dependent_property(property: &str) -> bool {
    matches!(
        property,
        "width"
            | "height"
            | "margin"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "padding"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
    )
}

fn read_computed(_: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node = get_node_id_from_this(args.get_or_undefined(0), context)?;
    let property = args
        .get_or_undefined(1)
        .to_string(context)?
        .to_std_string_escaped();
    let prop_lower = property.to_ascii_lowercase();

    let state = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned();
    let is_layout_dep = is_layout_dependent_property(&prop_lower);

    if let Some(state) = &state {
        let host = host(context)?;
        let document = state.borrow().document.clone();
        let doc = document.borrow();
        if let Some(style) = host.computed_style_for_node(&doc, node) {
            let is_disconnected = !shadow_dom::node_is_connected(node, &doc);
            let is_display_none = style.display == rix_css::Display::None;
            if !is_layout_dep || is_disconnected || is_display_none {
                let parent_style = doc
                    .parent(node)
                    .and_then(|parent| host.computed_style_for_node(&doc, parent));
                let dummy_snapshot = LayoutSnapshot::empty();
                let value = resolved_value(
                    &dummy_snapshot,
                    node,
                    &style,
                    parent_style.as_ref(),
                    &prop_lower,
                )
                .unwrap_or_default();
                return Ok(js_string!(value).into());
            }
        }
    }

    let Some((snapshot, document)) = snapshot_for(context, "computed_style")? else {
        return Ok(js_string!("").into());
    };
    let host = host(context)?;
    let Some(style) = host.computed_style(&document.borrow(), &snapshot, node) else {
        return Ok(js_string!("").into());
    };
    let parent_style = document
        .borrow()
        .parent(node)
        .and_then(|parent| host.computed_style(&document.borrow(), &snapshot, parent));
    let value = resolved_value(
        &snapshot,
        node,
        &style,
        parent_style.as_ref(),
        &prop_lower,
    )
    .unwrap_or_default();
    Ok(js_string!(value).into())
}

fn property_names(_: &JsValue, _: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let names = PROPERTIES
        .iter()
        .map(|name| JsValue::from(js_string!(*name)));
    Ok(boa_engine::object::builtins::JsArray::from_iter(names, context).into())
}

pub(super) fn register_get_computed_style(context: &mut Context) -> JsResult<()> {
    let factory = context.eval(boa_engine::Source::from_bytes(
        r#"
        (function(read, names) {
            const properties = names();
            const camel = name => name.replace(/-([a-z])/g, (_, c) => c.toUpperCase());
            const dashed = name => name.startsWith('--') ? name : name.replace(/[A-Z]/g, c => '-' + c.toLowerCase());
            const known = new Set(properties);
            const aliases = new Map(properties.map(name => [camel(name), name]));
            const declarationState = new WeakMap();
            function CSSStyleDeclaration() { throw new TypeError('Illegal constructor'); }
            const noModification = () => {
                throw new DOMException('Computed style declarations are read-only', 'NoModificationAllowedError');
            };
            const stateFor = declaration => {
                const state = declarationState.get(declaration);
                if (!state) throw new TypeError('Illegal CSSStyleDeclaration receiver');
                return state;
            };
            Object.defineProperties(CSSStyleDeclaration.prototype, {
                [Symbol.toStringTag]: {value: 'CSSStyleDeclaration', configurable: true},
                cssText: {
                    configurable: true,
                    enumerable: true,
                    get() {
                        const state = stateFor(this);
                        if (state.rule) return state.rule.__getStyleCssText();
                        return state.inline ? state.element.getAttribute('style') || '' : '';
                    },
                    set(value) {
                        const state = stateFor(this);
                        if (state.rule) {
                            state.rule.__setStyleCssText(String(value));
                            return;
                        }
                        if (!state.inline) noModification();
                        state.element.setAttribute('style', String(value));
                    },
                },
                length: {
                    configurable: true,
                    enumerable: true,
                    get() {
                        const state = stateFor(this);
                        if (state.rule) return state.rule.__getStyleLength();
                        return state.inline ? state.element.__styleLength() : properties.length;
                    },
                },
                parentRule: {
                    configurable: true,
                    enumerable: true,
                    get() {
                        const state = stateFor(this);
                        return state.rule || null;
                    },
                },
                cssFloat: {
                    configurable: true,
                    enumerable: true,
                    get() {
                        const state = stateFor(this);
                        if (state.rule) return state.rule.__getStyleValue('float');
                        if (state.inline) return state.element.__styleGet('float');
                        return state.empty ? '' : read(state.element, 'float');
                    },
                    set(value) {
                        const state = stateFor(this);
                        if (state.rule) {
                            state.rule.__setStyleProperty('float', String(value), '');
                            return;
                        }
                        if (!state.inline) noModification();
                        state.element.__styleSet('float', String(value), '');
                    },
                },
                getPropertyValue: {
                    value: function getPropertyValue(name) {
                        const state = stateFor(this);
                        if (state.rule) return state.rule.__getStyleValue(dashed(String(name)));
                        if (state.inline) return state.element.__styleGet(dashed(String(name)));
                        return state.empty ? '' : read(state.element, dashed(String(name)));
                    },
                    writable: true,
                    enumerable: true,
                    configurable: true,
                },
                getPropertyPriority: {
                    value: function getPropertyPriority(name) {
                        const state = stateFor(this);
                        if (state.rule) return state.rule.__getStylePriority(String(name));
                        return state.inline ? state.element.__stylePriority(String(name)) : (String(name), '');
                    },
                    writable: true,
                    enumerable: true,
                    configurable: true,
                },
                item: {
                    value: function item(index) {
                        const state = stateFor(this);
                        if (state.rule) return state.rule.__getStyleItem(Number(index) >>> 0);
                        return state.inline
                            ? state.element.__styleItem(Number(index) >>> 0)
                            : properties[Number(index) >>> 0] || '';
                    },
                    writable: true,
                    enumerable: true,
                    configurable: true,
                },
                removeProperty: {
                    value: function removeProperty(name) {
                        const state = stateFor(this);
                        const property = String(name);
                        if (state.rule) return state.rule.__removeStyleProperty(property);
                        if (!state.inline) noModification();
                        return state.element.__styleRemove(property);
                    },
                    writable: true,
                    enumerable: true,
                    configurable: true,
                },
                setProperty: {
                    value: function setProperty(name, value, priority) {
                        const state = stateFor(this);
                        const property = String(name);
                        const nextValue = String(value);
                        const nextPriority = priority === undefined ? '' : String(priority);
                        if (state.rule) {
                            state.rule.__setStyleProperty(property, nextValue, nextPriority);
                            return;
                        }
                        if (!state.inline) noModification();
                        state.element.__styleSet(property, nextValue, nextPriority);
                    },
                    writable: true,
                    enumerable: true,
                    configurable: true,
                },
            });
            function declaration(element, empty, inline) {
                const methods = Object.create(CSSStyleDeclaration.prototype);
                const declaration = new Proxy(methods, {
                    get(target, name, receiver) {
                        if (typeof name !== 'string') return Reflect.get(target, name, receiver);
                        if (Reflect.has(target, name)) return Reflect.get(target, name, receiver);
                        if (/^(0|[1-9]\d*)$/.test(name)) {
                            const index = Number(name);
                            return inline
                                ? element.__styleItem(index)
                                : properties[index];
                        }
                        const property = name === 'cssFloat' ? 'float' : aliases.get(name) || (known.has(name) ? name : null);
                        if (inline && name.startsWith('--')) return element.__styleGet(name);
                        if (inline && property) return element.__styleGet(property);
                        if (property) return empty ? '' : read(element, property);
                        return Reflect.get(target, name, receiver);
                    },
                    set(target, name, value, receiver) {
                        if (!inline) noModification();
                        if (typeof name !== 'string') return Reflect.set(target, name, value, receiver);
                        if (Reflect.has(target, name)) return Reflect.set(target, name, value, receiver);
                        if (/^(0|[1-9]\d*)$/.test(name)) return false;
                        const property = name === 'cssFloat' ? 'float' : aliases.get(name) || (known.has(name) ? name : null);
                        if (name.startsWith('--') || property) {
                            element.__styleSet(property || name, String(value), '');
                            return true;
                        }
                        return Reflect.set(target, name, value, receiver);
                    },
                    has(target, name) {
                        if (typeof name !== 'string') return Reflect.has(target, name);
                        if (inline && name.startsWith('--')) return true;
                        if (inline && /^(0|[1-9]\d*)$/.test(name)) {
                            return Number(name) < element.__styleLength();
                        }
                        return aliases.has(name) || known.has(name) ||
                            (/^(0|[1-9]\d*)$/.test(name) && Number(name) < properties.length) ||
                            Reflect.has(target, name);
                    },
                });
                declarationState.set(declaration, {element, empty, inline});
                return declaration;
            }
            function createRuleStyle(rule) {
                const methods = Object.create(CSSStyleDeclaration.prototype);
                const declaration = new Proxy(methods, {
                    get(target, name, receiver) {
                        if (typeof name !== 'string') return Reflect.get(target, name, receiver);
                        if (Reflect.has(target, name)) return Reflect.get(target, name, receiver);
                        if (/^(0|[1-9]\d*)$/.test(name)) {
                            return rule.__getStyleItem(Number(name));
                        }
                        const property = name === 'cssFloat' ? 'float' : aliases.get(name) || (known.has(name) ? name : (name.startsWith('--') ? name : dashed(name)));
                        const val = rule.__getStyleValue(property);
                        if (val !== undefined && val !== '') return val;
                        return Reflect.get(target, name, receiver);
                    },
                    set(target, name, value, receiver) {
                        if (typeof name !== 'string') return Reflect.set(target, name, value, receiver);
                        if (Reflect.has(target, name)) return Reflect.set(target, name, value, receiver);
                        if (/^(0|[1-9]\d*)$/.test(name)) return false;
                        const property = name === 'cssFloat' ? 'float' : aliases.get(name) || (known.has(name) ? name : (name.startsWith('--') ? name : dashed(name)));
                        rule.__setStyleProperty(property, String(value), '');
                        return true;
                    },
                    has(target, name) {
                        if (typeof name !== 'string') return Reflect.has(target, name);
                        if (/^(0|[1-9]\d*)$/.test(name)) {
                            return Number(name) < rule.__getStyleLength();
                        }
                        return aliases.has(name) || known.has(name) || Reflect.has(target, name);
                    },
                    ownKeys(target) {
                        const keys = [];
                        const len = rule.__getStyleLength();
                        for (let i = 0; i < len; i++) {
                            keys.push(String(i));
                        }
                        return keys;
                    },
                    getOwnPropertyDescriptor(target, prop) {
                        if (/^(0|[1-9]\d*)$/.test(prop) && Number(prop) < rule.__getStyleLength()) {
                            return {
                                value: rule.__getStyleItem(Number(prop)),
                                writable: true,
                                enumerable: true,
                                configurable: true,
                            };
                        }
                        return Reflect.getOwnPropertyDescriptor(target, prop);
                    }
                });
                declarationState.set(declaration, { rule, inline: false });
                return declaration;
            }
            Object.defineProperty(globalThis, '__createRuleStyle', { value: createRuleStyle, configurable: true });
            Object.defineProperty(globalThis, 'CSSStyleDeclaration', {value: CSSStyleDeclaration, writable: true, configurable: true});
            function getComputedStyle(element, pseudoElement) {
                if (!(element instanceof Element)) throw new TypeError("Failed to execute 'getComputedStyle': parameter 1 is not of type 'Element'.");
                const pseudo = pseudoElement != null && String(pseudoElement) !== '';
                return declaration(element, pseudo, false);
            }
            return {getComputedStyle, createInlineStyle: element => declaration(element, false, true), createRuleStyle};
        })
        "#,
    ))?;
    let factory = factory
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("computed style factory"))?;
    let read =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(read_computed))
            .name(js_string!("read"))
            .length(2)
            .build();
    let names =
        FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(property_names))
            .name(js_string!("names"))
            .length(0)
            .build();
    let api = factory.call(&JsValue::undefined(), &[read.into(), names.into()], context)?;
    let api = api
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("computed style API"))?;
    let function = api.get(js_string!("getComputedStyle"), context)?;
    let inline_style_factory = api.get(js_string!("createInlineStyle"), context)?;
    let window = context.global_object().get(js_string!("Window"), context)?;
    let window = window
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("Window missing"))?
        .get(js_string!("prototype"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("Window.prototype missing"))?;
    window.define_property_or_throw(
        js_string!("getComputedStyle"),
        PropertyDescriptor::builder()
            .value(function)
            .writable(true)
            .configurable(true)
            .build(),
        context,
    )?;
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        state.borrow_mut().inline_style_factory = inline_style_factory.as_object();
    }
    Ok(())
}
