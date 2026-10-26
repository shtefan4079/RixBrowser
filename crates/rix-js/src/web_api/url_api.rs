use super::*;
use ::url::{Url, form_urlencoded};
use boa_engine::object::builtins::JsArray;

const URL_API_SCRIPT: &str = r#"
(() => {
    const parseURL = globalThis.__rix_url_parse;
    const getComponent = globalThis.__rix_url_get_component;
    const setComponent = globalThis.__rix_url_set_component;
    const parseParams = globalThis.__rix_url_parse_params;
    const serializeParams = globalThis.__rix_url_serialize_params;
    const createBlobURL = globalThis.__rix_url_create_blob_url;
    const revokeBlobURL = globalThis.__rix_url_revoke_blob_url;
    delete globalThis.__rix_url_parse;
    delete globalThis.__rix_url_get_component;
    delete globalThis.__rix_url_set_component;
    delete globalThis.__rix_url_parse_params;
    delete globalThis.__rix_url_serialize_params;
    delete globalThis.__rix_url_create_blob_url;
    delete globalThis.__rix_url_revoke_blob_url;

    const urlSlots = new WeakMap();
    const paramsSlots = new WeakMap();
    const iteratorSlots = new WeakMap();
    const apply = Reflect.apply;
    const weakMapGet = WeakMap.prototype.get;
    const weakMapSet = WeakMap.prototype.set;
    const iteratorSymbol = Symbol.iterator;

    function getSlot(slots, value, name) {
        const slot = apply(weakMapGet, slots, [value]);
        if (slot === undefined) throw new TypeError(`Illegal invocation: ${name}`);
        return slot;
    }

    function toUSVString(value) {
        const input = `${value}`;
        let output = '';
        for (let index = 0; index < input.length; index++) {
            const unit = input.charCodeAt(index);
            if (unit >= 0xD800 && unit <= 0xDBFF) {
                const next = input.charCodeAt(index + 1);
                if (next >= 0xDC00 && next <= 0xDFFF) {
                    output += input[index] + input[++index];
                } else {
                    output += '\uFFFD';
                }
            } else if (unit >= 0xDC00 && unit <= 0xDFFF) {
                output += '\uFFFD';
            } else {
                output += input[index];
            }
        }
        return output;
    }

    function searchParamsFromInit(init, hasArgument) {
        if (!hasArgument || init === undefined) return [];
        if (typeof init === 'string') return parseParams(toUSVString(init));
        if (init !== null && (typeof init === 'object' || typeof init === 'function')) {
            const iterator = init[iteratorSymbol];
            if (iterator !== undefined && iterator !== null) {
                if (typeof iterator !== 'function') throw new TypeError('The iterator property is not callable');
                const pairs = [];
                const sourceIterator = apply(iterator, init, []);
                const iterable = { [iteratorSymbol]() { return sourceIterator; } };
                for (const entry of iterable) {
                    const tuple = sequenceFromIterable(entry);
                    if (tuple.length !== 2) throw new TypeError('Each query parameter entry must contain exactly two values');
                    pairs[pairs.length] = [toUSVString(tuple[0]), toUSVString(tuple[1])];
                }
                return pairs;
            }
            const pairs = [];
            for (const key of Object.keys(init)) {
                pairs[pairs.length] = [key, toUSVString(init[key])];
            }
            return pairs;
        }
        return parseParams(toUSVString(init));
    }

    function sequenceFromIterable(value) {
        if (value === null || value === undefined) throw new TypeError('Query parameter entry must be iterable');
        const iterator = value[iteratorSymbol];
        if (iterator === undefined || iterator === null) throw new TypeError('Query parameter entry must be iterable');
        if (typeof iterator !== 'function') throw new TypeError('Query parameter entry iterator is not callable');
        const sourceIterator = apply(iterator, value, []);
        const iterable = { [iteratorSymbol]() { return sourceIterator; } };
        const values = [];
        for (const item of iterable) values[values.length] = item;
        return values;
    }

    function commitParams(state) {
        if (state.url) {
            state.url.href = setComponent(state.url.href, 'search', serializeParams(state.pairs));
        }
    }

    function refreshParams(state) {
        if (state.searchParams) {
            getSlot(paramsSlots, state.searchParams, 'URLSearchParams').pairs =
                parseParams(getComponent(state.href, 'search'));
        }
    }

    function createParamsIterator(params, kind) {
        const state = getSlot(paramsSlots, params, 'URLSearchParams');
        const iterator = Object.create(urlSearchParamsIteratorPrototype);
        apply(weakMapSet, iteratorSlots, [iterator, { state, kind, index: 0, done: false }]);
        return iterator;
    }

    const iteratorBase = Object.getPrototypeOf(Object.getPrototypeOf([][iteratorSymbol]()));
    const urlSearchParamsIteratorPrototype = Object.create(iteratorBase);
    Object.defineProperties(urlSearchParamsIteratorPrototype, {
        next: {
            value: function next() {
                const slot = getSlot(iteratorSlots, this, 'URLSearchParams Iterator');
                if (slot.done) return { value: undefined, done: true };
                const pair = slot.state.pairs[slot.index++];
                if (pair === undefined) {
                    slot.done = true;
                    return { value: undefined, done: true };
                }
                return { value: slot.kind === 0 ? [pair[0], pair[1]] : pair[slot.kind - 1], done: false };
            },
            writable: true,
            configurable: true,
        },
        [Symbol.toStringTag]: {
            value: 'URLSearchParams Iterator',
            configurable: true,
        },
    });

    class URLSearchParams {
        constructor(init = '') {
            apply(weakMapSet, paramsSlots, [this, { pairs: searchParamsFromInit(init, arguments.length !== 0), url: null }]);
        }

        append(name, value) {
            const state = getSlot(paramsSlots, this, 'URLSearchParams');
            if (arguments.length < 2) throw new TypeError('append requires two arguments');
            state.pairs.push([toUSVString(name), toUSVString(value)]);
            commitParams(state);
        }

        delete(name, value = undefined) {
            const state = getSlot(paramsSlots, this, 'URLSearchParams');
            if (arguments.length < 1) throw new TypeError('delete requires a name argument');
            name = toUSVString(name);
            const hasValue = arguments.length > 1;
            if (hasValue) value = toUSVString(value);
            state.pairs = state.pairs.filter(pair => pair[0] !== name || (hasValue && pair[1] !== value));
            commitParams(state);
        }

        get(name) {
            if (arguments.length < 1) throw new TypeError('get requires a name argument');
            name = toUSVString(name);
            const pair = getSlot(paramsSlots, this, 'URLSearchParams').pairs.find(pair => pair[0] === name);
            return pair === undefined ? null : pair[1];
        }

        getAll(name) {
            if (arguments.length < 1) throw new TypeError('getAll requires a name argument');
            name = toUSVString(name);
            return getSlot(paramsSlots, this, 'URLSearchParams').pairs
                .filter(pair => pair[0] === name).map(pair => pair[1]);
        }

        has(name, value) {
            if (arguments.length < 1) throw new TypeError('has requires a name argument');
            name = toUSVString(name);
            const pairs = getSlot(paramsSlots, this, 'URLSearchParams').pairs;
            if (arguments.length > 1) {
                value = toUSVString(value);
                return pairs.some(pair => pair[0] === name && pair[1] === value);
            }
            return pairs.some(pair => pair[0] === name);
        }

        set(name, value) {
            const state = getSlot(paramsSlots, this, 'URLSearchParams');
            if (arguments.length < 2) throw new TypeError('set requires two arguments');
            name = toUSVString(name);
            value = toUSVString(value);
            const first = state.pairs.findIndex(pair => pair[0] === name);
            if (first < 0) state.pairs.push([name, value]);
            else {
                state.pairs[first][1] = value;
                state.pairs = state.pairs.filter((pair, index) => pair[0] !== name || index === first);
            }
            commitParams(state);
        }

        sort() {
            const state = getSlot(paramsSlots, this, 'URLSearchParams');
            state.pairs = state.pairs.map((pair, index) => ({ pair, index }))
                .sort((a, b) => a.pair[0] < b.pair[0] ? -1 : a.pair[0] > b.pair[0] ? 1 : a.index - b.index)
                .map(item => item.pair);
            commitParams(state);
        }

        forEach(callback, thisArg = undefined) {
            if (arguments.length < 1) throw new TypeError('forEach requires a callback argument');
            if (typeof callback !== 'function') throw new TypeError('callback must be a function');
            const state = getSlot(paramsSlots, this, 'URLSearchParams');
            for (let index = 0; index < state.pairs.length; index++) {
                const [name, value] = state.pairs[index];
                apply(callback, thisArg, [value, name, this]);
            }
        }

        entries() {
            return createParamsIterator(this, 0);
        }

        keys() {
            return createParamsIterator(this, 1);
        }

        values() {
            return createParamsIterator(this, 2);
        }

        toString() {
            return serializeParams(getSlot(paramsSlots, this, 'URLSearchParams').pairs);
        }

        get size() {
            return getSlot(paramsSlots, this, 'URLSearchParams').pairs.length;
        }

        get [Symbol.toStringTag]() {
            return 'URLSearchParams';
        }

        [iteratorSymbol]() {
            return this.entries();
        }
    }

    class URL {
        constructor(input, base = undefined) {
            if (arguments.length === 0) throw new TypeError('URL requires an input');
            const hasBase = arguments.length > 1 && base !== undefined;
            const href = hasBase
                ? parseURL(toUSVString(input), toUSVString(base))
                : parseURL(toUSVString(input));
            apply(weakMapSet, urlSlots, [this, { href, searchParams: null }]);
        }

        get href() {
            return getSlot(urlSlots, this, 'URL').href;
        }

        set href(value) {
            const state = getSlot(urlSlots, this, 'URL');
            state.href = parseURL(toUSVString(value), state.href);
            refreshParams(state);
        }

        get origin() { return getURLComponent(this, 'origin'); }
        get protocol() { return getURLComponent(this, 'protocol'); }
        set protocol(value) { setURLComponent(this, 'protocol', value); }
        get username() { return getURLComponent(this, 'username'); }
        set username(value) { setURLComponent(this, 'username', value); }
        get password() { return getURLComponent(this, 'password'); }
        set password(value) { setURLComponent(this, 'password', value); }
        get host() { return getURLComponent(this, 'host'); }
        set host(value) { setURLComponent(this, 'host', value); }
        get hostname() { return getURLComponent(this, 'hostname'); }
        set hostname(value) { setURLComponent(this, 'hostname', value); }
        get port() { return getURLComponent(this, 'port'); }
        set port(value) { setURLComponent(this, 'port', value); }
        get pathname() { return getURLComponent(this, 'pathname'); }
        set pathname(value) { setURLComponent(this, 'pathname', value); }

        get search() { return getURLComponent(this, 'search'); }
        set search(value) { setURLComponent(this, 'search', value); }

        get hash() { return getURLComponent(this, 'hash'); }
        set hash(value) { setURLComponent(this, 'hash', value); }

        get searchParams() {
            const state = getSlot(urlSlots, this, 'URL');
            if (state.searchParams === null) {
                const params = new URLSearchParams(this.search);
                getSlot(paramsSlots, params, 'URLSearchParams').url = state;
                state.searchParams = params;
            }
            return state.searchParams;
        }

        toString() { return getSlot(urlSlots, this, 'URL').href; }
        toJSON() { return getSlot(urlSlots, this, 'URL').href; }
        get [Symbol.toStringTag]() { return 'URL'; }

        static parse(input, base = undefined) {
            if (arguments.length < 1) throw new TypeError('URL.parse requires an input');
            input = toUSVString(input);
            const hasBase = arguments.length > 1 && base !== undefined;
            if (hasBase) base = toUSVString(base);
            try { return hasBase ? new URL(input, base) : new URL(input); }
            catch (_) { return null; }
        }

        static canParse(input, base = undefined) {
            if (arguments.length < 1) throw new TypeError('URL.canParse requires an input');
            input = toUSVString(input);
            const hasBase = arguments.length > 1 && base !== undefined;
            if (hasBase) base = toUSVString(base);
            try { return hasBase ? !!new URL(input, base) : !!new URL(input); }
            catch (_) { return false; }
        }

        static createObjectURL(obj) {
            if (arguments.length === 0 || obj == null) {
                throw new TypeError('URL.createObjectURL requires 1 argument');
            }
            if (globalThis.MediaSource && obj instanceof globalThis.MediaSource) {
                if (typeof globalThis.__rixCreateMediaSourceUrl === 'function') {
                    return globalThis.__rixCreateMediaSourceUrl(obj);
                }
            }
            if (typeof createBlobURL !== 'function') {
                throw new TypeError('URL.createObjectURL is unavailable');
            }
            return createBlobURL(obj);
        }

        static revokeObjectURL(url) {
            if (arguments.length === 0) return;
            const strUrl = toUSVString(url);
            if (typeof globalThis.__rixRevokeMediaSourceUrl === 'function') {
                globalThis.__rixRevokeMediaSourceUrl(strUrl);
            }
            if (typeof revokeBlobURL === 'function') {
                revokeBlobURL(strUrl);
            }
        }
    }

    function getURLComponent(receiver, component) {
        return getComponent(getSlot(urlSlots, receiver, 'URL').href, component);
    }

    function setURLComponent(receiver, component, value) {
        const state = getSlot(urlSlots, receiver, 'URL');
        state.href = setComponent(state.href, component, toUSVString(value));
        refreshParams(state);
    }

    function makeWebIDLPropertiesEnumerable(object, excludedKeys) {
        for (const key of Reflect.ownKeys(object)) {
            if (excludedKeys.includes(key)) continue;
            const descriptor = Object.getOwnPropertyDescriptor(object, key);
            Object.defineProperty(object, key, { ...descriptor, enumerable: true });
        }
    }

    Object.defineProperty(URLSearchParams.prototype, 'constructor', { value: URLSearchParams, writable: true, configurable: true });
    Object.defineProperty(URLSearchParams.prototype, Symbol.toStringTag, { value: 'URLSearchParams', configurable: true });
    Object.defineProperty(URL.prototype, 'constructor', { value: URL, writable: true, configurable: true });
    Object.defineProperty(URL.prototype, Symbol.toStringTag, { value: 'URL', configurable: true });
    Object.defineProperty(URLSearchParams.prototype, Symbol.iterator, { value: URLSearchParams.prototype.entries, writable: true, configurable: true });
    makeWebIDLPropertiesEnumerable(URLSearchParams.prototype, ['constructor', Symbol.toStringTag]);
    makeWebIDLPropertiesEnumerable(URL.prototype, ['constructor', Symbol.toStringTag]);
    makeWebIDLPropertiesEnumerable(URL, ['length', 'name', 'prototype']);
    Object.defineProperty(globalThis, 'URL', { value: URL, configurable: true, writable: true });
    Object.defineProperty(globalThis, 'URLSearchParams', { value: URLSearchParams, configurable: true, writable: true });
})();
"#;

pub(super) fn register_url_apis(context: &mut Context) -> Result<(), JsError> {
    context.register_global_callable(
        js_string!("__rix_url_parse"),
        2,
        NativeFunction::from_fn_ptr(url_parse),
    )?;
    context.register_global_callable(
        js_string!("__rix_url_get_component"),
        2,
        NativeFunction::from_fn_ptr(url_get_component),
    )?;
    context.register_global_callable(
        js_string!("__rix_url_set_component"),
        3,
        NativeFunction::from_fn_ptr(url_set_component),
    )?;
    context.register_global_callable(
        js_string!("__rix_url_parse_params"),
        1,
        NativeFunction::from_fn_ptr(url_parse_params),
    )?;
    context.register_global_callable(
        js_string!("__rix_url_serialize_params"),
        1,
        NativeFunction::from_fn_ptr(url_serialize_params),
    )?;
    context.register_global_callable(
        js_string!("__rix_url_create_blob_url"),
        1,
        NativeFunction::from_fn_ptr(url_create_blob_url),
    )?;
    context.register_global_callable(
        js_string!("__rix_url_revoke_blob_url"),
        1,
        NativeFunction::from_fn_ptr(url_revoke_blob_url),
    )?;
    context.eval(boa_engine::Source::from_bytes(URL_API_SCRIPT))?;
    Ok(())
}

fn url_create_blob_url(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let obj = args.get_or_undefined(0);
    if obj.is_null_or_undefined() {
        return Err(JsNativeError::typ()
            .with_message("URL.createObjectURL: Argument 1 is required")
            .into());
    }
    let obj_obj = obj.as_object().ok_or_else(|| {
        JsNativeError::typ().with_message("URL.createObjectURL: Argument 1 must be an object")
    })?;

    let blob_internals = context
        .get_data::<crate::web_api::blob::BlobInternals>()
        .cloned()
        .ok_or_else(|| {
            JsNativeError::typ().with_message("URL.createObjectURL: Blob internals unavailable")
        })?;

    let snapshot_res = blob_internals.snapshot.call(
        &JsValue::undefined(),
        &[obj.clone()],
        context,
    );
    let snapshot_val = match snapshot_res {
        Ok(v) => v,
        Err(_) => {
            return Err(JsNativeError::typ()
                .with_message("URL.createObjectURL: Argument 1 is not a Blob")
                .into());
        }
    };

    let snapshot_obj = snapshot_val.as_object().ok_or_else(|| {
        JsNativeError::typ().with_message("URL.createObjectURL: Argument 1 is not a Blob")
    })?;

    let bytes = boa_engine::object::builtins::JsUint8Array::from_object(snapshot_obj.clone())?
        .to_vec(context)?;

    let mime_type = match obj_obj.get(js_string!("type"), context) {
        Ok(v) => v.to_string(context)?.to_std_string_escaped(),
        Err(_) => String::new(),
    };

    let origin = context
        .get_data::<Rc<RefCell<crate::web_api::WebApiState>>>()
        .and_then(|state| state.borrow().current_url.as_ref().map(|u| u.inner().origin().ascii_serialization()))
        .unwrap_or_else(|| "null".to_string());

    let store_rc = context
        .get_data::<Rc<RefCell<crate::web_api::blob::BlobUrlStore>>>()
        .cloned()
        .unwrap_or_else(|| {
            let store = Rc::new(RefCell::new(crate::web_api::blob::BlobUrlStore::new()));
            context.insert_data(store.clone());
            store
        });

    let url_str = store_rc
        .borrow_mut()
        .create_blob_url(bytes, mime_type, Some(&origin));

    Ok(js_string!(url_str).into())
}

fn url_revoke_blob_url(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let url_val = args.get_or_undefined(0);
    if url_val.is_null_or_undefined() {
        return Ok(JsValue::undefined());
    }
    let url_str = url_val.to_string(context)?.to_std_string_escaped();
    if let Some(store_rc) = context
        .get_data::<Rc<RefCell<crate::web_api::blob::BlobUrlStore>>>()
        .cloned()
    {
        store_rc.borrow_mut().revoke_blob_url(&url_str);
    }
    Ok(JsValue::undefined())
}

fn url_parse(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let input = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let parsed = match args.get(1) {
        Some(base) if !base.is_undefined() => {
            let base = base.to_string(context)?.to_std_string_escaped();
            Url::parse(&base)
                .and_then(|base| base.join(&input))
                .map_err(|error| JsNativeError::typ().with_message(error.to_string()))?
        }
        _ => Url::parse(&input)
            .map_err(|error| JsNativeError::typ().with_message(error.to_string()))?,
    };
    Ok(js_string!(parsed.as_str()).into())
}

fn url_get_component(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let url = parse_argument_url(args, context)?;
    let component = args
        .get_or_undefined(1)
        .to_string(context)?
        .to_std_string_escaped();
    let value = match component.as_str() {
        "href" => url.as_str().to_owned(),
        "origin" => url.origin().ascii_serialization(),
        "protocol" => format!("{}:", url.scheme()),
        "username" => url.username().to_owned(),
        "password" => url.password().unwrap_or_default().to_owned(),
        "host" => host_with_port(&url),
        "hostname" => url.host_str().unwrap_or_default().to_owned(),
        "port" => url.port().map(|port| port.to_string()).unwrap_or_default(),
        "pathname" => url.path().to_owned(),
        "search" => url
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default(),
        "hash" => url
            .fragment()
            .map(|fragment| format!("#{fragment}"))
            .unwrap_or_default(),
        _ => {
            return Err(JsNativeError::typ()
                .with_message("Unknown URL component")
                .into());
        }
    };
    Ok(js_string!(value).into())
}

fn url_set_component(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let mut url = parse_argument_url(args, context)?;
    let component = args
        .get_or_undefined(1)
        .to_string(context)?
        .to_std_string_escaped();
    let value = args
        .get_or_undefined(2)
        .to_string(context)?
        .to_std_string_escaped();
    match component.as_str() {
        "protocol" => {
            let scheme = value.strip_suffix(':').unwrap_or(&value);
            if url.set_scheme(scheme).is_err() {
                return Ok(js_string!(url.as_str()).into());
            }
        }
        "username" => {
            if url.set_username(&value).is_err() {
                return Ok(js_string!(url.as_str()).into());
            }
        }
        "password" => {
            if url.set_password(Some(&value)).is_err() {
                return Ok(js_string!(url.as_str()).into());
            }
        }
        "host" => set_host_component(&mut url, &value),
        "hostname" => {
            if url.set_host(Some(&value)).is_err() {
                return Ok(js_string!(url.as_str()).into());
            }
        }
        "port" => {
            let port = if value.is_empty() {
                None
            } else {
                value.parse::<u16>().ok()
            };
            if value.is_empty() || port.is_some() {
                if url.set_port(port).is_err() {
                    return Ok(js_string!(url.as_str()).into());
                }
            }
        }
        "pathname" => url.set_path(&value),
        "search" => url.set_query(if value.is_empty() {
            None
        } else {
            Some(value.strip_prefix('?').unwrap_or(&value))
        }),
        "hash" => url.set_fragment(if value.is_empty() {
            None
        } else {
            Some(value.strip_prefix('#').unwrap_or(&value))
        }),
        _ => {
            return Err(JsNativeError::typ()
                .with_message("Unknown URL component")
                .into());
        }
    }
    Ok(js_string!(url.as_str()).into())
}

fn url_parse_params(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let input = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let input = input.strip_prefix('?').unwrap_or(&input);
    let mut pairs = Vec::new();
    for (name, value) in form_urlencoded::parse(input.as_bytes()) {
        let pair = JsArray::from_iter(
            [
                JsValue::from(js_string!(name)),
                JsValue::from(js_string!(value)),
            ],
            context,
        );
        pairs.push(pair.into());
    }
    Ok(JsArray::from_iter(pairs, context).into())
}

fn url_serialize_params(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(pairs) = args.get_or_undefined(0).as_object() else {
        return Err(JsNativeError::typ()
            .with_message("URLSearchParams entries must be an array")
            .into());
    };
    let length = pairs
        .get(js_string!("length"), context)?
        .to_length(context)?;
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for index in 0..length {
        let entry = pairs.get(index, context)?;
        let Some(entry) = entry.as_object() else {
            return Err(JsNativeError::typ()
                .with_message("URLSearchParams entry must be a pair")
                .into());
        };
        let name = entry
            .get(0, context)?
            .to_string(context)?
            .to_std_string_escaped();
        let value = entry
            .get(1, context)?
            .to_string(context)?
            .to_std_string_escaped();
        serializer.append_pair(&name, &value);
    }
    Ok(js_string!(serializer.finish()).into())
}

fn parse_argument_url(args: &[JsValue], context: &mut Context) -> JsResult<Url> {
    let input = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    Url::parse(&input).map_err(|error| JsNativeError::typ().with_message(error.to_string()).into())
}

fn host_with_port(url: &Url) -> String {
    let mut host = url.host_str().unwrap_or_default().to_owned();
    if let Some(port) = url.port() {
        host.push(':');
        host.push_str(&port.to_string());
    }
    host
}

fn set_host_component(url: &mut Url, value: &str) {
    let Ok(candidate) = Url::parse(&format!("{}://{value}/", url.scheme())) else {
        return;
    };
    if candidate.username() != ""
        || candidate.password().is_some()
        || candidate.path() != "/"
        || candidate.query().is_some()
        || candidate.fragment().is_some()
    {
        return;
    }
    if url.set_host(candidate.host_str()).is_err() {
        return;
    }
    if host_value_has_port(value) {
        if url.set_port(candidate.port()).is_err() {
            return;
        }
    }
}

fn host_value_has_port(value: &str) -> bool {
    match value.rfind(']') {
        Some(ipv6_end) => value
            .get(ipv6_end + 1..)
            .is_some_and(|suffix| suffix.starts_with(':')),
        None => value.contains(':'),
    }
}
