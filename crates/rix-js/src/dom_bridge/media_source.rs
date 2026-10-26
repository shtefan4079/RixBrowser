
use boa_engine::{Context, JsResult, Source};

pub(crate) fn register_media_source_interfaces(context: &mut Context) -> JsResult<()> {
    context.eval(Source::from_bytes(
        r#"(() => {
    'use strict';

    if (globalThis.MediaSource) return;

    const EventTarget = globalThis.EventTarget;
    if (!EventTarget) return;

    const apply = Reflect.apply;
    const weakMapGet = WeakMap.prototype.get;
    const weakMapSet = WeakMap.prototype.set;

    const HAVE_METADATA = 1;
    const HAVE_CURRENT_DATA = 2;
    const HAVE_FUTURE_DATA = 3;
    const HAVE_ENOUGH_DATA = 4;

    const NETWORK_IDLE = 1;
    const NETWORK_NO_SOURCE = 3;
    const MEDIA_ERR_NETWORK = 2;
    const MEDIA_ERR_DECODE = 3;
    const MEDIA_ERR_SRC_NOT_SUPPORTED = 4;

    function scheduleMicrotask(fn) {
        if (typeof queueMicrotask === 'function') {
            queueMicrotask(fn);
        } else if (typeof Promise === 'function') {
            Promise.resolve().then(fn);
        } else {
            fn();
        }
    }

    function createTimeRanges(ranges = []) {
        if (typeof globalThis.__rixCreateTimeRanges === 'function') {
            return globalThis.__rixCreateTimeRanges(ranges);
        }
        if (globalThis.TimeRanges) {
            const tr = Object.create(globalThis.TimeRanges.prototype);
            return tr;
        }
        return { length: ranges.length, start: (i) => ranges[i][0], end: (i) => ranges[i][1] };
    }

    function createMediaError(code, message) {
        if (typeof globalThis.__rixCreateMediaError === 'function') {
            return globalThis.__rixCreateMediaError(code, message);
        }
        return { code, message };
    }

    const mediaSourceUrlRegistry = new Map();

    globalThis.__rixRegisterMediaSourceUrl = function(url, mediaSource) {
        mediaSourceUrlRegistry.set(url, mediaSource);
    };

    globalThis.__rixResolveMediaSource = function(url) {
        return mediaSourceUrlRegistry.get(url) || null;
    };

    globalThis.__rixRevokeMediaSourceUrl = function(url) {
        mediaSourceUrlRegistry.delete(url);
    };

    globalThis.__rixCreateMediaSourceUrl = function(mediaSource) {
        const origin = (globalThis.location && globalThis.location.origin) || 'http://localhost';
        const uuid = 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function(c) {
            const r = Math.random() * 16 | 0;
            const v = c === 'x' ? r : (r & 0x3 | 0x8);
            return v.toString(16);
        });
        const url = `blob:${origin}/${uuid}`;
        mediaSourceUrlRegistry.set(url, mediaSource);
        return url;
    };

    function isTypeSupported(type) {
        if (!type || typeof type !== 'string') return false;
        if (typeof globalThis.__rixIsTypeSupported === 'function') {
            return globalThis.__rixIsTypeSupported(type);
        }
        return false;
    }

    function containerMimeType(type) {
        if (typeof type !== 'string') return '';
        const semi = type.indexOf(';');
        return (semi === -1 ? type : type.slice(0, semi)).trim().toLowerCase();
    }

    function normalizeRanges(ranges) {
        const sorted = [];
        for (const range of ranges) {
            if (!Array.isArray(range) || range.length < 2) continue;
            const start = Number(range[0]);
            const end = Number(range[1]);
            if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) continue;
            sorted.push([start, end]);
        }
        sorted.sort((a, b) => a[0] - b[0]);
        const merged = [];
        for (const range of sorted) {
            const last = merged[merged.length - 1];
            if (last && range[0] <= last[1]) {
                last[1] = Math.max(last[1], range[1]);
            } else {
                merged.push(range);
            }
        }
        return merged;
    }

    function intersectRangeLists(first, second) {
        const result = [];
        for (const [firstStart, firstEnd] of first) {
            for (const [secondStart, secondEnd] of second) {
                const start = Math.max(firstStart, secondStart);
                const end = Math.min(firstEnd, secondEnd);
                if (start < end) {
                    result.push([start, end]);
                }
            }
        }
        return result;
    }

    function highestRangeEnd(ranges) {
        let highest = 0;
        for (const range of ranges) {
            if (range[1] > highest) highest = range[1];
        }
        return highest;
    }

    function timeRangesContain(ranges, time) {
        for (const range of ranges) {
            if (time >= range[0] && time <= range[1]) return true;
        }
        return false;
    }

    function timeRangesHaveTimeAfter(ranges, time) {
        for (const range of ranges) {
            if (range[1] > time) return true;
        }
        return false;
    }

    function timeRangesEndAt(ranges, time) {
        for (const range of ranges) {
            if (range[1] === time) return true;
        }
        return false;
    }

    function trimRanges(ranges, start, end) {
        for (let i = ranges.length - 1; i >= 0; i--) {
            const [rangeStart, rangeEnd] = ranges[i];
            if (rangeEnd <= start || rangeStart >= end) {
                continue;
            }
            if (rangeStart >= start && rangeEnd <= end) {
                ranges.splice(i, 1);
            } else if (rangeStart < start && rangeEnd > end) {
                ranges.splice(i, 1, [rangeStart, start], [end, rangeEnd]);
            } else if (rangeStart < start) {
                ranges[i][1] = start;
            } else {
                ranges[i][0] = end;
            }
        }
    }

    const sourceBufferListSlots = new WeakMap();
    let allowInternalSourceBufferList = false;

    class SourceBufferList extends EventTarget {
        constructor() {
            if (!allowInternalSourceBufferList) {
                throw new TypeError('Illegal constructor');
            }
            super();
        }

        get length() {
            const list = apply(weakMapGet, sourceBufferListSlots, [this]);
            if (!list) throw new TypeError('Illegal SourceBufferList receiver');
            return list.buffers.length;
        }

        [Symbol.iterator]() {
            const list = apply(weakMapGet, sourceBufferListSlots, [this]);
            if (!list) throw new TypeError('Illegal SourceBufferList receiver');
            return list.buffers[Symbol.iterator]();
        }

        get [Symbol.toStringTag]() {
            return 'SourceBufferList';
        }
    }

    for (const name of ['addsourcebuffer', 'removesourcebuffer']) {
        Object.defineProperty(SourceBufferList.prototype, `on${name}`, {
            get() {
                const list = apply(weakMapGet, sourceBufferListSlots, [this]);
                return list ? list[`on${name}`] : null;
            },
            set(fn) {
                const list = apply(weakMapGet, sourceBufferListSlots, [this]);
                if (!list) return;
                const old = list[`on${name}`];
                if (old) this.removeEventListener(name, old);
                list[`on${name}`] = typeof fn === 'function' ? fn : null;
                if (list[`on${name}`]) this.addEventListener(name, list[`on${name}`]);
            },
            enumerable: true, configurable: true
        });
    }

    function createSourceBufferList() {
        allowInternalSourceBufferList = true;
        let instance;
        try {
            instance = new SourceBufferList();
        } finally {
            allowInternalSourceBufferList = false;
        }
        const state = {
            buffers: [],
            onaddsourcebuffer: null,
            onremovesourcebuffer: null,
        };
        apply(weakMapSet, sourceBufferListSlots, [instance, state]);
        return instance;
    }

    function addBufferToList(list, buffer) {
        const state = apply(weakMapGet, sourceBufferListSlots, [list]);
        if (!state || state.buffers.includes(buffer)) return;
        const idx = state.buffers.length;
        state.buffers.push(buffer);
        list[idx] = buffer;
        scheduleMicrotask(() => list.dispatchEvent(new Event('addsourcebuffer')));
    }

    function removeBufferFromList(list, buffer) {
        const state = apply(weakMapGet, sourceBufferListSlots, [list]);
        if (!state) return;
        const idx = state.buffers.indexOf(buffer);
        if (idx === -1) return;
        state.buffers.splice(idx, 1);
        for (let i = idx; i <= state.buffers.length; i++) {
            if (i < state.buffers.length) {
                list[i] = state.buffers[i];
            } else {
                delete list[i];
            }
        }
        scheduleMicrotask(() => list.dispatchEvent(new Event('removesourcebuffer')));
    }

    function removeAllBuffersFromList(list) {
        const state = apply(weakMapGet, sourceBufferListSlots, [list]);
        if (!state || state.buffers.length === 0) return;
        const count = state.buffers.length;
        state.buffers.length = 0;
        for (let i = 0; i < count; i++) delete list[i];
        scheduleMicrotask(() => list.dispatchEvent(new Event('removesourcebuffer')));
    }

    const sourceBufferSlots = new WeakMap();
    let allowInternalSourceBuffer = false;

    function bufferSlotOf(sourceBuffer) {
        const slot = apply(weakMapGet, sourceBufferSlots, [sourceBuffer]);
        if (!slot) throw new TypeError('Illegal SourceBuffer receiver');
        return slot;
    }

    class SourceBuffer extends EventTarget {
        constructor() {
            if (!allowInternalSourceBuffer) {
                throw new TypeError('Illegal constructor');
            }
            super();
        }

        get mode() {
            return bufferSlotOf(this).mode;
        }

        set mode(value) {
            const slot = bufferSlotOf(this);
            if (slot.removed) {
                throw new DOMException('SourceBuffer was removed from its parent MediaSource', 'InvalidStateError');
            }
            if (slot.updating) {
                throw new DOMException('Cannot change mode while updating', 'InvalidStateError');
            }
            if (value !== 'segments' && value !== 'sequence') {
                throw new TypeError("Mode must be 'segments' or 'sequence'");
            }
            reopenIfEnded(slot.mediaSource);
            slot.mode = value;
        }

        get updating() {
            return bufferSlotOf(this).updating;
        }

        get type() {
            return bufferSlotOf(this).type;
        }

        get buffered() {
            const slot = bufferSlotOf(this);
            if (slot.removed) {
                throw new DOMException('SourceBuffer was removed from its parent MediaSource', 'InvalidStateError');
            }
            return createTimeRanges(slot.bufferedRanges);
        }

        get timestampOffset() {
            return bufferSlotOf(this).timestampOffset;
        }

        set timestampOffset(value) {
            const slot = bufferSlotOf(this);
            if (slot.removed) {
                throw new DOMException('SourceBuffer was removed from its parent MediaSource', 'InvalidStateError');
            }
            if (slot.updating) {
                throw new DOMException('Cannot set timestampOffset while updating', 'InvalidStateError');
            }
            const num = Number(value);
            if (Number.isNaN(num)) throw new TypeError('Invalid timestampOffset');
            reopenIfEnded(slot.mediaSource);
            slot.timestampOffset = num;
        }

        get appendWindowStart() {
            return bufferSlotOf(this).appendWindowStart;
        }

        set appendWindowStart(value) {
            const slot = bufferSlotOf(this);
            if (slot.removed) {
                throw new DOMException('SourceBuffer was removed from its parent MediaSource', 'InvalidStateError');
            }
            if (slot.updating) {
                throw new DOMException('Cannot set appendWindowStart while updating', 'InvalidStateError');
            }
            const num = Number(value);
            if (Number.isNaN(num) || num < 0 || num >= slot.appendWindowEnd) {
                throw new TypeError('Invalid appendWindowStart');
            }
            slot.appendWindowStart = num;
        }

        get appendWindowEnd() {
            return bufferSlotOf(this).appendWindowEnd;
        }

        set appendWindowEnd(value) {
            const slot = bufferSlotOf(this);
            if (slot.removed) {
                throw new DOMException('SourceBuffer was removed from its parent MediaSource', 'InvalidStateError');
            }
            if (slot.updating) {
                throw new DOMException('Cannot set appendWindowEnd while updating', 'InvalidStateError');
            }
            const num = Number(value);
            if (Number.isNaN(num) || num <= slot.appendWindowStart) {
                throw new TypeError('Invalid appendWindowEnd');
            }
            slot.appendWindowEnd = num;
        }

        appendBuffer(data) {
            const slot = bufferSlotOf(this);

            let uint8Data = null;
            if (data instanceof ArrayBuffer) {
                uint8Data = new Uint8Array(data);
            } else if (ArrayBuffer.isView(data)) {
                uint8Data = new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
            } else {
                throw new TypeError('Argument 1 of SourceBuffer.appendBuffer is not a BufferSource');
            }

            if (slot.removed) {
                throw new DOMException('SourceBuffer was removed from its parent MediaSource', 'InvalidStateError');
            }
            if (slot.updating) {
                throw new DOMException('Cannot append while updating', 'InvalidStateError');
            }
            const mediaSource = slot.mediaSource;
            const msSlot = mediaSourceSlots.get(mediaSource);
            const element = msSlot ? msSlot.attachedElement : null;
            if (element && element.error) {
                throw new DOMException('The media element has an error', 'InvalidStateError');
            }
            if (msSlot && msSlot.readyState === 'ended') {
                msSlot.readyState = 'open';
                scheduleMicrotask(() => mediaSource.dispatchEvent(new Event('sourceopen')));
            } else if (!msSlot || msSlot.readyState !== 'open') {
                throw new DOMException('MediaSource is not open', 'InvalidStateError');
            }

            const byteLength = uint8Data.byteLength;
            const MAX_BUFFER_BYTES = 32 * 1024 * 1024;
            const MAX_TOTAL_BYTES = 64 * 1024 * 1024;
            if (slot.bytesAppended + byteLength > MAX_BUFFER_BYTES) {
                throw new DOMException('Buffer storage quota exceeded', 'QuotaExceededError');
            }
            if (mediaSource.__getTotalBytes() + byteLength > MAX_TOTAL_BYTES) {
                throw new DOMException('MediaSource total storage quota exceeded', 'QuotaExceededError');
            }

            slot.updating = true;
            const generation = ++slot.appendGeneration;
            this.dispatchEvent(new Event('updatestart'));

            scheduleMicrotask(() => {
                if (slot.removed || generation !== slot.appendGeneration) {
                    return;
                }

                let demux = null;
                if (slot.demuxerId > 0 && typeof globalThis.__rixAppendDemuxerData === 'function' && uint8Data) {
                    try {
                        demux = globalThis.__rixAppendDemuxerData(slot.demuxerId, uint8Data);
                    } catch (_) {}
                }

                if (demux && typeof demux === 'object') {
                    if (demux.initComplete) {
                        slot.initSegmentReceived = true;
                        if (Number.isNaN(msSlot.duration)) {
                            const initDuration = typeof demux.duration === 'number' && demux.duration > 0
                                ? demux.duration
                                : Infinity;
                            runDurationChangeAlgorithm(mediaSource, initDuration, false);
                        }
                    }
                    if (Array.isArray(demux.ranges) && demux.ranges.length > 0) {
                        slot.bufferedRanges = normalizeRanges(demux.ranges);
                        slot.highestPresentationTimestamp = highestRangeEnd(slot.bufferedRanges);
                    } else if (!demux.initComplete) {
                        advanceSyntheticTimeline(slot, byteLength);
                    }
                } else {
                    advanceSyntheticTimeline(slot, byteLength);
                }

                slot.updating = false;
                this.dispatchEvent(new Event('update'));
                this.dispatchEvent(new Event('updateend'));
                updateAttachedElement(mediaSource, demux && typeof demux === 'object' ? demux : null);
            });
        }

        abort() {
            const slot = bufferSlotOf(this);
            if (slot.removed) {
                throw new DOMException('SourceBuffer was removed from its parent MediaSource', 'InvalidStateError');
            }
            if (slot.mediaSource.readyState !== 'open') {
                throw new DOMException('MediaSource is not open', 'InvalidStateError');
            }
            if (slot.updating) {
                slot.appendGeneration++;
                slot.updating = false;
                if (slot.demuxerId > 0 && typeof globalThis.__rixResetDemuxer === 'function') {
                    globalThis.__rixResetDemuxer(slot.demuxerId);
                }
                this.dispatchEvent(new Event('abort'));
                this.dispatchEvent(new Event('updateend'));
            }
            slot.appendWindowStart = 0.0;
            slot.appendWindowEnd = Infinity;
        }

        remove(start, end) {
            const slot = bufferSlotOf(this);
            if (slot.removed) {
                throw new DOMException('SourceBuffer was removed from its parent MediaSource', 'InvalidStateError');
            }
            if (slot.updating) {
                throw new DOMException('Cannot remove while updating', 'InvalidStateError');
            }
            const mediaSource = slot.mediaSource;
            start = Number(start);
            end = Number(end);
            if (Number.isNaN(mediaSource.duration)) {
                throw new TypeError('Cannot remove media while the duration is unknown');
            }
            if (!(start >= 0) || start > mediaSource.duration) {
                throw new TypeError('Invalid remove start');
            }
            if (Number.isNaN(end) || !(end > start)) {
                throw new TypeError('Invalid remove end');
            }
            reopenIfEnded(mediaSource);
            if (mediaSource.readyState !== 'open') {
                throw new DOMException('MediaSource is not open', 'InvalidStateError');
            }

            slot.updating = true;
            const generation = ++slot.appendGeneration;
            this.dispatchEvent(new Event('updatestart'));

            scheduleMicrotask(() => {
                if (slot.removed || generation !== slot.appendGeneration) {
                    return;
                }
                if (slot.demuxerId > 0 && typeof globalThis.__rixRemoveDemuxerRange === 'function') {
                    globalThis.__rixRemoveDemuxerRange(slot.demuxerId, start, end);
                }
                trimRanges(slot.bufferedRanges, start, end);
                slot.updating = false;
                this.dispatchEvent(new Event('update'));
                this.dispatchEvent(new Event('updateend'));
                updateAttachedElement(mediaSource);
            });
        }

        changeType(type) {
            const slot = bufferSlotOf(this);
            if (typeof type !== 'string') type = String(type);
            if (type === '') {
                throw new TypeError('Type must be a non-empty string');
            }
            if (slot.removed) {
                throw new DOMException('SourceBuffer was removed from its parent MediaSource', 'InvalidStateError');
            }
            if (slot.updating) {
                throw new DOMException('Cannot changeType while updating', 'InvalidStateError');
            }
            if (!isTypeSupported(type)) {
                throw new DOMException('Unsupported type in changeType', 'NotSupportedError');
            }
            reopenIfEnded(slot.mediaSource);
            if (slot.demuxerId > 0 && typeof globalThis.__rixDestroyDemuxer === 'function') {
                globalThis.__rixDestroyDemuxer(slot.demuxerId);
            }
            slot.demuxerId = typeof globalThis.__rixCreateDemuxer === 'function' ? globalThis.__rixCreateDemuxer(type) : 0;
            slot.type = type;
            slot.initSegmentReceived = false;
        }

        get [Symbol.toStringTag]() {
            return 'SourceBuffer';
        }
    }

    for (const name of ['updatestart', 'update', 'updateend', 'error', 'abort']) {
        Object.defineProperty(SourceBuffer.prototype, `on${name}`, {
            get() {
                const slot = apply(weakMapGet, sourceBufferSlots, [this]);
                return slot ? slot[`on${name}`] : null;
            },
            set(fn) {
                const slot = apply(weakMapGet, sourceBufferSlots, [this]);
                if (!slot) return;
                const old = slot[`on${name}`];
                if (old) this.removeEventListener(name, old);
                slot[`on${name}`] = typeof fn === 'function' ? fn : null;
                if (slot[`on${name}`]) this.addEventListener(name, slot[`on${name}`]);
            },
            enumerable: true, configurable: true
        });
    }

    function advanceSyntheticTimeline(slot, byteLength) {
        const lastEnd = slot.bufferedRanges.length > 0
            ? slot.bufferedRanges[slot.bufferedRanges.length - 1][1]
            : 0;
        const start = slot.mode === 'sequence'
            ? (slot.timestampOffset + slot.highestPresentationTimestamp)
            : Math.max(slot.timestampOffset, lastEnd);
        const durationIncrement = Math.max(0.2, byteLength / (128 * 1024));
        const end = start + durationIncrement;
        slot.bufferedRanges = normalizeRanges(slot.bufferedRanges.concat([[start, end]]));
        slot.highestPresentationTimestamp = Math.max(slot.highestPresentationTimestamp, end);
    }

    function createSourceBuffer(mediaSource, type) {
        allowInternalSourceBuffer = true;
        let sb;
        try {
            sb = new SourceBuffer();
        } finally {
            allowInternalSourceBuffer = false;
        }
        const demuxerId = typeof globalThis.__rixCreateDemuxer === 'function' ? globalThis.__rixCreateDemuxer(type) : 0;
        if (typeof globalThis.__rixSourceBufferAdded === 'function') {
            globalThis.__rixSourceBufferAdded(mediaSource, demuxerId);
        }
        const slot = {
            mediaSource,
            type,
            demuxerId,
            mode: 'segments',
            updating: false,
            removed: false,
            appendGeneration: 0,
            initSegmentReceived: false,
            timestampOffset: 0.0,
            highestPresentationTimestamp: 0.0,
            appendWindowStart: 0.0,
            appendWindowEnd: Infinity,
            bufferedRanges: [],
            bytesAppended: 0,
            onupdatestart: null,
            onupdate: null,
            onupdateend: null,
            onerror: null,
            onabort: null,
        };
        apply(weakMapSet, sourceBufferSlots, [sb, slot]);
        return sb;
    }

    const mediaSourceSlots = new WeakMap();

    function mediaSourceSlotOf(mediaSource) {
        const slot = apply(weakMapGet, mediaSourceSlots, [mediaSource]);
        if (!slot) throw new TypeError('Illegal MediaSource receiver');
        return slot;
    }

    function sourceBufferListOf(mediaSource) {
        const slot = mediaSourceSlotOf(mediaSource);
        const state = apply(weakMapGet, sourceBufferListSlots, [slot.sourceBuffers]);
        return state ? state.buffers : [];
    }

    function activeSourceBufferListOf(mediaSource) {
        const slot = mediaSourceSlotOf(mediaSource);
        const state = apply(weakMapGet, sourceBufferListSlots, [slot.activeSourceBuffers]);
        return state ? state.buffers : [];
    }

    function hasInitializationSegment(mediaSource) {
        for (const sb of sourceBufferListOf(mediaSource)) {
            const bufSlot = apply(weakMapGet, sourceBufferSlots, [sb]);
            if (bufSlot && bufSlot.initSegmentReceived) return true;
        }
        return false;
    }

    function highestBufferedEnd(mediaSource) {
        let highest = 0;
        for (const sb of sourceBufferListOf(mediaSource)) {
            const bufSlot = apply(weakMapGet, sourceBufferSlots, [sb]);
            if (bufSlot) highest = Math.max(highest, highestRangeEnd(bufSlot.bufferedRanges));
        }
        return highest;
    }

    function mediaElementBufferedRanges(mediaSource) {
        const slot = mediaSourceSlotOf(mediaSource);
        const active = activeSourceBufferListOf(mediaSource);
        if (active.length === 0) return [];
        const perBuffer = [];
        let highestEnd = 0;
        for (const sb of active) {
            const bufSlot = apply(weakMapGet, sourceBufferSlots, [sb]);
            if (!bufSlot) continue;
            const ranges = bufSlot.bufferedRanges.map((range) => [range[0], range[1]]);
            perBuffer.push(ranges);
            highestEnd = Math.max(highestEnd, highestRangeEnd(ranges));
        }
        if (perBuffer.length === 0) return [];
        for (const ranges of perBuffer) {
            if (slot.readyState === 'ended' && ranges.length > 0) {
                ranges[ranges.length - 1][1] = highestEnd;
            }
        }
        let intersection = [[0, highestEnd]];
        for (const ranges of perBuffer) {
            intersection = intersectRangeLists(intersection, ranges);
        }
        return intersection;
    }

    function seekableRangesFor(mediaSource, buffered) {
        const slot = mediaSourceSlotOf(mediaSource);
        const duration = slot.readyState === 'closed' ? NaN : slot.duration;
        if (Number.isNaN(duration)) return [];
        if (duration === Infinity) {
            const live = slot.liveSeekableRange;
            if (live) {
                let start = live[0];
                let end = live[1];
                for (const range of buffered) {
                    start = Math.min(start, range[0]);
                    end = Math.max(end, range[1]);
                }
                return [[start, end]];
            }
            if (buffered.length === 0) return [];
            return [[0, highestRangeEnd(buffered)]];
        }
        return [[0, duration]];
    }

    function computeElementReadyState(mediaSource, buffered, element) {
        if (!element) return undefined;
        const slot = mediaSourceSlotOf(mediaSource);
        if (element.readyState === 0) {
            if (buffered.length === 0 && !hasInitializationSegment(mediaSource)) return undefined;
        }
        const position = element.currentTime;
        if (!timeRangesContain(buffered, position)) return HAVE_METADATA;
        if (slot.readyState === 'ended') return HAVE_ENOUGH_DATA;
        if (Number.isFinite(slot.duration) && highestRangeEnd(buffered) >= slot.duration) {
            return HAVE_ENOUGH_DATA;
        }
        if (timeRangesHaveTimeAfter(buffered, position)) return HAVE_FUTURE_DATA;
        if (timeRangesEndAt(buffered, position)) return HAVE_CURRENT_DATA;
        return HAVE_METADATA;
    }

    function elementDurationFor(mediaSource, buffered, element) {
        const slot = mediaSourceSlotOf(mediaSource);
        const duration = slot.duration;
        if (!Number.isNaN(duration)) return duration;
        const metadataKnown = buffered.length > 0
            || hasInitializationSegment(mediaSource)
            || (element && element.readyState > 0);
        return metadataKnown ? Infinity : NaN;
    }

    function updateAttachedElement(mediaSource, mediaInfo) {
        const slot = mediaSourceSlotOf(mediaSource);
        const element = slot.attachedElement;
        if (!element || typeof element.__rixUpdateMediaSourceData !== 'function') return;
        const buffered = mediaElementBufferedRanges(mediaSource);
        const readyState = computeElementReadyState(mediaSource, buffered, element);
        element.__rixUpdateMediaSourceData(
            elementDurationFor(mediaSource, buffered, element),
            createTimeRanges(buffered),
            readyState,
            mediaInfo ? mediaInfo.videoWidth : undefined,
            mediaInfo ? mediaInfo.videoHeight : undefined,
            createTimeRanges(seekableRangesFor(mediaSource, buffered))
        );
    }

    globalThis.__rixMediaSourceLiveRanges = (mediaSource) => {
        const slot = apply(weakMapGet, mediaSourceSlots, [mediaSource]);
        if (!slot) return null;
        const buffered = mediaElementBufferedRanges(mediaSource);
        return {
            buffered: createTimeRanges(buffered),
            seekable: createTimeRanges(seekableRangesFor(mediaSource, buffered)),
        };
    };

    function runDurationChangeAlgorithm(mediaSource, newDuration, throwOnInvalid) {
        const slot = mediaSourceSlotOf(mediaSource);
        if (slot.duration === newDuration) return;
        const highestEnd = highestBufferedEnd(mediaSource);
        if (newDuration < highestEnd) {
            if (throwOnInvalid) {
                throw new DOMException('New duration must not be less than the highest buffered end time', 'InvalidStateError');
            }
            return;
        }
        slot.duration = newDuration;
    }

    function reopenIfEnded(mediaSource) {
        const slot = mediaSourceSlotOf(mediaSource);
        if (slot.readyState === 'ended') {
            slot.readyState = 'open';
            scheduleMicrotask(() => mediaSource.dispatchEvent(new Event('sourceopen')));
        }
    }

    class MediaSource extends EventTarget {
        constructor() {
            super();
            const sourceBuffers = createSourceBufferList();
            const activeSourceBuffers = createSourceBufferList();
            const slot = {
                sourceBuffers,
                activeSourceBuffers,
                readyState: 'closed',
                duration: NaN,
                liveSeekableRange: null,
                attachedElement: null,
                onsourceopen: null,
                onsourceended: null,
                onsourceclose: null,
            };
            apply(weakMapSet, mediaSourceSlots, [this, slot]);
        }

        static isTypeSupported(type) {
            return isTypeSupported(type);
        }

        get sourceBuffers() {
            return mediaSourceSlotOf(this).sourceBuffers;
        }

        get activeSourceBuffers() {
            return mediaSourceSlotOf(this).activeSourceBuffers;
        }

        get readyState() {
            return mediaSourceSlotOf(this).readyState;
        }

        get duration() {
            const slot = mediaSourceSlotOf(this);
            if (slot.readyState === 'closed') return NaN;
            return slot.duration;
        }

        set duration(value) {
            const slot = mediaSourceSlotOf(this);
            const num = Number(value);
            if (Number.isNaN(num) || num < 0) {
                throw new TypeError('Duration must not be negative or NaN');
            }
            if (slot.readyState !== 'open') {
                throw new DOMException('MediaSource is not open', 'InvalidStateError');
            }
            for (const sb of sourceBufferListOf(this)) {
                const bufSlot = apply(weakMapGet, sourceBufferSlots, [sb]);
                if (bufSlot && bufSlot.updating) {
                    throw new DOMException('Cannot set duration while a SourceBuffer is updating', 'InvalidStateError');
                }
            }
            runDurationChangeAlgorithm(this, num, true);
            updateAttachedElement(this);
        }

        addSourceBuffer(type) {
            const slot = mediaSourceSlotOf(this);
            if (typeof type !== 'string') type = String(type);
            if (type === '') {
                throw new TypeError('Type must be a non-empty string');
            }
            if (!isTypeSupported(type)) {
                throw new DOMException(`Unsupported media format: ${type}`, 'NotSupportedError');
            }
            if (sourceBufferListOf(this).length >= 16) {
                throw new DOMException('Maximum active SourceBuffers reached', 'QuotaExceededError');
            }
            if (slot.readyState !== 'open') {
                throw new DOMException('MediaSource is not open', 'InvalidStateError');
            }
            const buffer = createSourceBuffer(this, type);
            addBufferToList(slot.sourceBuffers, buffer);
            addBufferToList(slot.activeSourceBuffers, buffer);
            return buffer;
        }

        removeSourceBuffer(sourceBuffer) {
            const slot = mediaSourceSlotOf(this);
            if (!sourceBufferListOf(this).includes(sourceBuffer)) {
                throw new DOMException('SourceBuffer not found in MediaSource', 'NotFoundError');
            }

            const bufSlot = apply(weakMapGet, sourceBufferSlots, [sourceBuffer]);
            if (bufSlot) {
                if (bufSlot.updating) {
                    bufSlot.appendGeneration++;
                    bufSlot.updating = false;
                    scheduleMicrotask(() => sourceBuffer.dispatchEvent(new Event('abort')));
                    scheduleMicrotask(() => sourceBuffer.dispatchEvent(new Event('updateend')));
                }
                bufSlot.removed = true;
                if (bufSlot.demuxerId > 0 && typeof globalThis.__rixDestroyDemuxer === 'function') {
                    globalThis.__rixDestroyDemuxer(bufSlot.demuxerId);
                    bufSlot.demuxerId = 0;
                }
            }

            removeBufferFromList(slot.activeSourceBuffers, sourceBuffer);
            removeBufferFromList(slot.sourceBuffers, sourceBuffer);
            updateAttachedElement(this);
        }

        endOfStream(error) {
            const slot = mediaSourceSlotOf(this);
            if (slot.readyState !== 'open') {
                throw new DOMException('MediaSource is not open', 'InvalidStateError');
            }
            for (const sb of sourceBufferListOf(this)) {
                const bufSlot = apply(weakMapGet, sourceBufferSlots, [sb]);
                if (bufSlot && bufSlot.updating) {
                    throw new DOMException('Cannot endOfStream while a SourceBuffer is updating', 'InvalidStateError');
                }
            }
            if (error !== undefined && error !== 'network' && error !== 'decode') {
                throw new TypeError("The error argument must be 'network' or 'decode'");
            }

            slot.readyState = 'ended';
            this.__rixNotifyPlayback('endOfStream', 0, 1, 1, false);
            scheduleMicrotask(() => this.dispatchEvent(new Event('sourceended')));

            if (error === undefined) {
                const highestEnd = highestBufferedEnd(this);
                if (highestEnd > 0) {
                    runDurationChangeAlgorithm(this, highestEnd, false);
                }
                updateAttachedElement(this);
            } else {
                const element = slot.attachedElement;
                if (element && typeof element.__rixSetMediaSourceError === 'function') {
                    const nothingKnown = element.readyState === 0;
                    if (nothingKnown) {
                        element.__rixSetMediaSourceError(NETWORK_NO_SOURCE, createMediaError(
                            MEDIA_ERR_SRC_NOT_SUPPORTED,
                            error === 'network' ? 'Media network error' : 'Media decode error'));
                    } else {
                        element.__rixSetMediaSourceError(NETWORK_IDLE, createMediaError(
                            error === 'network' ? MEDIA_ERR_NETWORK : MEDIA_ERR_DECODE,
                            error === 'network' ? 'Media network error' : 'Media decode error'));
                    }
                }
            }
        }

        setLiveSeekableRange(start, end) {
            const slot = mediaSourceSlotOf(this);
            if (slot.readyState !== 'open') {
                throw new DOMException('MediaSource is not open', 'InvalidStateError');
            }
            start = Number(start);
            end = Number(end);
            if (Number.isNaN(start) || Number.isNaN(end) || start < 0 || start > end) {
                throw new TypeError('Invalid live seekable range');
            }
            slot.liveSeekableRange = [start, end];
            updateAttachedElement(this);
        }

        clearLiveSeekableRange() {
            const slot = mediaSourceSlotOf(this);
            if (slot.readyState !== 'open') {
                throw new DOMException('MediaSource is not open', 'InvalidStateError');
            }
            slot.liveSeekableRange = null;
            updateAttachedElement(this);
        }

        __setReadyState(state) {
            const slot = apply(weakMapGet, mediaSourceSlots, [this]);
            if (slot) slot.readyState = state;
        }

        __monitorElement() {
            updateAttachedElement(this);
        }

        get __attachedElement() {
            const slot = apply(weakMapGet, mediaSourceSlots, [this]);
            return slot ? slot.attachedElement : null;
        }

        __attachToMediaElement(element) {
            const slot = apply(weakMapGet, mediaSourceSlots, [this]);
            if (!slot) return;
            slot.attachedElement = element;
            if (slot.readyState === 'closed') {
                slot.readyState = 'open';
                scheduleMicrotask(() => this.dispatchEvent(new Event('sourceopen')));
            }
        }

        __detachFromMediaElement() {
            const slot = apply(weakMapGet, mediaSourceSlots, [this]);
            if (!slot) return;
            slot.attachedElement = null;
            if (slot.readyState === 'closed') return;

            slot.readyState = 'closed';
            slot.duration = NaN;
            slot.liveSeekableRange = null;
            for (const sb of sourceBufferListOf(this)) {
                const bufSlot = apply(weakMapGet, sourceBufferSlots, [sb]);
                if (!bufSlot) continue;
                bufSlot.removed = true;
                bufSlot.appendGeneration++;
                bufSlot.updating = false;
                if (bufSlot.demuxerId > 0 && typeof globalThis.__rixDestroyDemuxer === 'function') {
                    globalThis.__rixDestroyDemuxer(bufSlot.demuxerId);
                }
                bufSlot.demuxerId = 0;
            }
            removeAllBuffersFromList(slot.activeSourceBuffers);
            removeAllBuffersFromList(slot.sourceBuffers);
            scheduleMicrotask(() => this.dispatchEvent(new Event('sourceclose')));
        }

        __getTotalBytes() {
            const slot = apply(weakMapGet, mediaSourceSlots, [this]);
            if (!slot) return 0;
            let total = 0;
            for (const sb of sourceBufferListOf(this)) {
                const bufSlot = apply(weakMapGet, sourceBufferSlots, [sb]);
                if (bufSlot) total += bufSlot.bytesAppended;
            }
            return total;
        }

        __rixNotifyPlayback(kind, position, rate, volume, muted) {
            const slot = apply(weakMapGet, mediaSourceSlots, [this]);
            if (!slot) return;
            const state = apply(weakMapGet, sourceBufferListSlots, [slot.sourceBuffers]);
            if (!state) return;
            for (const buf of state.buffers) {
                const bufSlot = apply(weakMapGet, sourceBufferSlots, [buf]);
                if (bufSlot && bufSlot.demuxerId > 0 && typeof globalThis.__rixMediaPlayback === 'function') {
                    globalThis.__rixMediaPlayback(bufSlot.demuxerId, kind, position, rate, volume, muted);
                }
            }
        }

        get [Symbol.toStringTag]() {
            return 'MediaSource';
        }
    }

    for (const name of ['sourceopen', 'sourceended', 'sourceclose']) {
        Object.defineProperty(MediaSource.prototype, `on${name}`, {
            get() {
                const slot = apply(weakMapGet, mediaSourceSlots, [this]);
                return slot ? slot[`on${name}`] : null;
            },
            set(fn) {
                const slot = apply(weakMapGet, mediaSourceSlots, [this]);
                if (!slot) return;
                const old = slot[`on${name}`];
                if (old) this.removeEventListener(name, old);
                slot[`on${name}`] = typeof fn === 'function' ? fn : null;
                if (slot[`on${name}`]) this.addEventListener(name, slot[`on${name}`]);
            },
            enumerable: true, configurable: true
        });
    }

    globalThis.MediaSource = MediaSource;
    globalThis.SourceBuffer = SourceBuffer;
    globalThis.SourceBufferList = SourceBufferList;
})();
"#),
    )?;

    Ok(())
}
