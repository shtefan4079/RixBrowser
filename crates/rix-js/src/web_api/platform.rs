use super::*;

pub(super) struct DomExceptionRegistered;
pub(super) struct StandardWebPlatformApisRegistered;

pub(super) fn register_dom_exception(context: &mut Context) -> JsResult<()> {
    if context.get_data::<DomExceptionRegistered>().is_some() {
        return Ok(());
    }
    context.eval(boa_engine::Source::from_bytes(r#"
        (function() {
            const NativeError = Error;
            const constructError = Reflect.construct;
            const stringify = String;
            const exceptions = new WeakMap();
            const codes = new Map([
                ['IndexSizeError', 1], ['HierarchyRequestError', 3], ['WrongDocumentError', 4],
                ['InvalidCharacterError', 5], ['NoModificationAllowedError', 7], ['NotFoundError', 8],
                ['NotSupportedError', 9], ['InUseAttributeError', 10], ['InvalidStateError', 11],
                ['SyntaxError', 12], ['InvalidModificationError', 13], ['NamespaceError', 14],
                ['InvalidAccessError', 15], ['TypeMismatchError', 17], ['SecurityError', 18],
                ['NetworkError', 19], ['AbortError', 20], ['URLMismatchError', 21],
                ['QuotaExceededError', 22], ['TimeoutError', 23], ['InvalidNodeTypeError', 24], ['DataCloneError', 25],
            ]);
            function domString(value) {
                if (typeof value === 'symbol') throw new TypeError('Cannot convert a Symbol to a DOMString');
                return stringify(value);
            }
            function slots(value) {
                const stored = exceptions.get(value);
                if (!stored) throw new TypeError('Illegal DOMException receiver');
                return stored;
            }
            function DOMException(message = '', name = 'Error') {
                if (!new.target) throw new TypeError('DOMException requires new');
                const convertedMessage = domString(message);
                const convertedName = domString(name);
                const exception = constructError(NativeError, [], new.target);
                exceptions.set(exception, {name: convertedName, message: convertedMessage});
                return exception;
            }
            const prototype = new NativeError();
            Object.defineProperty(DOMException, 'prototype', {value: prototype, writable: false});
            Object.defineProperty(prototype, 'constructor', {value: DOMException, writable: true, configurable: true});
            Object.defineProperties(prototype, {
                name: {get: function() { return slots(this).name; }, enumerable: true, configurable: true},
                message: {get: function() { return slots(this).message; }, enumerable: true, configurable: true},
                code: {get: function() { return codes.get(slots(this).name) || 0; }, enumerable: true, configurable: true},
                [Symbol.toStringTag]: {value: 'DOMException', configurable: true},
            });
            const constants = {
                INDEX_SIZE_ERR: 1, DOMSTRING_SIZE_ERR: 2, HIERARCHY_REQUEST_ERR: 3,
                WRONG_DOCUMENT_ERR: 4, INVALID_CHARACTER_ERR: 5, NO_DATA_ALLOWED_ERR: 6,
                NO_MODIFICATION_ALLOWED_ERR: 7, NOT_FOUND_ERR: 8, NOT_SUPPORTED_ERR: 9,
                INUSE_ATTRIBUTE_ERR: 10, INVALID_STATE_ERR: 11, SYNTAX_ERR: 12,
                INVALID_MODIFICATION_ERR: 13, NAMESPACE_ERR: 14, INVALID_ACCESS_ERR: 15,
                VALIDATION_ERR: 16, TYPE_MISMATCH_ERR: 17, SECURITY_ERR: 18, NETWORK_ERR: 19,
                ABORT_ERR: 20, URL_MISMATCH_ERR: 21, QUOTA_EXCEEDED_ERR: 22, TIMEOUT_ERR: 23,
                INVALID_NODE_TYPE_ERR: 24, DATA_CLONE_ERR: 25,
            };
            for (const [name, value] of Object.entries(constants)) {
                Object.defineProperty(DOMException, name, {value, enumerable: true});
                Object.defineProperty(prototype, name, {value, enumerable: true});
            }
            Object.defineProperty(globalThis, 'DOMException', {value: DOMException, configurable: true, writable: true});
        })();
    "#))?;
    context.insert_data(DomExceptionRegistered);
    Ok(())
}

pub fn register_standard_web_platform_apis(context: &mut Context) -> Result<(), JsError> {
    if context.get_data::<StandardWebPlatformApisRegistered>().is_some() {
        return Ok(());
    }
    context.insert_data(StandardWebPlatformApisRegistered);
    register_dom_exception(context)?;
    register_css_namespace(context)?;
    super::blob::register_blob_apis(context)?;
    super::url_api::register_url_apis(context)?;
    super::streams::register_streams_apis(context)?;
    super::text_codec::register_text_codec_apis(context)?;
    super::response::register_response_apis(context)?;
    crate::structured_clone::register(context)?;
    if context.get_data::<PerformanceClock>().is_none() {
        context.insert_data(PerformanceClock {
            started: Instant::now(),
            elapsed_at_start_ms: 0.0,
            time_origin: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64()
                * 1000.0,
            navigation: None,
            lifecycle: [None; 7],
        });
    }
    let time_origin = FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(performance_time_origin),
    )
    .name(js_string!("get timeOrigin"))
    .build();
    let performance = ObjectInitializer::new(context)
        .function(
            NativeFunction::from_fn_ptr(performance_now),
            js_string!("now"),
            0,
        )
        .accessor(
            js_string!("timeOrigin"),
            Some(time_origin),
            None,
            Attribute::ENUMERABLE,
        )
        .build();
    if context
        .get_data::<PerformanceClock>()
        .unwrap()
        .navigation
        .is_some()
    {
        let timing = navigation_timing_object(context);
        performance.define_property_or_throw(
            js_string!("timing"),
            PropertyDescriptor::builder()
                .value(timing)
                .enumerable(true)
                .build(),
            context,
        )?;
    }
    context.register_global_property(
        js_string!("performance"),
        performance.clone(),
        Attribute::CONFIGURABLE,
    )?;
    super::user_timing::register_user_timing_api(context, &performance)?;
    context.register_global_callable(
        js_string!("__rix_secure_random_bytes"),
        1,
        NativeFunction::from_fn_ptr(secure_random_values_bytes),
    )?;
    context.register_global_callable(
        js_string!("__rix_is_native_promise"),
        1,
        NativeFunction::from_fn_ptr(is_native_promise),
    )?;

    let script = r#"
        (function(markNonCloneable) {
        globalThis.window = globalThis;
        globalThis.self = globalThis;
        globalThis.top = globalThis;
        globalThis.parent = globalThis;
        markNonCloneable(globalThis);
        markNonCloneable(globalThis.performance);

        const isNativePromise = globalThis.__rix_is_native_promise;
        delete globalThis.__rix_is_native_promise;

        (function() {
            const secureRandomBytes = globalThis.__rix_secure_random_bytes;
            delete globalThis.__rix_secure_random_bytes;
            if (!globalThis.crypto) globalThis.crypto = {};
            if (!globalThis.crypto.getRandomValues) {
                globalThis.crypto.getRandomValues = function(view) {
                    const bytes = secureRandomBytes(view);
                    new Uint8Array(view.buffer, view.byteOffset, view.byteLength).set(bytes);
                    return view;
                };
            }
        })();

        let dispatchWindowListeners = null;
        const validWindows = new WeakSet();
        const activeWindowStack = [];
        const pushActiveWindow = win => { if (win) activeWindowStack.push(win); };
        const popActiveWindow = () => { activeWindowStack.pop(); };
        const getActiveWindow = () => activeWindowStack[activeWindowStack.length - 1] || globalThis.window || globalThis;
        const registerWindow = win => { if (win && typeof win === 'object') validWindows.add(win); };
        const isWindow = val => {
            if (!val || (typeof val !== 'object' && typeof val !== 'function')) return false;
            if (val === globalThis || val === globalThis.window) return true;
            if (validWindows.has(val)) return true;
            if (globalThis.Window && (val instanceof globalThis.Window)) return true;
            return false;
        };
        const getWindowOrigin = win => {
            if (!win) return 'null';
            if (typeof win.__rixGetOrigin === 'function') {
                try { return String(win.__rixGetOrigin()); } catch (_) { return 'null'; }
            }
            if (win === globalThis || win === globalThis.window) {
                return (globalThis.location && globalThis.location.origin) ? String(globalThis.location.origin) : 'null';
            }
            try {
                if (win.location && win.location.origin) return String(win.location.origin);
            } catch (_) {
                return 'null';
            }
            return 'null';
        };
        validWindows.add(globalThis);
        if (globalThis.window) validWindows.add(globalThis.window);
        Object.defineProperty(globalThis, '__rixPushActiveWindow', { value: pushActiveWindow, configurable: true, writable: true });
        Object.defineProperty(globalThis, '__rixPopActiveWindow', { value: popActiveWindow, configurable: true, writable: true });
        Object.defineProperty(globalThis, '__rixGetActiveWindow', { value: getActiveWindow, configurable: true, writable: true });
        Object.defineProperty(globalThis, '__rixRegisterWindow', { value: registerWindow, configurable: true, writable: true });

        const eventPathState = new WeakMap();
        const eventTargetState = new WeakMap();
        const getEventTarget = event => {
            if (!eventTargetState.has(event)) throw new TypeError('Illegal Event receiver');
            return eventTargetState.get(event);
        };
        if (!globalThis.EventTarget) {
            const eventTargetListeners = new WeakMap();
            const stringifyEventError = String;
            class EventTarget {
                constructor() {
                    markNonCloneable(this);
                    eventTargetListeners.set(this, new Map());
                }
                addEventListener(type, listener, options) {
                    const listeners = eventTargetListeners.get(this);
                    if (!listeners && globalThis.Node && this instanceof Node) {
                        return Node.prototype.addEventListener.call(this, type, listener, options);
                    }
                    if (!listeners) throw new TypeError('Illegal EventTarget receiver');
                    if (listener === null || listener === undefined) return;
                    if (typeof listener !== 'function' && typeof listener !== 'object') {
                        throw new TypeError('EventListener must be a function or object');
                    }
                    const eventType = '' + type;
                    const capture = typeof options === 'boolean' ? options : !!(options && options.capture);
                    const once = !!(options && (typeof options === 'object' || typeof options === 'function') && options.once);
                    const passive = !!(options && (typeof options === 'object' || typeof options === 'function') && options.passive);
                    const signal = (options && (typeof options === 'object' || typeof options === 'function')) ? options.signal : undefined;
                    if (signal !== undefined && signal !== null) {
                        if (typeof signal !== 'object' || typeof signal.aborted !== 'boolean') {
                            throw new TypeError("Failed to read the 'signal' property from 'AddEventListenerOptions': member is not of type AbortSignal.");
                        }
                        if (signal.aborted) {
                            return;
                        }
                    }
                    if (!listeners.has(eventType)) listeners.set(eventType, []);
                    const list = listeners.get(eventType);
                    if (!list.some(entry => entry.listener === listener && entry.capture === capture)) {
                        list.push({listener, capture, once, passive});
                        if (signal) {
                            const onAbort = () => {
                                this.removeEventListener(eventType, listener, { capture });
                            };
                            signal.addEventListener('abort', onAbort, { once: true });
                        }
                    }
                }
                removeEventListener(type, listener, options) {
                    const listeners = eventTargetListeners.get(this);
                    if (!listeners && globalThis.Node && this instanceof Node) {
                        return Node.prototype.removeEventListener.call(this, type, listener, options);
                    }
                    if (!listeners) throw new TypeError('Illegal EventTarget receiver');
                    if (listener === null || listener === undefined) return;
                    if (typeof listener !== 'function' && typeof listener !== 'object') {
                        throw new TypeError('EventListener must be a function or object');
                    }
                    const eventType = '' + type;
                    const list = listeners.get(eventType);
                    if (!list) return;
                    const capture = typeof options === 'boolean' ? options : !!(options && options.capture);
                    listeners.set(eventType, list.filter(entry => entry.listener !== listener || entry.capture !== capture));
                }
                dispatchEvent(event) {
                    const listeners = eventTargetListeners.get(this);
                    if (!listeners && globalThis.Node && this instanceof Node) {
                        return Node.prototype.dispatchEvent.call(this, event);
                    }
                    if (!listeners) throw new TypeError('Illegal EventTarget receiver');
                    if (!event || !event.type) throw new TypeError('dispatchEvent requires an Event');
                    if (event.__rixDispatching) throw new DOMException('The event is already being dispatched', 'InvalidStateError');
                    event.__rixDispatching = true;
                    eventTargetState.set(event, this);
                    event.currentTarget = this;
                    event._immediateStopped = false;
                    event.__rixStopped = false;
                    event.__rixImmediateStopped = false;
                    eventPathState.set(event, [this]);
                    event.eventPhase = Event.AT_TARGET;
                    const eventType = '' + event.type;
                    const list = (listeners.get(eventType) || []).slice();
                    let executingWindow = null;
                    if (isWindow(this)) {
                        executingWindow = this;
                    } else if (this && this.ownerDocument && this.ownerDocument.defaultView) {
                        executingWindow = this.ownerDocument.defaultView;
                    } else if (this && this.defaultView) {
                        executingWindow = this.defaultView;
                    }
                    if (executingWindow) pushActiveWindow(executingWindow);
                    try {
                        for (let i = 0; i < list.length; i++) {
                            const entry = list[i];
                            if (!(listeners.get(eventType) || []).includes(entry)) continue;
                            if (entry.once) {
                                const current = listeners.get(eventType) || [];
                                listeners.set(eventType, current.filter(currentEntry => currentEntry !== entry));
                            }
                            const listener = entry.listener;
                            const prevPassive = event.__rixInPassiveListener;
                            if (entry.passive) {
                                event.__rixInPassiveListener = true;
                            }
                            try {
                                if (typeof listener === 'function') {
                                    listener.call(this, event);
                                } else if (listener && typeof listener.handleEvent === 'function') {
                                    listener.handleEvent(event);
                                }
                            } catch (err) {
                                if (typeof console !== 'undefined' && console.error) console.error(err);
                            } finally {
                                if (entry.passive) {
                                    event.__rixInPassiveListener = prevPassive;
                                }
                            }
                            if (event._immediateStopped) break;
                        }
                        if (this !== globalThis) {
                            const handler = this['on' + event.type];
                            if (!event._immediateStopped && typeof handler === 'function') {
                                const result = handler.call(this, event);
                                if (result === false) event.preventDefault();
                            }
                        }
                        return !event.defaultPrevented;
                    } finally {
                        if (executingWindow) popActiveWindow();
                        event.currentTarget = null;
                        event.eventPhase = Event.NONE;
                        event.__rixDispatching = false;
                        event.__rixStopped = false;
                        event.__rixImmediateStopped = false;
                        event._immediateStopped = false;
                        eventPathState.delete(event);
                    }
                }
            }
            eventTargetListeners.set(globalThis, new Map());
            dispatchWindowListeners = function(event, capture, phase) {
                const listeners = eventTargetListeners.get(globalThis);
                if (!listeners) throw new TypeError('Illegal Window EventTarget state');
                const eventType = '' + event.type;
                event.currentTarget = globalThis;
                event.eventPhase = phase;
                const errors = [];
                const list = (listeners.get(eventType) || []).slice();
                pushActiveWindow(globalThis.window || globalThis);
                try {
                    for (const entry of list) {
                        if (!!entry.capture !== !!capture) continue;
                        if (!(listeners.get(eventType) || []).includes(entry)) continue;
                        if (entry.once) {
                            const current = listeners.get(eventType) || [];
                            listeners.set(eventType, current.filter(currentEntry => currentEntry !== entry));
                        }
                        const listener = entry.listener;
                        const prevPassive = event.__rixInPassiveListener;
                        if (entry.passive) {
                            event.__rixInPassiveListener = true;
                        }
                        try {
                            if (typeof listener === 'function') {
                                listener.call(globalThis, event);
                            } else if (listener && typeof listener.handleEvent === 'function') {
                                listener.handleEvent(event);
                            }
                        } catch (err) {
                            errors.push(err);
                        } finally {
                            if (entry.passive) {
                                event.__rixInPassiveListener = prevPassive;
                            }
                        }
                        if (event.__rixImmediateStopped || event._immediateStopped) break;
                    }
                } finally {
                    popActiveWindow();
                }
                return errors.map(error => stringifyEventError(error));
            };
            globalThis.EventTarget = EventTarget;
        }

        if (!globalThis.Window) {
            class Window extends EventTarget {
                constructor() { throw new TypeError('Illegal constructor: Window'); }
            }
            Object.defineProperty(Window.prototype, Symbol.toStringTag, {value: 'Window', configurable: true});
            globalThis.Window = Window;
            Object.setPrototypeOf(globalThis, Window.prototype);
        }

        if (!globalThis.SpeechRecognition) {
            class SpeechRecognition extends EventTarget {
                constructor() {
                    super();
                    this.continuous = false;
                    this.interimResults = false;
                    this.lang = '';
                    this.maxAlternatives = 1;
                }
                start() {}
                stop() {}
                abort() {}
            }
            globalThis.SpeechRecognition = SpeechRecognition;
            globalThis.webkitSpeechRecognition = SpeechRecognition;
        }

        if (typeof navigator !== 'undefined' && !navigator.mediaDevices) {
            navigator.mediaDevices = {
                getUserMedia: function() {
                    return Promise.reject(new DOMException('Media devices not supported', 'NotSupportedError'));
                },
                enumerateDevices: function() {
                    return Promise.resolve([]);
                }
            };
        }

        if (!Object.prototype.hasOwnProperty.call(globalThis, 'addEventListener')) {
            globalThis.addEventListener = function(type, listener, options) {
                'use strict';
                return EventTarget.prototype.addEventListener.call(this == null ? globalThis : this, type, listener, options);
            };
            globalThis.removeEventListener = function(type, listener, options) {
                'use strict';
                return EventTarget.prototype.removeEventListener.call(this == null ? globalThis : this, type, listener, options);
            };
            globalThis.dispatchEvent = function(event) {
                'use strict';
                return EventTarget.prototype.dispatchEvent.call(this == null ? globalThis : this, event);
            };
        }

        const uiEventSlots = new WeakMap();
        const uiEventLong = value => {
            const number = Number(value);
            if (!Number.isFinite(number) || number === 0) return 0;
            const wrapped = ((Math.trunc(number) % 4294967296) + 4294967296) % 4294967296;
            return wrapped >= 2147483648 ? wrapped - 4294967296 : wrapped;
        };

        if (!globalThis.Event) {
            class Event {
                constructor(type, eventInitDict) {
                    markNonCloneable(this);
                    this.type = String(type);
                    const init = eventInitDict || {};
                    this.bubbles = !!init.bubbles;
                    this.cancelable = !!init.cancelable;
                    this.composed = !!init.composed;
                    this.defaultPrevented = false;
                    eventTargetState.set(this, null);
                    this.currentTarget = null;
                    this.eventPhase = 0;
                    this.timeStamp = (globalThis.performance && globalThis.performance.now) ? globalThis.performance.now() : Date.now();
                    Object.defineProperties(this, {
                        __rixDispatching: {value: false, writable: true},
                        __rixStopped: {value: false, writable: true},
                        __rixImmediateStopped: {value: false, writable: true},
                        _immediateStopped: {value: false, writable: true},
                        __rixInPassiveListener: {value: false, writable: true},
                    });
                }
                composedPath() {
                    return (eventPathState.get(this) || []).slice();
                }
                get target() {
                    return getEventTarget(this);
                }
                preventDefault() {
                    if (this.__rixInPassiveListener) return;
                    if (this.cancelable) this.defaultPrevented = true;
                }
                stopPropagation() { this.__rixStopped = true; }
                stopImmediatePropagation() {
                    this.__rixStopped = true;
                    this.__rixImmediateStopped = true;
                    this._immediateStopped = true;
                }
                initEvent(type, bubbles = false, cancelable = false) {
                    if (this.__rixDispatching) return;
                    this.type = String(type);
                    this.bubbles = !!bubbles;
                    this.cancelable = !!cancelable;
                    this.defaultPrevented = false;
                    this._immediateStopped = false;
                    this.__rixStopped = false;
                    this.__rixImmediateStopped = false;
                    this._immediateStopped = false;
                }
            }
            Object.defineProperty(Event.prototype, Symbol.toStringTag, {value: 'Event', configurable: true});
            globalThis.Event = Event;
        } else if (typeof Event.prototype.composedPath !== 'function') {
            Object.defineProperty(Event.prototype, 'composedPath', {
                value: function() { return (eventPathState.get(this) || []).slice(); },
                writable: true,
                configurable: true,
            });
        }
        const eventConstants = {
            NONE: 0,
            CAPTURING_PHASE: 1,
            AT_TARGET: 2,
            BUBBLING_PHASE: 3,
        };
        for (const [name, value] of Object.entries(eventConstants)) {
            if (!Object.prototype.hasOwnProperty.call(Event, name)) {
                Object.defineProperty(Event, name, {value, enumerable: true});
            }
            if (!Object.prototype.hasOwnProperty.call(Event.prototype, name)) {
                Object.defineProperty(Event.prototype, name, {value, enumerable: true});
            }
        }
        const eventPrototype = globalThis.Event ? globalThis.Event.prototype : null;
        if (eventPrototype && !Object.getOwnPropertyDescriptor(eventPrototype, 'target')) {
            Object.defineProperty(eventPrototype, 'target', {
                get() { return getEventTarget(this); },
                configurable: true,
            });
        }

        let PromiseRejectionEventConstructor = globalThis.PromiseRejectionEvent;
        if (!PromiseRejectionEventConstructor) {
            const promiseRejectionEventSlots = new WeakMap();
            const weakMapGet = WeakMap.prototype.get;
            const apply = Reflect.apply;
            function promiseRejectionEventState(value) {
                const state = apply(weakMapGet, promiseRejectionEventSlots, [value]);
                if (state === undefined) throw new TypeError('Illegal PromiseRejectionEvent receiver');
                return state;
            }
            class PromiseRejectionEvent extends Event {
                constructor(type, eventInitDict = {}) {
                    const eventType = String(type);
                    const init = eventInitDict == null ? {} : Object(eventInitDict);
                    super(eventType, init);
                    const promise = init.promise;
                    if (!isNativePromise(promise)) {
                        throw new TypeError('PromiseRejectionEvent requires a Promise');
                    }
                    const reasonValue = init.reason;
                    const reason = reasonValue === undefined ? null : reasonValue;
                    promiseRejectionEventSlots.set(this, {promise, reason});
                }
                get promise() { return promiseRejectionEventState(this).promise; }
                get reason() { return promiseRejectionEventState(this).reason; }
            }
            Object.defineProperty(PromiseRejectionEvent.prototype, Symbol.toStringTag, {
                value: 'PromiseRejectionEvent', configurable: true
            });
            globalThis.PromiseRejectionEvent = PromiseRejectionEvent;
            PromiseRejectionEventConstructor = PromiseRejectionEvent;
        }

        if (!globalThis.UIEvent) {
            const uiSlots = event => {
                const slots = uiEventSlots.get(event);
                if (!slots) throw new TypeError('Illegal UIEvent receiver');
                return slots;
            };
            class UIEvent extends Event {
                constructor(type, eventInitDict = {}) {
                    super(type, eventInitDict);
                    const init = eventInitDict || {};
                    uiEventSlots.set(this, {
                        detail: uiEventLong(init.detail === undefined ? 0 : init.detail),
                        view: init.view == null ? null : init.view,
                    });
                }
                get detail() { return uiSlots(this).detail; }
                get view() { return uiSlots(this).view; }
                get which() { return 0; }
                initUIEvent(type, bubbles = false, cancelable = false, view = null, detail = 0) {
                    const slots = uiSlots(this);
                    if (this.__rixDispatching) return;
                    this.initEvent(type, bubbles, cancelable);
                    slots.view = view == null ? null : view;
                    slots.detail = uiEventLong(detail);
                }
            }
            Object.defineProperty(UIEvent.prototype, Symbol.toStringTag, {value: 'UIEvent', configurable: true});
            Object.defineProperty(UIEvent, 'name', {value: 'UIEvent'});
            globalThis.UIEvent = UIEvent;
        }

        if (!globalThis.MouseEvent) {
            const mouseEventSlots = new WeakMap();
            const short = value => {
                const number = Number(value);
                if (!Number.isFinite(number) || number === 0) return 0;
                const wrapped = ((Math.trunc(number) % 65536) + 65536) % 65536;
                return wrapped >= 32768 ? wrapped - 65536 : wrapped;
            };
            const unsignedShort = value => {
                const number = Number(value);
                if (!Number.isFinite(number) || number === 0) return 0;
                return ((Math.trunc(number) % 65536) + 65536) % 65536;
            };
            const mouseSlots = event => {
                const slots = mouseEventSlots.get(event);
                if (!slots) throw new TypeError('Illegal MouseEvent receiver');
                return slots;
            };
            class MouseEvent extends UIEvent {
                constructor(type, eventInitDict = {}) {
                    super(type, eventInitDict);
                    const init = eventInitDict || {};
                    mouseEventSlots.set(this, {
                        screenX: uiEventLong(init.screenX === undefined ? 0 : init.screenX),
                        screenY: uiEventLong(init.screenY === undefined ? 0 : init.screenY),
                        clientX: uiEventLong(init.clientX === undefined ? 0 : init.clientX),
                        clientY: uiEventLong(init.clientY === undefined ? 0 : init.clientY),
                        ctrlKey: !!init.ctrlKey, shiftKey: !!init.shiftKey,
                        altKey: !!init.altKey, metaKey: !!init.metaKey,
                        button: short(init.button === undefined ? 0 : init.button),
                        buttons: unsignedShort(init.buttons === undefined ? 0 : init.buttons),
                        relatedTarget: init.relatedTarget == null ? null : init.relatedTarget,
                    });
                }
                get screenX() { return mouseSlots(this).screenX; }
                get screenY() { return mouseSlots(this).screenY; }
                get clientX() { return mouseSlots(this).clientX; }
                get clientY() { return mouseSlots(this).clientY; }
                get ctrlKey() { return mouseSlots(this).ctrlKey; }
                get shiftKey() { return mouseSlots(this).shiftKey; }
                get altKey() { return mouseSlots(this).altKey; }
                get metaKey() { return mouseSlots(this).metaKey; }
                get button() { return mouseSlots(this).button; }
                get buttons() { return mouseSlots(this).buttons; }
                get relatedTarget() { return mouseSlots(this).relatedTarget; }
                get which() { return mouseSlots(this).button + 1; }
                get pageX() { return this.clientX + (globalThis.scrollX || 0); }
                get pageY() { return this.clientY + (globalThis.scrollY || 0); }
                get x() { return this.clientX; }
                get y() { return this.clientY; }
                get offsetX() { return this.clientX; }
                get offsetY() { return this.clientY; }
                getModifierState(key) {
                    const state = mouseSlots(this);
                    switch (String(key)) {
                        case 'Control': return state.ctrlKey;
                        case 'Shift': return state.shiftKey;
                        case 'Alt': return state.altKey;
                        case 'Meta': return state.metaKey;
                        default: return false;
                    }
                }
                initMouseEvent(type, bubbles = false, cancelable = false, view = null, detail = 0,
                    screenX = 0, screenY = 0, clientX = 0, clientY = 0, ctrlKey = false,
                    altKey = false, shiftKey = false, metaKey = false, button = 0,
                    relatedTarget = null) {
                    const ui = uiEventSlots.get(this);
                    const mouse = mouseSlots(this);
                    if (!ui) throw new TypeError('Illegal MouseEvent receiver');
                    if (this.__rixDispatching) return;
                    this.initEvent(type, bubbles, cancelable);
                    this.composed = false;
                    ui.view = view == null ? null : view;
                    ui.detail = uiEventLong(detail);
                    Object.assign(mouse, {
                        screenX: uiEventLong(screenX), screenY: uiEventLong(screenY),
                        clientX: uiEventLong(clientX), clientY: uiEventLong(clientY),
                        ctrlKey: !!ctrlKey, altKey: !!altKey, shiftKey: !!shiftKey,
                        metaKey: !!metaKey, button: short(button), buttons: 0,
                        relatedTarget: relatedTarget == null ? null : relatedTarget,
                    });
                }
            }
            Object.defineProperty(MouseEvent.prototype, Symbol.toStringTag, {value: 'MouseEvent', configurable: true});
            Object.defineProperty(MouseEvent, 'name', {value: 'MouseEvent'});
            globalThis.MouseEvent = MouseEvent;
        }

        if (!globalThis.PointerEvent) {
            const pointerEventSlots = new WeakMap();
            const pointerSlots = event => {
                const slots = pointerEventSlots.get(event);
                if (!slots) throw new TypeError('Illegal PointerEvent receiver');
                return slots;
            };
            const finiteDouble = value => {
                const number = Number(value);
                if (!Number.isFinite(number)) return 0;
                return number;
            };
            class PointerEvent extends MouseEvent {
                constructor(type, eventInitDict = {}) {
                    super(type, eventInitDict);
                    const init = eventInitDict || {};
                    pointerEventSlots.set(this, {
                        pointerId: uiEventLong(init.pointerId === undefined ? 0 : init.pointerId),
                        width: finiteDouble(init.width === undefined ? 1 : init.width),
                        height: finiteDouble(init.height === undefined ? 1 : init.height),
                        pressure: finiteDouble(init.pressure === undefined ? 0 : init.pressure),
                        tangentialPressure: finiteDouble(init.tangentialPressure === undefined ? 0 : init.tangentialPressure),
                        tiltX: uiEventLong(init.tiltX === undefined ? 0 : init.tiltX),
                        tiltY: uiEventLong(init.tiltY === undefined ? 0 : init.tiltY),
                        twist: uiEventLong(init.twist === undefined ? 0 : init.twist),
                        pointerType: init.pointerType === undefined ? '' : String(init.pointerType),
                        isPrimary: !!init.isPrimary,
                    });
                }
                get pointerId() { return pointerSlots(this).pointerId; }
                get width() { return pointerSlots(this).width; }
                get height() { return pointerSlots(this).height; }
                get pressure() { return pointerSlots(this).pressure; }
                get tangentialPressure() { return pointerSlots(this).tangentialPressure; }
                get tiltX() { return pointerSlots(this).tiltX; }
                get tiltY() { return pointerSlots(this).tiltY; }
                get twist() { return pointerSlots(this).twist; }
                get pointerType() { return pointerSlots(this).pointerType; }
                get isPrimary() { return pointerSlots(this).isPrimary; }
                getCoalescedEvents() { pointerSlots(this); return []; }
                getPredictedEvents() { pointerSlots(this); return []; }
            }
            Object.defineProperty(PointerEvent.prototype, Symbol.toStringTag, {value: 'PointerEvent', configurable: true});
            Object.defineProperty(PointerEvent, 'name', {value: 'PointerEvent'});
            globalThis.PointerEvent = PointerEvent;
        }

        if (!globalThis.WheelEvent) {
            const wheelEventSlots = new WeakMap();
            const wheelSlots = event => {
                const slots = wheelEventSlots.get(event);
                if (!slots) throw new TypeError('Illegal WheelEvent receiver');
                return slots;
            };
            const finiteDouble = value => {
                const number = Number(value);
                if (!Number.isFinite(number)) throw new TypeError('WheelEvent delta must be finite');
                return number;
            };
            const unsignedLong = value => {
                const number = Number(value);
                if (!Number.isFinite(number) || number === 0) return 0;
                return ((Math.trunc(number) % 4294967296) + 4294967296) % 4294967296;
            };
            class WheelEvent extends MouseEvent {
                constructor(type, eventInitDict = {}) {
                    super(type, eventInitDict);
                    const init = eventInitDict || {};
                    wheelEventSlots.set(this, {
                        deltaX: finiteDouble(init.deltaX === undefined ? 0 : init.deltaX),
                        deltaY: finiteDouble(init.deltaY === undefined ? 0 : init.deltaY),
                        deltaZ: finiteDouble(init.deltaZ === undefined ? 0 : init.deltaZ),
                        deltaMode: unsignedLong(init.deltaMode === undefined ? 0 : init.deltaMode),
                    });
                }
                get deltaX() { return wheelSlots(this).deltaX; }
                get deltaY() { return wheelSlots(this).deltaY; }
                get deltaZ() { return wheelSlots(this).deltaZ; }
                get deltaMode() { return wheelSlots(this).deltaMode; }
                initWheelEvent(type, bubbles = false, cancelable = false, view = null, detail = 0,
                    screenX = 0, screenY = 0, clientX = 0, clientY = 0, ctrlKey = false,
                    altKey = false, shiftKey = false, metaKey = false, button = 0,
                    relatedTarget = null, deltaX = 0, deltaY = 0, deltaZ = 0, deltaMode = 0) {
                    const slots = wheelSlots(this);
                    if (this.__rixDispatching) return;
                    MouseEvent.prototype.initMouseEvent.call(this, type, bubbles, cancelable, view,
                        detail, screenX, screenY, clientX, clientY, ctrlKey, altKey, shiftKey,
                        metaKey, button, relatedTarget);
                    Object.assign(slots, {
                        deltaX: finiteDouble(deltaX), deltaY: finiteDouble(deltaY),
                        deltaZ: finiteDouble(deltaZ), deltaMode: unsignedLong(deltaMode),
                    });
                }
            }
            Object.defineProperties(WheelEvent, {
                DOM_DELTA_PIXEL: {value: 0}, DOM_DELTA_LINE: {value: 1}, DOM_DELTA_PAGE: {value: 2},
            });
            Object.defineProperty(WheelEvent.prototype, Symbol.toStringTag, {value: 'WheelEvent', configurable: true});
            Object.defineProperty(WheelEvent, 'name', {value: 'WheelEvent'});
            globalThis.WheelEvent = WheelEvent;
        }

        if (!globalThis.FocusEvent) {
            const focusEventSlots = new WeakMap();
            const focusSlots = event => {
                const slots = focusEventSlots.get(event);
                if (!slots) throw new TypeError('Illegal FocusEvent receiver');
                return slots;
            };
            class FocusEvent extends UIEvent {
                constructor(type, eventInitDict = {}) {
                    super(type, eventInitDict);
                    const init = eventInitDict || {};
                    focusEventSlots.set(this, {relatedTarget: init.relatedTarget == null ? null : init.relatedTarget});
                }
                get relatedTarget() { return focusSlots(this).relatedTarget; }
                initFocusEvent(type, bubbles = false, cancelable = false, view = null, detail = 0, relatedTarget = null) {
                    const slots = focusSlots(this);
                    const ui = uiEventSlots.get(this);
                    if (!ui) throw new TypeError('Illegal FocusEvent receiver');
                    if (this.__rixDispatching) return;
                    this.initEvent(type, bubbles, cancelable);
                    this.composed = false;
                    ui.view = view == null ? null : view;
                    ui.detail = uiEventLong(detail);
                    slots.relatedTarget = relatedTarget == null ? null : relatedTarget;
                }
            }
            Object.defineProperty(FocusEvent.prototype, Symbol.toStringTag, {value: 'FocusEvent', configurable: true});
            Object.defineProperty(FocusEvent, 'name', {value: 'FocusEvent'});
            globalThis.FocusEvent = FocusEvent;
        }

        if (!globalThis.KeyboardEvent) {
            const keyboardEventSlots = new WeakMap();
            const keyboardSlots = event => {
                const slots = keyboardEventSlots.get(event);
                if (!slots) throw new TypeError('Illegal KeyboardEvent receiver');
                return slots;
            };
            const unsignedLong = value => {
                const number = Number(value);
                if (!Number.isFinite(number) || number === 0) return 0;
                return ((Math.trunc(number) % 4294967296) + 4294967296) % 4294967296;
            };
            const legacyKeyboardValues = event => {
                const state = keyboardEventSlots.get(event);
                if (!state) throw new TypeError('Illegal KeyboardEvent receiver');
                if (typeof globalThis.__rixLegacyKeyCode === 'function') {
                    const values = globalThis.__rixLegacyKeyCode(String(event.type || ''), state.key, state.code);
                    if (Array.isArray(values)) return values;
                }
                return [0, 0, 0];
            };
            class KeyboardEvent extends UIEvent {
                constructor(type, eventInitDict = {}) {
                    super(type, eventInitDict);
                    const init = eventInitDict || {};
                    keyboardEventSlots.set(this, {
                        key: init.key === undefined ? '' : String(init.key),
                        code: init.code === undefined ? '' : String(init.code),
                        location: unsignedLong(init.location === undefined ? 0 : init.location),
                        ctrlKey: !!init.ctrlKey, shiftKey: !!init.shiftKey,
                        altKey: !!init.altKey, metaKey: !!init.metaKey,
                        repeat: !!init.repeat, isComposing: !!init.isComposing,
                    });
                }
                get key() { return keyboardSlots(this).key; }
                get code() { return keyboardSlots(this).code; }
                get location() { return keyboardSlots(this).location; }
                get ctrlKey() { return keyboardSlots(this).ctrlKey; }
                get shiftKey() { return keyboardSlots(this).shiftKey; }
                get altKey() { return keyboardSlots(this).altKey; }
                get metaKey() { return keyboardSlots(this).metaKey; }
                get repeat() { return keyboardSlots(this).repeat; }
                get isComposing() { return keyboardSlots(this).isComposing; }
                get charCode() { return legacyKeyboardValues(this)[1]; }
                get keyCode() { return legacyKeyboardValues(this)[0]; }
                get which() { return legacyKeyboardValues(this)[2]; }
                getModifierState(key) {
                    const state = keyboardSlots(this);
                    switch (String(key)) {
                        case 'Control': return state.ctrlKey;
                        case 'Shift': return state.shiftKey;
                        case 'Alt': return state.altKey;
                        case 'Meta': return state.metaKey;
                        default: return false;
                    }
                }
                initKeyboardEvent(type, bubbles = false, cancelable = false, view = null,
                    key = '', location = 0, ctrlKey = false, altKey = false,
                    shiftKey = false, metaKey = false) {
                    const state = keyboardSlots(this);
                    const ui = uiEventSlots.get(this);
                    if (!ui) throw new TypeError('Illegal KeyboardEvent receiver');
                    if (this.__rixDispatching) return;
                    this.initEvent(type, bubbles, cancelable);
                    this.composed = false;
                    ui.view = view == null ? null : view;
                    ui.detail = 0;
                    Object.assign(state, {
                        key: String(key), code: '', location: unsignedLong(location),
                        ctrlKey: !!ctrlKey, altKey: !!altKey, shiftKey: !!shiftKey,
                        metaKey: !!metaKey, repeat: false, isComposing: false,
                    });
                }
            }
            Object.defineProperty(KeyboardEvent.prototype, Symbol.toStringTag, {value: 'KeyboardEvent', configurable: true});
            Object.defineProperty(KeyboardEvent, 'name', {value: 'KeyboardEvent'});
            globalThis.KeyboardEvent = KeyboardEvent;
        }

        if (!globalThis.CustomEvent) {
            const customEventSlots = new WeakMap();
            const customEventState = event => {
                const slots = customEventSlots.get(event);
                if (!slots) throw new TypeError('Illegal CustomEvent receiver');
                return slots;
            };
            class CustomEvent extends globalThis.Event {
                constructor(type, eventInitDict) {
                    super(type, eventInitDict);
                    const init = eventInitDict || {};
                    customEventSlots.set(this, {detail: init.detail !== undefined ? init.detail : null});
                }
                get detail() { return customEventState(this).detail; }
                initCustomEvent(type, bubbles = false, cancelable = false, detail = null) {
                    const slots = customEventState(this);
                    if (this.__rixDispatching) return;
                    this.initEvent(type, bubbles, cancelable);
                    slots.detail = detail;
                }
            }
            Object.defineProperty(CustomEvent.prototype, Symbol.toStringTag, {value: 'CustomEvent', configurable: true});
            globalThis.CustomEvent = CustomEvent;
        }

        if (!globalThis.InputEvent) {
            const inputEventSlots = new WeakMap();
            const inputSlots = event => {
                const slots = inputEventSlots.get(event);
                if (!slots) throw new TypeError('Illegal InputEvent receiver');
                return slots;
            };
            class InputEvent extends UIEvent {
                constructor(type, eventInitDict = {}) {
                    super(type, eventInitDict);
                    const init = eventInitDict || {};
                    inputEventSlots.set(this, {
                        data: init.data === undefined ? null : (init.data == null ? null : String(init.data)),
                        isComposing: !!init.isComposing,
                        inputType: init.inputType === undefined ? '' : String(init.inputType),
                        dataTransfer: init.dataTransfer == null ? null : init.dataTransfer,
                    });
                }
                get data() { return inputSlots(this).data; }
                get isComposing() { return inputSlots(this).isComposing; }
                get inputType() { return inputSlots(this).inputType; }
                get dataTransfer() { return inputSlots(this).dataTransfer; }
                getTargetRanges() { inputSlots(this); return []; }
            }
            Object.defineProperty(InputEvent.prototype, Symbol.toStringTag, {value: 'InputEvent', configurable: true});
            Object.defineProperty(InputEvent, 'name', {value: 'InputEvent'});
            globalThis.InputEvent = InputEvent;
        }

        if (!globalThis.PopStateEvent) {
            const popStateSlots = new WeakMap();
            const popState = event => {
                const state = popStateSlots.get(event);
                if (!state) throw new TypeError('Illegal PopStateEvent receiver');
                return state;
            };
            class PopStateEvent extends Event {
                constructor(type, eventInitDict = {}) {
                    super(type, eventInitDict);
                    const init = eventInitDict || {};
                    popStateSlots.set(this, {state: init.state === undefined ? null : init.state});
                }
                get state() { return popState(this).state; }
                initPopStateEvent(type, bubbles = false, cancelable = false, state = null) {
                    const slots = popState(this);
                    if (this.__rixDispatching) return;
                    this.initEvent(type, bubbles, cancelable);
                    slots.state = state;
                }
            }
            Object.defineProperty(PopStateEvent.prototype, Symbol.toStringTag, {value: 'PopStateEvent', configurable: true});
            Object.defineProperty(PopStateEvent, 'name', {value: 'PopStateEvent'});
            globalThis.PopStateEvent = PopStateEvent;
        }

        if (!globalThis.HashChangeEvent) {
            const hashChangeSlots = new WeakMap();
            const hashChange = event => {
                const state = hashChangeSlots.get(event);
                if (!state) throw new TypeError('Illegal HashChangeEvent receiver');
                return state;
            };
            class HashChangeEvent extends Event {
                constructor(type, eventInitDict = {}) {
                    super(type, eventInitDict);
                    const init = eventInitDict || {};
                    hashChangeSlots.set(this, {
                        oldURL: init.oldURL === undefined ? '' : String(init.oldURL),
                        newURL: init.newURL === undefined ? '' : String(init.newURL),
                    });
                }
                get oldURL() { return hashChange(this).oldURL; }
                get newURL() { return hashChange(this).newURL; }
                initHashChangeEvent(type, bubbles = false, cancelable = false, oldURL = '', newURL = '') {
                    const slots = hashChange(this);
                    if (this.__rixDispatching) return;
                    this.initEvent(type, bubbles, cancelable);
                    slots.oldURL = String(oldURL);
                    slots.newURL = String(newURL);
                }
            }
            Object.defineProperty(HashChangeEvent.prototype, Symbol.toStringTag, {value: 'HashChangeEvent', configurable: true});
            Object.defineProperty(HashChangeEvent, 'name', {value: 'HashChangeEvent'});
            globalThis.HashChangeEvent = HashChangeEvent;
        }

        if (!globalThis.MessageEvent) {
            const messageEventSlots = new WeakMap();
            const messageEventState = event => {
                const state = messageEventSlots.get(event);
                if (!state) throw new TypeError('Illegal MessageEvent receiver');
                return state;
            };
            class MessageEvent extends Event {
                constructor(type, eventInitDict = {}) {
                    super(type, eventInitDict);
                    const init = eventInitDict || {};
                    messageEventSlots.set(this, {
                        data: init.data === undefined ? null : init.data,
                        origin: init.origin === undefined ? '' : String(init.origin),
                        lastEventId: init.lastEventId === undefined ? '' : String(init.lastEventId),
                        source: init.source === undefined ? null : init.source,
                        ports: Object.freeze(Array.from(init.ports || [])),
                    });
                }
                get data() { return messageEventState(this).data; }
                get origin() { return messageEventState(this).origin; }
                get lastEventId() { return messageEventState(this).lastEventId; }
                get source() { return messageEventState(this).source; }
                get ports() { return messageEventState(this).ports; }
            }
            Object.defineProperty(MessageEvent.prototype, Symbol.toStringTag, {value: 'MessageEvent', configurable: true});
            globalThis.MessageEvent = MessageEvent;
        }

        const serializeMessageForTransfer = globalThis.__rixSerializeWindowMessage;
        delete globalThis.__rixSerializeWindowMessage;
        if (!globalThis.MessageChannel && !globalThis.MessagePort) {
            const messagePortSlots = new WeakMap();
            const messagePortEventHandlers = new WeakMap();
            const preparedPortTransfers = new WeakMap();
            const messagePortToken = Symbol('MessagePort constructor token');
            const messagePortState = port => {
                const state = messagePortSlots.get(port);
                if (!state) throw new DOMException('The MessagePort is detached or has an incompatible receiver', 'InvalidStateError');
                return state;
            };
            const makeMessagePort = state => {
                const port = new MessagePort(state, messagePortToken);
                state.owner = port;
                return port;
            };
            const schedulePortDelivery = state => {
                if (!state.started || state.closed || state.scheduled || state.queue.length === 0) return;
                state.scheduled = true;
                const deliver = () => {
                    state.scheduled = false;
                    if (!state.started || state.closed) return;
                    const entry = state.queue.shift();
                    if (entry) {
                        const event = new MessageEvent('message', {data: entry.data, ports: entry.ports});
                        state.owner.dispatchEvent(event);
                    }
                    schedulePortDelivery(state);
                };
                if (typeof globalThis.setTimeout === 'function') globalThis.setTimeout(deliver, 0);
                else queueMicrotask(deliver);
            };
            class MessagePort extends EventTarget {
                constructor(state, token) {
                    super();
                    if (token !== messagePortToken) throw new TypeError('Illegal constructor: MessagePort');
                    markNonCloneable(this);
                    messagePortSlots.set(this, state);
                    messagePortEventHandlers.set(this, {onmessage: null, onmessageerror: null});
                }
                postMessage(message, options = undefined) {
                    const state = messagePortState(this);
                    if (state.closed) return;
                    let transfer = options;
                    if (options && typeof options === 'object' && !Array.isArray(options) && typeof options[Symbol.iterator] !== 'function') {
                        transfer = options.transfer;
                    }
                    const serialized = serializeMessageForTransfer(message, transfer, this);
                    const receiver = state.peer;
                    if (!receiver || receiver.closed) return;
                    receiver.queue.push({data: serialized.data, ports: serialized.ports});
                    schedulePortDelivery(receiver);
                }
                start() {
                    const state = messagePortState(this);
                    if (!state.closed) {
                        state.started = true;
                        schedulePortDelivery(state);
                    }
                }
                close() {
                    const state = messagePortState(this);
                    state.closed = true;
                    state.queue.length = 0;
                }
                addEventListener(type, listener, options) {
                    super.addEventListener(type, listener, options);
                    if (String(type) === 'message') this.start();
                }
                removeEventListener(type, listener, options) {
                    super.removeEventListener(type, listener, options);
                }
                get onmessage() {
                    messagePortState(this);
                    return messagePortEventHandlers.get(this).onmessage;
                }
                set onmessage(callback) {
                    messagePortState(this);
                    if (callback !== null && callback !== undefined && typeof callback !== 'function') {
                        throw new TypeError('MessagePort.onmessage must be a function or null');
                    }
                    const handlers = messagePortEventHandlers.get(this);
                    handlers.onmessage = callback == null ? null : callback;
                    if (handlers.onmessage) this.start();
                }
                get onmessageerror() {
                    messagePortState(this);
                    return messagePortEventHandlers.get(this).onmessageerror;
                }
                set onmessageerror(callback) {
                    messagePortState(this);
                    if (callback !== null && callback !== undefined && typeof callback !== 'function') {
                        throw new TypeError('MessagePort.onmessageerror must be a function or null');
                    }
                    messagePortEventHandlers.get(this).onmessageerror = callback == null ? null : callback;
                }
            }
            Object.defineProperty(MessagePort.prototype, Symbol.toStringTag, {value: 'MessagePort', configurable: true});
            class MessageChannel {
                constructor() {
                    const port1State = {peer: null, owner: null, queue: [], started: false, closed: false, scheduled: false};
                    const port2State = {peer: port1State, owner: null, queue: [], started: false, closed: false, scheduled: false};
                    port1State.peer = port2State;
                    this.port1 = makeMessagePort(port1State);
                    this.port2 = makeMessagePort(port2State);
                }
            }
            const messagePortTransferHooks = {
                isMessagePort: port => messagePortSlots.has(port),
                prepareTransfer(port) {
                    const state = messagePortState(port);
                    if (state.closed) throw new DOMException('A closed MessagePort cannot be transferred', 'DataCloneError');
                    const target = new MessagePort(state, messagePortToken);
                    preparedPortTransfers.set(port, target);
                    return target;
                },
                validateTransfer(port, target) {
                    const state = messagePortState(port);
                    if (state.closed || preparedPortTransfers.get(port) !== target) {
                        throw new DOMException('The MessagePort cannot be transferred', 'DataCloneError');
                    }
                    return true;
                },
                commitTransfer(port, target) {
                    const state = messagePortState(port);
                    if (state.closed || preparedPortTransfers.get(port) !== target) {
                        throw new DOMException('The MessagePort cannot be transferred', 'DataCloneError');
                    }
                    messagePortSlots.delete(port);
                    state.owner = target;
                    preparedPortTransfers.delete(port);
                    return true;
                },
                abortTransfer(port) {
                    preparedPortTransfers.delete(port);
                    return true;
                },
            };
            Object.defineProperty(MessageChannel.prototype, Symbol.toStringTag, {value: 'MessageChannel', configurable: true});
            Object.defineProperty(MessagePort, 'name', {value: 'MessagePort'});
            Object.defineProperty(MessageChannel, 'name', {value: 'MessageChannel'});
            globalThis.MessagePort = MessagePort;
            globalThis.MessageChannel = MessageChannel;
            globalThis.__rixMessagePortTransferHooks = messagePortTransferHooks;
        }

        if (!globalThis.Window.prototype.postMessage) {
            const windowIdentity = globalThis.window || globalThis;
            const WindowPrototype = globalThis.Window.prototype;
            const URLConstructor = globalThis.URL;
            const DOMExceptionConstructor = globalThis.DOMException;
            const MessageEventConstructor = globalThis.MessageEvent;
            const dispatchWindowEvent = globalThis.EventTarget.prototype.dispatchEvent;
            const scheduleMessageTask = globalThis.setTimeout;
            const toUSVString = value => {
                if (typeof value === 'symbol') throw new TypeError('Cannot convert a Symbol value to a string');
                return String(value);
            };
            const parseTargetOrigin = (value, sourceOrigin) => {
                if (value === '/') return sourceOrigin;
                if (value === '*') return '*';
                let parsedOrigin;
                try { parsedOrigin = new URLConstructor(value).origin; }
                catch (_) { throw new DOMExceptionConstructor('Invalid target origin', 'SyntaxError'); }
                return parsedOrigin === 'null' ? null : parsedOrigin;
            };
            const postMessage = function(message, targetOriginOrOptions, transfer) {
                'use strict';
                if (!isWindow(this)) throw new TypeError('Illegal Window.postMessage receiver');

                if (this.__frameElement && !this.__frameElement.isConnected) return;

                const targetWindow = this;
                const sourceWindow = getActiveWindow();
                const sourceOrigin = getWindowOrigin(sourceWindow);

                let targetOrigin = '/';
                let transferValue;
                if (typeof targetOriginOrOptions === 'string') {
                    targetOrigin = toUSVString(targetOriginOrOptions);
                    transferValue = transfer;
                } else if (targetOriginOrOptions === undefined || targetOriginOrOptions === null) {
                    transferValue = undefined;
                } else if (typeof targetOriginOrOptions === 'object' || typeof targetOriginOrOptions === 'function') {
                    const requestedOrigin = targetOriginOrOptions.targetOrigin;
                    if (requestedOrigin !== undefined) targetOrigin = toUSVString(requestedOrigin);
                    transferValue = targetOriginOrOptions.transfer;
                } else {
                    throw new TypeError('Window.postMessage options must be a string or dictionary');
                }
                const parsedTargetOrigin = parseTargetOrigin(targetOrigin, sourceOrigin);
                const serialized = serializeMessageForTransfer(message, transferValue);

                const schedule = typeof scheduleMessageTask === 'function'
                    ? (fn) => scheduleMessageTask.call(windowIdentity, fn, 0)
                    : (typeof queueMicrotask === 'function' ? queueMicrotask : (fn) => fn());

                schedule(() => {
                    if (targetWindow.__frameElement && !targetWindow.__frameElement.isConnected) return;
                    const targetCurrentOrigin = getWindowOrigin(targetWindow);
                    if (parsedTargetOrigin !== '*' && (parsedTargetOrigin === null || parsedTargetOrigin !== targetCurrentOrigin)) return;
                    const event = new MessageEventConstructor('message', {
                        data: serialized.data,
                        origin: sourceOrigin,
                        source: sourceWindow,
                        ports: serialized.ports,
                    });
                    dispatchWindowEvent.call(targetWindow, event);
                });
            };
            Object.defineProperty(postMessage, 'length', {value: 2});
            Object.defineProperty(WindowPrototype, 'postMessage', {
                value: postMessage,
                writable: true,
                configurable: true,
            });
        }

        if (!globalThis.XMLHttpRequest && typeof globalThis.fetch === 'function') {
            const xhrSlots = new WeakMap();
            const uploadSlots = new WeakMap();
            const xhrState = xhr => {
                const state = xhrSlots.get(xhr);
                if (!state) throw new TypeError('Illegal XMLHttpRequest receiver');
                return state;
            };
            const uploadState = upload => {
                if (!uploadSlots.has(upload)) throw new TypeError('Illegal XMLHttpRequestUpload receiver');
            };
            const xhrHandlerSlots = new WeakMap();
            class XMLHttpRequestEventTarget extends EventTarget {
                constructor() { super(); xhrHandlerSlots.set(this, Object.create(null)); }
            }
            const defineHandler = (prototype, name) => Object.defineProperty(prototype, name, {
                get() {
                    const handlers = xhrHandlerSlots.get(this);
                    if (!handlers) throw new TypeError('Illegal XMLHttpRequestEventTarget receiver');
                    return handlers[name] ?? null;
                },
                set(value) {
                    const handlers = xhrHandlerSlots.get(this);
                    if (!handlers) throw new TypeError('Illegal XMLHttpRequestEventTarget receiver');
                    if (value !== null && value !== undefined && typeof value !== 'function') throw new TypeError(name + ' must be a function or null');
                    handlers[name] = value == null ? null : value;
                },
                enumerable: true, configurable: true,
            });
            for (const name of ['onloadstart', 'onprogress', 'onabort', 'onerror', 'onload', 'ontimeout', 'onloadend']) {
                defineHandler(XMLHttpRequestEventTarget.prototype, name);
            }
            class XMLHttpRequestUpload extends XMLHttpRequestEventTarget {
                constructor() { super(); uploadSlots.set(this, true); }
                addEventListener(...args) { uploadState(this); return super.addEventListener(...args); }
                removeEventListener(...args) { uploadState(this); return super.removeEventListener(...args); }
                dispatchEvent(event) { uploadState(this); return super.dispatchEvent(event); }
            }
            Object.defineProperty(XMLHttpRequestUpload.prototype, Symbol.toStringTag, {value: 'XMLHttpRequestUpload', configurable: true});
            class XMLHttpRequest extends XMLHttpRequestEventTarget {
                constructor() {
                    super();
                    xhrSlots.set(this, {
                        readyState: 0, method: 'GET', url: '', async: true, headers: Object.create(null),
                        sendFlag: false, generation: 0, responseType: '', response: null,
                        responseText: '', status: 0, statusText: '', responseURL: '',
                        responseHeaders: Object.create(null), timeout: 0, timeoutId: 0,
                        withCredentials: false, overrideMimeType: '', upload: new XMLHttpRequestUpload(),
                    });
                }
                open(method, url, async = true, username, password) {
                    const state = xhrState(this);
                    if (state.sendFlag) this.abort();
                    const methodText = String(method).toUpperCase();
                    if (!/^[!#$%&'*+\-.^_`|~0-9A-Z]+$/.test(methodText) ||
                        ['CONNECT', 'TRACE', 'TRACK'].includes(methodText)) {
                        throw new DOMException('The request method is not supported', 'SecurityError');
                    }
                    if (async === false) throw new DOMException('Synchronous requests are not supported', 'InvalidAccessError');
                    const urlText = String(url);
                    if (username !== undefined || password !== undefined) {
                        throw new DOMException('URL credentials are not supported by XMLHttpRequest.open', 'InvalidAccessError');
                    }
                    state.generation++;
                    state.method = methodText;
                    state.url = urlText;
                    state.async = true;
                    state.headers = Object.create(null);
                    state.sendFlag = false;
                    state.response = null;
                    state.responseText = '';
                    state.status = 0;
                    state.statusText = '';
                    state.responseURL = '';
                    state.responseHeaders = Object.create(null);
                    this.__setReadyState(1);
                }
                setRequestHeader(name, value) {
                    const state = xhrState(this);
                    if (state.readyState !== 1 || state.sendFlag) {
                        throw new DOMException('Request headers can only be set while OPENED and unsent', 'InvalidStateError');
                    }
                    const headerName = String(name).trim();
                    const headerValue = String(value).trim();
                    if (!/^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(headerName) || /[\r\n]/.test(headerValue)) {
                        throw new TypeError('Invalid HTTP header name or value');
                    }
                    if (/^(accept-charset|accept-encoding|access-control-request-headers|access-control-request-method|connection|content-length|cookie|cookie2|date|dnt|expect|host|keep-alive|origin|referer|te|trailer|transfer-encoding|upgrade|via)$/i.test(headerName)) return;
                    const key = headerName.toLowerCase();
                    state.headers[key] = state.headers[key] ? state.headers[key] + ', ' + headerValue : headerValue;
                }
                send(body = null) {
                    const state = xhrState(this);
                    if (state.readyState !== 1 || state.sendFlag) {
                        throw new DOMException('XMLHttpRequest is not in the OPENED unsent state', 'InvalidStateError');
                    }
                    const hasBody = body !== null && body !== undefined;
                    if ((state.method === 'GET' || state.method === 'HEAD') && hasBody) {
                        throw new TypeError('GET and HEAD requests cannot have a body');
                    }
                    let textBody = null;
                    let bytesBody = null;
                    let blobBody = null;
                    let defaultContentType = null;
                    if (hasBody) {
                        if (typeof body === 'string') {
                            textBody = String(body);
                            defaultContentType = 'text/plain;charset=UTF-8';
                        } else if (typeof URLSearchParams === 'function' && body instanceof URLSearchParams) {
                            textBody = String(body);
                            defaultContentType = 'application/x-www-form-urlencoded;charset=UTF-8';
                        } else if (typeof Blob === 'function' && body instanceof Blob) {
                            blobBody = body;
                            if (typeof body.type === 'string' && body.type) defaultContentType = body.type;
                        } else if (body instanceof ArrayBuffer) {
                            bytesBody = new Uint8Array(body.slice(0));
                        } else if (ArrayBuffer.isView(body)) {
                            const offset = body.byteOffset;
                            bytesBody = new Uint8Array(body.buffer.slice(offset, offset + body.byteLength));
                        } else {
                            throw new TypeError('XMLHttpRequest.send() accepts a string, Blob, ArrayBuffer, TypedArray, DataView or URLSearchParams body');
                        }
                    }
                    if (defaultContentType !== null && state.headers['content-type'] === undefined) {
                        state.headers['content-type'] = defaultContentType;
                    }
                    state.sendFlag = true;
                    const generation = state.generation;
                    this.dispatchEvent(new Event('loadstart'));
                    if (state.timeout > 0) {
                        state.timeoutId = setTimeout(() => {
                            const current = xhrState(this);
                            if (current.generation === generation && current.sendFlag) this.__requestError('timeout', generation);
                        }, state.timeout);
                    }
                    const start = payload => {
                        const init = {
                            method: state.method,
                            headers: state.headers,
                            credentials: state.withCredentials ? 'include' : 'same-origin',
                        };
                        if (payload !== null) init.body = payload;
                        globalThis.fetch(state.url, init).then(response => {
                            const current = xhrState(this);
                            if (current.generation !== generation || !current.sendFlag) return null;
                            current.status = response.status;
                            current.statusText = response.statusText;
                            current.responseURL = response.url;
                            const contentType = response.headers && response.headers.get ? response.headers.get('content-type') : null;
                            current.responseHeaders = Object.create(null);
                            if (contentType !== null) current.responseHeaders['content-type'] = contentType;
                            this.__setReadyState(2);
                            if (current.responseType === 'arraybuffer') {
                                return response.arrayBuffer().then(buffer => ({kind: 'arraybuffer', value: buffer}));
                            }
                            return response.text().then(text => ({kind: 'text', value: text}));
                        }).then(result => {
                            const current = xhrState(this);
                            if (!result || current.generation !== generation || !current.sendFlag) return;
                            if (result.kind === 'text') {
                                current.responseText = result.value;
                                if (current.responseType === 'json') {
                                    try { current.response = JSON.parse(result.value); }
                                    catch (_) { current.response = null; }
                                } else {
                                    current.response = result.value;
                                }
                            } else {
                                current.response = result.value;
                            }
                            this.__setReadyState(3);
                            this.dispatchEvent(new Event('progress'));
                            current.sendFlag = false;
                            if (current.timeoutId) clearTimeout(current.timeoutId);
                            current.timeoutId = 0;
                            this.__setReadyState(4);
                            this.dispatchEvent(new Event('load'));
                            this.dispatchEvent(new Event('loadend'));
                        }).catch(() => {
                            const current = xhrState(this);
                            if (current.generation === generation && current.sendFlag) this.__requestError('error', generation);
                        });
                    };
                    if (blobBody !== null) {
                        blobBody.arrayBuffer().then(buffer => {
                            const current = xhrState(this);
                            if (current.generation !== generation || !current.sendFlag) return;
                            start(new Uint8Array(buffer));
                        }).catch(() => {
                            const current = xhrState(this);
                            if (current.generation === generation && current.sendFlag) this.__requestError('error', generation);
                        });
                        return;
                    }
                    start(textBody !== null ? textBody : bytesBody);
                }
                abort() {
                    const state = xhrState(this);
                    if (state.timeoutId) clearTimeout(state.timeoutId);
                    state.timeoutId = 0;
                    const active = state.sendFlag;
                    state.generation++;
                    state.sendFlag = false;
                    state.response = null;
                    state.responseText = '';
                    state.status = 0;
                    state.statusText = '';
                    state.responseURL = '';
                    if (state.readyState === 1 || state.readyState === 2 || state.readyState === 3) {
                        this.__setReadyState(0);
                    }
                    if (active) {
                        this.dispatchEvent(new Event('abort'));
                        this.dispatchEvent(new Event('loadend'));
                    }
                }
                getAllResponseHeaders() {
                    const state = xhrState(this);
                    if (state.readyState < 2) return '';
                    return Object.keys(state.responseHeaders).sort().map(name => name + ': ' + state.responseHeaders[name] + '\r\n').join('');
                }
                getResponseHeader(name) {
                    const state = xhrState(this);
                    if (state.readyState < 2) return null;
                    return state.responseHeaders[String(name).toLowerCase()] ?? null;
                }
                overrideMimeType(mime) {
                    const state = xhrState(this);
                    if (state.readyState === 3 || state.readyState === 4) throw new DOMException('MIME type cannot be changed after loading begins', 'InvalidStateError');
                    state.overrideMimeType = String(mime);
                }
                __setReadyState(value) {
                    const state = xhrState(this);
                    state.readyState = value;
                    this.dispatchEvent(new Event('readystatechange'));
                }
                __requestError(type, generation) {
                    const state = xhrState(this);
                    if (state.generation !== generation || !state.sendFlag) return;
                    state.sendFlag = false;
                    state.response = null;
                    state.responseText = '';
                    state.status = 0;
                    state.statusText = '';
                    state.responseURL = '';
                    if (state.timeoutId) clearTimeout(state.timeoutId);
                    state.timeoutId = 0;
                    this.__setReadyState(4);
                    this.dispatchEvent(new Event(type));
                    this.dispatchEvent(new Event('loadend'));
                }
                addEventListener(...args) { xhrState(this); return super.addEventListener(...args); }
                removeEventListener(...args) { xhrState(this); return super.removeEventListener(...args); }
                dispatchEvent(event) { xhrState(this); return super.dispatchEvent(event); }
            }
            for (const [name, value] of Object.entries({UNSENT: 0, OPENED: 1, HEADERS_RECEIVED: 2, LOADING: 3, DONE: 4})) {
                Object.defineProperty(XMLHttpRequest, name, {value, enumerable: true});
                Object.defineProperty(XMLHttpRequest.prototype, name, {value, enumerable: true});
            }
            for (const [name, get] of Object.entries({
                readyState: state => state.readyState, response: state => state.response,
                responseText: state => {
                    if (state.responseType !== '' && state.responseType !== 'text') throw new DOMException('responseText is unavailable for this responseType', 'InvalidStateError');
                    return state.responseText;
                },
                responseType: state => state.responseType, status: state => state.status,
                statusText: state => state.statusText, responseURL: state => state.responseURL,
                upload: state => state.upload, withCredentials: state => state.withCredentials,
                timeout: state => state.timeout,
            })) {
                Object.defineProperty(XMLHttpRequest.prototype, name, {
                    get() { return get(xhrState(this)); },
                    set: ['responseType', 'withCredentials', 'timeout'].includes(name) ? function(value) {
                        const state = xhrState(this);
                        if (name === 'withCredentials') {
                            if (state.readyState !== 0 && state.readyState !== 1) {
                                throw new DOMException('withCredentials can only be set when state is UNSENT or OPENED', 'InvalidStateError');
                            }
                            if (state.sendFlag) {
                                throw new DOMException('withCredentials cannot be set while request is sending', 'InvalidStateError');
                            }
                            state.withCredentials = !!value;
                        } else if (name === 'responseType') {
                            if (state.readyState === 3 || state.readyState === 4) throw new DOMException('Property cannot be changed while loading', 'InvalidStateError');
                            const type = String(value);
                            if (!['', 'text', 'json', 'arraybuffer'].includes(type)) throw new DOMException('This XMLHttpRequest responseType is not supported', 'NotSupportedError');
                            state.responseType = type;
                        } else {
                            if (state.readyState === 3 || state.readyState === 4) throw new DOMException('Property cannot be changed while loading', 'InvalidStateError');
                            state.timeout = Math.max(0, Math.min(2147483647, Number(value) || 0));
                        }
                    } : undefined,
                    enumerable: true, configurable: true,
                });
            }
            Object.defineProperty(XMLHttpRequest.prototype, Symbol.toStringTag, {value: 'XMLHttpRequest', configurable: true});
            defineHandler(XMLHttpRequest.prototype, 'onreadystatechange');
            Object.defineProperty(XMLHttpRequestEventTarget.prototype, Symbol.toStringTag, {value: 'XMLHttpRequestEventTarget', configurable: true});
            Object.defineProperty(XMLHttpRequestUpload.prototype, 'constructor', {value: XMLHttpRequestUpload, writable: true, configurable: true});
            Object.defineProperty(XMLHttpRequest, 'name', {value: 'XMLHttpRequest'});
            Object.defineProperty(XMLHttpRequestUpload, 'name', {value: 'XMLHttpRequestUpload'});
            Object.defineProperty(XMLHttpRequestEventTarget, 'name', {value: 'XMLHttpRequestEventTarget'});
            globalThis.XMLHttpRequestEventTarget = XMLHttpRequestEventTarget;
            globalThis.XMLHttpRequestUpload = XMLHttpRequestUpload;
            globalThis.XMLHttpRequest = XMLHttpRequest;
        }

        const messagePortTransferHooks = globalThis.__rixMessagePortTransferHooks || null;
        delete globalThis.__rixMessagePortTransferHooks;
        return {
            dispatchWindowListeners,
            messagePortTransferHooks,
            dispatchPromiseRejection(type, promise, reason) {
                const event = new PromiseRejectionEventConstructor(type, {
                    promise,
                    reason,
                    cancelable: type === 'unhandledrejection'
                });
                return globalThis.dispatchEvent(event);
            },
            setEventPath(event, target, path) {
                if (eventPrototype && !eventPrototype.isPrototypeOf(event)) {
                    const ownTarget = Object.getOwnPropertyDescriptor(event, 'target');
                    if (ownTarget && ownTarget.configurable) delete event.target;
                    Object.setPrototypeOf(event, eventPrototype);
                }
                eventTargetState.set(event, target);
                eventPathState.set(event, path);
            },
            clearEventPath(event) {
                eventPathState.delete(event);
            },
        };
        })
    "#;

    let dispatcher =
        crate::structured_clone::register_platform_objects_with_result(context, script)?;
    if let Some(hooks) = dispatcher
        .as_object()
        .map(|dispatcher| dispatcher.get(js_string!("messagePortTransferHooks"), context))
        .transpose()?
        .and_then(|hooks| hooks.as_object())
    {
        crate::structured_clone::register_message_port_transfer_hooks(context, &hooks)?;
    }
    if context.get_data::<EventDispatchHooks>().is_none() {
        if let Some(hooks) = dispatcher.as_object() {
            let dispatch_window = hooks
                .get(js_string!("dispatchWindowListeners"), context)?
                .as_object();
            let set_path = hooks.get(js_string!("setEventPath"), context)?.as_object();
            let clear_path = hooks
                .get(js_string!("clearEventPath"), context)?
                .as_object();
            let dispatch_promise_rejection = hooks
                .get(js_string!("dispatchPromiseRejection"), context)?
                .as_object();
            if let (
                Some(dispatch_window),
                Some(dispatch_promise_rejection),
                Some(set_path),
                Some(clear_path),
            ) = (
                dispatch_window,
                dispatch_promise_rejection,
                set_path,
                clear_path,
            ) {
                context.insert_data(EventDispatchHooks {
                    dispatch_window,
                    dispatch_promise_rejection,
                    set_path,
                    clear_path,
                });
            }
        }
    }
    register_abort_api(context)?;
    super::register_history(context)?;
    Ok(())
}

pub(super) fn secure_random_values_bytes(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let object = args.get_or_undefined(0).as_object().ok_or_else(|| {
        JsNativeError::typ().with_message("crypto.getRandomValues: expected an integer TypedArray")
    })?;
    let typed_array = JsTypedArray::from_object(object).map_err(|_| {
        JsNativeError::typ().with_message("crypto.getRandomValues: expected an integer TypedArray")
    })?;
    let is_integer_array = matches!(
        typed_array.kind(),
        Some(
            TypedArrayKind::Int8
                | TypedArrayKind::Uint8
                | TypedArrayKind::Uint8Clamped
                | TypedArrayKind::Int16
                | TypedArrayKind::Uint16
                | TypedArrayKind::Int32
                | TypedArrayKind::Uint32
                | TypedArrayKind::BigInt64
                | TypedArrayKind::BigUint64
        )
    );
    if !is_integer_array {
        return Err(JsNativeError::typ()
            .with_message("crypto.getRandomValues: expected an integer TypedArray")
            .into());
    }

    let byte_length = typed_array.byte_length(context)?;
    if byte_length > 65_536 {
        return Err(JsNativeError::range()
            .with_message("crypto.getRandomValues: requested view exceeds 65536 bytes")
            .into());
    }

    let mut bytes = vec![0; byte_length];
    read_secure_entropy(&mut bytes).map_err(|error| {
        JsNativeError::error().with_message(format!(
            "crypto.getRandomValues: secure OS entropy unavailable: {error}"
        ))
    })?;

    Ok(JsUint8Array::from_iter(bytes, context)?.into())
}

pub(super) fn is_native_promise(
    _this: &JsValue,
    args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    let is_promise = args
        .get_or_undefined(0)
        .as_object()
        .is_some_and(|value| JsPromise::from_object(value.clone()).is_ok());
    Ok(JsValue::new(is_promise))
}

#[cfg(unix)]
pub(super) fn read_secure_entropy(bytes: &mut [u8]) -> std::io::Result<()> {
    use std::io::Read as _;

    std::fs::File::open("/dev/urandom")?.read_exact(bytes)
}

#[cfg(not(unix))]
pub(super) fn read_secure_entropy(_bytes: &mut [u8]) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no secure OS entropy source is configured for this target",
    ))
}
