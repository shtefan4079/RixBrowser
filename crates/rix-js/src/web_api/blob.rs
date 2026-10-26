use super::*;
use std::borrow::Cow;

const MAX_BLOB_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct BlobInternals {
    pub(crate) snapshot: JsObject,
    pub(crate) decode: JsObject,
}

#[derive(Debug, Clone)]
pub struct StoredBlob {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

#[derive(Debug, Clone, Default)]
pub struct BlobUrlStore {
    pub urls: HashMap<String, StoredBlob>,
    pub next_id: u64,
}

impl BlobUrlStore {
    pub fn new() -> Self {
        Self {
            urls: HashMap::new(),
            next_id: 1,
        }
    }

    pub fn create_blob_url(
        &mut self,
        bytes: Vec<u8>,
        mime_type: String,
        origin: Option<&str>,
    ) -> String {
        let counter = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        let uuid = generate_blob_uuid(counter);
        let origin_str = origin.unwrap_or("null");
        let url = format!("blob:{origin_str}/{uuid}");
        self.urls.insert(url.clone(), StoredBlob { bytes, mime_type });
        url
    }

    pub fn revoke_blob_url(&mut self, url: &str) {
        self.urls.remove(url);
    }

    pub fn resolve_blob_url(&self, url: &str) -> Option<StoredBlob> {
        self.urls.get(url).cloned()
    }
}

pub(crate) fn generate_blob_uuid(counter: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&now.to_le_bytes()[..8]);
    bytes[8..].copy_from_slice(&counter.to_le_bytes()[..8]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    )
}

const BLOB_API_SCRIPT: &str = r#"
(() => {
    const encodeString = globalThis.__rix_blob_encode_string;
    const decodeBytes = globalThis.__rix_blob_decode_bytes;
    const getMaximumSize = globalThis.__rix_blob_maximum_size;
    delete globalThis.__rix_blob_encode_string;
    delete globalThis.__rix_blob_decode_bytes;
    delete globalThis.__rix_blob_maximum_size;

    const slots = new WeakMap();
    const maximumSize = getMaximumSize();
    const apply = Reflect.apply;
    const weakMapGet = WeakMap.prototype.get;
    const weakMapSet = WeakMap.prototype.set;
    const weakMapHas = WeakMap.prototype.has;
    const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
    const createObject = Object.create;
    const ArrayBufferConstructor = ArrayBuffer;
    const arrayBufferIsView = ArrayBuffer.isView;
    const arrayBufferByteLengthGetter = getOwnPropertyDescriptor(ArrayBuffer.prototype, 'byteLength').get;
    const Uint8ArrayConstructor = Uint8Array;
    const Uint8ArraySet = Uint8Array.prototype.set;
    const typedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype);
    const typedArrayByteLengthGetter = getOwnPropertyDescriptor(typedArrayPrototype, 'byteLength').get;
    const typedArrayByteOffsetGetter = getOwnPropertyDescriptor(typedArrayPrototype, 'byteOffset').get;
    const typedArrayBufferGetter = getOwnPropertyDescriptor(typedArrayPrototype, 'buffer').get;
    const typedArrayLengthGetter = getOwnPropertyDescriptor(typedArrayPrototype, 'length').get;
    const dataViewByteLengthGetter = getOwnPropertyDescriptor(DataView.prototype, 'byteLength').get;
    const dataViewByteOffsetGetter = getOwnPropertyDescriptor(DataView.prototype, 'byteOffset').get;
    const dataViewBufferGetter = getOwnPropertyDescriptor(DataView.prototype, 'buffer').get;
    const hasInstance = Function.prototype[Symbol.hasInstance];
    const arrayBufferTypeError = TypeError;
    const arrayBufferRangeError = RangeError;
    const promiseResolve = Promise.resolve;
    const promiseThen = Promise.prototype.then;
    const PromiseConstructor = Promise;
    const toLowerCase = String.prototype.toLowerCase;
    const typeError = message => new arrayBufferTypeError(message);
    const rangeError = message => new arrayBufferRangeError(message);
    const isArrayBufferTypeError = error => apply(hasInstance, arrayBufferTypeError, [error]);

    function slot(value) {
        const state = apply(weakMapGet, slots, [value]);
        if (state === undefined) throw typeError('Illegal invocation: Blob');
        return state;
    }

    function byteLength(value) {
        return apply(typedArrayLengthGetter, value, []);
    }

    function copyViewBytes(buffer, offset, length) {
        const source = new Uint8ArrayConstructor(buffer, offset, length);
        return copyBytes(source, 0, length);
    }

    function copyBytes(source, offset, length) {
        const copy = new Uint8ArrayConstructor(length);
        const buffer = apply(typedArrayBufferGetter, source, []);
        const sourceOffset = apply(typedArrayByteOffsetGetter, source, []);
        const sourceView = new Uint8ArrayConstructor(buffer, sourceOffset + offset, length);
        apply(Uint8ArraySet, copy, [sourceView]);
        return copy;
    }

    function copyBufferSource(value) {
        let buffer;
        let offset;
        let length;
        try {
            length = apply(dataViewByteLengthGetter, value, []);
            offset = apply(dataViewByteOffsetGetter, value, []);
            buffer = apply(dataViewBufferGetter, value, []);
        } catch (error) {
            if (!isArrayBufferTypeError(error)) throw error;
            length = apply(typedArrayByteLengthGetter, value, []);
            offset = apply(typedArrayByteOffsetGetter, value, []);
            buffer = apply(typedArrayBufferGetter, value, []);
        }
        return copyViewBytes(buffer, offset, length);
    }

    function arrayBufferByteLength(value) {
        try {
            return apply(arrayBufferByteLengthGetter, value, []);
        } catch (error) {
            if (isArrayBufferTypeError(error)) return undefined;
            throw error;
        }
    }

    function copyArrayBuffer(value, length) {
        const source = new Uint8ArrayConstructor(value);
        return copyViewBytes(apply(typedArrayBufferGetter, source, []), 0, length);
    }

    function resolveLater(callback) {
        const promise = apply(promiseResolve, PromiseConstructor, [undefined]);
        return apply(promiseThen, promise, [callback]);
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

    function normalizeType(value) {
        const type = toUSVString(value);
        const length = type.length;
        for (let index = 0; index < length; index++) {
            const unit = type.charCodeAt(index);
            if (unit < 0x20 || unit > 0x7E) return '';
        }
        return apply(toLowerCase, type, []);
    }

    function normalizeInteger(value, fallback) {
        if (value === undefined) return fallback;
        const number = +value;
        if (Number.isNaN(number) || number === 0) return 0;
        if (!Number.isFinite(number)) return number;
        return Math.trunc(number);
    }

    function relativeIndex(value, size) {
        if (value < 0) return Math.max(size + value, 0);
        return Math.min(value, size);
    }

    function makeBlob(bytes, type) {
        const blob = createObject(Blob.prototype);
        apply(weakMapSet, slots, [blob, {bytes: copyBytes(bytes, 0, byteLength(bytes)), type}]);
        return blob;
    }

    class Blob {
        constructor(blobParts = [], options = {}) {
            const convertedParts = [];
            for (const part of blobParts) {
                if (apply(weakMapHas, slots, [part]) ||
                    apply(arrayBufferIsView, ArrayBufferConstructor, [part]) ||
                    (part !== null && (typeof part === 'object' || typeof part === 'function') &&
                        arrayBufferByteLength(part) !== undefined)) {
                    convertedParts[convertedParts.length] = part;
                } else {
                    convertedParts[convertedParts.length] = toUSVString(part);
                }
            }
            const typeMember = options == null ? undefined : options.type;
            const type = typeMember === undefined ? '' : normalizeType(typeMember);
            const endingsMember = options == null ? undefined : options.endings;
            const endings = endingsMember === undefined ? 'transparent' : `${endingsMember}`;
            if (endings !== 'transparent' && endings !== 'native') {
                throw typeError('Blob endings must be "transparent" or "native"');
            }
            const parts = [];
            let size = 0;
            for (let index = 0; index < convertedParts.length; index++) {
                const part = convertedParts[index];
                let bytes;
                if (apply(weakMapHas, slots, [part])) {
                    const sourceBytes = slot(part).bytes;
                    const partSize = byteLength(sourceBytes);
                    if (partSize > maximumSize - size) {
                        throw rangeError('Blob exceeds the per-object byte budget');
                    }
                    bytes = copyBytes(sourceBytes, 0, partSize);
                } else if (apply(arrayBufferIsView, ArrayBufferConstructor, [part])) {
                    const sourceSize = (() => {
                        try { return apply(dataViewByteLengthGetter, part, []); }
                        catch (error) {
                            if (!isArrayBufferTypeError(error)) throw error;
                            return apply(typedArrayByteLengthGetter, part, []);
                        }
                    })();
                    if (sourceSize > maximumSize - size) {
                        throw rangeError('Blob exceeds the per-object byte budget');
                    }
                    bytes = copyBufferSource(part);
                } else {
                    const bufferSize = part !== null && (typeof part === 'object' || typeof part === 'function')
                        ? arrayBufferByteLength(part)
                        : undefined;
                    if (bufferSize !== undefined) {
                        if (bufferSize > maximumSize - size) {
                            throw rangeError('Blob exceeds the per-object byte budget');
                        }
                        bytes = copyArrayBuffer(part, bufferSize);
                    } else {
                        bytes = encodeString(toUSVString(part), endings === 'native');
                    }
                }
                const partSize = byteLength(bytes);
                if (partSize > maximumSize - size) {
                    throw rangeError('Blob exceeds the per-object byte budget');
                }
                size += partSize;
                parts[parts.length] = bytes;
            }
            const bytes = new Uint8ArrayConstructor(size);
            let offset = 0;
            for (let partIndex = 0; partIndex < parts.length; partIndex++) {
                const part = parts[partIndex];
                const partSize = byteLength(part);
                apply(Uint8ArraySet, bytes, [part, offset]);
                offset += partSize;
            }
            apply(weakMapSet, slots, [this, {bytes, type}]);
        }

        get size() { return byteLength(slot(this).bytes); }
        get type() { return slot(this).type; }

        slice(start = 0, end = undefined, contentType = '') {
            const state = slot(this);
            const size = byteLength(state.bytes);
            const startInteger = normalizeInteger(start, 0);
            const endInteger = normalizeInteger(end, size);
            const type = normalizeType(contentType);
            const relativeStart = relativeIndex(startInteger, size);
            const relativeEnd = relativeIndex(endInteger, size);
            const span = Math.max(relativeEnd - relativeStart, 0);
            const bytes = copyBytes(state.bytes, relativeStart, span);
            return makeBlob(bytes, type);
        }

        text() {
            const bytes = copyBytes(slot(this).bytes, 0, byteLength(slot(this).bytes));
            return resolveLater(() => decodeBytes(bytes));
        }

        arrayBuffer() {
            const bytes = copyBytes(slot(this).bytes, 0, byteLength(slot(this).bytes));
            return resolveLater(() => apply(typedArrayBufferGetter, bytes, []));
        }
    }

    for (const name of ['size', 'type', 'slice', 'text', 'arrayBuffer']) {
        const descriptor = getOwnPropertyDescriptor(Blob.prototype, name);
        Object.defineProperty(Blob.prototype, name, {...descriptor, enumerable: true});
    }
    Object.defineProperty(Blob.prototype, 'constructor', {
        value: Blob, writable: true, configurable: true
    });
    Object.defineProperty(Blob.prototype, Symbol.toStringTag, {
        value: 'Blob', configurable: true
    });
    Object.defineProperty(globalThis, 'Blob', {
        value: Blob, writable: true, configurable: true
    });
    return {
        snapshot(value) {
            const bytes = slot(value).bytes;
            return copyBytes(bytes, 0, byteLength(bytes));
        },
        decode(bytes) { return decodeBytes(bytes); }
    };
})();
"#;

pub(super) fn register_blob_apis(context: &mut Context) -> Result<(), JsError> {
    context.register_global_callable(
        js_string!("__rix_blob_encode_string"),
        2,
        NativeFunction::from_fn_ptr(blob_encode_string),
    )?;
    context.register_global_callable(
        js_string!("__rix_blob_decode_bytes"),
        1,
        NativeFunction::from_fn_ptr(blob_decode_bytes),
    )?;
    context.register_global_callable(
        js_string!("__rix_blob_maximum_size"),
        0,
        NativeFunction::from_fn_ptr(blob_maximum_size),
    )?;
    let internals_value = context.eval(boa_engine::Source::from_bytes(BLOB_API_SCRIPT))?;
    let internals = internals_value
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("Blob internals were not registered"))?;
    let snapshot = internals
        .get(js_string!("snapshot"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("Blob snapshot was not registered"))?;
    let decode = internals
        .get(js_string!("decode"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("Blob decoder was not registered"))?;
    context.insert_data(BlobInternals { snapshot, decode });
    context.insert_data(Rc::new(RefCell::new(BlobUrlStore::new())));
    Ok(())
}

fn blob_encode_string(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let input = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_escaped();
    let endings = args.get_or_undefined(1).to_boolean();
    let normalized = if endings {
        normalize_native_line_endings(&input)
    } else {
        Cow::Borrowed(input.as_str())
    };
    if normalized.len() > MAX_BLOB_BYTES {
        return Err(JsNativeError::range()
            .with_message("Blob part exceeds the per-object byte budget")
            .into());
    }
    Ok(JsUint8Array::from_iter(normalized.as_bytes().iter().copied(), context)?.into())
}

fn blob_decode_bytes(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let object = args
        .get_or_undefined(0)
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("Blob decoder expects a Uint8Array"))?;
    let bytes = JsUint8Array::from_object(object)?.to_vec(context)?;
    Ok(js_string!(String::from_utf8_lossy(&bytes).into_owned()).into())
}

fn blob_maximum_size(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    Ok(JsValue::new(MAX_BLOB_BYTES))
}

fn normalize_native_line_endings(value: &str) -> Cow<'_, str> {
    #[cfg(windows)]
    {
        Cow::Owned(
            value
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .replace('\n', "\r\n"),
        )
    }
    #[cfg(not(windows))]
    {
        if value.contains('\r') {
            Cow::Owned(value.replace("\r\n", "\n").replace('\r', "\n"))
        } else {
            Cow::Borrowed(value)
        }
    }
}
