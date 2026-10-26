
use super::*;

const INTERFACES: &[(&str, &str)] = &[
    ("HTMLUnknownElement", "HTMLElement"),
    ("HTMLAnchorElement", "HTMLElement"),
    ("HTMLAreaElement", "HTMLElement"),
    ("HTMLBaseElement", "HTMLElement"),
    ("HTMLBodyElement", "HTMLElement"),
    ("HTMLBRElement", "HTMLElement"),
    ("HTMLButtonElement", "HTMLElement"),
    ("HTMLCanvasElement", "HTMLElement"),
    ("HTMLDataElement", "HTMLElement"),
    ("HTMLDataListElement", "HTMLElement"),
    ("HTMLDetailsElement", "HTMLElement"),
    ("HTMLDialogElement", "HTMLElement"),
    ("HTMLDirectoryElement", "HTMLElement"),
    ("HTMLDivElement", "HTMLElement"),
    ("HTMLDListElement", "HTMLElement"),
    ("HTMLEmbedElement", "HTMLElement"),
    ("HTMLFieldSetElement", "HTMLElement"),
    ("HTMLFormElement", "HTMLElement"),
    ("HTMLFrameElement", "HTMLElement"),
    ("HTMLFrameSetElement", "HTMLElement"),
    ("HTMLHeadingElement", "HTMLElement"),
    ("HTMLHeadElement", "HTMLElement"),
    ("HTMLHRElement", "HTMLElement"),
    ("HTMLHtmlElement", "HTMLElement"),
    ("HTMLIFrameElement", "HTMLElement"),
    ("HTMLInputElement", "HTMLElement"),
    ("HTMLLabelElement", "HTMLElement"),
    ("HTMLLegendElement", "HTMLElement"),
    ("HTMLLIElement", "HTMLElement"),
    ("HTMLLinkElement", "HTMLElement"),
    ("HTMLMapElement", "HTMLElement"),
    ("HTMLMarqueeElement", "HTMLElement"),
    ("HTMLMenuElement", "HTMLElement"),
    ("HTMLMetaElement", "HTMLElement"),
    ("HTMLMeterElement", "HTMLElement"),
    ("HTMLModElement", "HTMLElement"),
    ("HTMLObjectElement", "HTMLElement"),
    ("HTMLOListElement", "HTMLElement"),
    ("HTMLOptGroupElement", "HTMLElement"),
    ("HTMLOptionElement", "HTMLElement"),
    ("HTMLOutputElement", "HTMLElement"),
    ("HTMLParagraphElement", "HTMLElement"),
    ("HTMLParamElement", "HTMLElement"),
    ("HTMLPictureElement", "HTMLElement"),
    ("HTMLPreElement", "HTMLElement"),
    ("HTMLProgressElement", "HTMLElement"),
    ("HTMLQuoteElement", "HTMLElement"),
    ("HTMLScriptElement", "HTMLElement"),
    ("HTMLSelectElement", "HTMLElement"),
    ("HTMLSlotElement", "HTMLElement"),
    ("HTMLSourceElement", "HTMLElement"),
    ("HTMLSpanElement", "HTMLElement"),
    ("HTMLStyleElement", "HTMLElement"),
    ("HTMLTableCaptionElement", "HTMLElement"),
    ("HTMLTableCellElement", "HTMLElement"),
    ("HTMLTableColElement", "HTMLElement"),
    ("HTMLTableElement", "HTMLElement"),
    ("HTMLTableRowElement", "HTMLElement"),
    ("HTMLTableSectionElement", "HTMLElement"),
    ("HTMLTextAreaElement", "HTMLElement"),
    ("HTMLTimeElement", "HTMLElement"),
    ("HTMLTitleElement", "HTMLElement"),
    ("HTMLTrackElement", "HTMLElement"),
    ("HTMLUListElement", "HTMLElement"),
    ("HTMLMediaElement", "HTMLElement"),
    ("HTMLAudioElement", "HTMLMediaElement"),
    ("HTMLVideoElement", "HTMLMediaElement"),
];

pub(crate) const TAGS: &[(&str, &str)] = &[
    ("a", "HTMLAnchorElement"),
    ("area", "HTMLAreaElement"),
    ("audio", "HTMLAudioElement"),
    ("base", "HTMLBaseElement"),
    ("body", "HTMLBodyElement"),
    ("br", "HTMLBRElement"),
    ("button", "HTMLButtonElement"),
    ("canvas", "HTMLCanvasElement"),
    ("caption", "HTMLTableCaptionElement"),
    ("col", "HTMLTableColElement"),
    ("colgroup", "HTMLTableColElement"),
    ("data", "HTMLDataElement"),
    ("datalist", "HTMLDataListElement"),
    ("details", "HTMLDetailsElement"),
    ("dialog", "HTMLDialogElement"),
    ("dir", "HTMLDirectoryElement"),
    ("div", "HTMLDivElement"),
    ("dl", "HTMLDListElement"),
    ("embed", "HTMLEmbedElement"),
    ("fieldset", "HTMLFieldSetElement"),
    ("form", "HTMLFormElement"),
    ("frame", "HTMLFrameElement"),
    ("frameset", "HTMLFrameSetElement"),
    ("h1", "HTMLHeadingElement"),
    ("h2", "HTMLHeadingElement"),
    ("h3", "HTMLHeadingElement"),
    ("h4", "HTMLHeadingElement"),
    ("h5", "HTMLHeadingElement"),
    ("h6", "HTMLHeadingElement"),
    ("head", "HTMLHeadElement"),
    ("hr", "HTMLHRElement"),
    ("html", "HTMLHtmlElement"),
    ("iframe", "HTMLIFrameElement"),
    ("img", "HTMLImageElement"),
    ("input", "HTMLInputElement"),
    ("label", "HTMLLabelElement"),
    ("legend", "HTMLLegendElement"),
    ("li", "HTMLLIElement"),
    ("link", "HTMLLinkElement"),
    ("map", "HTMLMapElement"),
    ("marquee", "HTMLMarqueeElement"),
    ("menu", "HTMLMenuElement"),
    ("meta", "HTMLMetaElement"),
    ("meter", "HTMLMeterElement"),
    ("del", "HTMLModElement"),
    ("ins", "HTMLModElement"),
    ("object", "HTMLObjectElement"),
    ("ol", "HTMLOListElement"),
    ("optgroup", "HTMLOptGroupElement"),
    ("option", "HTMLOptionElement"),
    ("output", "HTMLOutputElement"),
    ("p", "HTMLParagraphElement"),
    ("param", "HTMLParamElement"),
    ("picture", "HTMLPictureElement"),
    ("pre", "HTMLPreElement"),
    ("progress", "HTMLProgressElement"),
    ("blockquote", "HTMLQuoteElement"),
    ("q", "HTMLQuoteElement"),
    ("script", "HTMLScriptElement"),
    ("select", "HTMLSelectElement"),
    ("slot", "HTMLSlotElement"),
    ("source", "HTMLSourceElement"),
    ("span", "HTMLSpanElement"),
    ("style", "HTMLStyleElement"),
    ("td", "HTMLTableCellElement"),
    ("th", "HTMLTableCellElement"),
    ("table", "HTMLTableElement"),
    ("tr", "HTMLTableRowElement"),
    ("tbody", "HTMLTableSectionElement"),
    ("tfoot", "HTMLTableSectionElement"),
    ("thead", "HTMLTableSectionElement"),
    ("textarea", "HTMLTextAreaElement"),
    ("time", "HTMLTimeElement"),
    ("title", "HTMLTitleElement"),
    ("track", "HTMLTrackElement"),
    ("ul", "HTMLUListElement"),
    ("video", "HTMLVideoElement"),
];

pub(crate) const GENERIC_TAGS: &[&str] = &[
    "abbr",
    "address",
    "article",
    "aside",
    "b",
    "bdi",
    "bdo",
    "cite",
    "code",
    "dd",
    "dfn",
    "dt",
    "em",
    "figcaption",
    "figure",
    "footer",
    "header",
    "hgroup",
    "i",
    "kbd",
    "main",
    "mark",
    "nav",
    "noscript",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "search",
    "section",
    "small",
    "strong",
    "sub",
    "summary",
    "sup",
    "u",
    "var",
    "wbr",
];

pub(super) fn register_html_element_interfaces(context: &mut Context) -> JsResult<()> {
    let construction = context
        .get_data::<Rc<RefCell<DomBridgeState>>>()
        .and_then(|state| state.borrow().custom_element_construction.clone())
        .ok_or_else(|| {
            JsNativeError::error().with_message("HTML interface bootstrap unavailable")
        })?;
    let define = construction
        .get(js_string!("defineHTMLInterface"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::typ().with_message("HTML interface registration is unavailable")
        })?;
    for (name, parent) in INTERFACES {
        define.call(
            &construction.clone().into(),
            &[js_string!(*name).into(), js_string!(*parent).into()],
            context,
        )?;
    }
    let register = construction
        .get(js_string!("registerHTMLTag"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("HTML tag registry is unavailable"))?;
    for tag in GENERIC_TAGS {
        register.call(
            &construction.clone().into(),
            &[js_string!(*tag).into(), js_string!("HTMLElement").into()],
            context,
        )?;
    }
    for (tag, interface) in TAGS {
        register.call(
            &construction.clone().into(),
            &[js_string!(*tag).into(), js_string!(*interface).into()],
            context,
        )?;
    }
    for (tag, interface) in [
        ("template", "HTMLTemplateElement"),
        ("input", "HTMLInputElement"),
        ("img", "HTMLImageElement"),
    ] {
        register.call(
            &construction.clone().into(),
            &[js_string!(tag).into(), js_string!(interface).into()],
            context,
        )?;
    }
    let lookup = construction
        .get(js_string!("interfaceForTag"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::typ().with_message("HTML element interface lookup is unavailable")
        })?;
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        state.borrow_mut().html_interface_lookup = Some(lookup);
    }
    Ok(())
}
