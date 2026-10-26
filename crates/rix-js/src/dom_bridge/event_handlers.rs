use boa_engine::{Context, JsResult, Source};

pub(super) fn register_event_handler_idl(context: &mut Context) -> JsResult<()> {
    let script = r#"
        (function() {
            const globalHandlers = [
                'abort', 'animationcancel', 'animationend', 'animationiteration',
                'animationstart', 'auxclick', 'beforeinput', 'beforematch', 'beforetoggle', 'blur',
                'cancel', 'canplay', 'canplaythrough', 'change', 'click', 'close', 'command',
                'contextlost', 'contextmenu', 'contextrestored', 'copy', 'cuechange', 'cut',
                'dblclick', 'drag', 'dragend', 'dragenter', 'dragleave', 'dragover',
                'dragstart', 'drop', 'durationchange', 'emptied', 'ended', 'error', 'focus',
                'formdata', 'input', 'invalid', 'keydown', 'keypress', 'keyup', 'load',
                'loadeddata', 'loadedmetadata', 'loadstart', 'mousedown', 'mouseenter',
                'mouseleave', 'mousemove', 'mouseout', 'mouseover', 'mouseup', 'paste',
                'pause', 'play', 'playing', 'progress', 'ratechange', 'reset', 'resize',
                'scroll', 'scrollend', 'securitypolicyviolation', 'seeked', 'seeking',
                'select', 'slotchange', 'stalled', 'submit', 'suspend', 'timeupdate',
                'toggle', 'transitioncancel', 'transitionend', 'transitionrun',
                'transitionstart', 'volumechange', 'waiting', 'webkitanimationend',
                'webkitanimationiteration', 'webkitanimationstart', 'webkittransitionend',
                'wheel', 'pointercancel', 'pointerdown', 'pointerenter', 'pointerleave',
                'pointermove', 'pointerout', 'pointerover', 'pointerup',
                'gotpointercapture', 'lostpointercapture'
            ];
            const windowHandlers = [
                'afterprint', 'beforeprint', 'beforeunload', 'hashchange', 'languagechange',
                'message', 'messageerror', 'offline', 'online', 'pagehide', 'pagereveal',
                'pageshow', 'pageswap', 'popstate', 'rejectionhandled', 'storage',
                'unhandledrejection', 'unload', 'readystatechange'
            ];

            function install(prototype, names, receiver, isWindow) {
                const handlers = new WeakMap();
                for (const type of names) {
                    const name = 'on' + type;
                    Object.defineProperty(prototype, name, {
                        get: function() {
                            if (!(this instanceof receiver)) throw new TypeError('Illegal ' + name + ' receiver');
                            const entry = handlers.get(this);
                            return entry && entry.has(type) ? entry.get(type).callback : null;
                        },
                        set: function(value) {
                            if (!(this instanceof receiver)) throw new TypeError('Illegal ' + name + ' receiver');
                            if (value === undefined || value === null) value = null;
                            else if (typeof value !== 'function') {
                                if (typeof value !== 'object') value = null;
                                else throw new TypeError(name + ' must be a function or null');
                            }

                            let entries = handlers.get(this);
                            const current = entries && entries.get(type);
                            if (current && current.callback === value) return;
                            if (current) {
                                if (value === null) {
                                    this.removeEventListener(type, current.listener);
                                    entries.delete(type);
                                    return;
                                }
                                current.callback = value;
                                return;
                            }
                            if (value === null) return;

                            if (!entries) {
                                entries = new Map();
                                handlers.set(this, entries);
                            }
                            const entry = {callback: value, listener: null};
                            entry.listener = function(event) {
                                const specialError = isWindow && type === 'error' &&
                                    typeof globalThis.ErrorEvent === 'function' && event instanceof ErrorEvent;
                                const result = specialError
                                    ? entry.callback.call(this, event.message, event.filename, event.lineno, event.colno, event.error)
                                    : entry.callback.call(this, event);
                                if ((specialError && result === true) || (!specialError && result === false)) {
                                    event.preventDefault();
                                }
                            };
                            this.addEventListener(type, entry.listener);
                            entries.set(type, entry);
                        },
                        enumerable: true,
                        configurable: true
                    });
                }
            }

            install(HTMLElement.prototype, globalHandlers, HTMLElement, false);
            install(SVGElement.prototype, globalHandlers, SVGElement, false);
            install(Document.prototype, globalHandlers.concat(['readystatechange', 'visibilitychange']), Document, false);
            install(Window.prototype, Array.from(new Set(globalHandlers.concat(windowHandlers))), Window, true);
        })();
    "#;
    context.eval(Source::from_bytes(script.as_bytes()))?;
    Ok(())
}
