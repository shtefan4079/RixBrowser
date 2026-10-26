use super::*;

#[derive(Clone)]
pub(super) struct ResponseFactory(pub(super) JsObject);

const RESPONSE_API_SCRIPT: &str = r#"
(() => {
    const snapshotBlob = globalThis.__rix_response_snapshot_blob;
    const decodeBytes = globalThis.__rix_response_decode_bytes;
    delete globalThis.__rix_response_snapshot_blob;
    delete globalThis.__rix_response_decode_bytes;

    const BlobConstructor = Blob;
    const URLConstructor = globalThis.URL;
    const responseSlots = new WeakMap();
    const requestSlots = new WeakMap();
    const headerSlots = new WeakMap();
    const apply = Reflect.apply;
    const promiseResolve = Promise.resolve;
    const promiseReject = Promise.reject;
    const promiseThen = Promise.prototype.then;
    const PromiseConstructor = Promise;
    const jsonParse = JSON.parse;
    const jsonStringify = JSON.stringify;
    const stringConstructor = String;
    const objectKeys = Object.keys;
    const objectCreate = Object.create;
    const Uint8ArraySet = Uint8Array.prototype.set;
    const typedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype);
    const typedArrayLengthGetter = Object.getOwnPropertyDescriptor(typedArrayPrototype, 'length').get;
    const arrayFrom = Array.from;
    const arrayIterator = Array.prototype[Symbol.iterator];
    const arrayPush = Array.prototype.push;
    const arraySplice = Array.prototype.splice;
    const arraySort = Array.prototype.sort;
    const stringReplace = String.prototype.replace;
    const weakMapGet = WeakMap.prototype.get;
    const weakMapSet = WeakMap.prototype.set;
    const weakMapHas = WeakMap.prototype.has;
    const regexpTest = RegExp.prototype.test;
    const maximumHeaders = 256;
    const maximumHeaderBytes = 65536;

    function typeError(message) { return new TypeError(message); }

    function domString(value) {
        if (typeof value === 'symbol') throw typeError('Cannot convert a Symbol to a DOMString');
        return stringConstructor(value);
    }

    function byteString(value, label) {
        const result = domString(value);
        for (let index = 0; index < result.length; index++) {
            if (result.charCodeAt(index) > 255) throw typeError(label + ' is not a ByteString');
        }
        return result;
    }

    function copyBytes(source) {
        const length = apply(typedArrayLengthGetter, source, []);
        const result = new Uint8Array(length);
        apply(Uint8ArraySet, result, [source]);
        return result;
    }

    function normalizeHeaderName(value) {
        const name = byteString(value, 'Header name');
        if (!apply(regexpTest, /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/, [name])) {
            throw typeError('Invalid HTTP header name');
        }
        return name.toLowerCase();
    }

    function normalizeHeaderValue(value) {
        const input = byteString(value, 'Header value');
        for (let index = 0; index < input.length; index++) {
            const unit = input.charCodeAt(index);
            if (unit === 0 || unit === 10 || unit === 13) throw typeError('Invalid HTTP header value');
        }
        return apply(stringReplace, input, [/^[\t ]+|[\t ]+$/g, '']);
    }

    function headerByteSize(entries) {
        let size = 0;
        for (let index = 0; index < entries.length; index++) {
            size += entries[index][0].length + entries[index][1].length;
            if (size > maximumHeaderBytes) throw new RangeError('Headers exceed the per-object byte budget');
        }
        return size;
    }

    const forbiddenRequestHeaderNames = [
        'accept-charset',
        'accept-encoding',
        'access-control-request-headers',
        'access-control-request-method',
        'connection',
        'content-length',
        'cookie',
        'cookie2',
        'date',
        'dnt',
        'expect',
        'host',
        'keep-alive',
        'origin',
        'referer',
        'te',
        'trailer',
        'transfer-encoding',
        'upgrade',
        'via'
    ];

    function isForbiddenRequestHeader(name) {
        if (forbiddenRequestHeaderNames.includes(name)) return true;
        if (name.startsWith('sec-') || name.startsWith('proxy-')) return true;
        return false;
    }

    function isForbiddenResponseHeader(name) {
        return name === 'set-cookie' || name === 'set-cookie2';
    }

    function appendEntry(entries, name, value, guard) {
        if (guard === 'immutable') throw typeError('Headers are immutable');
        const normalizedName = normalizeHeaderName(name);
        if (guard === 'request' && isForbiddenRequestHeader(normalizedName)) return;
        if (guard === 'response' && isForbiddenResponseHeader(normalizedName)) return;

        if (entries.length >= maximumHeaders) throw new RangeError('Headers exceed the per-object count budget');
        const normalizedValue = normalizeHeaderValue(value);
        if (headerByteSize(entries) + normalizedName.length + normalizedValue.length > maximumHeaderBytes) {
            throw new RangeError('Headers exceed the per-object byte budget');
        }
        apply(arrayPush, entries, [[normalizedName, normalizedValue]]);
    }

    function initializeHeaders(entries, init, guard) {
        if (init === null || init === undefined) return;
        if ((typeof init !== 'object' && typeof init !== 'function')) {
            throw typeError('Headers initializer must be an object or sequence');
        }
        if (apply(weakMapHas, headerSlots, [init])) {
            const source = apply(weakMapGet, headerSlots, [init]).entries;
            for (let index = 0; index < source.length; index++) {
                appendEntry(entries, source[index][0], source[index][1], guard);
            }
            return;
        }
        const iterator = init[Symbol.iterator];
        if (iterator !== undefined && iterator !== null) {
            if (typeof iterator !== 'function') throw typeError('Headers initializer is not iterable');
            for (const pair of init) {
                if (pair === null || (typeof pair !== 'object' && typeof pair !== 'function')) {
                    throw typeError('Each header pair must be iterable');
                }
                const values = apply(arrayFrom, Array, [pair]);
                if (values.length !== 2) throw typeError('Each header pair must contain exactly two items');
                appendEntry(entries, values[0], values[1], guard);
            }
            return;
        }
        for (const name of objectKeys(init)) appendEntry(entries, name, init[name], guard);
    }

    function headerState(value) {
        const state = apply(weakMapGet, headerSlots, [value]);
        if (state === undefined) throw typeError('Illegal invocation: Headers');
        return state;
    }

    function setHeadersGuard(headers, guard) {
        const slot = apply(weakMapGet, headerSlots, [headers]);
        if (slot !== undefined) {
            slot.guard = guard;
        }
    }

    function createHeaders(init, guard = 'none') {
        const h = new Headers(init);
        setHeadersGuard(h, guard);
        return h;
    }

    function sortedEntries(entries) {
        const result = [];
        for (let index = 0; index < entries.length; index++) {
            apply(arrayPush, result, [[entries[index][0], entries[index][1]]]);
        }
        apply(arraySort, result, [(left, right) => left[0] < right[0] ? -1 : left[0] > right[0] ? 1 : 0]);
        const combined = [];
        for (let index = 0; index < result.length; index++) {
            const entry = result[index];
            const last = combined.length - 1;
            if (last >= 0 && combined[last][0] === entry[0]) combined[last][1] += ', ' + entry[1];
            else apply(arrayPush, combined, [entry]);
        }
        return combined;
    }

    class Headers {
        constructor(init = undefined) {
            const entries = [];
            apply(weakMapSet, headerSlots, [this, { entries, guard: 'none' }]);
            initializeHeaders(entries, init, 'none');
        }

        append(name, value) {
            const state = headerState(this);
            appendEntry(state.entries, name, value, state.guard);
        }

        delete(name) {
            const state = headerState(this);
            if (state.guard === 'immutable') throw typeError('Headers are immutable');
            const normalized = normalizeHeaderName(name);
            if (state.guard === 'request' && isForbiddenRequestHeader(normalized)) return;
            if (state.guard === 'response' && isForbiddenResponseHeader(normalized)) return;
            for (let index = state.entries.length - 1; index >= 0; index--) {
                if (state.entries[index][0] === normalized) apply(arraySplice, state.entries, [index, 1]);
            }
        }

        get(name) {
            const normalized = normalizeHeaderName(name);
            const state = headerState(this);
            let values = '';
            let found = false;
            for (let index = 0; index < state.entries.length; index++) {
                if (state.entries[index][0] === normalized) {
                    values += (found ? ', ' : '') + state.entries[index][1];
                    found = true;
                }
            }
            return found ? values : null;
        }

        getSetCookie() {
            const state = headerState(this);
            const result = [];
            for (let index = 0; index < state.entries.length; index++) {
                if (state.entries[index][0] === 'set-cookie') {
                    apply(arrayPush, result, [state.entries[index][1]]);
                }
            }
            return result;
        }

        has(name) {
            const normalized = normalizeHeaderName(name);
            const state = headerState(this);
            for (let index = 0; index < state.entries.length; index++) {
                if (state.entries[index][0] === normalized) return true;
            }
            return false;
        }

        set(name, value) {
            const state = headerState(this);
            if (state.guard === 'immutable') throw typeError('Headers are immutable');
            const normalized = normalizeHeaderName(name);
            if (state.guard === 'request' && isForbiddenRequestHeader(normalized)) return;
            if (state.guard === 'response' && isForbiddenResponseHeader(normalized)) return;

            const normalizedValue = normalizeHeaderValue(value);
            const replacement = [];
            let inserted = false;
            for (let index = 0; index < state.entries.length; index++) {
                if (state.entries[index][0] === normalized) {
                    if (!inserted) {
                        apply(arrayPush, replacement, [[normalized, normalizedValue]]);
                        inserted = true;
                    }
                } else {
                    apply(arrayPush, replacement, [state.entries[index]]);
                }
            }
            if (!inserted) apply(arrayPush, replacement, [[normalized, normalizedValue]]);
            if (replacement.length > maximumHeaders) throw new RangeError('Headers exceed the per-object count budget');
            headerByteSize(replacement);
            apply(arraySplice, state.entries, [0, state.entries.length, ...replacement]);
        }

        entries() {
            return apply(arrayIterator, sortedEntries(headerState(this).entries), []);
        }

        keys() {
            const entries = sortedEntries(headerState(this).entries);
            const values = [];
            for (let index = 0; index < entries.length; index++) apply(arrayPush, values, [entries[index][0]]);
            return apply(arrayIterator, values, []);
        }

        values() {
            const entries = sortedEntries(headerState(this).entries);
            const values = [];
            for (let index = 0; index < entries.length; index++) apply(arrayPush, values, [entries[index][1]]);
            return apply(arrayIterator, values, []);
        }

        forEach(callback, thisArg = undefined) {
            if (typeof callback !== 'function') throw typeError('Headers.forEach callback must be callable');
            const entries = sortedEntries(headerState(this).entries);
            for (let index = 0; index < entries.length; index++) {
                apply(callback, thisArg, [entries[index][1], entries[index][0], this]);
            }
        }

        [Symbol.iterator]() { return this.entries(); }
    }

    function responseState(value) {
        const state = apply(weakMapGet, responseSlots, [value]);
        if (state === undefined) throw typeError('Illegal invocation: Response');
        return state;
    }

    function responseHeaders(init) {
        return createHeaders(init.headers, 'response');
    }

    function createResponse(bytes, metadata) {
        const status = metadata.status;
        const statusText = byteString(metadata.statusText === undefined ? '' : metadata.statusText, 'statusText');
        const headers = createHeaders(metadata.headers, 'response');
        const statusHasNoBody = status === 204 || status === 205 || status === 304;
        const response = objectCreate(Response.prototype);
        apply(weakMapSet, responseSlots, [response, {
            bytes: copyBytes(bytes),
            bodyPresent: metadata.bodyPresent !== false && !statusHasNoBody,
            used: false,
            status,
            statusText,
            headers,
            url: metadata.url === undefined ? '' : domString(metadata.url),
            redirected: Boolean(metadata.redirected),
            type: metadata.type === undefined ? 'default' : domString(metadata.type),
            bodyStream: metadata.stream === undefined || metadata.stream === null ? null : metadata.stream
        }]);
        return response;
    }

    function bodyBytes(value) {
        const blob = new BlobConstructor([value]);
        return snapshotBlob(blob);
    }

    function readAllStream(stream) {
        const reader = stream.getReader();
        const parts = [];
        let total = 0;
        function pump() {
            return reader.read().then(result => {
                if (result.done) {
                    const bytes = new Uint8Array(total);
                    let offset = 0;
                    for (let index = 0; index < parts.length; index++) {
                        bytes.set(parts[index], offset);
                        offset += parts[index].length;
                    }
                    return bytes;
                }
                const value = result.value;
                const part = value instanceof Uint8Array ? copyBytes(value) : new Uint8Array(0);
                apply(arrayPush, parts, [part]);
                total += part.length;
                return pump();
            });
        }
        return pump();
    }

    function consume(target, convert, isRequest = false) {
        const state = isRequest ? requestState(target) : responseState(target);
        if (state.bodyPresent && (state.used || (state.bodyStream !== null && (state.bodyStream.disturbed || state.bodyStream.locked)))) {
            return apply(promiseReject, PromiseConstructor, [typeError((isRequest ? 'Request' : 'Response') + ' body is already used')]);
        }
        if (state.bodyPresent) {
            state.used = true;
            if (state.bodyStream !== null) {
                return readAllStream(state.bodyStream).then(bytes => convert(bytes));
            }
        }
        const bytes = state.bytes ? copyBytes(state.bytes) : new Uint8Array(0);
        const resolved = apply(promiseResolve, PromiseConstructor, [undefined]);
        return apply(promiseThen, resolved, [() => convert(bytes)]);
    }

    function validateStatus(value) {
        const number = Number(value);
        if (!Number.isFinite(number) || Math.trunc(number) !== number || number < 200 || number > 599) {
            throw new RangeError('Response status must be an integer from 200 through 599');
        }
        return number;
    }

    function dictionary(value) {
        return value === null || value === undefined ? {} : Object(value);
    }

    class Response {
        constructor(body = null, init = {}) {
            init = dictionary(init);
            const statusMember = init.status;
            const status = statusMember === undefined ? 200 : validateStatus(statusMember);
            const statusTextMember = init.statusText;
            const statusText = statusTextMember === undefined ? '' : byteString(statusTextMember, 'statusText');
            const headers = responseHeaders(init);
            const bodyPresent = body !== null && body !== undefined;
            if (bodyPresent && (status === 204 || status === 205 || status === 304)) {
                throw typeError('A response with this status cannot have a body');
            }
            if (bodyPresent && typeof body === 'string' && !headers.has('content-type')) {
                headers.set('content-type', 'text/plain;charset=UTF-8');
            }
            if (bodyPresent && typeof globalThis.ReadableStream === 'function' && body instanceof globalThis.ReadableStream) {
                if (body.locked) {
                    throw typeError('Cannot construct a Response with a locked ReadableStream');
                }
                const res = createResponse(new Uint8Array(0), {
                    status, statusText, headers, bodyPresent: true, url: '', redirected: false, type: 'default'
                });
                const s = responseState(res);
                s.bodyStream = body;
                return res;
            }
            const bytes = bodyPresent ? bodyBytes(body) : new Uint8Array(0);
            return createResponse(bytes, {
                status, statusText, headers, bodyPresent, url: '', redirected: false, type: 'default'
            });
        }

        get type() { return responseState(this).type; }
        get url() { return responseState(this).url; }
        get redirected() { return responseState(this).redirected; }
        get status() { return responseState(this).status; }
        get ok() {
            const status = responseState(this).status;
            return status >= 200 && status < 300;
        }
        get statusText() { return responseState(this).statusText; }
        get headers() { return responseState(this).headers; }
        get bodyUsed() {
            const state = responseState(this);
            if (!state.bodyPresent) return false;
            if (state.used) return true;
            if (state.bodyStream !== null && state.bodyStream.disturbed) return true;
            return false;
        }

        get body() {
            const state = responseState(this);
            if (!state.bodyPresent) return null;
            if (state.bodyStream === null) {
                if (state.used) {
                    const stream = new globalThis.ReadableStream({
                        start(controller) {
                            controller.close();
                        }
                    });
                    stream.getReader().read();
                    state.bodyStream = stream;
                    return stream;
                }
                const bytes = copyBytes(state.bytes);
                state.bodyStream = new globalThis.ReadableStream({
                    start(controller) {
                        if (bytes.length > 0) {
                            controller.enqueue(bytes);
                        }
                        controller.close();
                    }
                });
            }
            return state.bodyStream;
        }

        arrayBuffer() {
            return consume(this, bytes => copyBytes(bytes).buffer);
        }

        text() {
            return consume(this, bytes => decodeBytes(bytes));
        }

        json() {
            return consume(this, bytes => apply(jsonParse, JSON, [decodeBytes(bytes)]));
        }

        blob() {
            return consume(this, bytes => new BlobConstructor([bytes], {
                type: responseState(this).headers.get('content-type') || ''
            }));
        }

        clone() {
            const state = responseState(this);
            if (this.bodyUsed) throw typeError('Cannot clone a Response whose body is used');
            let clonedStream = null;
            if (state.bodyStream !== null) {
                const [b1, b2] = state.bodyStream.tee();
                state.bodyStream = b1;
                clonedStream = b2;
            }
            const res = createResponse(state.bytes, {
                status: state.status,
                statusText: state.statusText,
                headers: state.headers,
                bodyPresent: state.bodyPresent,
                url: state.url,
                redirected: state.redirected,
                type: state.type
            });
            if (clonedStream !== null) {
                const cloneState = responseState(res);
                cloneState.bodyStream = clonedStream;
            }
            return res;
        }

        static error() {
            const res = createResponse(new Uint8Array(0), {
                status: 0, statusText: '', headers: undefined, bodyPresent: false,
                url: '', redirected: false, type: 'error'
            });
            setHeadersGuard(responseState(res).headers, 'immutable');
            return res;
        }

        static json(data, init = {}) {
            const body = apply(jsonStringify, JSON, [data]);
            if (body === undefined) throw typeError('Response.json data cannot be serialized');
            const record = dictionary(init);
            const headers = new Headers(record.headers);
            if (!headers.has('content-type')) headers.set('content-type', 'application/json');
            return new Response(body, {
                status: record.status,
                statusText: record.statusText,
                headers
            });
        }

        static redirect(url, status = 302) {
            const redirectStatus = Number(status);
            if (![301, 302, 303, 307, 308].includes(redirectStatus)) {
                throw new RangeError('Invalid redirect status');
            }
            let parsed;
            try {
                const base = typeof globalThis.location === 'object' ? globalThis.location.href : undefined;
                parsed = new URLConstructor(domString(url), base);
            } catch (_) {
                throw typeError('Invalid redirect URL');
            }
            const headers = new Headers([['location', parsed.href]]);
            return createResponse(new Uint8Array(0), {
                status: redirectStatus,
                statusText: '',
                headers,
                bodyPresent: false,
                url: '',
                redirected: false,
                type: 'default'
            });
        }
    }

    function requestState(value) {
        const state = apply(weakMapGet, requestSlots, [value]);
        if (state === undefined) throw typeError('Illegal invocation: Request');
        return state;
    }

    const validModes = ['cors', 'no-cors', 'same-origin', 'navigate'];
    const validCredentials = ['same-origin', 'include', 'omit'];
    const validCaches = ['default', 'no-store', 'reload', 'no-cache', 'force-cache', 'only-if-cached'];
    const validRedirects = ['follow', 'error', 'manual'];
    const validReferrerPolicies = [
        '', 'no-referrer', 'no-referrer-when-downgrade', 'origin',
        'origin-when-cross-origin', 'same-origin', 'strict-origin',
        'strict-origin-when-cross-origin', 'unsafe-url'
    ];

    class Request {
        constructor(input, init = {}) {
            let inputWasRequest = false;
            let inputState = null;
            if (typeof input === 'object' && input !== null && apply(weakMapHas, requestSlots, [input])) {
                inputWasRequest = true;
                inputState = requestState(input);
            }

            let url;
            let method = 'GET';
            let headers;
            let mode = 'cors';
            let credentials = 'same-origin';
            let cache = 'default';
            let redirect = 'follow';
            let referrer = 'about:client';
            let referrerPolicy = '';
            let integrity = '';
            let keepalive = false;
            let duplex = 'half';
            let duplexSpecified = false;
            let signal = undefined;
            let bodyPresent = false;
            let bytes = null;
            let bodyStream = null;

            if (inputWasRequest) {
                url = inputState.url;
                method = inputState.method;
                headers = new Headers(inputState.headers);
                mode = inputState.mode;
                credentials = inputState.credentials;
                cache = inputState.cache;
                redirect = inputState.redirect;
                referrer = inputState.referrer;
                referrerPolicy = inputState.referrerPolicy;
                integrity = inputState.integrity;
                keepalive = inputState.keepalive;
                duplex = inputState.duplex;
                signal = inputState.signal;
                bodyPresent = inputState.bodyPresent;
                bytes = inputState.bytes;
                if (inputState.bodyStream !== null) {
                    const [b1, b2] = inputState.bodyStream.tee();
                    inputState.bodyStream = b1;
                    bodyStream = b2;
                }
            } else {
                let base = undefined;
                if (typeof globalThis.location === 'object' && globalThis.location !== null && typeof globalThis.location.href === 'string') {
                    base = globalThis.location.href;
                }
                let parsed;
                try {
                    parsed = new URLConstructor(domString(input), base);
                } catch (e) {
                    throw typeError('Invalid URL in Request constructor');
                }
                url = parsed.href;
                headers = new Headers();
                if (typeof globalThis.AbortController === 'function') {
                    signal = new globalThis.AbortController().signal;
                }
            }

            init = dictionary(init);

            if (init.method !== undefined) {
                const m = byteString(init.method, 'method').toUpperCase();
                if (!apply(regexpTest, /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/, [m])) {
                    throw typeError('Invalid HTTP method');
                }
                if (m === 'CONNECT' || m === 'TRACE' || m === 'TRACK') {
                    throw typeError('Forbidden HTTP method: ' + m);
                }
                method = m;
            }

            if (init.headers !== undefined) {
                headers = new Headers(init.headers);
            }
            setHeadersGuard(headers, 'request');

            if (init.mode !== undefined) {
                const m = domString(init.mode);
                if (!validModes.includes(m)) throw typeError('Invalid mode');
                mode = m;
            }

            if (init.credentials !== undefined) {
                const c = domString(init.credentials);
                if (!validCredentials.includes(c)) throw typeError('Invalid credentials');
                credentials = c;
            }

            if (init.cache !== undefined) {
                const c = domString(init.cache);
                if (!validCaches.includes(c)) throw typeError('Invalid cache');
                cache = c;
            }

            if (init.redirect !== undefined) {
                const r = domString(init.redirect);
                if (!validRedirects.includes(r)) throw typeError('Invalid redirect');
                redirect = r;
            }

            if (init.referrer !== undefined) {
                referrer = domString(init.referrer);
            }

            if (init.referrerPolicy !== undefined) {
                const rp = domString(init.referrerPolicy);
                if (!validReferrerPolicies.includes(rp)) throw typeError('Invalid referrerPolicy');
                referrerPolicy = rp;
            }

            if (init.integrity !== undefined) {
                integrity = domString(init.integrity);
            }

            if (init.keepalive !== undefined) {
                keepalive = Boolean(init.keepalive);
            }

            if (init.signal !== undefined && init.signal !== null) {
                if (typeof init.signal !== 'object') throw typeError('Request signal must be an AbortSignal');
                signal = init.signal;
            }

            if (init.duplex !== undefined) {
                const value = domString(init.duplex);
                if (value !== 'half') {
                    throw typeError("Failed to read the 'duplex' property from 'RequestInit': The provided value '" + value + "' is not a valid enum value of type RequestDuplex.");
                }
                duplex = value;
                duplexSpecified = true;
            }

            if (init.body !== undefined && init.body !== null) {
                if (method === 'GET' || method === 'HEAD') {
                    throw typeError('Request with GET/HEAD method cannot have body');
                }
                const bodyVal = init.body;
                if (typeof globalThis.ReadableStream === 'function' && bodyVal instanceof globalThis.ReadableStream) {
                    if (!duplexSpecified) {
                        throw typeError('The `duplex` member must be specified for a request with a streaming body');
                    }
                    if (bodyVal.locked) throw typeError('The provided ReadableStream is locked');
                    bodyPresent = true;
                    bytes = null;
                    bodyStream = bodyVal;
                } else {
                    bodyPresent = true;
                    bytes = bodyBytes(bodyVal);
                    bodyStream = null;
                    if (typeof bodyVal === 'string' && !headers.has('content-type')) {
                        headers.set('content-type', 'text/plain;charset=UTF-8');
                    }
                }
            } else if (inputWasRequest && inputState.bodyPresent) {
                if (method === 'GET' || method === 'HEAD') {
                    throw typeError('Request with GET/HEAD method cannot have body');
                }
                if (input.bodyUsed) {
                    throw typeError('Cannot construct a Request with a used body');
                }
            }

            apply(weakMapSet, requestSlots, [this, {
                method,
                url,
                headers,
                destination: '',
                referrer,
                referrerPolicy,
                mode,
                credentials,
                cache,
                redirect,
                integrity,
                keepalive,
                duplex,
                signal,
                bytes,
                bodyPresent,
                bodyStream,
                used: false
            }]);
        }

        get method() { return requestState(this).method; }
        get url() { return requestState(this).url; }
        get headers() { return requestState(this).headers; }
        get destination() { return requestState(this).destination; }
        get referrer() { return requestState(this).referrer; }
        get referrerPolicy() { return requestState(this).referrerPolicy; }
        get mode() { return requestState(this).mode; }
        get credentials() { return requestState(this).credentials; }
        get cache() { return requestState(this).cache; }
        get redirect() { return requestState(this).redirect; }
        get integrity() { return requestState(this).integrity; }
        get keepalive() { return requestState(this).keepalive; }
        get duplex() { return requestState(this).duplex; }
        get isReloadNavigation() { return false; }
        get isHistoryNavigation() { return false; }
        get signal() { return requestState(this).signal; }

        get bodyUsed() {
            const state = requestState(this);
            if (!state.bodyPresent) return false;
            if (state.used) return true;
            if (state.bodyStream !== null && state.bodyStream.disturbed) return true;
            return false;
        }

        get body() {
            const state = requestState(this);
            if (!state.bodyPresent) return null;
            if (state.bodyStream === null) {
                if (state.used) {
                    const stream = new globalThis.ReadableStream({
                        start(controller) { controller.close(); }
                    });
                    stream.getReader().read();
                    state.bodyStream = stream;
                    return stream;
                }
                const b = state.bytes ? copyBytes(state.bytes) : new Uint8Array(0);
                state.bodyStream = new globalThis.ReadableStream({
                    start(controller) {
                        if (b.length > 0) controller.enqueue(b);
                        controller.close();
                    }
                });
            }
            return state.bodyStream;
        }

        get __rix_request_body_bytes() {
            return requestState(this).bytes;
        }

        get __rix_request_body_stream() {
            return requestState(this).bodyStream;
        }

        __rix_mark_used() {
            const state = requestState(this);
            if (state.bodyPresent) state.used = true;
        }

        arrayBuffer() {
            return consume(this, bytes => copyBytes(bytes).buffer, true);
        }

        text() {
            return consume(this, bytes => decodeBytes(bytes), true);
        }

        json() {
            return consume(this, bytes => apply(jsonParse, JSON, [decodeBytes(bytes)]), true);
        }

        blob() {
            return consume(this, bytes => new BlobConstructor([bytes], {
                type: requestState(this).headers.get('content-type') || ''
            }), true);
        }

        clone() {
            const state = requestState(this);
            if (this.bodyUsed) throw typeError('Cannot clone a Request whose body is used');
            let clonedStream = null;
            if (state.bodyStream !== null) {
                const [b1, b2] = state.bodyStream.tee();
                state.bodyStream = b1;
                clonedStream = b2;
            }
            const req = objectCreate(Request.prototype);
            const clonedHeaders = new Headers(state.headers);
            setHeadersGuard(clonedHeaders, 'request');
            apply(weakMapSet, requestSlots, [req, {
                method: state.method,
                url: state.url,
                headers: clonedHeaders,
                destination: state.destination,
                referrer: state.referrer,
                referrerPolicy: state.referrerPolicy,
                mode: state.mode,
                credentials: state.credentials,
                cache: state.cache,
                redirect: state.redirect,
                integrity: state.integrity,
                keepalive: state.keepalive,
                duplex: state.duplex,
                signal: state.signal,
                bytes: state.bytes ? copyBytes(state.bytes) : null,
                bodyPresent: state.bodyPresent,
                bodyStream: clonedStream,
                used: false
            }]);
            return req;
        }
    }

    Object.defineProperty(Headers.prototype, 'constructor', {
        value: Headers, writable: true, configurable: true
    });
    Object.defineProperty(Headers.prototype, Symbol.toStringTag, {
        value: 'Headers', configurable: true
    });
    Object.defineProperty(Response.prototype, 'constructor', {
        value: Response, writable: true, configurable: true
    });
    Object.defineProperty(Response.prototype, Symbol.toStringTag, {
        value: 'Response', configurable: true
    });
    Object.defineProperty(Request.prototype, 'constructor', {
        value: Request, writable: true, configurable: true
    });
    Object.defineProperty(Request.prototype, Symbol.toStringTag, {
        value: 'Request', configurable: true
    });

    Object.defineProperty(globalThis, 'Headers', {
        value: Headers, writable: true, configurable: true
    });
    Object.defineProperty(globalThis, 'Response', {
        value: Response, writable: true, configurable: true
    });
    Object.defineProperty(globalThis, 'Request', {
        value: Request, writable: true, configurable: true
    });

    return [Response, (bytes, metadata) => createResponse(bytes, metadata), Request];
})();
"#;

pub(super) fn register_response_apis(context: &mut Context) -> Result<(), JsError> {
    let blob_internals = context
        .get_data::<super::blob::BlobInternals>()
        .cloned()
        .ok_or_else(|| JsNativeError::error().with_message("Blob internals are unavailable"))?;
    context.register_global_callable(
        js_string!("__rix_response_snapshot_blob"),
        1,
        NativeFunction::from_copy_closure_with_captures(
            |_this, args, snapshot, context| {
                snapshot.call(
                    &JsValue::undefined(),
                    &[args.get_or_undefined(0).clone()],
                    context,
                )
            },
            blob_internals.snapshot,
        ),
    )?;
    context.register_global_callable(
        js_string!("__rix_response_decode_bytes"),
        1,
        NativeFunction::from_copy_closure_with_captures(
            |_this, args, decode, context| {
                decode.call(
                    &JsValue::undefined(),
                    &[args.get_or_undefined(0).clone()],
                    context,
                )
            },
            blob_internals.decode,
        ),
    )?;
    let api = context.eval(boa_engine::Source::from_bytes(RESPONSE_API_SCRIPT))?;
    let api = api
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("Response API was not registered"))?;
    let factory = api.get(1, context)?.as_object().ok_or_else(|| {
        JsNativeError::error().with_message("Response factory was not registered")
    })?;
    context.insert_data(ResponseFactory(factory));
    Ok(())
}

fn build_response_metadata(
    context: &mut Context,
    headers: &HashMap<String, String>,
    status: u16,
    status_text: &str,
    url: &str,
    redirected: bool,
    stream: Option<JsValue>,
) -> JsObject {
    let mut header_properties = ObjectInitializer::new(context);
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("set-cookie") || name.eq_ignore_ascii_case("set-cookie2") {
            continue;
        }
        header_properties.property(
            js_string!(name.as_str()),
            js_string!(value.as_str()),
            Attribute::READONLY | Attribute::ENUMERABLE,
        );
    }
    let header_object = header_properties.build();
    let mut metadata = ObjectInitializer::new(context);
    metadata.property(
        js_string!("status"),
        JsValue::new(status),
        Attribute::READONLY,
    );
    metadata.property(
        js_string!("statusText"),
        js_string!(status_text),
        Attribute::READONLY,
    );
    metadata.property(js_string!("headers"), header_object, Attribute::READONLY);
    metadata.property(js_string!("url"), js_string!(url), Attribute::READONLY);
    metadata.property(
        js_string!("redirected"),
        JsValue::new(redirected),
        Attribute::READONLY,
    );
    if let Some(stream) = stream {
        metadata.property(js_string!("stream"), stream, Attribute::READONLY);
    }
    metadata.build()
}

fn response_factory(context: &mut Context) -> JsResult<JsObject> {
    context
        .get_data::<ResponseFactory>()
        .map(|factory| factory.0.clone())
        .ok_or_else(|| {
            JsNativeError::error()
                .with_message("Response factory is unavailable")
                .into()
        })
}

pub(super) fn build_fetch_response(
    context: &mut Context,
    resolved: &super::fetch::ResolvedFetch,
) -> JsResult<JsObject> {
    let factory = response_factory(context)?;
    let bytes = JsUint8Array::from_iter(resolved.body.iter().copied(), context)?;
    let metadata = build_response_metadata(
        context,
        &resolved.headers,
        resolved.status,
        &resolved.status_text,
        &resolved.url,
        resolved.redirected,
        None,
    );
    let response = factory.call(
        &JsValue::undefined(),
        &[bytes.into(), metadata.into()],
        context,
    )?;
    Ok(response.as_object().ok_or_else(|| {
        JsNativeError::error().with_message("Response factory returned a non-object")
    })?)
}

pub(super) fn build_streaming_fetch_response(
    context: &mut Context,
    head: &super::fetch::StreamHead,
    stream: JsValue,
) -> JsResult<JsObject> {
    let factory = response_factory(context)?;
    let bytes = JsUint8Array::from_iter(std::iter::empty::<u8>(), context)?;
    let metadata = build_response_metadata(
        context,
        &head.headers,
        head.status,
        "",
        &head.url,
        head.redirected,
        Some(stream),
    );
    let response = factory.call(
        &JsValue::undefined(),
        &[bytes.into(), metadata.into()],
        context,
    )?;
    Ok(response.as_object().ok_or_else(|| {
        JsNativeError::error().with_message("Response factory returned a non-object")
    })?)
}
