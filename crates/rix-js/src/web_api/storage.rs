use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageMutation {
    SetItem {
        origin: String,
        key: String,
        value: String,
    },
    RemoveItem {
        origin: String,
        key: String,
    },
    Clear {
        origin: String,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StorageKind {
    Local,
    Session,
}

const STORAGE_QUOTA_BYTES: usize = 5 * 1024 * 1024;

fn kind_from_arg(val: &JsValue, context: &mut Context) -> StorageKind {
    if let Ok(s) = val.to_string(context) {
        if s.to_std_string_escaped() == "session" {
            return StorageKind::Session;
        }
    }
    StorageKind::Local
}

pub(super) fn storage_get_native(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let kind = kind_from_arg(args.get_or_undefined(0), context);
    let key = args.get_or_undefined(1).to_string(context)?.to_std_string_escaped();
    if let Some(state) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        let borrowed = state.borrow();
        let map = match kind {
            StorageKind::Local => &borrowed.local_storage,
            StorageKind::Session => &borrowed.session_storage,
        };
        if let Some(val) = map.get(&key) {
            return Ok(js_string!(val.clone()).into());
        }
    }
    Ok(JsValue::null())
}

pub(super) fn storage_set_native(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let kind = kind_from_arg(args.get_or_undefined(0), context);
    let key = args.get_or_undefined(1).to_string(context)?.to_std_string_escaped();
    let val = args.get_or_undefined(2).to_string(context)?.to_std_string_escaped();

    if let Some(state_rc) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        let mut state = state_rc.borrow_mut();
        let current_size: usize = match kind {
            StorageKind::Local => state.local_storage.iter().map(|(k, v)| k.len() + v.len()).sum(),
            StorageKind::Session => state.session_storage.iter().map(|(k, v)| k.len() + v.len()).sum(),
        };
        let old_size = match kind {
            StorageKind::Local => state.local_storage.get(&key).map(|v| key.len() + v.len()).unwrap_or(0),
            StorageKind::Session => state.session_storage.get(&key).map(|v| key.len() + v.len()).unwrap_or(0),
        };
        let new_size = current_size.saturating_sub(old_size) + key.len() + val.len();

        if new_size > STORAGE_QUOTA_BYTES {
            let res = ObjectInitializer::new(context)
                .property(js_string!("error"), js_string!("QuotaExceededError"), Attribute::all())
                .property(js_string!("message"), js_string!("Storage quota of 5 MiB exceeded"), Attribute::all())
                .build();
            return Ok(res.into());
        }

        match kind {
            StorageKind::Local => {
                state.local_storage.insert(key.clone(), val.clone());
                let origin = state.current_url.as_ref().map(|u| u.origin()).unwrap_or_else(|| "null".to_string());
                if let Some(store) = &state.cookie_store {
                    let _ = store.set_local_storage(&origin, &key, &val);
                }
                state.record_storage_mutation(StorageMutation::SetItem {
                    origin,
                    key,
                    value: val,
                });
            }
            StorageKind::Session => {
                state.session_storage.insert(key, val);
            }
        }
    }
    Ok(JsValue::undefined())
}

pub(super) fn storage_remove_native(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let kind = kind_from_arg(args.get_or_undefined(0), context);
    let key = args.get_or_undefined(1).to_string(context)?.to_std_string_escaped();

    if let Some(state_rc) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        let mut state = state_rc.borrow_mut();
        match kind {
            StorageKind::Local => {
                state.local_storage.remove(&key);
                let origin = state.current_url.as_ref().map(|u| u.origin()).unwrap_or_else(|| "null".to_string());
                if let Some(store) = &state.cookie_store {
                    let _ = store.remove_local_storage(&origin, &key);
                }
                state.record_storage_mutation(StorageMutation::RemoveItem {
                    origin,
                    key,
                });
            }
            StorageKind::Session => {
                state.session_storage.remove(&key);
            }
        }
    }
    Ok(JsValue::undefined())
}

pub(super) fn storage_clear_native(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let kind = kind_from_arg(args.get_or_undefined(0), context);

    if let Some(state_rc) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        let mut state = state_rc.borrow_mut();
        match kind {
            StorageKind::Local => {
                state.local_storage.clear();
                let origin = state.current_url.as_ref().map(|u| u.origin()).unwrap_or_else(|| "null".to_string());
                if let Some(store) = &state.cookie_store {
                    let _ = store.clear_local_storage(&origin);
                }
                state.record_storage_mutation(StorageMutation::Clear { origin });
            }
            StorageKind::Session => {
                state.session_storage.clear();
            }
        }
    }
    Ok(JsValue::undefined())
}

pub(super) fn storage_key_native(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let kind = kind_from_arg(args.get_or_undefined(0), context);
    let index = args.get_or_undefined(1).to_number(context).unwrap_or(-1.0);
    if index < 0.0 || !index.is_finite() {
        return Ok(JsValue::null());
    }
    let idx = index as usize;
    if let Some(state) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        let borrowed = state.borrow();
        let map = match kind {
            StorageKind::Local => &borrowed.local_storage,
            StorageKind::Session => &borrowed.session_storage,
        };
        if let Some(k) = map.keys().nth(idx) {
            return Ok(js_string!(k.clone()).into());
        }
    }
    Ok(JsValue::null())
}

pub(super) fn storage_length_native(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let kind = kind_from_arg(args.get_or_undefined(0), context);
    if let Some(state) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        let borrowed = state.borrow();
        let map = match kind {
            StorageKind::Local => &borrowed.local_storage,
            StorageKind::Session => &borrowed.session_storage,
        };
        return Ok(JsValue::new(map.len() as u32));
    }
    Ok(JsValue::new(0))
}

pub(super) fn storage_keys_native(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let kind = kind_from_arg(args.get_or_undefined(0), context);
    if let Some(state) = context.get_data::<Rc<RefCell<WebApiState>>>().cloned() {
        let borrowed = state.borrow();
        let map = match kind {
            StorageKind::Local => &borrowed.local_storage,
            StorageKind::Session => &borrowed.session_storage,
        };
        let keys: Vec<JsValue> = map.keys().map(|k| js_string!(k.clone()).into()).collect();
        let arr = boa_engine::object::builtins::JsArray::from_iter(keys, context);
        return Ok(arr.into());
    }
    let arr = boa_engine::object::builtins::JsArray::new(context)?;
    Ok(arr.into())
}

pub(super) fn register_local_storage(context: &mut Context) -> Result<(), JsError> {
    context.register_global_callable(
        js_string!("__rixStorageGet"),
        2,
        NativeFunction::from_fn_ptr(storage_get_native),
    )?;
    context.register_global_callable(
        js_string!("__rixStorageSet"),
        3,
        NativeFunction::from_fn_ptr(storage_set_native),
    )?;
    context.register_global_callable(
        js_string!("__rixStorageRemove"),
        2,
        NativeFunction::from_fn_ptr(storage_remove_native),
    )?;
    context.register_global_callable(
        js_string!("__rixStorageClear"),
        1,
        NativeFunction::from_fn_ptr(storage_clear_native),
    )?;
    context.register_global_callable(
        js_string!("__rixStorageKey"),
        2,
        NativeFunction::from_fn_ptr(storage_key_native),
    )?;
    context.register_global_callable(
        js_string!("__rixStorageLength"),
        1,
        NativeFunction::from_fn_ptr(storage_length_native),
    )?;
    context.register_global_callable(
        js_string!("__rixStorageKeys"),
        1,
        NativeFunction::from_fn_ptr(storage_keys_native),
    )?;

    let script = r#"
        (function() {
            const STORAGE_KIND = Symbol('StorageKind');

            function Storage() {
                throw new TypeError('Illegal constructor');
            }

            Object.defineProperty(Storage, 'name', { value: 'Storage' });

            const proto = Storage.prototype;

            function getStorageKind(receiver) {
                if (!receiver) throw new TypeError('Illegal invocation');
                const kind = receiver[STORAGE_KIND];
                if (kind !== 'local' && kind !== 'session') {
                    throw new TypeError('Illegal invocation');
                }
                return kind;
            }

            Object.defineProperty(proto, 'length', {
                get() {
                    const kind = getStorageKind(this);
                    return globalThis.__rixStorageLength(kind);
                },
                enumerable: false,
                configurable: true,
            });

            Object.defineProperty(proto, 'key', {
                value(index) {
                    const kind = getStorageKind(this);
                    const n = Number(index);
                    if (!Number.isFinite(n) || n < 0) return null;
                    return globalThis.__rixStorageKey(kind, Math.trunc(n));
                },
                writable: true,
                enumerable: false,
                configurable: true,
            });

            Object.defineProperty(proto, 'getItem', {
                value(key) {
                    const kind = getStorageKind(this);
                    return globalThis.__rixStorageGet(kind, String(key));
                },
                writable: true,
                enumerable: false,
                configurable: true,
            });

            Object.defineProperty(proto, 'setItem', {
                value(key, value) {
                    const kind = getStorageKind(this);
                    const res = globalThis.__rixStorageSet(kind, String(key), String(value));
                    if (res && res.error === 'QuotaExceededError') {
                        throw new DOMException(res.message || 'Storage quota exceeded', 'QuotaExceededError');
                    }
                },
                writable: true,
                enumerable: false,
                configurable: true,
            });

            Object.defineProperty(proto, 'removeItem', {
                value(key) {
                    const kind = getStorageKind(this);
                    globalThis.__rixStorageRemove(kind, String(key));
                },
                writable: true,
                enumerable: false,
                configurable: true,
            });

            Object.defineProperty(proto, 'clear', {
                value() {
                    const kind = getStorageKind(this);
                    globalThis.__rixStorageClear(kind);
                },
                writable: true,
                enumerable: false,
                configurable: true,
            });

            Object.defineProperty(proto, Symbol.toStringTag, {
                value: 'Storage',
                configurable: true,
            });

            function createStorageProxy(kind) {
                const target = Object.create(Storage.prototype);
                target[STORAGE_KIND] = kind;

                const proxy = new Proxy(target, {
                    get(target, prop, receiver) {
                        if (typeof prop === 'symbol') {
                            if (prop === STORAGE_KIND) return target[STORAGE_KIND];
                            return Reflect.get(target, prop, receiver);
                        }
                        if (prop in Storage.prototype || prop === 'constructor') {
                            return Reflect.get(target, prop, receiver);
                        }
                        const val = target.getItem(prop);
                        return val !== null ? val : undefined;
                    },
                    set(target, prop, value, receiver) {
                        if (typeof prop === 'symbol') {
                            return Reflect.set(target, prop, value, receiver);
                        }
                        if (prop in Storage.prototype) {
                            return Reflect.set(target, prop, value, receiver);
                        }
                        target.setItem(prop, String(value));
                        return true;
                    },
                    deleteProperty(target, prop) {
                        if (typeof prop === 'symbol') {
                            return Reflect.deleteProperty(target, prop);
                        }
                        if (prop in Storage.prototype) {
                            return false;
                        }
                        target.removeItem(prop);
                        return true;
                    },
                    has(target, prop) {
                        if (typeof prop === 'symbol') {
                            return Reflect.has(target, prop);
                        }
                        if (prop in Storage.prototype) {
                            return true;
                        }
                        return target.getItem(prop) !== null;
                    },
                    ownKeys(target) {
                        const keys = globalThis.__rixStorageKeys(target[STORAGE_KIND]);
                        const seen = new Set();
                        const out = [];
                        if (Array.isArray(keys)) {
                            for (const k of keys) {
                                if (!seen.has(k)) {
                                    seen.add(k);
                                    out.push(k);
                                }
                            }
                        }
                        for (const k of Reflect.ownKeys(target)) {
                            if (!seen.has(k)) {
                                seen.add(k);
                                out.push(k);
                            }
                        }
                        return out;
                    },
                    getOwnPropertyDescriptor(target, prop) {
                        if (typeof prop === 'symbol') {
                            return Reflect.getOwnPropertyDescriptor(target, prop);
                        }
                        if (prop in Storage.prototype) {
                            return Reflect.getOwnPropertyDescriptor(target, prop);
                        }
                        const val = target.getItem(prop);
                        if (val !== null) {
                            return {
                                value: val,
                                writable: true,
                                enumerable: true,
                                configurable: true,
                            };
                        }
                        return Reflect.getOwnPropertyDescriptor(target, prop);
                    },
                });
                return proxy;
            }

            Object.defineProperty(globalThis, 'Storage', {
                value: Storage,
                writable: true,
                configurable: true,
            });

            const localProxy = createStorageProxy('local');
            const sessionProxy = createStorageProxy('session');

            Object.defineProperty(globalThis, 'localStorage', {
                value: localProxy,
                writable: false,
                configurable: true,
            });

            Object.defineProperty(globalThis, 'sessionStorage', {
                value: sessionProxy,
                writable: false,
                configurable: true,
            });

            if (!globalThis.StorageEvent && globalThis.Event) {
                const storageEventSlots = new WeakMap();
                const storageEventState = (event) => {
                    const state = storageEventSlots.get(event);
                    if (!state) throw new TypeError('Illegal StorageEvent receiver');
                    return state;
                };

                class StorageEvent extends Event {
                    constructor(type, eventInitDict = {}) {
                        super(type, eventInitDict);
                        const init = eventInitDict || {};
                        storageEventSlots.set(this, {
                            key: init.key !== undefined ? (init.key === null ? null : String(init.key)) : null,
                            oldValue: init.oldValue !== undefined ? (init.oldValue === null ? null : String(init.oldValue)) : null,
                            newValue: init.newValue !== undefined ? (init.newValue === null ? null : String(init.newValue)) : null,
                            url: init.url !== undefined ? String(init.url) : '',
                            storageArea: init.storageArea !== undefined ? init.storageArea : null,
                        });
                    }

                    get key() { return storageEventState(this).key; }
                    get oldValue() { return storageEventState(this).oldValue; }
                    get newValue() { return storageEventState(this).newValue; }
                    get url() { return storageEventState(this).url; }
                    get storageArea() { return storageEventState(this).storageArea; }

                    initStorageEvent(type, bubbles = false, cancelable = false, key = null, oldValue = null, newValue = null, url = '', storageArea = null) {
                        const slots = storageEventState(this);
                        if (this.__rixDispatching) return;
                        this.initEvent(type, bubbles, cancelable);
                        slots.key = key !== null ? String(key) : null;
                        slots.oldValue = oldValue !== null ? String(oldValue) : null;
                        slots.newValue = newValue !== null ? String(newValue) : null;
                        slots.url = String(url);
                        slots.storageArea = storageArea;
                    }
                }
                Object.defineProperty(StorageEvent.prototype, Symbol.toStringTag, { value: 'StorageEvent', configurable: true });
                Object.defineProperty(StorageEvent, 'name', { value: 'StorageEvent' });
                globalThis.StorageEvent = StorageEvent;
            }

            if (globalThis.Window && Window.prototype) {
                let windowOnStorage = null;
                Object.defineProperty(Window.prototype, 'onstorage', {
                    get() { return windowOnStorage; },
                    set(handler) {
                        if (windowOnStorage) {
                            this.removeEventListener('storage', windowOnStorage);
                        }
                        windowOnStorage = typeof handler === 'function' ? handler : null;
                        if (windowOnStorage) {
                            this.addEventListener('storage', windowOnStorage);
                        }
                    },
                    enumerable: true,
                    configurable: true,
                });
            }
        })();
    "#;

    context.eval(boa_engine::Source::from_bytes(script.as_bytes()))?;
    Ok(())
}
