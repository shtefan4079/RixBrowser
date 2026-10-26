
use super::*;
use boa_engine::object::builtins::JsArray;
use boa_engine::object::FunctionObjectBuilder;
use boa_engine::property::Attribute;

fn parsed_rule_to_js(
    rule: &rix_css::CssomParsedRule,
    context: &mut Context,
) -> JsResult<JsObject> {
    let mut decl_objs = Vec::with_capacity(rule.declarations.len());
    for d in &rule.declarations {
        let mut d_obj = ObjectInitializer::new(context);
        d_obj.property(
            js_string!("name"),
            js_string!(d.name.as_str()),
            Attribute::all(),
        );
        d_obj.property(
            js_string!("value"),
            js_string!(d.value.clone()),
            Attribute::all(),
        );
        d_obj.property(
            js_string!("important"),
            JsValue::new(d.important),
            Attribute::all(),
        );
        decl_objs.push(JsValue::from(d_obj.build()));
    }
    let decls_array = JsArray::from_iter(decl_objs, context);

    let mut child_objs = Vec::with_capacity(rule.children.len());
    for c in &rule.children {
        let child_js = parsed_rule_to_js(c, context)?;
        child_objs.push(JsValue::from(child_js));
    }
    let children_array = JsArray::from_iter(child_objs, context);

    let mut obj = ObjectInitializer::new(context);
    obj.property(
        js_string!("type"),
        JsValue::new(rule.rule_type),
        Attribute::all(),
    );
    obj.property(
        js_string!("cssText"),
        js_string!(rule.css_text.clone()),
        Attribute::all(),
    );
    if let Some(sel) = &rule.selector_text {
        obj.property(
            js_string!("selectorText"),
            js_string!(sel.clone()),
            Attribute::all(),
        );
    } else {
        obj.property(
            js_string!("selectorText"),
            JsValue::null(),
            Attribute::all(),
        );
    }
    if let Some(cond) = &rule.condition_text {
        obj.property(
            js_string!("conditionText"),
            js_string!(cond.clone()),
            Attribute::all(),
        );
    } else {
        obj.property(
            js_string!("conditionText"),
            JsValue::null(),
            Attribute::all(),
        );
    }
    obj.property(js_string!("declarations"), decls_array, Attribute::all());
    obj.property(js_string!("children"), children_array, Attribute::all());
    Ok(obj.build())
}

pub(super) fn cssom_parse_rule_native(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let css = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let res = match rix_css::parse_cssom_rule(&css) {
        Ok(rule) => {
            let rule_js = parsed_rule_to_js(&rule, context)?;
            let mut obj = ObjectInitializer::new(context);
            obj.property(js_string!("rule"), rule_js, Attribute::all());
            obj.build()
        }
        Err(err) => {
            let mut obj = ObjectInitializer::new(context);
            obj.property(js_string!("error"), js_string!(err), Attribute::all());
            obj.build()
        }
    };
    Ok(res.into())
}

pub(super) fn cssom_parse_stylesheet_native(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let css = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let rules = rix_css::parse_cssom_stylesheet(&css);
    let mut rule_objs = Vec::with_capacity(rules.len());
    for r in &rules {
        let rule_js = parsed_rule_to_js(r, context)?;
        rule_objs.push(JsValue::from(rule_js));
    }
    let array = JsArray::from_iter(rule_objs, context);
    Ok(array.into())
}

pub(super) fn cssom_sync_element_sheet_native(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let target = args.get_or_undefined(0);
    let Ok(node_id) = get_node_id_from_this(target, context) else {
        return Ok(JsValue::undefined());
    };
    let css = args
        .get_or_undefined(1)
        .to_string(context)?
        .to_std_string_escaped();
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        state
            .borrow()
            .document
            .borrow_mut()
            .set_dynamic_stylesheet(node_id, css);
    }
    Ok(JsValue::undefined())
}

pub(super) fn cssom_sync_adopted_native(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let target = args.get_or_undefined(0);
    let Ok(node_id) = get_node_id_from_this(target, context) else {
        return Ok(JsValue::undefined());
    };
    let array_val = args.get_or_undefined(1);
    let mut sheets = Vec::new();
    if let Some(arr) = array_val.as_object() {
        let len_val = arr.get(js_string!("length"), context)?;
        let len = len_val.to_number(context)? as usize;
        for i in 0..len {
            let item = arr.get(i as u64, context)?;
            if let Ok(s) = item.to_string(context) {
                sheets.push(s.to_std_string_escaped());
            }
        }
    }
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        state
            .borrow()
            .document
            .borrow_mut()
            .set_adopted_stylesheets(node_id, sheets);
    }
    Ok(JsValue::undefined())
}

pub(super) fn cssom_get_connected_text_native(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let target = args.get_or_undefined(0);
    let Ok(node_id) = get_node_id_from_this(target, context) else {
        return Ok(js_string!("").into());
    };
    if let Some(state) = context.get_data::<Rc<RefCell<DomBridgeState>>>().cloned() {
        let state_ref = state.borrow();
        let doc = state_ref.document.borrow();
        let mut text = String::new();
        for child in doc.children(node_id) {
            if let Some(NodeData::Text(t)) = doc.get(child) {
                text.push_str(t);
            }
        }
        return Ok(js_string!(text).into());
    }
    Ok(js_string!("").into())
}

pub fn register_cssom(context: &mut Context) -> Result<(), JsError> {
    let parse_rule = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(cssom_parse_rule_native),
    )
    .name(js_string!("__cssomParseRule"))
    .length(1)
    .build();
    context.register_global_property(
        js_string!("__cssomParseRule"),
        parse_rule,
        Attribute::WRITABLE | Attribute::CONFIGURABLE,
    )?;

    let parse_stylesheet = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(cssom_parse_stylesheet_native),
    )
    .name(js_string!("__cssomParseStylesheet"))
    .length(1)
    .build();
    context.register_global_property(
        js_string!("__cssomParseStylesheet"),
        parse_stylesheet,
        Attribute::WRITABLE | Attribute::CONFIGURABLE,
    )?;

    let sync_elem = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(cssom_sync_element_sheet_native),
    )
    .name(js_string!("__cssomSyncElementSheet"))
    .length(2)
    .build();
    context.register_global_property(
        js_string!("__cssomSyncElementSheet"),
        sync_elem,
        Attribute::WRITABLE | Attribute::CONFIGURABLE,
    )?;

    let sync_adopted = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(cssom_sync_adopted_native),
    )
    .name(js_string!("__cssomSyncAdopted"))
    .length(2)
    .build();
    context.register_global_property(
        js_string!("__cssomSyncAdopted"),
        sync_adopted,
        Attribute::WRITABLE | Attribute::CONFIGURABLE,
    )?;

    let get_connected_text = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(cssom_get_connected_text_native),
    )
    .name(js_string!("__cssomGetConnectedText"))
    .length(1)
    .build();
    context.register_global_property(
        js_string!("__cssomGetConnectedText"),
        get_connected_text,
        Attribute::WRITABLE | Attribute::CONFIGURABLE,
    )?;

    context.eval(boa_engine::Source::from_bytes(
        r#"
        (function() {
            function illegalConstructor(name) {
                throw new TypeError(`Illegal constructor: ${name}`);
            }

            function MediaList() {
                illegalConstructor('MediaList');
            }

            function createMediaList(initialText) {
                const methods = Object.create(MediaList.prototype);
                let mediaArray = [];
                function parseText(text) {
                    if (!text || String(text).trim() === '') return [];
                    return String(text).split(',').map(s => s.trim()).filter(Boolean);
                }
                mediaArray = parseText(initialText);

                Object.defineProperties(methods, {
                    mediaText: {
                        get() { return mediaArray.join(', '); },
                        set(val) { mediaArray = parseText(val); },
                        enumerable: true,
                        configurable: true,
                    },
                    length: {
                        get() { return mediaArray.length; },
                        enumerable: true,
                        configurable: true,
                    },
                    item: {
                        value: function item(index) {
                            const idx = Number(index) >>> 0;
                            return mediaArray[idx] !== undefined ? mediaArray[idx] : null;
                        },
                        writable: true,
                        enumerable: true,
                        configurable: true,
                    },
                    appendMedium: {
                        value: function appendMedium(medium) {
                            const trimmed = String(medium).trim();
                            if (trimmed && !mediaArray.includes(trimmed)) {
                                mediaArray.push(trimmed);
                            }
                        },
                        writable: true,
                        enumerable: true,
                        configurable: true,
                    },
                    deleteMedium: {
                        value: function deleteMedium(medium) {
                            const trimmed = String(medium).trim();
                            const idx = mediaArray.indexOf(trimmed);
                            if (idx === -1) {
                                throw new DOMException("Failed to execute 'deleteMedium': The medium provided was not found.", "NotFoundError");
                            }
                            mediaArray.splice(idx, 1);
                        },
                        writable: true,
                        enumerable: true,
                        configurable: true,
                    },
                    toString: {
                        value: function toString() { return this.mediaText; },
                        writable: true,
                        configurable: true,
                    },
                });

                return new Proxy(methods, {
                    get(target, prop, receiver) {
                        if (typeof prop === 'string' && /^(0|[1-9]\d*)$/.test(prop)) {
                            const idx = Number(prop);
                            return mediaArray[idx];
                        }
                        if (prop === Symbol.iterator) {
                            return function* () {
                                for (const item of mediaArray) yield item;
                            };
                        }
                        return Reflect.get(target, prop, receiver);
                    },
                    has(target, prop) {
                        if (typeof prop === 'string' && /^(0|[1-9]\d*)$/.test(prop)) {
                            return Number(prop) < mediaArray.length;
                        }
                        return Reflect.has(target, prop);
                    },
                    ownKeys(target) {
                        const keys = Reflect.ownKeys(target);
                        for (let i = 0; i < mediaArray.length; i++) {
                            keys.push(String(i));
                        }
                        return keys;
                    },
                    getOwnPropertyDescriptor(target, prop) {
                        if (typeof prop === 'string' && /^(0|[1-9]\d*)$/.test(prop)) {
                            const idx = Number(prop);
                            if (idx < mediaArray.length) {
                                return {
                                    value: mediaArray[idx],
                                    writable: true,
                                    enumerable: true,
                                    configurable: true,
                                };
                            }
                        }
                        return Reflect.getOwnPropertyDescriptor(target, prop);
                    },
                });
            }
            Object.defineProperty(MediaList.prototype, Symbol.toStringTag, { value: 'MediaList', configurable: true });
            Object.defineProperty(globalThis, 'MediaList', { value: MediaList, writable: true, configurable: true });

            function CSSRule() {
                illegalConstructor('CSSRule');
            }

            const CSSRULE_CONSTANTS = {
                STYLE_RULE: 1,
                CHARSET_RULE: 2,
                IMPORT_RULE: 3,
                MEDIA_RULE: 4,
                FONT_FACE_RULE: 5,
                PAGE_RULE: 6,
                KEYFRAMES_RULE: 7,
                KEYFRAME_RULE: 8,
                MARGIN_RULE: 9,
                NAMESPACE_RULE: 10,
                COUNTER_STYLE_RULE: 11,
                SUPPORTS_RULE: 12,
                DOCUMENT_RULE: 13,
                FONT_FEATURE_VALUES_RULE: 14,
                VIEWPORT_RULE: 15,
                REGION_STYLE_RULE: 16,
            };

            for (const [key, val] of Object.entries(CSSRULE_CONSTANTS)) {
                Object.defineProperty(CSSRule, key, { value: val, enumerable: true });
                Object.defineProperty(CSSRule.prototype, key, { value: val, enumerable: true });
            }

            Object.defineProperties(CSSRule.prototype, {
                type: {
                    get() {
                        if (!this || !(this instanceof CSSRule)) throw new TypeError('Illegal invocation');
                        return this._type || 0;
                    },
                    enumerable: true,
                    configurable: true,
                },
                cssText: {
                    get() {
                        if (!this || !(this instanceof CSSRule)) throw new TypeError('Illegal invocation');
                        return this._cssText || '';
                    },
                    set(val) {
                        if (!this || !(this instanceof CSSRule)) throw new TypeError('Illegal invocation');
                        this._setCssText(String(val));
                    },
                    enumerable: true,
                    configurable: true,
                },
                parentRule: {
                    get() {
                        if (!this || !(this instanceof CSSRule)) throw new TypeError('Illegal invocation');
                        return this._parentRule || null;
                    },
                    enumerable: true,
                    configurable: true,
                },
                parentStyleSheet: {
                    get() {
                        if (!this || !(this instanceof CSSRule)) throw new TypeError('Illegal invocation');
                        return this._parentStyleSheet || null;
                    },
                    enumerable: true,
                    configurable: true,
                },
                [Symbol.toStringTag]: { value: 'CSSRule', configurable: true },
            });
            Object.defineProperty(globalThis, 'CSSRule', { value: CSSRule, writable: true, configurable: true });

            function CSSStyleRule() {
                illegalConstructor('CSSStyleRule');
            }
            Object.setPrototypeOf(CSSStyleRule.prototype, CSSRule.prototype);
            Object.setPrototypeOf(CSSStyleRule, CSSRule);

            Object.defineProperties(CSSStyleRule.prototype, {
                selectorText: {
                    get() {
                        if (!this || !(this instanceof CSSStyleRule)) throw new TypeError('Illegal invocation');
                        return this._selectorText || '';
                    },
                    set(val) {
                        if (!this || !(this instanceof CSSStyleRule)) throw new TypeError('Illegal invocation');
                        const res = __cssomParseRule(String(val) + ' {}');
                        if (res && res.rule && res.rule.selectorText) {
                            this._selectorText = res.rule.selectorText;
                            this.__rebuildCssText();
                            this.__notifyMutated();
                        }
                    },
                    enumerable: true,
                    configurable: true,
                },
                style: {
                    get() {
                        if (!this || !(this instanceof CSSStyleRule)) throw new TypeError('Illegal invocation');
                        return this._styleDeclaration;
                    },
                    enumerable: true,
                    configurable: true,
                },
                [Symbol.toStringTag]: { value: 'CSSStyleRule', configurable: true },
            });
            Object.defineProperty(globalThis, 'CSSStyleRule', { value: CSSStyleRule, writable: true, configurable: true });

            function CSSGroupingRule() {
                illegalConstructor('CSSGroupingRule');
            }
            Object.setPrototypeOf(CSSGroupingRule.prototype, CSSRule.prototype);
            Object.setPrototypeOf(CSSGroupingRule, CSSRule);

            Object.defineProperties(CSSGroupingRule.prototype, {
                cssRules: {
                    get() {
                        if (!this || !(this instanceof CSSGroupingRule)) throw new TypeError('Illegal invocation');
                        return this._cssRulesList;
                    },
                    enumerable: true,
                    configurable: true,
                },
                insertRule: {
                    value: function insertRule(rule, index = 0) {
                        if (!this || !(this instanceof CSSGroupingRule)) throw new TypeError('Illegal invocation');
                        const idx = index === undefined ? 0 : Number(index);
                        if (isNaN(idx) || idx < 0 || idx > this._rules.length) {
                            throw new DOMException("Failed to execute 'insertRule': The index provided is outside the range.", "IndexSizeError");
                        }
                        const res = __cssomParseRule(String(rule));
                        if (res.error || !res.rule) {
                            throw new DOMException("Failed to execute 'insertRule': The rule provided could not be parsed.", "SyntaxError");
                        }
                        const newRule = createRuleObject(res.rule, this.parentStyleSheet, this);
                        this._rules.splice(idx, 0, newRule);
                        this.__notifyMutated();
                        return idx;
                    },
                    writable: true,
                    enumerable: true,
                    configurable: true,
                },
                deleteRule: {
                    value: function deleteRule(index) {
                        if (!this || !(this instanceof CSSGroupingRule)) throw new TypeError('Illegal invocation');
                        const idx = Number(index);
                        if (isNaN(idx) || idx < 0 || idx >= this._rules.length) {
                            throw new DOMException("Failed to execute 'deleteRule': The index provided is outside the range.", "IndexSizeError");
                        }
                        const removed = this._rules.splice(idx, 1)[0];
                        if (removed) {
                            removed._parentStyleSheet = null;
                            removed._parentRule = null;
                        }
                        this.__notifyMutated();
                    },
                    writable: true,
                    enumerable: true,
                    configurable: true,
                },
                [Symbol.toStringTag]: { value: 'CSSGroupingRule', configurable: true },
            });
            Object.defineProperty(globalThis, 'CSSGroupingRule', { value: CSSGroupingRule, writable: true, configurable: true });

            function CSSMediaRule() {
                illegalConstructor('CSSMediaRule');
            }
            Object.setPrototypeOf(CSSMediaRule.prototype, CSSGroupingRule.prototype);
            Object.setPrototypeOf(CSSMediaRule, CSSGroupingRule);

            Object.defineProperties(CSSMediaRule.prototype, {
                conditionText: {
                    get() {
                        if (!this || !(this instanceof CSSMediaRule)) throw new TypeError('Illegal invocation');
                        return this._conditionText || '';
                    },
                    enumerable: true,
                    configurable: true,
                },
                media: {
                    get() {
                        if (!this || !(this instanceof CSSMediaRule)) throw new TypeError('Illegal invocation');
                        return this._mediaList;
                    },
                    enumerable: true,
                    configurable: true,
                },
                [Symbol.toStringTag]: { value: 'CSSMediaRule', configurable: true },
            });
            Object.defineProperty(globalThis, 'CSSMediaRule', { value: CSSMediaRule, writable: true, configurable: true });

            function createRuleObject(parsed, parentSheet, parentRule) {
                if (parsed.type === 1) {
                    const rule = Object.create(CSSStyleRule.prototype);
                    rule._type = 1;
                    rule._selectorText = parsed.selectorText || '';
                    rule._declarations = parsed.declarations || [];
                    rule._cssText = parsed.cssText;
                    rule._parentStyleSheet = parentSheet;
                    rule._parentRule = parentRule;

                    rule.__rebuildCssText = function() {
                        const decls = this.__getStyleCssText();
                        this._cssText = decls.length > 0
                            ? `${this._selectorText} { ${decls} }`
                            : `${this._selectorText} {}`;
                    };

                    rule.__notifyMutated = function() {
                        if (this._parentRule) this._parentRule.__notifyMutated();
                        else if (this._parentStyleSheet) this._parentStyleSheet.__notifyMutated();
                    };

                    rule._setCssText = function(val) {
                        const res = __cssomParseRule(String(val));
                        if (res && res.rule && res.rule.type === 1) {
                            this._selectorText = res.rule.selectorText || '';
                            this._declarations = res.rule.declarations || [];
                            this._cssText = res.rule.cssText;
                            this.__notifyMutated();
                        }
                    };

                    rule.__getStyleLength = function() { return this._declarations.length; };
                    rule.__getStyleItem = function(i) { return this._declarations[i] ? this._declarations[i].name : ''; };
                    rule.__getStyleValue = function(prop) {
                        const lower = String(prop).toLowerCase();
                        for (let i = this._declarations.length - 1; i >= 0; i--) {
                            if (this._declarations[i].name.toLowerCase() === lower) {
                                return this._declarations[i].value;
                            }
                        }
                        return '';
                    };
                    rule.__getStylePriority = function(prop) {
                        const lower = String(prop).toLowerCase();
                        for (let i = this._declarations.length - 1; i >= 0; i--) {
                            if (this._declarations[i].name.toLowerCase() === lower) {
                                return this._declarations[i].important ? 'important' : '';
                            }
                        }
                        return '';
                    };
                    rule.__setStyleProperty = function(prop, value, priority) {
                        const name = String(prop).trim().toLowerCase();
                        const val = String(value).trim();
                        if (!val) {
                            this.__removeStyleProperty(name);
                            return;
                        }
                        const important = String(priority).trim().toLowerCase() === 'important';
                        let found = false;
                        for (let i = 0; i < this._declarations.length; i++) {
                            if (this._declarations[i].name.toLowerCase() === name) {
                                this._declarations[i].value = val;
                                this._declarations[i].important = important;
                                found = true;
                                break;
                            }
                        }
                        if (!found) {
                            this._declarations.push({ name, value: val, important });
                        }
                        this.__rebuildCssText();
                        this.__notifyMutated();
                    };
                    rule.__removeStyleProperty = function(prop) {
                        const lower = String(prop).toLowerCase();
                        for (let i = 0; i < this._declarations.length; i++) {
                            if (this._declarations[i].name.toLowerCase() === lower) {
                                const old = this._declarations[i].value;
                                this._declarations.splice(i, 1);
                                this.__rebuildCssText();
                                this.__notifyMutated();
                                return old;
                            }
                        }
                        return '';
                    };
                    rule.__getStyleCssText = function() {
                        return this._declarations
                            .map(d => `${d.name}: ${d.value}${d.important ? ' !important' : ''};`)
                            .join(' ');
                    };
                    rule.__setStyleCssText = function(text) {
                        const dummy = __cssomParseRule(`* { ${text} }`);
                        if (dummy && dummy.rule) {
                            this._declarations = dummy.rule.declarations || [];
                        } else {
                            this._declarations = [];
                        }
                        this.__rebuildCssText();
                        this.__notifyMutated();
                    };

                    rule._styleDeclaration = globalThis.__createRuleStyle(rule);
                    return rule;
                } else if (parsed.type === 4) {
                    const rule = Object.create(CSSMediaRule.prototype);
                    rule._type = 4;
                    rule._conditionText = parsed.conditionText || '';
                    rule._mediaList = createMediaList(parsed.conditionText || '');
                    rule._parentStyleSheet = parentSheet;
                    rule._parentRule = parentRule;
                    rule._rules = (parsed.children || []).map(c => createRuleObject(c, parentSheet, rule));
                    rule._cssRulesList = createCSSRuleList(rule._rules);

                    rule.__notifyMutated = function() {
                        const childrenText = this._rules.map(r => r.cssText).join(' ');
                        this._cssText = childrenText ? `@media ${this._conditionText} { ${childrenText} }` : `@media ${this._conditionText} {}`;
                        if (this._parentRule) this._parentRule.__notifyMutated();
                        else if (this._parentStyleSheet) this._parentStyleSheet.__notifyMutated();
                    };
                    rule._cssText = parsed.cssText;
                    rule._setCssText = function(val) {
                        const res = __cssomParseRule(String(val));
                        if (res && res.rule && res.rule.type === 4) {
                            this._conditionText = res.rule.conditionText || '';
                            this._mediaList = createMediaList(this._conditionText);
                            this._rules = (res.rule.children || []).map(c => createRuleObject(c, this._parentStyleSheet, this));
                            this._cssRulesList = createCSSRuleList(this._rules);
                            this._cssText = res.rule.cssText;
                            this.__notifyMutated();
                        }
                    };
                    return rule;
                } else {
                    const rule = Object.create(CSSRule.prototype);
                    rule._type = parsed.type;
                    rule._cssText = parsed.cssText;
                    rule._parentStyleSheet = parentSheet;
                    rule._parentRule = parentRule;
                    rule._setCssText = function(val) {
                        this._cssText = String(val);
                        if (this._parentRule) this._parentRule.__notifyMutated();
                        else if (this._parentStyleSheet) this._parentStyleSheet.__notifyMutated();
                    };
                    return rule;
                }
            }

            function CSSRuleList() {
                illegalConstructor('CSSRuleList');
            }

            function createCSSRuleList(rulesArray) {
                const methods = Object.create(CSSRuleList.prototype);
                Object.defineProperties(methods, {
                    length: {
                        get() { return rulesArray.length; },
                        enumerable: true,
                        configurable: true,
                    },
                    item: {
                        value: function item(index) {
                            const idx = Number(index) >>> 0;
                            return rulesArray[idx] !== undefined ? rulesArray[idx] : null;
                        },
                        writable: true,
                        enumerable: true,
                        configurable: true,
                    },
                });

                return new Proxy(methods, {
                    get(target, prop, receiver) {
                        if (typeof prop === 'string' && /^(0|[1-9]\d*)$/.test(prop)) {
                            const idx = Number(prop);
                            return rulesArray[idx];
                        }
                        if (prop === Symbol.iterator) {
                            return function* () {
                                for (const item of rulesArray) yield item;
                            };
                        }
                        return Reflect.get(target, prop, receiver);
                    },
                    has(target, prop) {
                        if (typeof prop === 'string' && /^(0|[1-9]\d*)$/.test(prop)) {
                            return Number(prop) < rulesArray.length;
                        }
                        return Reflect.has(target, prop);
                    },
                    ownKeys(target) {
                        const keys = Reflect.ownKeys(target);
                        for (let i = 0; i < rulesArray.length; i++) {
                            keys.push(String(i));
                        }
                        return keys;
                    },
                    getOwnPropertyDescriptor(target, prop) {
                        if (typeof prop === 'string' && /^(0|[1-9]\d*)$/.test(prop)) {
                            const idx = Number(prop);
                            if (idx < rulesArray.length) {
                                return {
                                    value: rulesArray[idx],
                                    writable: false,
                                    enumerable: true,
                                    configurable: true,
                                };
                            }
                        }
                        return Reflect.getOwnPropertyDescriptor(target, prop);
                    },
                });
            }
            Object.defineProperty(CSSRuleList.prototype, Symbol.toStringTag, { value: 'CSSRuleList', configurable: true });
            Object.defineProperty(globalThis, 'CSSRuleList', { value: CSSRuleList, writable: true, configurable: true });

            function StyleSheet() {
                illegalConstructor('StyleSheet');
            }

            Object.defineProperties(StyleSheet.prototype, {
                type: {
                    get() {
                        if (!this || !(this instanceof StyleSheet)) throw new TypeError('Illegal invocation');
                        return 'text/css';
                    },
                    enumerable: true,
                    configurable: true,
                },
                href: {
                    get() {
                        if (!this || !(this instanceof StyleSheet)) throw new TypeError('Illegal invocation');
                        return this._href !== undefined ? this._href : null;
                    },
                    enumerable: true,
                    configurable: true,
                },
                ownerNode: {
                    get() {
                        if (!this || !(this instanceof StyleSheet)) throw new TypeError('Illegal invocation');
                        return this._ownerNode || null;
                    },
                    enumerable: true,
                    configurable: true,
                },
                parentStyleSheet: {
                    get() {
                        if (!this || !(this instanceof StyleSheet)) throw new TypeError('Illegal invocation');
                        return this._parentStyleSheet || null;
                    },
                    enumerable: true,
                    configurable: true,
                },
                title: {
                    get() {
                        if (!this || !(this instanceof StyleSheet)) throw new TypeError('Illegal invocation');
                        return this._title || '';
                    },
                    enumerable: true,
                    configurable: true,
                },
                media: {
                    get() {
                        if (!this || !(this instanceof StyleSheet)) throw new TypeError('Illegal invocation');
                        return this._mediaList;
                    },
                    enumerable: true,
                    configurable: true,
                },
                disabled: {
                    get() {
                        if (!this || !(this instanceof StyleSheet)) throw new TypeError('Illegal invocation');
                        return Boolean(this._disabled);
                    },
                    set(val) {
                        if (!this || !(this instanceof StyleSheet)) throw new TypeError('Illegal invocation');
                        this._disabled = Boolean(val);
                        this.__notifyMutated();
                    },
                    enumerable: true,
                    configurable: true,
                },
                [Symbol.toStringTag]: { value: 'StyleSheet', configurable: true },
            });
            Object.defineProperty(globalThis, 'StyleSheet', { value: StyleSheet, writable: true, configurable: true });

            function CSSStyleSheet(options) {
                if (!new.target) {
                    throw new TypeError("Failed to construct 'CSSStyleSheet': Please use the 'new' operator");
                }
                const opts = options || {};
                const mediaText = opts.media !== undefined ? String(opts.media) : '';
                const disabled = Boolean(opts.disabled);
                const baseURL = opts.baseURL !== undefined ? String(opts.baseURL) : '';

                this.__constructed = true;
                this._disabled = disabled;
                this._baseURL = baseURL;
                this._mediaList = createMediaList(mediaText);
                this._rules = [];
                this._cssRulesList = createCSSRuleList(this._rules);
                this._ownerNode = null;
                this._href = null;
                this._title = '';
                this._adopters = new Set();
            }
            Object.setPrototypeOf(CSSStyleSheet.prototype, StyleSheet.prototype);
            Object.setPrototypeOf(CSSStyleSheet, StyleSheet);

            CSSStyleSheet.prototype.__notifyMutated = function() {
                const fullCss = this._rules.map(r => r.cssText).join('\n');
                if (this._ownerNode) {
                    __cssomSyncElementSheet(this._ownerNode, fullCss);
                }
                for (const adopter of this._adopters) {
                    syncAdopterAdopted(adopter);
                }
            };

            function syncElementSheet(sheet) {
                if (!sheet || !sheet._ownerNode || sheet.__dynamicMutated) return;
                const currentText = __cssomGetConnectedText(sheet._ownerNode);
                if (sheet.__lastText !== currentText) {
                    sheet.__lastText = currentText;
                    const parsedRules = __cssomParseStylesheet(currentText);
                    for (const r of sheet._rules) {
                        r._parentStyleSheet = null;
                        r._parentRule = null;
                    }
                    sheet._rules.length = 0;
                    for (const p of parsedRules) {
                        sheet._rules.push(createRuleObject(p, sheet, null));
                    }
                }
            }

            Object.defineProperties(CSSStyleSheet.prototype, {
                ownerRule: {
                    get() {
                        if (!this || !(this instanceof CSSStyleSheet)) throw new TypeError('Illegal invocation');
                        return null;
                    },
                    enumerable: true,
                    configurable: true,
                },
                cssRules: {
                    get() {
                        if (!this || !(this instanceof CSSStyleSheet)) throw new TypeError('Illegal invocation');
                        syncElementSheet(this);
                        return this._cssRulesList;
                    },
                    enumerable: true,
                    configurable: true,
                },
                rules: {
                    get() {
                        if (!this || !(this instanceof CSSStyleSheet)) throw new TypeError('Illegal invocation');
                        syncElementSheet(this);
                        return this._cssRulesList;
                    },
                    enumerable: true,
                    configurable: true,
                },
                insertRule: {
                    value: function insertRule(rule, index = 0) {
                        if (!this || !(this instanceof CSSStyleSheet)) throw new TypeError('Illegal invocation');
                        syncElementSheet(this);
                        const idx = index === undefined ? 0 : Number(index);
                        if (isNaN(idx) || idx < 0 || idx > this._rules.length) {
                            throw new DOMException("Failed to execute 'insertRule': The index provided is outside the range.", "IndexSizeError");
                        }
                        const res = __cssomParseRule(String(rule));
                        if (res.error || !res.rule) {
                            throw new DOMException("Failed to execute 'insertRule': The rule provided could not be parsed.", "SyntaxError");
                        }
                        if (res.rule.type === 3) {
                            for (let i = 0; i < idx; i++) {
                                if (this._rules[i]._type !== 3 && this._rules[i]._type !== 2) {
                                    throw new DOMException("Failed to execute 'insertRule': An @import rule cannot be inserted after regular rules.", "HierarchyRequestError");
                                }
                            }
                        }
                        const newRule = createRuleObject(res.rule, this, null);
                        this._rules.splice(idx, 0, newRule);
                        this.__notifyMutated();
                        return idx;
                    },
                    writable: true,
                    enumerable: true,
                    configurable: true,
                },
                deleteRule: {
                    value: function deleteRule(index) {
                        if (!this || !(this instanceof CSSStyleSheet)) throw new TypeError('Illegal invocation');
                        syncElementSheet(this);
                        const idx = Number(index);
                        if (isNaN(idx) || idx < 0 || idx >= this._rules.length) {
                            throw new DOMException("Failed to execute 'deleteRule': The index provided is outside the range.", "IndexSizeError");
                        }
                        const removed = this._rules.splice(idx, 1)[0];
                        if (removed) {
                            removed._parentStyleSheet = null;
                            removed._parentRule = null;
                        }
                        this.__notifyMutated();
                    },
                    writable: true,
                    enumerable: true,
                    configurable: true,
                },
                replaceSync: {
                    value: function replaceSync(text) {
                        if (!this || !(this instanceof CSSStyleSheet)) throw new TypeError('Illegal invocation');
                        if (!this.__constructed) {
                            throw new DOMException("Failed to execute 'replaceSync': Can't call replaceSync on non-constructed stylesheet", "NotAllowedError");
                        }
                        const parsedRules = __cssomParseStylesheet(String(text));
                        for (const r of this._rules) {
                            r._parentStyleSheet = null;
                            r._parentRule = null;
                        }
                        this._rules = parsedRules.map(p => createRuleObject(p, this, null));
                        this._cssRulesList = createCSSRuleList(this._rules);
                        this.__notifyMutated();
                    },
                    writable: true,
                    enumerable: true,
                    configurable: true,
                },
                replace: {
                    value: function replace(text) {
                        if (!this || !(this instanceof CSSStyleSheet)) {
                            return Promise.reject(new TypeError('Illegal invocation'));
                        }
                        if (!this.__constructed) {
                            return Promise.reject(new DOMException("Failed to execute 'replace': Can't call replace on non-constructed stylesheet", "NotAllowedError"));
                        }
                        try {
                            this.replaceSync(text);
                            return Promise.resolve(this);
                        } catch (e) {
                            return Promise.reject(e);
                        }
                    },
                    writable: true,
                    enumerable: true,
                    configurable: true,
                },
                [Symbol.toStringTag]: { value: 'CSSStyleSheet', configurable: true },
            });
            Object.defineProperty(globalThis, 'CSSStyleSheet', { value: CSSStyleSheet, writable: true, configurable: true });

            function StyleSheetList() {
                illegalConstructor('StyleSheetList');
            }

            function createStyleSheetList(sheetsArray) {
                const methods = Object.create(StyleSheetList.prototype);
                Object.defineProperties(methods, {
                    length: {
                        get() { return sheetsArray.length; },
                        enumerable: true,
                        configurable: true,
                    },
                    item: {
                        value: function item(index) {
                            const idx = Number(index) >>> 0;
                            return sheetsArray[idx] !== undefined ? sheetsArray[idx] : null;
                        },
                        writable: true,
                        enumerable: true,
                        configurable: true,
                    },
                });

                return new Proxy(methods, {
                    get(target, prop, receiver) {
                        if (typeof prop === 'string' && /^(0|[1-9]\d*)$/.test(prop)) {
                            const idx = Number(prop);
                            return sheetsArray[idx];
                        }
                        if (prop === Symbol.iterator) {
                            return function* () {
                                for (const item of sheetsArray) yield item;
                            };
                        }
                        return Reflect.get(target, prop, receiver);
                    },
                    has(target, prop) {
                        if (typeof prop === 'string' && /^(0|[1-9]\d*)$/.test(prop)) {
                            return Number(prop) < sheetsArray.length;
                        }
                        return Reflect.has(target, prop);
                    },
                    ownKeys(target) {
                        const keys = Reflect.ownKeys(target);
                        for (let i = 0; i < sheetsArray.length; i++) {
                            keys.push(String(i));
                        }
                        return keys;
                    },
                    getOwnPropertyDescriptor(target, prop) {
                        if (typeof prop === 'string' && /^(0|[1-9]\d*)$/.test(prop)) {
                            const idx = Number(prop);
                            if (idx < sheetsArray.length) {
                                return {
                                    value: sheetsArray[idx],
                                    writable: false,
                                    enumerable: true,
                                    configurable: true,
                                };
                            }
                        }
                        return Reflect.getOwnPropertyDescriptor(target, prop);
                    },
                });
            }
            Object.defineProperty(StyleSheetList.prototype, Symbol.toStringTag, { value: 'StyleSheetList', configurable: true });
            Object.defineProperty(globalThis, 'StyleSheetList', { value: StyleSheetList, writable: true, configurable: true });

            const elementSheets = new WeakMap();

            function syncAdopterAdopted(adopter) {
                const sheets = adopter.__adoptedStyleSheets || [];
                const cssArray = sheets
                    .filter(s => !s.disabled)
                    .map(s => s._rules.map(r => r.cssText).join('\n'));
                __cssomSyncAdopted(adopter, cssArray);
            }

            function setupAdoptedStyleSheets(targetPrototype) {
                Object.defineProperty(targetPrototype, 'adoptedStyleSheets', {
                    get() {
                        if (!this) throw new TypeError('Illegal invocation');
                        return Object.freeze([...(this.__adoptedStyleSheets || [])]);
                    },
                    set(val) {
                        if (!this) throw new TypeError('Illegal invocation');
                        if (!val || typeof val[Symbol.iterator] !== 'function') {
                            throw new TypeError("Failed to set 'adoptedStyleSheets': Value is not iterable");
                        }
                        const sheets = [...val];
                        for (const s of sheets) {
                            if (!s || !(s instanceof CSSStyleSheet)) {
                                throw new TypeError("Failed to set 'adoptedStyleSheets': Value is not of type 'CSSStyleSheet'");
                            }
                            if (!s.__constructed) {
                                throw new DOMException("Failed to set 'adoptedStyleSheets': Only constructed stylesheets can be adopted", "NotAllowedError");
                            }
                        }
                        if (this.__adoptedStyleSheets) {
                            for (const s of this.__adoptedStyleSheets) {
                                s._adopters.delete(this);
                            }
                        }
                        this.__adoptedStyleSheets = sheets;
                        for (const s of sheets) {
                            s._adopters.add(this);
                        }
                        syncAdopterAdopted(this);
                    },
                    enumerable: true,
                    configurable: true,
                });
            }

            setupAdoptedStyleSheets(Document.prototype);
            if (globalThis.ShadowRoot) {
                setupAdoptedStyleSheets(ShadowRoot.prototype);
            }

            Object.defineProperty(HTMLStyleElement.prototype, 'sheet', {
                get() {
                    if (!this || !(this instanceof HTMLStyleElement)) {
                        throw new TypeError('Illegal invocation');
                    }
                    if (!this.isConnected) {
                        return null;
                    }
                    const type = this.getAttribute('type');
                    if (type != null && type.trim() !== '' && type.trim().toLowerCase() !== 'text/css') {
                        return null;
                    }
                    let sheet = elementSheets.get(this);
                    if (!sheet) {
                        sheet = Object.create(CSSStyleSheet.prototype);
                        sheet.__constructed = false;
                        sheet._disabled = false;
                        sheet._ownerNode = this;
                        sheet._href = null;
                        sheet._title = this.getAttribute('title') || '';
                        sheet._mediaList = createMediaList(this.getAttribute('media') || '');
                        sheet._adopters = new Set();
                        sheet._rules = [];
                        sheet._cssRulesList = createCSSRuleList(sheet._rules);
                        sheet.__lastText = null;
                        syncElementSheet(sheet);
                        elementSheets.set(this, sheet);
                    } else {
                        syncElementSheet(sheet);
                    }
                    return sheet;
                },
                enumerable: true,
                configurable: true,
            });

            Object.defineProperty(HTMLLinkElement.prototype, 'sheet', {
                get() {
                    if (!this || !(this instanceof HTMLLinkElement)) {
                        throw new TypeError('Illegal invocation');
                    }
                    if (!this.isConnected) {
                        return null;
                    }
                    const rel = this.getAttribute('rel') || '';
                    if (!rel.split(/\s+/).some(t => t.toLowerCase() === 'stylesheet')) {
                        return null;
                    }
                    let sheet = elementSheets.get(this);
                    if (!sheet) {
                        const href = this.href || this.getAttribute('href') || null;
                        sheet = Object.create(CSSStyleSheet.prototype);
                        sheet.__constructed = false;
                        sheet._disabled = false;
                        sheet._ownerNode = this;
                        sheet._href = href;
                        sheet._title = this.getAttribute('title') || '';
                        sheet._mediaList = createMediaList(this.getAttribute('media') || '');
                        sheet._adopters = new Set();
                        sheet._rules = [];
                        sheet._cssRulesList = createCSSRuleList(sheet._rules);
                        elementSheets.set(this, sheet);
                    }
                    return sheet;
                },
                enumerable: true,
                configurable: true,
            });

            Object.defineProperty(Document.prototype, 'styleSheets', {
                get() {
                    if (!this || !(this instanceof Document)) {
                        throw new TypeError('Illegal invocation');
                    }
                    const list = [];
                    const elements = this.querySelectorAll('style, link');
                    for (let i = 0; i < elements.length; i++) {
                        const s = elements[i].sheet;
                        if (s !== null) {
                            list.push(s);
                        }
                    }
                    return createStyleSheetList(list);
                },
                enumerable: true,
                configurable: true,
            });
        })();
        "#,
    ))?;

    Ok(())
}
