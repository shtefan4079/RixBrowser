
use super::*;
use boa_engine::object::builtins::{JsArray, JsArrayBuffer, JsUint8Array};
use boa_engine::Source;
use rix_media::demuxer::{create_demuxer_for_mime, MediaDemuxer};
use rix_media::sample::MediaSample;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub struct DemuxerStore {
    next_id: u32,
    pub demuxers: HashMap<u32, Box<dyn MediaDemuxer>>,
    pub pending_samples: Vec<(u32, Vec<MediaSample>)>,
    pub pending_playback: Vec<QueuedPlaybackCommand>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MediaPlaybackCommand {
    Play { rate: f64 },
    Pause,
    Seek { position: f64 },
    SetRate { rate: f64 },
    SetVolume { volume: f32, muted: bool },
    EndOfStream,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QueuedPlaybackCommand {
    pub demuxer_id: u32,
    pub command: MediaPlaybackCommand,
}

impl DemuxerStore {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            demuxers: HashMap::new(),
            pending_samples: Vec::new(),
            pending_playback: Vec::new(),
        }
    }

    pub fn take_pending_samples(&mut self) -> Vec<(u32, Vec<MediaSample>)> {
        std::mem::take(&mut self.pending_samples)
    }

    pub fn take_pending_playback(&mut self) -> Vec<QueuedPlaybackCommand> {
        std::mem::take(&mut self.pending_playback)
    }

    pub fn demuxer(&self, id: u32) -> Option<&dyn rix_media::demuxer::MediaDemuxer> {
        self.demuxers.get(&id).map(|d| d.as_ref())
    }
}

const MEDIA_API_SCRIPT: &str = r#"
(() => {
    const timeRangesSlots = new WeakMap();
    const mediaErrorSlots = new WeakMap();
    const mediaSlots = new WeakMap();

    const apply = Reflect.apply;
    const weakMapGet = WeakMap.prototype.get;
    const weakMapSet = WeakMap.prototype.set;

    function getSlot(slots, value, name) {
        const slot = apply(weakMapGet, slots, [value]);
        if (slot === undefined) throw new TypeError(`Illegal invocation: ${name}`);
        return slot;
    }

    class TimeRanges {
        constructor() {
            throw new TypeError('Illegal constructor');
        }
        get length() {
            const ranges = getSlot(timeRangesSlots, this, 'TimeRanges');
            return ranges.length;
        }
        start(index) {
            const ranges = getSlot(timeRangesSlots, this, 'TimeRanges');
            index = Number(index);
            if (!Number.isInteger(index) || index < 0 || index >= ranges.length) {
                throw new DOMException('Index out of bounds', 'IndexSizeError');
            }
            return ranges[index][0];
        }
        end(index) {
            const ranges = getSlot(timeRangesSlots, this, 'TimeRanges');
            index = Number(index);
            if (!Number.isInteger(index) || index < 0 || index >= ranges.length) {
                throw new DOMException('Index out of bounds', 'IndexSizeError');
            }
            return ranges[index][1];
        }
        get [Symbol.toStringTag]() {
            return 'TimeRanges';
        }
    }

    function createTimeRanges(ranges = []) {
        const tr = Object.create(TimeRanges.prototype);
        apply(weakMapSet, timeRangesSlots, [tr, ranges.slice()]);
        return tr;
    }
    globalThis.__rixCreateTimeRanges = createTimeRanges;

    class MediaError {
        constructor() {
            throw new TypeError('Illegal constructor');
        }
        get code() {
            return getSlot(mediaErrorSlots, this, 'MediaError').code;
        }
        get message() {
            return getSlot(mediaErrorSlots, this, 'MediaError').message;
        }
        get [Symbol.toStringTag]() {
            return 'MediaError';
        }
    }
    const MEDIA_ERR_ABORTED = 1;
    const MEDIA_ERR_NETWORK = 2;
    const MEDIA_ERR_DECODE = 3;
    const MEDIA_ERR_SRC_NOT_SUPPORTED = 4;
    for (const target of [MediaError, MediaError.prototype]) {
        Object.defineProperties(target, {
            MEDIA_ERR_ABORTED: { value: MEDIA_ERR_ABORTED, enumerable: true },
            MEDIA_ERR_NETWORK: { value: MEDIA_ERR_NETWORK, enumerable: true },
            MEDIA_ERR_DECODE: { value: MEDIA_ERR_DECODE, enumerable: true },
            MEDIA_ERR_SRC_NOT_SUPPORTED: { value: MEDIA_ERR_SRC_NOT_SUPPORTED, enumerable: true },
        });
    }

    function createMediaError(code, message = '') {
        const err = Object.create(MediaError.prototype);
        apply(weakMapSet, mediaErrorSlots, [err, { code, message }]);
        return err;
    }
    globalThis.__rixCreateMediaError = createMediaError;

    const NETWORK_EMPTY = 0;
    const NETWORK_IDLE = 1;
    const NETWORK_LOADING = 2;
    const NETWORK_NO_SOURCE = 3;

    const HAVE_NOTHING = 0;
    const HAVE_METADATA = 1;
    const HAVE_CURRENT_DATA = 2;
    const HAVE_FUTURE_DATA = 3;
    const HAVE_ENOUGH_DATA = 4;

    const HTMLMediaElement = globalThis.HTMLMediaElement;
    const HTMLAudioElement = globalThis.HTMLAudioElement;
    const HTMLVideoElement = globalThis.HTMLVideoElement;

    if (!HTMLMediaElement) return;

    for (const target of [HTMLMediaElement, HTMLMediaElement.prototype]) {
        Object.defineProperties(target, {
            NETWORK_EMPTY: { value: NETWORK_EMPTY, enumerable: true },
            NETWORK_IDLE: { value: NETWORK_IDLE, enumerable: true },
            NETWORK_LOADING: { value: NETWORK_LOADING, enumerable: true },
            NETWORK_NO_SOURCE: { value: NETWORK_NO_SOURCE, enumerable: true },
            HAVE_NOTHING: { value: HAVE_NOTHING, enumerable: true },
            HAVE_METADATA: { value: HAVE_METADATA, enumerable: true },
            HAVE_CURRENT_DATA: { value: HAVE_CURRENT_DATA, enumerable: true },
            HAVE_FUTURE_DATA: { value: HAVE_FUTURE_DATA, enumerable: true },
            HAVE_ENOUGH_DATA: { value: HAVE_ENOUGH_DATA, enumerable: true },
        });
    }

    function scheduleMicrotask(fn) {
        if (typeof queueMicrotask === 'function') {
            queueMicrotask(fn);
        } else if (typeof Promise === 'function') {
            Promise.resolve().then(fn);
        } else {
            fn();
        }
    }

    function timeRangesContain(ranges, time) {
        if (!ranges) return false;
        const length = Number(ranges.length) || 0;
        for (let i = 0; i < length; i++) {
            if (time >= ranges.start(i) && time <= ranges.end(i)) return true;
        }
        return false;
    }

    function nearestSeekablePosition(ranges, time) {
        const length = ranges && typeof ranges.length === 'number' ? ranges.length : 0;
        if (length === 0) return null;
        let best = null;
        let bestDistance = Infinity;
        for (let i = 0; i < length; i++) {
            const start = ranges.start(i);
            const end = ranges.end(i);
            const candidate = time < start ? start : (time > end ? end : time);
            const distance = Math.abs(candidate - time);
            if (distance < bestDistance) {
                bestDistance = distance;
                best = candidate;
            }
        }
        return best;
    }

    function announceReadyState(element, state, newValue, beforeLoadedMetadata) {
        const value = Number(newValue);
        if (!Number.isFinite(value)) return;
        const target = Math.max(HAVE_NOTHING, Math.min(HAVE_ENOUGH_DATA, Math.floor(value)));
        const announced = typeof state.readyStateTarget === 'number' ? state.readyStateTarget : state.readyState;
        if (target === announced) return;

        if (target < announced) {
            state.readyStateTarget = target;
            state.readyState = target;
            if (announced >= HAVE_FUTURE_DATA && target <= HAVE_CURRENT_DATA && !state.paused && !state.ended) {
                state.waitingForData = true;
                scheduleMicrotask(() => {
                    element.dispatchEvent(new Event('timeupdate'));
                    element.dispatchEvent(new Event('waiting'));
                });
            }
            return;
        }

        const stages = [];
        for (let next = announced + 1; next <= target; next++) {
            let eventName = null;
            if (next === HAVE_METADATA) {
                eventName = 'loadedmetadata';
            } else if (next === HAVE_CURRENT_DATA) {
                eventName = 'loadeddata';
            } else if (next === HAVE_FUTURE_DATA) {
                eventName = 'canplay';
            } else if (next === HAVE_ENOUGH_DATA) {
                eventName = 'canplaythrough';
            }
            stages.push({ readyState: next, eventName });
        }
        state.readyStateTarget = target;
        for (const stage of stages) {
            scheduleMicrotask(() => {
                if (state.readyStateTarget < stage.readyState) return;
                state.readyState = stage.readyState;
                if (stage.eventName === 'loadedmetadata' && typeof beforeLoadedMetadata === 'function') {
                    beforeLoadedMetadata();
                }
                if (stage.eventName) {
                    element.dispatchEvent(new Event(stage.eventName));
                }
                if (stage.readyState === HAVE_FUTURE_DATA
                    && !state.paused
                    && !state.ended
                    && state.waitingForData) {
                    state.waitingForData = false;
                    const resolve = state.pendingPlayResolve;
                    state.pendingPlayPromise = null;
                    state.pendingPlayResolve = null;
                    element.dispatchEvent(new Event('playing'));
                    if (typeof resolve === 'function') resolve();
                }
            });
        }
    }

    function seekMediaElement(element, state, newPosition) {
        if (state.readyState === HAVE_NOTHING && Number.isNaN(state.duration)) return;
        state.seeking = true;
        const duration = state.duration;
        let position = newPosition;
        if (typeof duration === 'number' && Number.isFinite(duration) && position > duration) {
            position = duration;
        }
        if (state.seekable.length > 0) {
            const clamped = nearestSeekablePosition(state.seekable, position);
            if (clamped === null) {
                state.seeking = false;
                return;
            }
            position = clamped;
        }
        position = Math.max(0, position);
        state.currentTime = position;
        state.lastClockTick = clockNowSeconds();
        if (Number.isFinite(state.duration) && position < state.duration) {
            state.ended = false;
        }
        notifyPlayback(element, state, 'seek');
        if (!timeRangesContain(state.buffered, position) && state.readyState > HAVE_METADATA) {
            announceReadyState(element, state, HAVE_METADATA);
        }
        scheduleMicrotask(() => {
            element.dispatchEvent(new Event('seeking'));
        });
        scheduleMicrotask(() => {
            state.seeking = false;
            const attached = state.attachedMediaSource;
            if (attached && typeof attached.__monitorElement === 'function') {
                attached.__monitorElement();
            }
            element.dispatchEvent(new Event('timeupdate'));
            element.dispatchEvent(new Event('seeked'));
        });
    }

    function resetMediaElementState(state) {
        state.networkState = NETWORK_EMPTY;
        state.readyState = HAVE_NOTHING;
        state.readyStateTarget = HAVE_NOTHING;
        state.paused = true;
        state.seeking = false;
        state.ended = false;
        state.currentTime = 0;
        state.defaultPlaybackStartPosition = 0;
        state.duration = NaN;
        state.error = null;
        state.pendingPlayPromise = null;
        state.pendingPlayResolve = null;
        state.clockStalled = false;
        state.waitingForData = false;
        state.buffered = createTimeRanges([]);
        state.played = createTimeRanges([]);
        state.seekable = createTimeRanges([]);
        state.videoWidth = 0;
        state.videoHeight = 0;
        state.lastClockTick = clockNowSeconds();
        state.lastTimeupdate = undefined;
    }

    function getMediaState(element) {
        let state = apply(weakMapGet, mediaSlots, [element]);
        if (!state) {
            state = {
                networkState: NETWORK_EMPTY,
                readyState: HAVE_NOTHING,
                readyStateTarget: HAVE_NOTHING,
                paused: true,
                currentTime: 0,
                defaultPlaybackStartPosition: 0,
                duration: NaN,
                defaultPlaybackRate: 1.0,
                playbackRate: 1.0,
                seeking: false,
                ended: false,
                volume: 1.0,
                muted: false,
                error: null,
                buffered: createTimeRanges([]),
                played: createTimeRanges([]),
                seekable: createTimeRanges([]),
                attachedMediaSource: null,
                srcObject: null,
                pendingPlayPromise: null,
                pendingPlayResolve: null,
                waitingForData: false,
                clockStalled: false,
                videoWidth: 0,
                videoHeight: 0,
            };
            apply(weakMapSet, mediaSlots, [element, state]);
            mediaElements.add(element);
            ensureMediaClockTimer();

            element.__rixUpdateMediaSourceData = (duration, ranges, readyState, videoWidth, videoHeight, seekableRanges) => {
                let durationChangePending = false;
                if (typeof duration === 'number' && !isNaN(duration) && duration !== state.duration) {
                    state.duration = duration;
                    durationChangePending = true;
                }
                if (ranges) {
                    state.buffered = ranges;
                }
                if (seekableRanges) {
                    state.seekable = seekableRanges;
                }
                if (typeof videoWidth === 'number' && videoWidth > 0) {
                    state.videoWidth = videoWidth;
                }
                if (typeof videoHeight === 'number' && videoHeight > 0) {
                    state.videoHeight = videoHeight;
                }
                if (readyState !== undefined) {
                    const target = Math.max(HAVE_NOTHING, Math.min(HAVE_ENOUGH_DATA, Math.floor(Number(readyState))));
                    if (target >= HAVE_METADATA && state.defaultPlaybackStartPosition !== 0) {
                        const pendingStart = state.defaultPlaybackStartPosition;
                        state.defaultPlaybackStartPosition = 0;
                        state.currentTime = 0;
                        if (pendingStart > 0) {
                            seekMediaElement(element, state, pendingStart);
                        }
                    }
                    const announced = typeof state.readyStateTarget === 'number'
                        ? state.readyStateTarget
                        : state.readyState;
                    if (durationChangePending && announced < HAVE_METADATA && target >= HAVE_METADATA) {
                        durationChangePending = false;
                        announceReadyState(element, state, readyState, () => {
                            element.dispatchEvent(new Event('durationchange'));
                        });
                    } else {
                        if (durationChangePending) {
                            durationChangePending = false;
                            scheduleMicrotask(() => {
                                element.dispatchEvent(new Event('durationchange'));
                            });
                        }
                        announceReadyState(element, state, readyState);
                    }
                }
                if (durationChangePending) {
                    scheduleMicrotask(() => {
                        element.dispatchEvent(new Event('durationchange'));
                    });
                }
            };

            element.__rixSetMediaSourceError = (networkState, error) => {
                if (networkState !== undefined) state.networkState = networkState;
                if (error !== undefined) state.error = error;
                scheduleMicrotask(() => {
                    element.dispatchEvent(new Event('error'));
                });
            };
        }
        return state;
    }

    function ensureMediaElement(receiver, name) {
        if (!(receiver instanceof HTMLMediaElement)) {
            throw new TypeError(`'${name}' called on an object that does not implement HTMLMediaElement`);
        }
        return getMediaState(receiver);
    }

    function refreshMediaSourceRanges(state) {
        const mediaSource = state.attachedMediaSource;
        if (!mediaSource) return;
        if (typeof globalThis.__rixMediaSourceLiveRanges !== 'function') return;
        const live = globalThis.__rixMediaSourceLiveRanges(mediaSource);
        if (!live) return;
        state.buffered = live.buffered;
        state.seekable = live.seekable;
    }

    const mediaElements = new Set();
    const CLOCK_TICK_MS = 50;
    const TIMEUPDATE_INTERVAL_SECONDS = 0.25;

    function clockNowSeconds() {
        if (typeof performance === 'object' && performance && typeof performance.now === 'function') {
            return performance.now() / 1000;
        }
        return Date.now() / 1000;
    }

    function notifyPlayback(element, state, kind, position) {
        const source = state.attachedMediaSource;
        if (source && typeof source.__rixNotifyPlayback === 'function') {
            const at = typeof position === 'number' ? position : state.currentTime;
            try {
                source.__rixNotifyPlayback(kind, at, state.playbackRate, state.volume, state.muted);
            } catch (_) {}
        }
    }

    function finishPlayback(element, state) {
        state.currentTime = state.duration;
        state.paused = true;
        state.ended = true;
        state.waitingForData = false;
        state.clockStalled = false;
        state.lastClockTick = clockNowSeconds();
        notifyPlayback(element, state, 'pause');
        element.dispatchEvent(new Event('timeupdate'));
        element.dispatchEvent(new Event('pause'));
        element.dispatchEvent(new Event('ended'));
    }

    function bufferedEndAt(state, position) {
        const ranges = state.buffered;
        if (!ranges || !ranges.length) return NaN;
        for (let index = 0; index < ranges.length; index += 1) {
            const start = ranges.start(index);
            const end = ranges.end(index);
            if (position >= start && position < end) return end;
            if (position < start) return position;
        }
        return NaN;
    }

    function tickMediaClocks() {
        const now = clockNowSeconds();
        for (const element of mediaElements) {
            const state = apply(weakMapGet, mediaSlots, [element]);
            if (!state) continue;
            if (state.lastClockTick === undefined) {
                state.lastClockTick = now;
                continue;
            }
            const previousTick = state.lastClockTick;
            state.lastClockTick = now;
            if (state.paused) continue;
            const rate = Number.isFinite(state.playbackRate) ? state.playbackRate : 1;
            const delta = Math.max(0, now - previousTick) * (rate > 0 ? rate : 1);
            if (delta <= 0) continue;
            const lastUpdate = state.lastTimeupdate === undefined ? -Infinity : state.lastTimeupdate;
            const emitTimeupdate = () => {
                if (now - lastUpdate >= TIMEUPDATE_INTERVAL_SECONDS) {
                    state.lastTimeupdate = now;
                    element.dispatchEvent(new Event('timeupdate'));
                }
            };
            const next = state.currentTime + delta;
            if (Number.isFinite(state.duration) && next >= state.duration) {
                finishPlayback(element, state);
                continue;
            }
            const bufferedEnd = bufferedEndAt(state, state.currentTime);
            const noData = !Number.isFinite(bufferedEnd)
                && state.attachedMediaSource !== null;
            const reachedEnd = Number.isFinite(bufferedEnd) && next >= bufferedEnd;
            if (noData || reachedEnd) {
                if (reachedEnd) state.currentTime = bufferedEnd;
                if (!state.clockStalled) {
                    state.clockStalled = true;
                    notifyPlayback(element, state, 'pause');
                }
                if (reachedEnd && !state.waitingForData) {
                    announceReadyState(element, state, HAVE_CURRENT_DATA);
                }
                emitTimeupdate();
                continue;
            }
            if (state.clockStalled) {
                state.clockStalled = false;
                notifyPlayback(element, state, 'play');
            }
            state.currentTime = next;
            emitTimeupdate();
        }
    }

    globalThis.__rixTickMediaClocks = tickMediaClocks;

    globalThis.__rixSourceBufferAdded = function(mediaSource, demuxerId) {
        if (!demuxerId || typeof globalThis.__rixMediaPlayback !== 'function') return;
        for (const element of mediaElements) {
            const state = apply(weakMapGet, mediaSlots, [element]);
            if (!state || state.attachedMediaSource !== mediaSource) continue;
            if (!state.paused && !state.waitingForData) {
                globalThis.__rixMediaPlayback(demuxerId, 'play', state.currentTime,
                    state.playbackRate, state.volume, state.muted);
            } else if (state.currentTime > 0) {
                globalThis.__rixMediaPlayback(demuxerId, 'seek', state.currentTime,
                    state.playbackRate, state.volume, state.muted);
            }
        }
    };

    let clockTimer = 0;
    function ensureMediaClockTimer() {
        if (clockTimer) return;
        if (typeof setInterval !== 'function') return;
        clockTimer = setInterval(tickMediaClocks, CLOCK_TICK_MS);
    }

    Object.defineProperties(HTMLMediaElement.prototype, {
        networkState: {
            get() { return ensureMediaElement(this, 'networkState').networkState; },
            enumerable: true, configurable: true
        },
        readyState: {
            get() { return ensureMediaElement(this, 'readyState').readyState; },
            enumerable: true, configurable: true
        },
        error: {
            get() { return ensureMediaElement(this, 'error').error; },
            enumerable: true, configurable: true
        },
        src: {
            get() {
                ensureMediaElement(this, 'src');
                const raw = this.getAttribute('src');
                if (raw === null) return '';
                try {
                    const base = (this.ownerDocument && this.ownerDocument.baseURI) || (globalThis.location && globalThis.location.href) || 'http://localhost/';
                    return new URL(raw, base).href;
                } catch (_) {
                    return raw;
                }
            },
            set(value) {
                ensureMediaElement(this, 'src');
                this.setAttribute('src', value);
                const state = getMediaState(this);
                if (state.attachedMediaSource) {
                    if (typeof state.attachedMediaSource.__detachFromMediaElement === 'function') {
                        state.attachedMediaSource.__detachFromMediaElement();
                    }
                    state.attachedMediaSource = null;
                }
                resetMediaElementState(state);
                if (value) {
                    state.networkState = NETWORK_LOADING;
                    scheduleMicrotask(() => {
                        this.dispatchEvent(new Event('loadstart'));
                    });
                    if (typeof globalThis.__rixResolveMediaSource === 'function') {
                        const ms = globalThis.__rixResolveMediaSource(value);
                        if (ms) {
                            state.attachedMediaSource = ms;
                            if (typeof ms.__attachToMediaElement === 'function') {
                                ms.__attachToMediaElement(this);
                            }
                        }
                    }
                } else {
                    scheduleMicrotask(() => {
                        this.dispatchEvent(new Event('emptied'));
                    });
                }
            },
            enumerable: true, configurable: true
        },
        currentSrc: {
            get() {
                return this.src;
            },
            enumerable: true, configurable: true
        },
        srcObject: {
            get() {
                return ensureMediaElement(this, 'srcObject').srcObject || null;
            },
            set(value) {
                const state = ensureMediaElement(this, 'srcObject');
                if (state.attachedMediaSource) {
                    if (typeof state.attachedMediaSource.__detachFromMediaElement === 'function') {
                        state.attachedMediaSource.__detachFromMediaElement();
                    }
                    state.attachedMediaSource = null;
                }
                state.srcObject = value && typeof value === 'object' ? value : null;
                resetMediaElementState(state);
                if (state.srcObject) {
                    state.networkState = NETWORK_LOADING;
                    scheduleMicrotask(() => {
                        this.dispatchEvent(new Event('loadstart'));
                    });
                    if (typeof state.srcObject.__attachToMediaElement === 'function') {
                        state.attachedMediaSource = state.srcObject;
                        state.srcObject.__attachToMediaElement(this);
                    }
                } else {
                    scheduleMicrotask(() => {
                        this.dispatchEvent(new Event('emptied'));
                    });
                }
            },
            enumerable: true, configurable: true
        },
        preload: {
            get() {
                ensureMediaElement(this, 'preload');
                const val = this.getAttribute('preload');
                return val === null ? '' : val.toLowerCase();
            },
            set(value) {
                ensureMediaElement(this, 'preload');
                this.setAttribute('preload', value);
            },
            enumerable: true, configurable: true
        },
        buffered: {
            get() {
                const state = ensureMediaElement(this, 'buffered');
                refreshMediaSourceRanges(state);
                return state.buffered;
            },
            enumerable: true, configurable: true
        },
        played: {
            get() { return ensureMediaElement(this, 'played').played; },
            enumerable: true, configurable: true
        },
        seekable: {
            get() {
                const state = ensureMediaElement(this, 'seekable');
                refreshMediaSourceRanges(state);
                return state.seekable;
            },
            enumerable: true, configurable: true
        },
        currentTime: {
            get() {
                const state = ensureMediaElement(this, 'currentTime');
                if (state.defaultPlaybackStartPosition !== 0) {
                    return state.defaultPlaybackStartPosition;
                }
                return state.currentTime;
            },
            set(value) {
                const state = ensureMediaElement(this, 'currentTime');
                const val = Number(value);
                if (!Number.isFinite(val)) throw new TypeError('currentTime must be a finite number');
                if (state.readyState === HAVE_NOTHING && Number.isNaN(state.duration)) {
                    state.defaultPlaybackStartPosition = val;
                    notifyPlayback(this, state, 'seek', val);
                    return;
                }
                seekMediaElement(this, state, val);
            },
            enumerable: true, configurable: true
        },
        duration: {
            get() { return ensureMediaElement(this, 'duration').duration; },
            enumerable: true, configurable: true
        },
        paused: {
            get() { return ensureMediaElement(this, 'paused').paused; },
            enumerable: true, configurable: true
        },
        defaultPlaybackRate: {
            get() { return ensureMediaElement(this, 'defaultPlaybackRate').defaultPlaybackRate; },
            set(value) {
                const state = ensureMediaElement(this, 'defaultPlaybackRate');
                const val = Number(value);
                if (!Number.isFinite(val)) throw new TypeError('defaultPlaybackRate must be finite');
                state.defaultPlaybackRate = val;
                this.dispatchEvent(new Event('ratechange'));
            },
            enumerable: true, configurable: true
        },
        playbackRate: {
            get() { return ensureMediaElement(this, 'playbackRate').playbackRate; },
            set(value) {
                const state = ensureMediaElement(this, 'playbackRate');
                const val = Number(value);
                if (!Number.isFinite(val)) throw new TypeError('playbackRate must be finite');
                state.playbackRate = val;
                state.lastClockTick = clockNowSeconds();
                notifyPlayback(this, state, 'rate');
                this.dispatchEvent(new Event('ratechange'));
            },
            enumerable: true, configurable: true
        },
        seeking: {
            get() { return ensureMediaElement(this, 'seeking').seeking; },
            enumerable: true, configurable: true
        },
        ended: {
            get() { return ensureMediaElement(this, 'ended').ended; },
            enumerable: true, configurable: true
        },
        autoplay: {
            get() { ensureMediaElement(this, 'autoplay'); return this.hasAttribute('autoplay'); },
            set(value) { ensureMediaElement(this, 'autoplay'); if (value) this.setAttribute('autoplay', ''); else this.removeAttribute('autoplay'); },
            enumerable: true, configurable: true
        },
        loop: {
            get() { ensureMediaElement(this, 'loop'); return this.hasAttribute('loop'); },
            set(value) { ensureMediaElement(this, 'loop'); if (value) this.setAttribute('loop', ''); else this.removeAttribute('loop'); },
            enumerable: true, configurable: true
        },
        controls: {
            get() { ensureMediaElement(this, 'controls'); return this.hasAttribute('controls'); },
            set(value) { ensureMediaElement(this, 'controls'); if (value) this.setAttribute('controls', ''); else this.removeAttribute('controls'); },
            enumerable: true, configurable: true
        },
        volume: {
            get() { return ensureMediaElement(this, 'volume').volume; },
            set(value) {
                const state = ensureMediaElement(this, 'volume');
                const val = Number(value);
                if (!Number.isFinite(val) || val < 0.0 || val > 1.0) {
                    throw new DOMException('Volume value must be between 0.0 and 1.0', 'IndexSizeError');
                }
                if (state.volume !== val) {
                    state.volume = val;
                    notifyPlayback(this, state, 'volume');
                    this.dispatchEvent(new Event('volumechange'));
                }
            },
            enumerable: true, configurable: true
        },
        muted: {
            get() { return ensureMediaElement(this, 'muted').muted; },
            set(value) {
                const state = ensureMediaElement(this, 'muted');
                const val = Boolean(value);
                if (state.muted !== val) {
                    state.muted = val;
                    notifyPlayback(this, state, 'volume');
                    this.dispatchEvent(new Event('volumechange'));
                }
            },
            enumerable: true, configurable: true
        },
        defaultMuted: {
            get() { ensureMediaElement(this, 'defaultMuted'); return this.hasAttribute('muted'); },
            set(value) { ensureMediaElement(this, 'defaultMuted'); if (value) this.setAttribute('muted', ''); else this.removeAttribute('muted'); },
            enumerable: true, configurable: true
        },
    });

    HTMLMediaElement.prototype.load = function() {
        const state = ensureMediaElement(this, 'load');
        const raw = this.getAttribute('src');
        if (state.attachedMediaSource) {
            if (typeof state.attachedMediaSource.__detachFromMediaElement === 'function') {
                state.attachedMediaSource.__detachFromMediaElement();
            }
            state.attachedMediaSource = null;
        }
        resetMediaElementState(state);
        if (raw) {
            state.networkState = NETWORK_LOADING;
            this.dispatchEvent(new Event('loadstart'));
        } else {
            this.dispatchEvent(new Event('emptied'));
        }
    };

    HTMLMediaElement.prototype.canPlayType = function(type) {
        ensureMediaElement(this, 'canPlayType');
        if (!type || typeof type !== 'string') return '';
        const trimmed = type.trim();
        if (!trimmed) return '';

        const semi = trimmed.indexOf(';');
        const mime = (semi === -1 ? trimmed : trimmed.slice(0, semi)).trim().toLowerCase();
        const params = semi === -1 ? '' : trimmed.slice(semi + 1);

        const isContainerSupported = typeof globalThis.__rixIsContainerSupported === 'function'
            ? globalThis.__rixIsContainerSupported(mime)
            : false;

        if (!isContainerSupported) {
            return '';
        }

        const codecsMatch = params.match(/codecs\s*=\s*(?:"([^"]+)"|'([^']+)'|([^,;]+))/i);
        if (!codecsMatch) {
            return 'maybe';
        }

        if (typeof globalThis.__rixIsTypeSupported === 'function') {
            return globalThis.__rixIsTypeSupported(trimmed) ? 'probably' : '';
        }

        return '';
    };

    HTMLMediaElement.prototype.play = function() {
        const state = ensureMediaElement(this, 'play');
        if (state.error) {
            return Promise.reject(new DOMException('Media error occurred', 'NotSupportedError'));
        }
        if (state.ended) {
            state.ended = false;
            state.currentTime = 0;
            state.defaultPlaybackStartPosition = 0;
            state.lastClockTick = clockNowSeconds();
            notifyPlayback(this, state, 'seek');
            scheduleMicrotask(() => {
                this.dispatchEvent(new Event('seeking'));
            });
            scheduleMicrotask(() => {
                state.seeking = false;
                this.dispatchEvent(new Event('timeupdate'));
                this.dispatchEvent(new Event('seeked'));
            });
        }
        const waitsForData = state.attachedMediaSource !== null;
        const wasPaused = state.paused;
        if (wasPaused) {
            state.paused = false;
            state.clockStalled = false;
            state.lastClockTick = clockNowSeconds();
            this.dispatchEvent(new Event('play'));
            notifyPlayback(this, state, 'play');
        }
        if (!waitsForData || state.readyState >= HAVE_FUTURE_DATA) {
            if (wasPaused) {
                state.waitingForData = false;
                this.dispatchEvent(new Event('playing'));
            }
            return Promise.resolve();
        }
        if (!state.pendingPlayPromise) {
            state.pendingPlayPromise = new Promise((resolve) => {
                state.pendingPlayResolve = resolve;
            });
        }
        state.waitingForData = true;
        if (wasPaused) {
            scheduleMicrotask(() => {
                if (!state.paused && !state.ended && state.readyState < HAVE_FUTURE_DATA) {
                    this.dispatchEvent(new Event('waiting'));
                }
            });
        }
        return state.pendingPlayPromise;
    };

    HTMLMediaElement.prototype.pause = function() {
        const state = ensureMediaElement(this, 'pause');
        if (!state.paused) {
            state.paused = true;
            state.waitingForData = false;
            state.clockStalled = false;
            state.lastClockTick = clockNowSeconds();
            notifyPlayback(this, state, 'pause');
            this.dispatchEvent(new Event('pause'));
        }
    };

    HTMLMediaElement.prototype.fastSeek = function(time) {
        this.currentTime = time;
    };

    if (HTMLVideoElement) {
        Object.defineProperties(HTMLVideoElement.prototype, {
            width: {
                get() {
                    const val = this.getAttribute('width');
                    const parsed = parseInt(val, 10);
                    return Number.isFinite(parsed) && parsed >= 0 ? parsed : 0;
                },
                set(value) {
                    this.setAttribute('width', String(Math.max(0, parseInt(value, 10) || 0)));
                },
                enumerable: true, configurable: true
            },
            height: {
                get() {
                    const val = this.getAttribute('height');
                    const parsed = parseInt(val, 10);
                    return Number.isFinite(parsed) && parsed >= 0 ? parsed : 0;
                },
                set(value) {
                    this.setAttribute('height', String(Math.max(0, parseInt(value, 10) || 0)));
                },
                enumerable: true, configurable: true
            },
            videoWidth: {
                get() {
                    return ensureMediaElement(this, 'videoWidth').videoWidth || 0;
                },
                enumerable: true, configurable: true
            },
            videoHeight: {
                get() {
                    return ensureMediaElement(this, 'videoHeight').videoHeight || 0;
                },
                enumerable: true, configurable: true
            },
            poster: {
                get() {
                    const raw = this.getAttribute('poster');
                    if (raw === null) return '';
                    try {
                        const base = (this.ownerDocument && this.ownerDocument.baseURI) || (globalThis.location && globalThis.location.href) || 'http://localhost/';
                        return new URL(raw, base).href;
                    } catch (_) {
                        return raw;
                    }
                },
                set(value) {
                    this.setAttribute('poster', value);
                },
                enumerable: true, configurable: true
            }
        });
    }

    function Audio(src) {
        if (!new.target) {
            throw new TypeError("Failed to construct 'Audio': Please use the 'new' operator");
        }
        const doc = globalThis.document;
        if (!doc) {
            throw new TypeError("Audio constructor requires an active document");
        }
        const audio = doc.createElement('audio');
        audio.preload = 'auto';
        if (src !== undefined) {
            audio.src = String(src);
        }
        return audio;
    }
    Audio.prototype = HTMLAudioElement ? HTMLAudioElement.prototype : Object.create(HTMLMediaElement.prototype);
    Audio.prototype.constructor = Audio;

    Object.defineProperty(globalThis, 'TimeRanges', { value: TimeRanges, writable: true, configurable: true });
    Object.defineProperty(globalThis, 'MediaError', { value: MediaError, writable: true, configurable: true });
    Object.defineProperty(globalThis, 'Audio', { value: Audio, writable: true, configurable: true });
})();
"#;

fn media_source_is_type_supported(type_str: &str) -> bool {
    let trimmed = type_str.trim();
    if trimmed.is_empty() {
        return false;
    }

    let mut parts = trimmed.split(';');
    let mime = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    if create_demuxer_for_mime(&mime).is_none() {
        return false;
    }

    let mut codecs_found = Vec::new();
    for param in parts {
        let lower_param = param.trim().to_ascii_lowercase();
        if let Some(rest) = lower_param.strip_prefix("codecs=") {
            let unquoted = rest.trim_matches('"').trim_matches('\'');
            for codec in unquoted.split(',') {
                let cleaned = codec.trim();
                if !cleaned.is_empty() {
                    codecs_found.push(cleaned.to_string());
                }
            }
        }
    }

    if codecs_found.is_empty() {
        return false;
    }

    codecs_found
        .iter()
        .all(|codec| rix_media::decoder::CodecRegistry::is_codec_supported(&mime, codec))
}

pub(super) fn register_media_interfaces(context: &mut Context) -> JsResult<()> {
    context.register_global_builtin_callable(
        boa_engine::js_string!("__rixIsTypeSupported"),
        1,
        boa_engine::NativeFunction::from_copy_closure(|_this, args, context| {
            let type_str = match args.get(0) {
                Some(v) => v.to_string(context)?.to_std_string_escaped(),
                None => return Ok(boa_engine::JsValue::new(false)),
            };
            let supported = media_source_is_type_supported(&type_str);
            Ok(boa_engine::JsValue::new(supported))
        }),
    )?;

    context.register_global_builtin_callable(
        boa_engine::js_string!("__rixIsContainerSupported"),
        1,
        boa_engine::NativeFunction::from_copy_closure(|_this, args, context| {
            let container_str = match args.get(0) {
                Some(v) => v.to_string(context)?.to_std_string_escaped(),
                None => return Ok(boa_engine::JsValue::new(false)),
            };
            let supported =
                rix_media::decoder::CodecRegistry::is_container_supported(&container_str);
            Ok(boa_engine::JsValue::new(supported))
        }),
    )?;

    context.register_global_builtin_callable(
        boa_engine::js_string!("__rixCreateDemuxer"),
        1,
        boa_engine::NativeFunction::from_copy_closure(|_this, args, context| {
            let type_str = match args.get(0) {
                Some(v) => v.to_string(context)?.to_std_string_escaped(),
                None => return Ok(boa_engine::JsValue::new(0)),
            };
            let demuxer = create_demuxer_for_mime(&type_str);
            let store = match context.get_data::<Rc<RefCell<DemuxerStore>>>().cloned() {
                Some(s) => s,
                None => {
                    let s = Rc::new(RefCell::new(DemuxerStore::new()));
                    context.insert_data(s.clone());
                    s
                }
            };
            if let Some(d) = demuxer {
                let mut s = store.borrow_mut();
                let id = s.next_id;
                s.next_id += 1;
                s.demuxers.insert(id, d);
                Ok(boa_engine::JsValue::new(id))
            } else {
                Ok(boa_engine::JsValue::new(0))
            }
        }),
    )?;

    context.register_global_builtin_callable(
        boa_engine::js_string!("__rixAppendDemuxerData"),
        2,
        boa_engine::NativeFunction::from_copy_closure(|_this, args, context| {
            let id = match args.get(0) {
                Some(v) => v.to_u32(context)?,
                None => return Ok(boa_engine::JsValue::null()),
            };
            if id == 0 {
                return Ok(boa_engine::JsValue::null());
            }
            let bytes = match args.get(1).and_then(|v| v.as_object()) {
                Some(obj) => {
                    if let Ok(u8arr) = JsUint8Array::from_object(obj.clone()) {
                        u8arr.to_vec(context)?
                    } else if let Ok(arrbuf) = JsArrayBuffer::from_object(obj.clone()) {
                        arrbuf
                            .borrow()
                            .data()
                            .data()
                            .map(|d| d.to_vec())
                            .unwrap_or_default()
                    } else {
                        Vec::new()
                    }
                }
                None => Vec::new(),
            };
            if bytes.is_empty() {
                return Ok(boa_engine::JsValue::null());
            }
            let store = match context.get_data::<Rc<RefCell<DemuxerStore>>>().cloned() {
                Some(s) => s,
                None => return Ok(boa_engine::JsValue::null()),
            };
            let mut s = store.borrow_mut();
            let demuxer = match s.demuxers.get_mut(&id) {
                Some(d) => d,
                None => return Ok(boa_engine::JsValue::null()),
            };
            let mut payload = bytes.clone();
            let is_mp4_box = payload.len() >= 8
                && payload[4..8].iter().all(|b| b.is_ascii_alphabetic());
            if !is_mp4_box {
                let segs = rix_net::sabr::parse_ump_response(&payload);
                if !segs.is_empty() {
                    let mut combined = Vec::new();
                    for seg in &segs {
                        combined.extend_from_slice(&seg.data);
                    }
                    if !combined.is_empty() {
                        payload = combined;
                    }
                }
            }
            let append_ok = demuxer.append_data(&payload).is_ok();
            let init_complete = demuxer.is_init_complete();
            let duration = demuxer.duration();
            let ranges = demuxer.buffered_ranges();
            let mut video_width: Option<u32> = None;
            let mut video_height: Option<u32> = None;
            for track in demuxer.tracks() {
                if track.track_type == rix_media::sample::TrackType::Video {
                    if let Some(ref cfg) = track.video_config {
                        video_width = Some(cfg.width);
                        video_height = Some(cfg.height);
                        break;
                    }
                }
            }
            let mut polled_samples = Vec::new();
            while let Some(sample) = demuxer.poll_sample() {
                polled_samples.push(sample);
            }
            let _ = demuxer;
            if !polled_samples.is_empty() {
                s.pending_samples.push((id, polled_samples));
            }
            let js_ranges = JsArray::new(context)?;
            for (start, end) in ranges {
                let pair = JsArray::new(context)?;
                pair.push(boa_engine::JsValue::new(start), context)?;
                pair.push(boa_engine::JsValue::new(end), context)?;
                js_ranges.push(pair, context)?;
            }
            let obj = boa_engine::JsObject::with_object_proto(context.intrinsics());
            obj.set(
                boa_engine::js_string!("success"),
                boa_engine::JsValue::new(append_ok),
                false,
                context,
            )?;
            obj.set(
                boa_engine::js_string!("initComplete"),
                boa_engine::JsValue::new(init_complete),
                false,
                context,
            )?;
            obj.set(
                boa_engine::js_string!("duration"),
                duration.map_or(boa_engine::JsValue::null(), boa_engine::JsValue::new),
                false,
                context,
            )?;
            obj.set(
                boa_engine::js_string!("ranges"),
                js_ranges,
                false,
                context,
            )?;
            obj.set(
                boa_engine::js_string!("videoWidth"),
                video_width.map_or(boa_engine::JsValue::null(), boa_engine::JsValue::new),
                false,
                context,
            )?;
            obj.set(
                boa_engine::js_string!("videoHeight"),
                video_height.map_or(boa_engine::JsValue::null(), boa_engine::JsValue::new),
                false,
                context,
            )?;
            Ok(boa_engine::JsValue::new(obj))
        }),
    )?;

    context.register_global_builtin_callable(
        boa_engine::js_string!("__rixMediaPlayback"),
        6,
        boa_engine::NativeFunction::from_copy_closure(|_this, args, context| {
            let demuxer_id = args.get_or_undefined(0).to_u32(context)?;
            if demuxer_id == 0 {
                return Ok(boa_engine::JsValue::undefined());
            }
            let kind = args
                .get_or_undefined(1)
                .to_string(context)?
                .to_std_string_escaped();
            let position = args.get_or_undefined(2).to_number(context)?;
            let rate = args.get_or_undefined(3).to_number(context)?;
            let volume = args.get_or_undefined(4).to_number(context)?;
            let muted = args.get_or_undefined(5).to_boolean();
            let valid_rate = if rate.is_finite() && rate > 0.0 { rate } else { 1.0 };
            let command = match kind.as_str() {
                "play" => MediaPlaybackCommand::Play { rate: valid_rate },
                "pause" => MediaPlaybackCommand::Pause,
                "seek" => MediaPlaybackCommand::Seek {
                    position: if position.is_finite() { position.max(0.0) } else { 0.0 },
                },
                "rate" => MediaPlaybackCommand::SetRate { rate: valid_rate },
                "volume" => MediaPlaybackCommand::SetVolume {
                    volume: if volume.is_finite() { (volume as f32).clamp(0.0, 1.0) } else { 1.0 },
                    muted,
                },
                "endOfStream" => MediaPlaybackCommand::EndOfStream,
                _ => return Ok(boa_engine::JsValue::undefined()),
            };
            let store = match context.get_data::<Rc<RefCell<DemuxerStore>>>().cloned() {
                Some(store) => store,
                None => return Ok(boa_engine::JsValue::undefined()),
            };
            store
                .borrow_mut()
                .pending_playback
                .push(QueuedPlaybackCommand { demuxer_id, command });
            Ok(boa_engine::JsValue::undefined())
        }),
    )?;

    context.register_global_builtin_callable(
        boa_engine::js_string!("__rixRemoveDemuxerRange"),
        3,
        boa_engine::NativeFunction::from_copy_closure(|_this, args, context| {
            let id = match args.get(0) {
                Some(v) => v.to_u32(context)?,
                None => return Ok(boa_engine::JsValue::undefined()),
            };
            let start = match args.get(1) {
                Some(v) => v.to_number(context)?,
                None => 0.0,
            };
            let end = match args.get(2) {
                Some(v) => v.to_number(context)?,
                None => 0.0,
            };
            if let Some(store) = context.get_data::<Rc<RefCell<DemuxerStore>>>().cloned() {
                if let Some(demuxer) = store.borrow_mut().demuxers.get_mut(&id) {
                    demuxer.remove_range(start, end);
                }
            }
            Ok(boa_engine::JsValue::undefined())
        }),
    )?;

    context.register_global_builtin_callable(
        boa_engine::js_string!("__rixResetDemuxer"),
        1,
        boa_engine::NativeFunction::from_copy_closure(|_this, args, context| {
            let id = match args.get(0) {
                Some(v) => v.to_u32(context)?,
                None => return Ok(boa_engine::JsValue::undefined()),
            };
            if let Some(store) = context.get_data::<Rc<RefCell<DemuxerStore>>>().cloned() {
                if let Some(demuxer) = store.borrow_mut().demuxers.get_mut(&id) {
                    demuxer.reset();
                }
            }
            Ok(boa_engine::JsValue::undefined())
        }),
    )?;

    context.register_global_builtin_callable(
        boa_engine::js_string!("__rixDestroyDemuxer"),
        1,
        boa_engine::NativeFunction::from_copy_closure(|_this, args, context| {
            let id = match args.get(0) {
                Some(v) => v.to_u32(context)?,
                None => return Ok(boa_engine::JsValue::undefined()),
            };
            if let Some(store) = context.get_data::<Rc<RefCell<DemuxerStore>>>().cloned() {
                store.borrow_mut().demuxers.remove(&id);
            }
            Ok(boa_engine::JsValue::undefined())
        }),
    )?;

    context.eval(Source::from_bytes(MEDIA_API_SCRIPT.as_bytes()))?;
    Ok(())
}

