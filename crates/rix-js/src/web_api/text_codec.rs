
use super::*;
use encoding_rs::{CoderResult, Decoder, DecoderResult, Encoding};
use std::collections::HashMap;

pub(super) struct TextCodecRegistered;

#[derive(Default)]
struct TextDecoderRegistry {
    next_handle: u64,
    decoders: HashMap<u64, TextDecoderEntry>,
}

struct TextDecoderEntry {
    encoding: &'static Encoding,
    ignore_bom: bool,
    fatal: bool,
    decoder: Decoder,
}

fn new_decoder(encoding: &'static Encoding, ignore_bom: bool) -> Decoder {
    if ignore_bom {
        encoding.new_decoder_without_bom_handling()
    } else {
        encoding.new_decoder_with_bom_removal()
    }
}

impl TextDecoderRegistry {
    fn insert(&mut self, encoding: &'static Encoding, ignore_bom: bool, fatal: bool) -> u64 {
        self.next_handle += 1;
        let handle = self.next_handle;
        let decoder = new_decoder(encoding, ignore_bom);
        self.decoders.insert(
            handle,
            TextDecoderEntry {
                encoding,
                ignore_bom,
                fatal,
                decoder,
            },
        );
        handle
    }

    fn remove(&mut self, handle: u64) {
        self.decoders.remove(&handle);
    }
}

fn decoder_registry(context: &Context) -> JsResult<Rc<RefCell<TextDecoderRegistry>>> {
    context
        .get_data::<Rc<RefCell<TextDecoderRegistry>>>()
        .cloned()
        .ok_or_else(|| {
            JsNativeError::error()
                .with_message("TextDecoder support is not registered")
                .into()
        })
}

fn decode_chunk(entry: &mut TextDecoderEntry, bytes: &[u8], flush: bool) -> JsResult<String> {
    let mut output = String::with_capacity(bytes.len().saturating_mul(3).saturating_add(16));
    let mut offset = 0;
    loop {
        let consumed = if entry.fatal {
            match entry.decoder.decode_to_string_without_replacement(
                &bytes[offset..],
                &mut output,
                flush,
            ) {
                (DecoderResult::Malformed(..), _) => {
                    return Err(JsNativeError::typ()
                        .with_message("The encoded data was not valid.")
                        .into());
                }
                (DecoderResult::InputEmpty, read) => {
                    offset += read;
                    true
                }
                (DecoderResult::OutputFull, read) => {
                    offset += read;
                    false
                }
            }
        } else {
            let (result, read, _) =
                entry
                    .decoder
                    .decode_to_string(&bytes[offset..], &mut output, flush);
            offset += read;
            matches!(result, CoderResult::InputEmpty)
        };
        if consumed {
            return Ok(output);
        }
        output.reserve(16);
    }
}

fn text_canonical_encoding(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let label = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_lossy();
    match Encoding::for_label_no_replacement(label.as_bytes()) {
        Some(encoding) => Ok(JsString::from(encoding.name().to_ascii_lowercase()).into()),
        None => Ok(JsValue::undefined()),
    }
}

fn text_decoder_create(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let name = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_lossy();
    let ignore_bom = args.get_or_undefined(1).to_boolean();
    let fatal = args.get_or_undefined(2).to_boolean();
    let encoding = Encoding::for_label_no_replacement(name.as_bytes()).ok_or_else(|| {
        JsNativeError::range().with_message(format!(
            "The encoding label provided ('{name}') is invalid."
        ))
    })?;
    let registry = decoder_registry(context)?;
    let handle = registry.borrow_mut().insert(encoding, ignore_bom, fatal);
    Ok(JsValue::new(handle as f64))
}

fn text_decoder_decode(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let handle = args.get_or_undefined(0).to_number(context)?;
    let object = args
        .get_or_undefined(1)
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("decode expects a Uint8Array"))?;
    let bytes = JsUint8Array::from_object(object)?.to_vec(context)?;
    let flush = args.get_or_undefined(2).to_boolean();

    let registry = decoder_registry(context)?;
    let mut registry = registry.borrow_mut();
    let entry = registry.decoders.get_mut(&(handle as u64)).ok_or_else(|| {
        JsNativeError::typ().with_message("TextDecoder instance is no longer usable")
    })?;
    let text = decode_chunk(entry, &bytes, flush);
    if flush {
        entry.decoder = new_decoder(entry.encoding, entry.ignore_bom);
    }
    Ok(JsString::from(text?).into())
}

fn text_decoder_release(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let handle = args.get_or_undefined(0).to_number(context)? as u64;
    if let Some(registry) = context
        .get_data::<Rc<RefCell<TextDecoderRegistry>>>()
        .cloned()
    {
        registry.borrow_mut().remove(handle);
    }
    Ok(JsValue::undefined())
}

fn text_encode(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let text = args
        .get_or_undefined(0)
        .to_string(context)?
        .to_std_string_lossy();
    Ok(JsUint8Array::from_iter(text.into_bytes(), context)?.into())
}

const TEXT_CODEC_SCRIPT: &str = r#"
(function() {
    'use strict';
    const CanonicalEncoding = __rix_text_canonical_encoding;
    const DecoderCreate = __rix_text_decoder_create;
    const DecoderDecode = __rix_text_decoder_decode;
    const DecoderRelease = __rix_text_decoder_release;
    const EncodeUtf8 = __rix_text_encode;

    const weakMapGet = WeakMap.prototype.get;
    const weakMapSet = WeakMap.prototype.set;
    const typedArraySet = Uint8Array.prototype.set;
    const typedArraySubarray = Uint8Array.prototype.subarray;
    const getOwnPropertyDescriptor = Object.getOwnPropertyDescriptor;
    const typedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype);
    const typedArrayByteOffsetGetter = getOwnPropertyDescriptor(typedArrayPrototype, 'byteOffset').get;
    const typedArrayByteLengthGetter = getOwnPropertyDescriptor(typedArrayPrototype, 'byteLength').get;
    const typedArrayBufferGetter = getOwnPropertyDescriptor(typedArrayPrototype, 'buffer').get;
    const dataViewByteLengthGetter = getOwnPropertyDescriptor(DataView.prototype, 'byteLength').get;
    const dataViewByteOffsetGetter = getOwnPropertyDescriptor(DataView.prototype, 'byteOffset').get;
    const dataViewBufferGetter = getOwnPropertyDescriptor(DataView.prototype, 'buffer').get;
    const Uint8ArrayCtor = Uint8Array;
    const ArrayBufferCtor = ArrayBuffer;
    const TypeErrorCtor = TypeError;
    const RangeErrorCtor = RangeError;
    const FinalizationRegistryCtor =
        typeof FinalizationRegistry === 'function' ? FinalizationRegistry : undefined;

    const decoderSlots = new WeakMap();
    const decoderFinalizer = FinalizationRegistryCtor === undefined
        ? undefined
        : new FinalizationRegistryCtor(handle => DecoderRelease(handle));

    const typeError = message => new TypeErrorCtor(message);
    const rangeError = message => new RangeErrorCtor(message);

    function slot(value) {
        const state = weakMapGet.call(decoderSlots, value);
        if (state === undefined) throw typeError('Illegal invocation: TextDecoder');
        return state;
    }

    function bufferSourceView(value) {
        let buffer;
        let offset;
        let length;
        try {
            length = dataViewByteLengthGetter.call(value);
            offset = dataViewByteOffsetGetter.call(value);
            buffer = dataViewBufferGetter.call(value);
        } catch (error) {
            if (!(error instanceof TypeErrorCtor)) throw error;
            if (value instanceof ArrayBufferCtor) return new Uint8ArrayCtor(value);
            length = typedArrayByteLengthGetter.call(value);
            offset = typedArrayByteOffsetGetter.call(value);
            buffer = typedArrayBufferGetter.call(value);
        }
        return new Uint8ArrayCtor(buffer, offset, length);
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

    class TextDecoder {
        constructor(label = 'utf-8', options = {}) {
            const name = CanonicalEncoding(label);
            if (name === undefined) {
                throw rangeError(`Failed to construct 'TextDecoder': The encoding label provided ('${label}') is invalid.`);
            }
            const fatal = options !== null && options !== undefined && Boolean(options.fatal);
            const ignoreBOM = options !== null && options !== undefined && Boolean(options.ignoreBOM);
            const handle = DecoderCreate(name, ignoreBOM, fatal);
            weakMapSet.call(decoderSlots, this, {name, fatal, ignoreBOM, handle});
            if (decoderFinalizer !== undefined) decoderFinalizer.register(this, handle, this);
        }
        get encoding() { return slot(this).name; }
        get fatal() { return slot(this).fatal; }
        get ignoreBOM() { return slot(this).ignoreBOM; }
        decode(input = undefined, options = {}) {
            const state = slot(this);
            const stream = options !== null && options !== undefined && Boolean(options.stream);
            const bytes = input === undefined
                ? new Uint8ArrayCtor(0)
                : bufferSourceView(input);
            return DecoderDecode(state.handle, bytes, !stream);
        }
    }

    class TextEncoder {
        constructor() {}
        get encoding() { return 'utf-8'; }
        encode(input = '') { return EncodeUtf8(input); }
        encodeInto(source, destination) {
            if (!(destination instanceof Uint8ArrayCtor)) {
                throw typeError("Failed to execute 'encodeInto' on 'TextEncoder': parameter 2 is not of type 'Uint8Array'.");
            }
            const text = toUSVString(source);
            const bytes = EncodeUtf8(text);
            const capacity = destination.length;
            let read = 0;
            let written = 0;
            while (read < text.length) {
                const unit = text.charCodeAt(read);
                let codePoint = unit;
                let units = 1;
                if (unit >= 0xD800 && unit <= 0xDBFF && read + 1 < text.length) {
                    const next = text.charCodeAt(read + 1);
                    if (next >= 0xDC00 && next <= 0xDFFF) {
                        codePoint = 0x10000 + ((unit - 0xD800) << 10) + (next - 0xDC00);
                        units = 2;
                    }
                }
                const size = codePoint < 0x80 ? 1 : codePoint < 0x800 ? 2 : codePoint < 0x10000 ? 3 : 4;
                if (written + size > capacity) break;
                written += size;
                read += units;
            }
            if (written > 0) {
                typedArraySet.call(destination, typedArraySubarray.call(bytes, 0, written));
            }
            return {read, written};
        }
    }

    for (const [name, ctor] of [['TextDecoder', TextDecoder], ['TextEncoder', TextEncoder]]) {
        Object.defineProperty(ctor.prototype, Symbol.toStringTag, {value: name, configurable: true});
        Object.defineProperty(ctor.prototype, 'constructor', {value: ctor, writable: true, configurable: true});
        Object.defineProperty(globalThis, name, {value: ctor, writable: true, configurable: true});
    }
})();
"#;

pub(super) fn register_text_codec_apis(context: &mut Context) -> Result<(), JsError> {
    if context.get_data::<TextCodecRegistered>().is_some() {
        return Ok(());
    }
    context.insert_data(TextCodecRegistered);
    context.insert_data(Rc::new(RefCell::new(TextDecoderRegistry::default())));
    context.register_global_callable(
        js_string!("__rix_text_canonical_encoding"),
        1,
        NativeFunction::from_fn_ptr(text_canonical_encoding),
    )?;
    context.register_global_callable(
        js_string!("__rix_text_decoder_create"),
        3,
        NativeFunction::from_fn_ptr(text_decoder_create),
    )?;
    context.register_global_callable(
        js_string!("__rix_text_decoder_decode"),
        3,
        NativeFunction::from_fn_ptr(text_decoder_decode),
    )?;
    context.register_global_callable(
        js_string!("__rix_text_decoder_release"),
        1,
        NativeFunction::from_fn_ptr(text_decoder_release),
    )?;
    context.register_global_callable(
        js_string!("__rix_text_encode"),
        1,
        NativeFunction::from_fn_ptr(text_encode),
    )?;
    context.eval(boa_engine::Source::from_bytes(TEXT_CODEC_SCRIPT))?;
    Ok(())
}
