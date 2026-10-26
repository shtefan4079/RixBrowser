
use super::*;
use boa_engine::Source;

struct DeliveryFunction(JsObject);

pub(super) fn register(context: &mut Context) -> JsResult<()> {
    let delivery = context.eval(Source::from_bytes(
        r#"
        (() => {
            const layoutRevision = globalThis.__rix_layout_revision;
            const observers = new Set();
            const observerSlots = new WeakMap();
            const entrySlots = new WeakMap();
            const entryToken = Symbol('IntersectionObserverEntry');
            let lastLayoutRevision = null;
            let pendingInitialUpdate = false;
            const px = value => {
                const number = Number.parseFloat(value);
                return Number.isFinite(number) ? number : 0;
            };
            const rect = (x, y, width, height) => ({x, y, width, height});
            const copyRect = value => rect(Number(value.x), Number(value.y),
                Number(value.width), Number(value.height));
            const rectRight = value => value.x + value.width;
            const rectBottom = value => value.y + value.height;
            const area = value => Math.max(0, value.width) * Math.max(0, value.height);
            const toNumber = value => +value;
            const toDouble = value => {
                const number = toNumber(value);
                if (!Number.isFinite(number)) throw new TypeError('IntersectionObserver threshold must be finite');
                return number;
            };

            function composedParentElement(element) {
                const parent = element.parentNode;
                if (parent instanceof Element) return parent;
                if (parent && parent.nodeType === 11 && parent.host instanceof Element) return parent.host;
                return null;
            }

            function parseMargin(source) {
                let tokens = (source + '').trim().split(/\s+/).filter(Boolean);
                if (tokens.length === 0) tokens = ['0px'];
                if (tokens.length > 4) throw new SyntaxError('Invalid margin value');
                const parsed = tokens.map(token => {
                    const match = /^([+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?)(px|%|in|cm|mm|q|pt|pc)$/i.exec(token);
                    if (!match) throw new SyntaxError('Invalid margin value');
                    let value = Number(match[1]);
                    const unit = match[2].toLowerCase();
                    if (!Number.isFinite(value)) throw new SyntaxError('Invalid margin value');
                    const factors = {px: 1, in: 96, cm: 96 / 2.54, mm: 96 / 25.4,
                        q: 96 / 101.6, pt: 96 / 72, pc: 16};
                    if (unit !== '%') value *= factors[unit];
                    return {value, percent: unit === '%'};
                });
                if (parsed.length === 1) parsed.push(parsed[0], parsed[0], parsed[0]);
                else if (parsed.length === 2) parsed.push(parsed[0], parsed[1]);
                else if (parsed.length === 3) parsed.push(parsed[1]);
                return parsed;
            }
            function serializeMargin(values) {
                return values.map(({value, percent}) => `${value}${percent ? '%' : 'px'}`).join(' ');
            }
            function applyMargin(value, margin) {
                const resolved = margin.map(item => item.percent ? value.width * item.value / 100 : item.value);
                return rect(value.x - resolved[3], value.y - resolved[0],
                    Math.max(0, value.width + resolved[1] + resolved[3]),
                    Math.max(0, value.height + resolved[0] + resolved[2]));
            }
            function intersect(left, right) {
                const x = Math.max(left.x, right.x);
                const y = Math.max(left.y, right.y);
                return rect(x, y, Math.max(0, Math.min(rectRight(left), rectRight(right)) - x),
                    Math.max(0, Math.min(rectBottom(left), rectBottom(right)) - y));
            }
            function intersectAxis(value, clip, axis) {
                if (axis === 'x') {
                    const x = Math.max(value.x, clip.x);
                    return rect(x, value.y, Math.max(0, Math.min(rectRight(value), rectRight(clip)) - x), value.height);
                }
                const y = Math.max(value.y, clip.y);
                return rect(value.x, y, value.width, Math.max(0, Math.min(rectBottom(value), rectBottom(clip)) - y));
            }
            function paddingRect(element) {
                const border = copyRect(element.getBoundingClientRect());
                const style = getComputedStyle(element);
                const left = px(style.getPropertyValue('border-left-width'));
                const right = px(style.getPropertyValue('border-right-width'));
                const top = px(style.getPropertyValue('border-top-width'));
                const bottom = px(style.getPropertyValue('border-bottom-width'));
                return rect(border.x + left, border.y + top,
                    Math.max(0, border.width - left - right), Math.max(0, border.height - top - bottom));
            }
            function viewportRect() {
                return rect(0, 0, Number(window.innerWidth) || 0, Number(window.innerHeight) || 0);
            }
            function rootRect(state) {
                if (state.root === null || state.root === document) return viewportRect();
                if (state.root.ownerDocument !== document) return rect(0, 0, 0, 0);
                const style = getComputedStyle(state.root);
                const overflowX = style.getPropertyValue('overflow-x');
                const overflowY = style.getPropertyValue('overflow-y');
                const hasContentClip = overflowX !== 'visible' || overflowY !== 'visible';
                let value = hasContentClip ? paddingRect(state.root) : copyRect(state.root.getBoundingClientRect());
                const scrollContainer = [overflowX, overflowY].some(item =>
                    item !== 'visible' && item !== 'clip');
                if (scrollContainer) value = applyMargin(value, state.scrollMargin);
                return value;
            }
            function isDescendant(target, root) {
                for (let parent = composedParentElement(target); parent; parent = composedParentElement(parent)) {
                    if (parent === root) return true;
                }
                return false;
            }
            function clippedTargetRect(target, state, rootBounds) {
                let result = copyRect(target.getBoundingClientRect());
                for (let ancestor = composedParentElement(target); ancestor; ancestor = composedParentElement(ancestor)) {
                    if (state.root instanceof Element && ancestor === state.root) break;
                    const style = getComputedStyle(ancestor);
                    const overflowX = style.getPropertyValue('overflow-x');
                    const overflowY = style.getPropertyValue('overflow-y');
                    const clipsX = overflowX !== 'visible';
                    const clipsY = overflowY !== 'visible';
                    if (!clipsX && !clipsY) continue;
                    let clip = paddingRect(ancestor);
                    const scrollContainer = [overflowX, overflowY].some(value =>
                        value !== 'visible' && value !== 'clip');
                    if (scrollContainer) clip = applyMargin(clip, state.scrollMargin);
                    if (clipsX) result = intersectAxis(result, clip, 'x');
                    if (clipsY) result = intersectAxis(result, clip, 'y');
                }
                return intersect(result, rootBounds);
            }
            function intersectsOrTouches(left, right) {
                return Math.max(left.x, right.x) <= Math.min(rectRight(left), rectRight(right)) &&
                    Math.max(left.y, right.y) <= Math.min(rectBottom(left), rectBottom(right));
            }
            function thresholdIndex(thresholds, ratio) {
                const index = thresholds.findIndex(value => value > ratio);
                return index < 0 ? thresholds.length : index;
            }

            class IntersectionObserverEntry {
                constructor(init) {
                    let value;
                    if (init === entryToken) {
                        value = arguments[1];
                    } else {
                        if (init === null || typeof init !== 'object') {
                            throw new TypeError('IntersectionObserverEntryInit must be an object');
                        }
                        for (const key of ['time', 'rootBounds', 'boundingClientRect', 'intersectionRect',
                            'isIntersecting', 'isVisible', 'intersectionRatio', 'target']) {
                            if (!(key in init)) throw new TypeError(`Missing IntersectionObserverEntryInit.${key}`);
                        }
                        if (!(init.target instanceof Element)) throw new TypeError('target must be an Element');
                        const toRect = value => value === null ? null :
                            DOMRectReadOnly.fromRect(value);
                        value = {
                            time: Number(init.time),
                            rootBounds: toRect(init.rootBounds),
                            boundingClientRect: toRect(init.boundingClientRect),
                            intersectionRect: toRect(init.intersectionRect),
                            isIntersecting: Boolean(init.isIntersecting),
                            intersectionRatio: Number(init.intersectionRatio),
                            isVisible: Boolean(init.isVisible),
                            target: init.target,
                        };
                    }
                    entrySlots.set(this, value);
                }
                get time() { return entrySlots.get(this)?.time ?? illegalEntry(); }
                get rootBounds() { return entrySlots.get(this)?.rootBounds ?? illegalEntry(); }
                get boundingClientRect() { return entrySlots.get(this)?.boundingClientRect ?? illegalEntry(); }
                get intersectionRect() { return entrySlots.get(this)?.intersectionRect ?? illegalEntry(); }
                get isIntersecting() { return entrySlots.get(this)?.isIntersecting ?? illegalEntry(); }
                get isVisible() { return entrySlots.get(this)?.isVisible ?? illegalEntry(); }
                get intersectionRatio() { return entrySlots.get(this)?.intersectionRatio ?? illegalEntry(); }
                get target() { return entrySlots.get(this)?.target ?? illegalEntry(); }
            }
            function illegalEntry() { throw new TypeError('Illegal invocation'); }

            class IntersectionObserver {
                constructor(callback, options = {}) {
                    if (typeof callback !== 'function') throw new TypeError('IntersectionObserver callback must be a function');
                    const init = options == null ? {} : Object(options);
                    const rootValue = init.root;
                    const root = rootValue == null ? null : rootValue;
                    if (root !== null && !(root instanceof Element) && !(root instanceof Document)) {
                        throw new TypeError('IntersectionObserver root must be an Element, Document, or null');
                    }
                    const rootMarginValue = init.rootMargin;
                    const rootMargin = parseMargin(rootMarginValue === undefined ? '0px' : rootMarginValue);
                    const scrollMarginValue = init.scrollMargin;
                    const scrollMargin = parseMargin(scrollMarginValue === undefined ? '0px' : scrollMarginValue);
                    const thresholdValue = init.threshold;
                    let thresholds;
                    if (thresholdValue === undefined) thresholds = [0];
                    else if (typeof thresholdValue === 'number') thresholds = [toDouble(thresholdValue)];
                    else if (typeof thresholdValue === 'string' || typeof thresholdValue === 'boolean' || thresholdValue == null) {
                        thresholds = [toDouble(thresholdValue)];
                    } else if (typeof thresholdValue === 'bigint') {
                        throw new TypeError('BigInt threshold cannot be converted to a number');
                    } else if (thresholdValue && typeof thresholdValue[Symbol.iterator] === 'function') {
                        thresholds = Array.from(thresholdValue, toDouble);
                    } else {
                        thresholds = [toDouble(thresholdValue)];
                    }
                    if (thresholds.some(value => !Number.isFinite(value) || value < 0 || value > 1)) {
                        throw new RangeError('IntersectionObserver threshold must be between 0 and 1');
                    }
                    if (thresholds.length === 0) thresholds = [0];
                    thresholds.sort((left, right) => left - right);
                    const delayValue = init.delay;
                    const delay = delayValue === undefined ? 0 : toNumber(delayValue) | 0;
                    const trackVisibilityValue = init.trackVisibility;
                    const trackVisibility = trackVisibilityValue === undefined ? false : Boolean(trackVisibilityValue);
                    if (trackVisibility) throw new TypeError('IntersectionObserver trackVisibility is not supported');
                    observerSlots.set(this, {
                        callback, root, rootMargin, scrollMargin, thresholds: Object.freeze(thresholds),
                        delay, targets: new Map(), records: [],
                    });
                }
                get root() { return observerState(this).root; }
                get rootMargin() { return serializeMargin(observerState(this).rootMargin); }
                get scrollMargin() { return serializeMargin(observerState(this).scrollMargin); }
                get thresholds() { return observerState(this).thresholds; }
                get delay() { return observerState(this).delay; }
                get trackVisibility() { observerState(this); return false; }
                observe(target) {
                    const state = observerState(this);
                    if (!(target instanceof Element)) throw new TypeError('IntersectionObserver target must be an Element');
                    if (!state.targets.has(target)) {
                        state.targets.set(target, {thresholdIndex: -1, isIntersecting: false, lastUpdate: -Infinity});
                        pendingInitialUpdate = true;
                    }
                    observers.add(this);
                }
                unobserve(target) {
                    const state = observerState(this);
                    if (!(target instanceof Element)) throw new TypeError('IntersectionObserver target must be an Element');
                    state.targets.delete(target);
                    if (state.targets.size === 0) observers.delete(this);
                }
                disconnect() {
                    const state = observerState(this);
                    state.targets.clear();
                    observers.delete(this);
                }
                takeRecords() {
                    const records = observerState(this).records;
                    observerState(this).records = [];
                    return records;
                }
            }
            function observerState(observer) {
                const state = observerSlots.get(observer);
                if (!state) throw new TypeError('Illegal IntersectionObserver receiver');
                return state;
            }

            function deliverIntersectionObservers() {
                if (observers.size === 0) return;
                const revision = layoutRevision(true);
                const hasDelayedObservers = Array.from(observers).some(observer => observerState(observer).delay > 0);
                if (!pendingInitialUpdate && revision === lastLayoutRevision && !hasDelayedObservers) return;
                pendingInitialUpdate = false;
                const now = Number(performance.now());
                for (const observer of observers) {
                    const state = observerSlots.get(observer);
                    if (!state) continue;
                    const baseRoot = rootRect(state);
                    const rootBounds = applyMargin(baseRoot, state.rootMargin);
                    for (const [target, observation] of state.targets) {
                        if (now - observation.lastUpdate < state.delay) continue;
                        observation.lastUpdate = now;
                        let bounding = rect(0, 0, 0, 0);
                        let intersection = rect(0, 0, 0, 0);
                        const validRoot = target.isConnected && target.ownerDocument === document &&
                            (state.root === null || state.root === document ||
                                (state.root.ownerDocument === document && state.root.isConnected &&
                                    isDescendant(target, state.root)));
                        if (validRoot) {
                            bounding = copyRect(target.getBoundingClientRect());
                            intersection = clippedTargetRect(target, state, rootBounds);
                        }
                        const isIntersecting = validRoot && intersectsOrTouches(bounding, rootBounds);
                        const targetArea = area(bounding);
                        const ratio = targetArea > 0 ? area(intersection) / targetArea : (isIntersecting ? 1 : 0);
                        const index = thresholdIndex(state.thresholds, ratio);
                        if (index === observation.thresholdIndex && isIntersecting === observation.isIntersecting) continue;
                        observation.thresholdIndex = index;
                        observation.isIntersecting = isIntersecting;
                        state.records.push(new IntersectionObserverEntry(entryToken, {
                            time: now,
                            rootBounds: new DOMRectReadOnly(rootBounds.x, rootBounds.y, rootBounds.width, rootBounds.height),
                            boundingClientRect: new DOMRectReadOnly(bounding.x, bounding.y, bounding.width, bounding.height),
                            intersectionRect: new DOMRectReadOnly(intersection.x, intersection.y, intersection.width, intersection.height),
                            isIntersecting,
                            intersectionRatio: ratio,
                            isVisible: false,
                            target,
                        }));
                    }
                    if (state.records.length === 0) continue;
                    const entries = state.records;
                    state.records = [];
                    try {
                        state.callback.call(observer, entries, observer);
                    } catch (error) {
                        if (globalThis.console) console.error(error);
                    }
                }
                lastLayoutRevision = revision;
            }

            for (const [name, value] of [['IntersectionObserver', IntersectionObserver],
                ['IntersectionObserverEntry', IntersectionObserverEntry]]) {
                Object.defineProperty(value.prototype, Symbol.toStringTag, {value: name, configurable: true});
                Object.defineProperty(globalThis, name, {value, writable: true, configurable: true});
            }
            return deliverIntersectionObservers;
        })();
        "#,
    ))?;
    let delivery = delivery
        .as_object()
        .filter(JsObject::is_callable)
        .ok_or_else(|| {
            JsNativeError::error()
                .with_message("IntersectionObserver delivery function was not created")
        })?;
    context.insert_data(DeliveryFunction(delivery));
    Ok(())
}

pub(super) fn deliver(context: &mut Context) -> JsResult<()> {
    let Some(callback) = context
        .get_data::<DeliveryFunction>()
        .map(|value| value.0.clone())
    else {
        return Ok(());
    };
    callback.call(&JsValue::undefined(), &[], context)?;
    Ok(())
}
