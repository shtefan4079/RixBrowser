use super::*;

#[derive(Clone)]
pub(super) struct AbortInternals {
    pub(super) read_state: JsObject,
    pub(super) add_algorithm: JsObject,
    pub(super) remove_algorithm: JsObject,
}

#[derive(Clone, Copy)]
struct AbortApiRegistered;

pub(super) fn register_abort_api(context: &mut Context) -> JsResult<()> {
    if context.get_data::<AbortApiRegistered>().is_some() {
        return Ok(());
    }

    let internals = context.eval(boa_engine::Source::from_bytes(
        r#"
        (function() {
            if (typeof globalThis.AbortController === 'function' &&
                typeof globalThis.AbortSignal === 'function') {
                const readState = function(signal) {
                    if (!(signal instanceof AbortSignal)) throw new TypeError('Expected an AbortSignal');
                    return {aborted: signal.aborted, reason: signal.reason};
                };
                const addAlgorithm = function(signal, algorithm) {
                    if (!(signal instanceof AbortSignal)) throw new TypeError('Expected an AbortSignal');
                    if (signal.aborted) { algorithm(signal.reason); return false; }
                    const callback = () => algorithm(signal.reason);
                    signal.addEventListener('abort', callback, {once: true});
                    return callback;
                };
                const removeAlgorithm = function(signal, algorithm) {
                    if (!(signal instanceof AbortSignal)) throw new TypeError('Expected an AbortSignal');
                    if (typeof algorithm === 'function') signal.removeEventListener('abort', algorithm);
                };
                return {readState, addAlgorithm, removeAlgorithm};
            }

            const signalSlots = new WeakMap();
            const controllerSlots = new WeakMap();
            const constructorToken = {};

            function signalState(signal) {
                const state = signalSlots.get(signal);
                if (!state) throw new TypeError('Illegal AbortSignal receiver');
                return state;
            }

            function defaultAbortReason() {
                return new DOMException('This operation was aborted', 'AbortError');
            }

            function setAborted(signal, reason, dispatch) {
                const state = signalState(signal);
                if (state.aborted) return false;
                state.aborted = true;
                state.reason = reason === undefined ? defaultAbortReason() : reason;
                const algorithms = Array.from(state.algorithms);
                state.algorithms.clear();
                for (const algorithm of algorithms) algorithm(state.reason);
                if (dispatch) signal.dispatchEvent(new Event('abort'));
                return true;
            }

            class AbortSignal extends EventTarget {
                constructor(token) {
                    super();
                    if (token !== constructorToken) throw new TypeError('Illegal constructor');
                    signalSlots.set(this, {aborted: false, reason: undefined, algorithms: new Set(), onabort: null});
                }
                get aborted() { return signalState(this).aborted; }
                get reason() { return signalState(this).reason; }
                get onabort() { return signalState(this).onabort; }
                set onabort(callback) {
                    if (callback !== null && callback !== undefined && typeof callback !== 'function') {
                        throw new TypeError('onabort must be a function or null');
                    }
                    signalState(this).onabort = callback == null ? null : callback;
                }
                throwIfAborted() {
                    const state = signalState(this);
                    if (state.aborted) throw state.reason;
                }
                static abort(reason = undefined) {
                    const signal = new AbortSignal(constructorToken);
                    setAborted(signal, reason, false);
                    return signal;
                }
                static timeout(milliseconds) {
                    if (arguments.length === 0) throw new TypeError('AbortSignal.timeout requires milliseconds');
                    const delay = +milliseconds;
                    if (Number.isNaN(delay) || delay < 0 || delay > 18446744073709551615) {
                        throw new RangeError('Invalid timeout');
                    }
                    const signal = new AbortSignal(constructorToken);
                    setTimeout(() => setAborted(signal,
                        new DOMException('The operation timed out', 'TimeoutError'), true), delay);
                    return signal;
                }
                static any(signals) {
                    const sources = [];
                    for (const source of signals) {
                        signalState(source);
                        sources.push(source);
                    }
                    for (const source of sources) {
                        const sourceState = signalState(source);
                        if (sourceState.aborted) {
                            const alreadyAborted = new AbortSignal(constructorToken);
                            setAborted(alreadyAborted, sourceState.reason, false);
                            return alreadyAborted;
                        }
                    }
                    const combined = new AbortSignal(constructorToken);
                    const registrations = [];
                    for (const source of sources) {
                        const sourceState = signalState(source);
                        const callback = () => {
                            const current = signalState(source);
                            for (const [registeredSignal, registeredCallback] of registrations) {
                                signalState(registeredSignal).algorithms.delete(registeredCallback);
                            }
                            registrations.length = 0;
                            setAborted(combined, current.reason, true);
                        };
                        sourceState.algorithms.add(callback);
                        registrations.push([source, callback]);
                    }
                    return combined;
                }
            }

            class AbortController {
                constructor() {
                    const signal = new AbortSignal(constructorToken);
                    controllerSlots.set(this, signal);
                }
                get signal() {
                    const signal = controllerSlots.get(this);
                    if (!signal) throw new TypeError('Illegal AbortController receiver');
                    return signal;
                }
                abort(reason = undefined) {
                    const signal = controllerSlots.get(this);
                    if (!signal) throw new TypeError('Illegal AbortController receiver');
                    setAborted(signal, reason, true);
                }
            }

            Object.defineProperty(AbortSignal.prototype, Symbol.toStringTag,
                {value: 'AbortSignal', configurable: true});
            Object.defineProperty(AbortController.prototype, Symbol.toStringTag,
                {value: 'AbortController', configurable: true});
            for (const [prototype, members] of [
                [AbortSignal.prototype, ['aborted', 'reason', 'onabort', 'throwIfAborted']],
                [AbortController.prototype, ['signal', 'abort']],
            ]) {
                for (const name of members) {
                    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                    Object.defineProperty(prototype, name, {...descriptor, enumerable: true});
                }
            }
            for (const name of ['abort', 'timeout', 'any']) {
                const descriptor = Object.getOwnPropertyDescriptor(AbortSignal, name);
                Object.defineProperty(AbortSignal, name, {...descriptor, enumerable: true});
            }
            Object.defineProperty(globalThis, 'AbortSignal',
                {value: AbortSignal, configurable: true, writable: true});
            Object.defineProperty(globalThis, 'AbortController',
                {value: AbortController, configurable: true, writable: true});

            return {
                readState(signal) {
                    const state = signalState(signal);
                    return {aborted: state.aborted, reason: state.reason};
                },
                addAlgorithm(signal, algorithm) {
                    const state = signalState(signal);
                    if (state.aborted) { algorithm(state.reason); return false; }
                    state.algorithms.add(algorithm);
                    return true;
                },
                removeAlgorithm(signal, algorithm) {
                    signalState(signal).algorithms.delete(algorithm);
                },
            };
        })()
        "#,
    ))?;

    let internals = internals
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("Abort API registration failed"))?;
    let read_state = internals
        .get(js_string!("readState"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::typ().with_message("AbortSignal state reader is unavailable")
        })?;
    let add_algorithm = internals
        .get(js_string!("addAlgorithm"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::typ().with_message("AbortSignal algorithm hook is unavailable")
        })?;
    let remove_algorithm = internals
        .get(js_string!("removeAlgorithm"), context)?
        .as_object()
        .ok_or_else(|| {
            JsNativeError::typ().with_message("AbortSignal algorithm cleanup is unavailable")
        })?;
    context.insert_data(AbortInternals {
        read_state,
        add_algorithm,
        remove_algorithm,
    });
    context.insert_data(AbortApiRegistered);
    Ok(())
}

pub(super) fn read_abort_state(
    signal: &JsObject,
    context: &mut Context,
) -> JsResult<(bool, JsValue)> {
    let internals = context
        .get_data::<AbortInternals>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("AbortSignal is unavailable"))?;
    let state =
        internals
            .read_state
            .call(&JsValue::undefined(), &[signal.clone().into()], context)?;
    let state = state
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("Invalid AbortSignal state"))?;
    Ok((
        state.get(js_string!("aborted"), context)?.to_boolean(),
        state.get(js_string!("reason"), context)?,
    ))
}

pub(super) fn add_abort_algorithm(
    signal: &JsObject,
    algorithm: &JsObject,
    context: &mut Context,
) -> JsResult<JsValue> {
    let internals = context
        .get_data::<AbortInternals>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("AbortSignal is unavailable"))?;
    internals.add_algorithm.call(
        &JsValue::undefined(),
        &[signal.clone().into(), algorithm.clone().into()],
        context,
    )
}

pub(super) fn remove_abort_algorithm(
    signal: &JsObject,
    algorithm: &JsValue,
    context: &mut Context,
) -> JsResult<()> {
    let internals = context
        .get_data::<AbortInternals>()
        .cloned()
        .ok_or_else(|| JsNativeError::typ().with_message("AbortSignal is unavailable"))?;
    internals.remove_algorithm.call(
        &JsValue::undefined(),
        &[signal.clone().into(), algorithm.clone()],
        context,
    )?;
    Ok(())
}
