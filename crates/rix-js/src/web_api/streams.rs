use super::*;

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct StreamsInternals {
    pub(crate) readable_stream: JsObject,
    pub(crate) create_byte_stream: JsObject,
    pub(crate) create_push_stream: JsObject,
    pub(crate) is_readable_stream: JsObject,
}

const STREAMS_API_SCRIPT: &str = r#"
(() => {
    const streamSlots = new WeakMap();
    const readerSlots = new WeakMap();
    const controllerSlots = new WeakMap();
    const apply = Reflect.apply;
    const promiseResolve = Promise.resolve;
    const promiseReject = Promise.reject;
    const PromiseConstructor = Promise;
    const arrayPush = Array.prototype.push;
    const arrayShift = Array.prototype.shift;
    const weakMapGet = WeakMap.prototype.get;
    const weakMapSet = WeakMap.prototype.set;
    const weakMapHas = WeakMap.prototype.has;

    function typeError(message) { return new TypeError(message); }

    function streamState(value) {
        const state = apply(weakMapGet, streamSlots, [value]);
        if (state === undefined) throw typeError('Illegal invocation: not a ReadableStream');
        return state;
    }

    function readerState(value) {
        const state = apply(weakMapGet, readerSlots, [value]);
        if (state === undefined) throw typeError('Illegal invocation: not a ReadableStreamDefaultReader');
        return state;
    }

    function controllerState(value) {
        const state = apply(weakMapGet, controllerSlots, [value]);
        if (state === undefined) throw typeError('Illegal invocation: not a ReadableStreamDefaultController');
        return state;
    }

    function createPromiseCapability() {
        let resolve, reject;
        const promise = new PromiseConstructor((res, rej) => {
            resolve = res;
            reject = rej;
        });
        return { promise, resolve, reject };
    }

    function isReadableStream(value) {
        return value !== null && (typeof value === 'object' || typeof value === 'function') && apply(weakMapHas, streamSlots, [value]);
    }

    function readableStreamError(stream, error) {
        const state = apply(weakMapGet, streamSlots, [stream]);
        if (!state || state.state !== 'readable') return;
        state.state = 'errored';
        state.storedError = error;

        const reader = state.reader;
        if (reader) {
            const rState = apply(weakMapGet, readerSlots, [reader]);
            if (rState) {
                while (rState.readRequests.length > 0) {
                    const req = apply(arrayShift, rState.readRequests, []);
                    req.reject(error);
                }
                rState.closedReject(error);
            }
        }
    }

    function readableStreamClose(stream) {
        const state = apply(weakMapGet, streamSlots, [stream]);
        if (!state || state.state !== 'readable') return;
        state.state = 'closed';

        const reader = state.reader;
        if (reader) {
            const rState = apply(weakMapGet, readerSlots, [reader]);
            if (rState) {
                while (rState.readRequests.length > 0) {
                    const req = apply(arrayShift, rState.readRequests, []);
                    req.resolve({ value: undefined, done: true });
                }
                rState.closedResolve(undefined);
            }
        }
    }

    function readableStreamDefaultControllerCallPullIfNeeded(controller) {
        const cState = apply(weakMapGet, controllerSlots, [controller]);
        if (!cState) return;
        const stream = cState.stream;
        const sState = apply(weakMapGet, streamSlots, [stream]);
        if (!sState || sState.state !== 'readable') return;
        if (cState.closeRequested) return;
        if (!cState.started) return;

        const reader = sState.reader;
        const hasPendingRead = reader && apply(weakMapGet, readerSlots, [reader]).readRequests.length > 0;
        const desired = readableStreamDefaultControllerGetDesiredSize(controller);
        if (!hasPendingRead && desired !== null && desired <= 0) return;

        if (cState.pulling) {
            cState.pullAgain = true;
            return;
        }

        cState.pulling = true;
        const pullAlgorithm = cState.pullAlgorithm;
        if (!pullAlgorithm) {
            cState.pulling = false;
            return;
        }

        let pullPromise;
        try {
            pullPromise = apply(promiseResolve, PromiseConstructor, [pullAlgorithm(controller)]);
        } catch (error) {
            pullPromise = apply(promiseReject, PromiseConstructor, [error]);
        }

        pullPromise.then(
            () => {
                cState.pulling = false;
                if (cState.pullAgain) {
                    cState.pullAgain = false;
                    readableStreamDefaultControllerCallPullIfNeeded(controller);
                }
            },
            (error) => {
                readableStreamError(stream, error);
            }
        );
    }

    function readableStreamDefaultControllerGetDesiredSize(controller) {
        const cState = apply(weakMapGet, controllerSlots, [controller]);
        if (!cState) return null;
        const stream = cState.stream;
        const sState = apply(weakMapGet, streamSlots, [stream]);
        if (!sState) return null;
        if (sState.state === 'errored') return null;
        if (sState.state === 'closed') return 0;
        return cState.highWaterMark - cState.queueTotalSize;
    }

    class ReadableStreamDefaultController {
        constructor() {
            throw typeError('Illegal constructor: ReadableStreamDefaultController cannot be constructed directly');
        }

        get desiredSize() {
            controllerState(this);
            return readableStreamDefaultControllerGetDesiredSize(this);
        }

        close() {
            const cState = controllerState(this);
            const stream = cState.stream;
            const sState = apply(weakMapGet, streamSlots, [stream]);
            if (sState.state !== 'readable') {
                throw typeError('Cannot close a stream that is not readable');
            }
            if (cState.closeRequested) {
                throw typeError('Cannot close a stream that is already close-requested');
            }
            cState.closeRequested = true;
            if (cState.queue.length === 0) {
                readableStreamClose(stream);
            }
        }

        enqueue(chunk) {
            const cState = controllerState(this);
            const stream = cState.stream;
            const sState = apply(weakMapGet, streamSlots, [stream]);
            if (sState.state !== 'readable' || cState.closeRequested) {
                throw typeError('Cannot enqueue chunk into a stream that is not readable or is close-requested');
            }

            const reader = sState.reader;
            if (reader) {
                const rState = apply(weakMapGet, readerSlots, [reader]);
                if (rState && rState.readRequests.length > 0) {
                    const req = apply(arrayShift, rState.readRequests, []);
                    req.resolve({ value: chunk, done: false });
                    return;
                }
            }

            let chunkSize = 1;
            try {
                if (cState.sizeAlgorithm) {
                    chunkSize = cState.sizeAlgorithm(chunk);
                }
            } catch (error) {
                readableStreamError(stream, error);
                throw error;
            }

            apply(arrayPush, cState.queue, [{ value: chunk, size: chunkSize }]);
            cState.queueTotalSize += chunkSize;
            readableStreamDefaultControllerCallPullIfNeeded(this);
        }

        error(e) {
            const cState = controllerState(this);
            const stream = cState.stream;
            const sState = apply(weakMapGet, streamSlots, [stream]);
            if (sState.state !== 'readable') {
                throw typeError('Cannot error a stream that is not readable');
            }
            readableStreamError(stream, e);
        }
    }

    class ReadableStreamDefaultReader {
        constructor(stream) {
            if (!isReadableStream(stream)) {
                throw typeError('ReadableStreamDefaultReader constructor requires a ReadableStream');
            }
            const sState = apply(weakMapGet, streamSlots, [stream]);
            if (sState.reader !== undefined) {
                throw typeError('ReadableStream is already locked to a reader');
            }

            sState.reader = this;
            const closedCap = createPromiseCapability();
            apply(weakMapSet, readerSlots, [this, {
                stream,
                readRequests: [],
                closedPromise: closedCap.promise,
                closedResolve: closedCap.resolve,
                closedReject: closedCap.reject
            }]);

            if (sState.state === 'closed') {
                closedCap.resolve(undefined);
            } else if (sState.state === 'errored') {
                closedCap.reject(sState.storedError);
            }
        }

        get closed() {
            return readerState(this).closedPromise;
        }

        cancel(reason = undefined) {
            const rState = readerState(this);
            if (!rState.stream) {
                return apply(promiseReject, PromiseConstructor, [typeError('Reader has no stream')]);
            }
            return readableStreamCancel(rState.stream, reason);
        }

        read() {
            const rState = readerState(this);
            const stream = rState.stream;
            if (!stream) {
                return apply(promiseReject, PromiseConstructor, [typeError('Reader has no stream')]);
            }
            const sState = apply(weakMapGet, streamSlots, [stream]);
            sState.disturbed = true;
            if (sState.state === 'errored') {
                return apply(promiseReject, PromiseConstructor, [sState.storedError]);
            }
            if (sState.state === 'closed') {
                return apply(promiseResolve, PromiseConstructor, [{ value: undefined, done: true }]);
            }

            const controller = sState.controller;
            const cState = apply(weakMapGet, controllerSlots, [controller]);
            if (cState.queue.length > 0) {
                const item = apply(arrayShift, cState.queue, []);
                cState.queueTotalSize -= item.size;
                if (cState.closeRequested && cState.queue.length === 0) {
                    readableStreamClose(stream);
                } else {
                    readableStreamDefaultControllerCallPullIfNeeded(controller);
                }
                return apply(promiseResolve, PromiseConstructor, [{ value: item.value, done: false }]);
            }

            const cap = createPromiseCapability();
            apply(arrayPush, rState.readRequests, [cap]);
            readableStreamDefaultControllerCallPullIfNeeded(controller);
            return cap.promise;
        }

        releaseLock() {
            const rState = readerState(this);
            const stream = rState.stream;
            if (!stream) return;
            if (rState.readRequests.length > 0) {
                throw typeError('Cannot release lock with pending read requests');
            }

            const sState = apply(weakMapGet, streamSlots, [stream]);
            sState.reader = undefined;
            if (sState.state === 'readable') {
                rState.closedReject(typeError('Reader was released and stream is still readable'));
            }
            rState.stream = undefined;
        }
    }

    function readableStreamCancel(stream, reason) {
        const sState = apply(weakMapGet, streamSlots, [stream]);
        if (sState.state === 'closed') {
            return apply(promiseResolve, PromiseConstructor, [undefined]);
        }
        if (sState.state === 'errored') {
            return apply(promiseReject, PromiseConstructor, [sState.storedError]);
        }

        sState.disturbed = true;
        const controller = sState.controller;
        const cState = apply(weakMapGet, controllerSlots, [controller]);
        cState.queue = [];
        cState.queueTotalSize = 0;

        readableStreamClose(stream);

        const cancelAlgorithm = cState.cancelAlgorithm;
        if (cancelAlgorithm) {
            try {
                return apply(promiseResolve, PromiseConstructor, [cancelAlgorithm(reason)]);
            } catch (error) {
                return apply(promiseReject, PromiseConstructor, [error]);
            }
        }
        return apply(promiseResolve, PromiseConstructor, [undefined]);
    }

    class ReadableStream {
        constructor(underlyingSource = {}, strategy = {}) {
            if (underlyingSource === undefined) underlyingSource = {};
            if (underlyingSource === null || (typeof underlyingSource !== 'object' && typeof underlyingSource !== 'function')) {
                throw typeError('underlyingSource must be an object');
            }
            if (strategy === undefined) strategy = {};
            if (strategy === null || (typeof strategy !== 'object' && typeof strategy !== 'function')) {
                throw typeError('strategy must be an object');
            }

            const highWaterMark = strategy.highWaterMark === undefined ? 1 : Number(strategy.highWaterMark);
            if (Number.isNaN(highWaterMark) || highWaterMark < 0) {
                throw new RangeError('highWaterMark must be a non-negative number');
            }
            const sizeAlgorithm = typeof strategy.size === 'function' ? strategy.size : () => 1;

            apply(weakMapSet, streamSlots, [this, {
                state: 'readable',
                storedError: undefined,
                reader: undefined,
                controller: undefined,
                disturbed: false
            }]);

            const controller = Object.create(ReadableStreamDefaultController.prototype);
            apply(weakMapGet, streamSlots, [this]).controller = controller;

            const pullAlgorithm = typeof underlyingSource.pull === 'function' ? underlyingSource.pull.bind(underlyingSource) : undefined;
            const cancelAlgorithm = typeof underlyingSource.cancel === 'function' ? underlyingSource.cancel.bind(underlyingSource) : undefined;

            apply(weakMapSet, controllerSlots, [controller, {
                stream: this,
                underlyingSource,
                queue: [],
                queueTotalSize: 0,
                highWaterMark,
                sizeAlgorithm,
                closeRequested: false,
                pulling: false,
                pullAgain: false,
                started: false,
                pullAlgorithm,
                cancelAlgorithm
            }]);

            if (typeof underlyingSource.start === 'function') {
                try {
                    const startResult = underlyingSource.start(controller);
                    apply(promiseResolve, PromiseConstructor, [startResult]).then(
                        () => {
                            apply(weakMapGet, controllerSlots, [controller]).started = true;
                            readableStreamDefaultControllerCallPullIfNeeded(controller);
                        },
                        (error) => {
                            readableStreamError(this, error);
                        }
                    );
                } catch (error) {
                    readableStreamError(this, error);
                }
            } else {
                apply(weakMapGet, controllerSlots, [controller]).started = true;
                readableStreamDefaultControllerCallPullIfNeeded(controller);
            }
        }

        get locked() {
            return streamState(this).reader !== undefined;
        }

        get disturbed() {
            return streamState(this).disturbed;
        }

        cancel(reason = undefined) {
            const sState = streamState(this);
            if (sState.reader !== undefined) {
                return apply(promiseReject, PromiseConstructor, [typeError('Cannot cancel a locked stream')]);
            }
            return readableStreamCancel(this, reason);
        }

        getReader(options = {}) {
            streamState(this);
            if (options !== null && typeof options === 'object' && options.mode !== undefined) {
                if (options.mode === 'byob') {
                    throw typeError('BYOB reader is not supported in this stream');
                }
                throw new RangeError('Invalid reader mode');
            }
            return new ReadableStreamDefaultReader(this);
        }

        tee() {
            streamState(this);
            const reader = this.getReader();
            let branch1Controller, branch2Controller;
            let canceled1 = false, canceled2 = false;
            let reason1, reason2;
            let cancelResolve;
            const cancelPromise = new PromiseConstructor((res) => { cancelResolve = res; });

            function pull(controller) {
                return reader.read().then(({ value, done }) => {
                    if (done) {
                        if (!canceled1) branch1Controller.close();
                        if (!canceled2) branch2Controller.close();
                        return;
                    }
                    if (!canceled1) branch1Controller.enqueue(value);
                    if (!canceled2) branch2Controller.enqueue(value);
                }, (error) => {
                    if (!canceled1) branch1Controller.error(error);
                    if (!canceled2) branch2Controller.error(error);
                });
            }

            function cancel1(reason) {
                canceled1 = true;
                reason1 = reason;
                if (canceled2) {
                    const cancelResult = reader.cancel([reason1, reason2]);
                    cancelResolve(cancelResult);
                }
                return cancelPromise;
            }

            function cancel2(reason) {
                canceled2 = true;
                reason2 = reason;
                if (canceled1) {
                    const cancelResult = reader.cancel([reason1, reason2]);
                    cancelResolve(cancelResult);
                }
                return cancelPromise;
            }

            const branch1 = new ReadableStream({
                start(c) { branch1Controller = c; },
                pull,
                cancel: cancel1
            });
            const branch2 = new ReadableStream({
                start(c) { branch2Controller = c; },
                pull,
                cancel: cancel2
            });

            return [branch1, branch2];
        }
    }

    class ByteLengthQueuingStrategy {
        constructor(options) {
            if (options === null || typeof options !== 'object') {
                throw typeError('ByteLengthQueuingStrategy options must be an object');
            }
            this.highWaterMark = Number(options.highWaterMark);
        }
        size(chunk) {
            return chunk.byteLength;
        }
    }

    class CountQueuingStrategy {
        constructor(options) {
            if (options === null || typeof options !== 'object') {
                throw typeError('CountQueuingStrategy options must be an object');
            }
            this.highWaterMark = Number(options.highWaterMark);
        }
        size() {
            return 1;
        }
    }

    for (const [cls, tag] of [
        [ReadableStream, 'ReadableStream'],
        [ReadableStreamDefaultReader, 'ReadableStreamDefaultReader'],
        [ReadableStreamDefaultController, 'ReadableStreamDefaultController'],
        [ByteLengthQueuingStrategy, 'ByteLengthQueuingStrategy'],
        [CountQueuingStrategy, 'CountQueuingStrategy']
    ]) {
        Object.defineProperty(cls.prototype, 'constructor', {
            value: cls, writable: true, configurable: true
        });
        Object.defineProperty(cls.prototype, Symbol.toStringTag, {
            value: tag, configurable: true
        });
        Object.defineProperty(globalThis, cls.name, {
            value: cls, writable: true, configurable: true
        });
    }

    function createByteStream(bytes) {
        return new ReadableStream({
            start(controller) {
                if (bytes && bytes.length > 0) {
                    controller.enqueue(bytes);
                }
                controller.close();
            }
        });
    }

    function createPushStream(onCancel) {
        let controller;
        let cancelled = false;
        const stream = new ReadableStream({
            start(c) { controller = c; },
            cancel(reason) {
                cancelled = true;
                if (typeof onCancel === 'function') {
                    try { onCancel(reason); } catch (_) {}
                }
            }
        });
        return { stream, controller, isCancelled() { return cancelled; } };
    }

    return {
        ReadableStream,
        ReadableStreamDefaultReader,
        ReadableStreamDefaultController,
        createByteStream,
        createPushStream,
        isReadableStream
    };
})();
"#;

pub(super) fn register_streams_apis(context: &mut Context) -> Result<(), JsError> {
    let result = context.eval(boa_engine::Source::from_bytes(STREAMS_API_SCRIPT))?;
    let internals = result
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("Streams internals missing"))?;
    let readable_stream = internals
        .get(js_string!("ReadableStream"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("ReadableStream constructor missing"))?;
    let create_byte_stream = internals
        .get(js_string!("createByteStream"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("createByteStream helper missing"))?;
    let create_push_stream = internals
        .get(js_string!("createPushStream"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("createPushStream helper missing"))?;
    let is_readable_stream = internals
        .get(js_string!("isReadableStream"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("isReadableStream helper missing"))?;

    context.insert_data(StreamsInternals {
        readable_stream,
        create_byte_stream,
        create_push_stream,
        is_readable_stream,
    });
    Ok(())
}
