use boa_engine::{js_string, Context, JsError, JsNativeError, JsObject, JsResult, JsValue};
use std::collections::HashMap;

#[derive(Clone)]
pub struct DomPrototypes {
    pub node: JsObject,
    pub element: JsObject,
    pub html_element: JsObject,
    pub html_unknown: JsObject,
    pub svg_element: Option<JsObject>,
    pub text: JsObject,
    pub comment: JsObject,
    pub document: JsObject,
    pub document_type: JsObject,
    pub document_fragment: JsObject,
    pub shadow_root: Option<JsObject>,
    pub dom_token_list: Option<JsObject>,
    pub html_tags: HashMap<String, JsObject>,
    pub svg_tags: HashMap<String, JsObject>,
}

fn get_proto(name: &str, context: &mut Context) -> JsResult<JsObject> {
    let global = context.global_object();
    let constructor = global.get(js_string!(name), context)?;
    let constructor_obj = constructor.as_object().ok_or_else(|| {
        JsError::from(JsNativeError::typ().with_message(format!("{name} constructor missing")))
    })?;
    let proto = constructor_obj.get(js_string!("prototype"), context)?;
    proto.as_object().ok_or_else(|| {
        JsError::from(JsNativeError::typ().with_message(format!("{name}.prototype missing")))
    })
}

pub fn init_dom_prototypes(context: &mut Context) -> JsResult<DomPrototypes> {
    let node = get_proto("Node", context)?;
    let element = get_proto("Element", context)?;
    let html_element = get_proto("HTMLElement", context)?;
    let html_unknown = get_proto("HTMLUnknownElement", context).unwrap_or_else(|_| html_element.clone());
    let svg_element = get_proto("SVGElement", context).ok();
    let text = get_proto("Text", context)?;
    let comment = get_proto("Comment", context)?;
    let document = get_proto("Document", context)?;
    let document_type = get_proto("DocumentType", context)?;
    let document_fragment = get_proto("DocumentFragment", context)?;
    let shadow_root = get_proto("ShadowRoot", context).ok();
    let dom_token_list = get_proto("DOMTokenList", context).ok();

    let mut svg_tags = HashMap::new();
    let svg_mappings = [
        ("svg", "SVGSVGElement"),
        ("path", "SVGPathElement"),
        ("g", "SVGGElement"),
        ("circle", "SVGCircleElement"),
        ("rect", "SVGRectElement"),
        ("use", "SVGUseElement"),
        ("defs", "SVGDefsElement"),
        ("clipPath", "SVGClipPathElement"),
        ("polyline", "SVGPolylineElement"),
        ("polygon", "SVGPolygonElement"),
        ("line", "SVGLineElement"),
        ("ellipse", "SVGEllipseElement"),
        ("text", "SVGTextElement"),
    ];
    for (tag, interface_name) in svg_mappings {
        if let Ok(proto) = get_proto(interface_name, context) {
            svg_tags.insert(tag.to_string(), proto);
        }
    }

    let mut html_tags = HashMap::new();
    for (tag, interface_name) in super::html_elements::TAGS {
        if let Ok(proto) = get_proto(interface_name, context) {
            html_tags.insert((*tag).to_string(), proto);
        }
    }
    for tag in super::html_elements::GENERIC_TAGS {
        html_tags.insert((*tag).to_string(), html_element.clone());
    }
    for (tag, interface_name) in [
        ("template", "HTMLTemplateElement"),
        ("input", "HTMLInputElement"),
        ("img", "HTMLImageElement"),
        ("textarea", "HTMLTextAreaElement"),
    ] {
        if let Ok(proto) = get_proto(interface_name, context) {
            html_tags.insert(tag.to_string(), proto);
        }
    }

    Ok(DomPrototypes {
        node,
        element,
        html_element,
        html_unknown,
        svg_element,
        text,
        comment,
        document,
        document_type,
        document_fragment,
        shadow_root,
        dom_token_list,
        html_tags,
        svg_tags,
    })
}

pub fn install_prototype_accessors(
    context: &mut Context,
    inline_style_factory: Option<&JsObject>,
) -> JsResult<()> {
    let factory_val = inline_style_factory.map_or(JsValue::undefined(), |f| f.clone().into());
    let script = r#"
        (function(createInlineStyle) {
            const _styleMap = new WeakMap();
            const styleDescriptor = {
                get() {
                    let s = _styleMap.get(this);
                    if (!s) {
                        if (typeof createInlineStyle !== 'function') {
                            throw new TypeError('CSSStyleDeclaration factory is unavailable');
                        }
                        s = createInlineStyle(this);
                        _styleMap.set(this, s);
                    }
                    return s;
                },
                set(value) {
                    this.setAttribute('style', String(value));
                },
                configurable: true,
                enumerable: true,
            };
            if (globalThis.HTMLElement) {
                Object.defineProperty(HTMLElement.prototype, 'style', styleDescriptor);
            }
            if (globalThis.SVGElement) {
                Object.defineProperty(SVGElement.prototype, 'style', styleDescriptor);
            }

            const _datasetMap = new WeakMap();
            const datasetAttribute = name => 'data-' + String(name).replace(/[A-Z]/g, ch => '-' + ch.toLowerCase());
            function getOrCreateDataset(el) {
                let d = _datasetMap.get(el);
                if (!d) {
                    d = new Proxy({}, {
                        get(target, name) {
                            if (typeof name === 'symbol') return undefined;
                            return el.getAttribute(datasetAttribute(name)) ?? undefined;
                        },
                        set(target, name, value) {
                            el.setAttribute(datasetAttribute(name), String(value));
                            return true;
                        },
                        deleteProperty(target, name) {
                            el.removeAttribute(datasetAttribute(name));
                            return true;
                        },
                        has(target, name) {
                            return el.hasAttribute(datasetAttribute(name));
                        },
                        ownKeys() {
                            return el.getAttributeNames()
                                .filter(name => name.startsWith('data-'))
                                .map(name => name.slice(5).replace(/-([a-z])/g, (_, ch) => ch.toUpperCase()));
                        },
                        getOwnPropertyDescriptor(target, name) {
                            if (!el.hasAttribute(datasetAttribute(name))) return undefined;
                            return {
                                value: el.getAttribute(datasetAttribute(name)),
                                enumerable: true,
                                configurable: true,
                                writable: true,
                            };
                        },
                    });
                    _datasetMap.set(el, d);
                }
                return d;
            }
            const datasetDescriptor = {
                get() {
                    return getOrCreateDataset(this);
                },
                configurable: true,
                enumerable: true,
            };
            if (globalThis.HTMLElement) {
                Object.defineProperty(HTMLElement.prototype, 'dataset', datasetDescriptor);
            }
            if (globalThis.SVGElement) {
                Object.defineProperty(SVGElement.prototype, 'dataset', datasetDescriptor);
            }
        })
    "#;
    let func = context.eval(boa_engine::Source::from_bytes(script.as_bytes()))?;
    if let Some(func_obj) = func.as_object() {
        func_obj.call(&JsValue::undefined(), &[factory_val], context)?;
    }
    Ok(())
}
