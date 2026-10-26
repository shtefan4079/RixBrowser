
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
            const sizeSlots = new WeakMap();
            const entrySlots = new WeakMap();
            const sizeToken = Symbol('ResizeObserverSize');
            const entryToken = Symbol('ResizeObserverEntry');
            let lastLayoutRevision = null;
            let pendingInitialDelivery = false;
            const clamp = value => Math.max(0, value);
            const pixelValue = value => {
                const parsed = Number.parseFloat(value);
                return Number.isFinite(parsed) ? parsed : 0;
            };

            class ResizeObserverSize {
                constructor(token, inlineSize, blockSize) {
                    if (token !== sizeToken) throw new TypeError('Illegal constructor');
                    sizeSlots.set(this, Object.freeze({inlineSize, blockSize}));
                    Object.freeze(this);
                }
                get inlineSize() {
                    const value = sizeSlots.get(this);
                    if (!value) throw new TypeError('Illegal invocation');
                    return value.inlineSize;
                }
                get blockSize() {
                    const value = sizeSlots.get(this);
                    if (!value) throw new TypeError('Illegal invocation');
                    return value.blockSize;
                }
            }

            class ResizeObserverEntry {
                constructor(token, target, contentRect, contentBoxSize, borderBoxSize, devicePixelContentBoxSize) {
                    if (token !== entryToken) throw new TypeError('Illegal constructor');
                    entrySlots.set(this, Object.freeze({
                        target, contentRect, contentBoxSize, borderBoxSize, devicePixelContentBoxSize,
                    }));
                    Object.freeze(this);
                }
                get target() { return entrySlots.get(this)?.target ?? illegalEntry(); }
                get contentRect() { return entrySlots.get(this)?.contentRect ?? illegalEntry(); }
                get contentBoxSize() { return entrySlots.get(this)?.contentBoxSize ?? illegalEntry(); }
                get borderBoxSize() { return entrySlots.get(this)?.borderBoxSize ?? illegalEntry(); }
                get devicePixelContentBoxSize() { return entrySlots.get(this)?.devicePixelContentBoxSize ?? illegalEntry(); }
            }
            function illegalEntry() { throw new TypeError('Illegal invocation'); }

            function boxSizes(target) {
                const style = getComputedStyle(target);
                const paddingLeft = pixelValue(style.getPropertyValue('padding-left'));
                const paddingRight = pixelValue(style.getPropertyValue('padding-right'));
                const paddingTop = pixelValue(style.getPropertyValue('padding-top'));
                const paddingBottom = pixelValue(style.getPropertyValue('padding-bottom'));
                const borderLeft = pixelValue(style.getPropertyValue('border-left-width'));
                const borderRight = pixelValue(style.getPropertyValue('border-right-width'));
                const borderTop = pixelValue(style.getPropertyValue('border-top-width'));
                const borderBottom = pixelValue(style.getPropertyValue('border-bottom-width'));
                const rects = Array.from(target.getClientRects());
                if (rects.length === 0) {
                    const zero = new ResizeObserverSize(sizeToken, 0, 0);
                    return {contentRect: new DOMRectReadOnly(0, 0, 0, 0),
                        contentBoxSize: Object.freeze([zero]), borderBoxSize: Object.freeze([zero]),
                        devicePixelContentBoxSize: Object.freeze([zero])};
                }
                const contentRects = [];
                const contentBoxSize = [];
                const borderBoxSize = [];
                const devicePixelContentBoxSize = [];
                const devicePixelRatio = Number(globalThis.devicePixelRatio) || 1;
                for (const rect of rects) {
                    const contentWidth = clamp(rect.width - borderLeft - borderRight - paddingLeft - paddingRight);
                    const contentHeight = clamp(rect.height - borderTop - borderBottom - paddingTop - paddingBottom);
                    contentRects.push(new DOMRectReadOnly(paddingLeft, paddingTop, contentWidth, contentHeight));
                    contentBoxSize.push(new ResizeObserverSize(sizeToken, contentWidth, contentHeight));
                    borderBoxSize.push(new ResizeObserverSize(sizeToken, rect.width, rect.height));
                    devicePixelContentBoxSize.push(new ResizeObserverSize(sizeToken,
                        Math.round(contentWidth * devicePixelRatio),
                        Math.round(contentHeight * devicePixelRatio)));
                }
                const left = Math.min(...contentRects.map(rect => rect.left));
                const top = Math.min(...contentRects.map(rect => rect.top));
                const right = Math.max(...contentRects.map(rect => rect.right));
                const bottom = Math.max(...contentRects.map(rect => rect.bottom));
                return {
                    contentRect: new DOMRectReadOnly(left, top, right - left, bottom - top),
                    contentBoxSize: Object.freeze(contentBoxSize),
                    borderBoxSize: Object.freeze(borderBoxSize),
                    devicePixelContentBoxSize: Object.freeze(devicePixelContentBoxSize),
                };
            }

            function selectedSizes(measurement, box) {
                if (box === 'border-box') return measurement.borderBoxSize;
                if (box === 'device-pixel-content-box') return measurement.devicePixelContentBoxSize;
                return measurement.contentBoxSize;
            }
            function sizesEqual(left, right) {
                return left !== null && left.length === right.length &&
                    left.every((size, index) => size.inlineSize === right[index].inlineSize &&
                        size.blockSize === right[index].blockSize);
            }

            class ResizeObserver {
                constructor(callback) {
                    if (typeof callback !== 'function') throw new TypeError('ResizeObserver callback must be a function');
                    observerSlots.set(this, {callback, targets: new Map()});
                }
                observe(target, options = {}) {
                    const state = observerSlots.get(this);
                    if (!state) throw new TypeError('Illegal invocation');
                    if (!(target instanceof Element)) throw new TypeError('ResizeObserver target must be an Element');
                    const box = options == null ? 'content-box' : options.box === undefined ? 'content-box' : String(options.box);
                    if (!['content-box', 'border-box', 'device-pixel-content-box'].includes(box)) {
                        throw new TypeError(`Invalid ResizeObserver box option: ${box}`);
                    }
                    state.targets.set(target, {box, last: null});
                    observers.add(this);
                    pendingInitialDelivery = true;
                }
                unobserve(target) {
                    const state = observerSlots.get(this);
                    if (!state) throw new TypeError('Illegal invocation');
                    if (!(target instanceof Element)) throw new TypeError('ResizeObserver target must be an Element');
                    state.targets.delete(target);
                    if (state.targets.size === 0) observers.delete(this);
                }
                disconnect() {
                    const state = observerSlots.get(this);
                    if (!state) throw new TypeError('Illegal invocation');
                    state.targets.clear();
                    observers.delete(this);
                }
            }

            function deliverResizeObservers() {
                if (observers.size === 0) return;
                if (!pendingInitialDelivery && layoutRevision(false) === lastLayoutRevision) return;
                for (let iteration = 0; iteration < 10; iteration++) {
                    pendingInitialDelivery = false;
                    const revisionBeforeDelivery = layoutRevision(false);
                    let delivered = false;
                    for (const observer of observers) {
                        const state = observerSlots.get(observer);
                        if (!state) continue;
                        const entries = [];
                        for (const [target, observation] of state.targets) {
                            const measured = boxSizes(target);
                            const sizes = selectedSizes(measured, observation.box);
                            if (sizesEqual(observation.last, sizes)) continue;
                            observation.last = sizes.map(size => ({
                                inlineSize: size.inlineSize,
                                blockSize: size.blockSize,
                            }));
                            entries.push(new ResizeObserverEntry(
                                entryToken, target, measured.contentRect,
                                measured.contentBoxSize, measured.borderBoxSize,
                                measured.devicePixelContentBoxSize));
                        }
                        if (entries.length === 0) continue;
                        delivered = true;
                        try {
                            state.callback.call(observer, Object.freeze(entries), observer);
                        } catch (error) {
                            if (globalThis.console) console.error(error);
                        }
                    }
                    const revisionAfterDelivery = layoutRevision(false);
                    lastLayoutRevision = revisionAfterDelivery;
                    if (!delivered || (revisionBeforeDelivery === revisionAfterDelivery && !pendingInitialDelivery)) return;
                }
                if (globalThis.console) console.error(new Error('ResizeObserver loop limit exceeded'));
            }

            for (const [name, value] of [['ResizeObserver', ResizeObserver],
                ['ResizeObserverEntry', ResizeObserverEntry], ['ResizeObserverSize', ResizeObserverSize]]) {
                Object.defineProperty(value.prototype, Symbol.toStringTag, {value: name, configurable: true});
                Object.defineProperty(globalThis, name, {value, writable: true, configurable: true});
            }
            return deliverResizeObservers;
        })();
        "#,
    ))?;
    let delivery = delivery
        .as_object()
        .filter(JsObject::is_callable)
        .ok_or_else(|| {
            JsNativeError::error().with_message("ResizeObserver delivery function was not created")
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
