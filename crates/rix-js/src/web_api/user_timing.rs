
use super::*;
use boa_engine::object::builtins::JsArray;

const LEGACY_TIMING_NAMES: &[&str] = &[
    "navigationStart",
    "unloadEventStart",
    "unloadEventEnd",
    "redirectStart",
    "redirectEnd",
    "fetchStart",
    "domainLookupStart",
    "domainLookupEnd",
    "connectStart",
    "connectEnd",
    "secureConnectionStart",
    "requestStart",
    "responseStart",
    "responseEnd",
    "domLoading",
    "domInteractive",
    "domContentLoadedEventStart",
    "domContentLoadedEventEnd",
    "domComplete",
    "loadEventStart",
    "loadEventEnd",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryType {
    Mark,
    Measure,
}

impl EntryType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Mark => "mark",
            Self::Measure => "measure",
        }
    }
}

#[derive(Clone)]
struct PerformanceEntryRecord {
    name: String,
    entry_type: EntryType,
    start_time: f64,
    object: JsObject,
}

#[derive(Clone, Default)]
struct UserTimingState {
    entries: Vec<PerformanceEntryRecord>,
}

#[derive(Clone)]
struct UserTimingBindings {
    performance: JsObject,
    performance_prototype: JsObject,
    create_entry: JsObject,
}

pub(super) fn register_user_timing_api(
    context: &mut Context,
    performance: &JsObject,
) -> JsResult<()> {
    if context.get_data::<UserTimingState>().is_none() {
        context.insert_data(UserTimingState::default());
    }

    let mut bindings = if let Some(bindings) = context.get_data::<UserTimingBindings>().cloned() {
        bindings
    } else {
        let objects = context
            .eval(boa_engine::Source::from_bytes(INTERFACE_FACTORY))?
            .as_object()
            .ok_or_else(invariant_violation)?;
        let get_object = |name: &str, context: &mut Context| -> JsResult<JsObject> {
            objects
                .get(js_string!(name), context)?
                .as_object()
                .ok_or_else(invariant_violation)
        };
        let performance_prototype = get_object("performancePrototype", context)?;
        let create_entry = get_object("createEntry", context)?;
        for (name, getter) in [
            ("Performance", get_object("Performance", context)?),
            ("PerformanceEntry", get_object("PerformanceEntry", context)?),
            ("PerformanceMark", get_object("PerformanceMark", context)?),
            (
                "PerformanceMeasure",
                get_object("PerformanceMeasure", context)?,
            ),
        ] {
            context.register_global_property(
                js_string!(name),
                getter,
                Attribute::WRITABLE | Attribute::CONFIGURABLE,
            )?;
        }
        let bindings = UserTimingBindings {
            performance: performance.clone(),
            performance_prototype,
            create_entry,
        };
        install_methods(&bindings.performance_prototype, context)?;
        bindings
    };

    if !performance.set_prototype(Some(bindings.performance_prototype.clone())) {
        return Err(JsNativeError::error()
            .with_message("Could not initialize Performance prototype")
            .into());
    }
    bindings.performance = performance.clone();
    context.insert_data(bindings);
    Ok(())
}

fn install_methods(prototype: &JsObject, context: &mut Context) -> JsResult<()> {
    for (name, length, function) in [
        ("mark", 1, performance_mark as NativeMethod),
        ("clearMarks", 0, performance_clear_marks),
        ("measure", 1, performance_measure),
        ("clearMeasures", 0, performance_clear_measures),
        ("getEntries", 0, performance_get_entries),
        ("getEntriesByType", 1, performance_get_entries_by_type),
        ("getEntriesByName", 1, performance_get_entries_by_name),
    ] {
        let method =
            FunctionObjectBuilder::new(context.realm(), NativeFunction::from_fn_ptr(function))
                .name(js_string!(name))
                .length(length)
                .constructor(false)
                .build();
        prototype.define_property_or_throw(
            js_string!(name),
            PropertyDescriptor::builder()
                .value(method)
                .writable(true)
                .enumerable(false)
                .configurable(true)
                .build(),
            context,
        )?;
    }
    Ok(())
}

type NativeMethod = fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>;

fn performance_mark(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    require_performance_receiver(this, context)?;
    let name = required_dom_string(args, 0, "performance.mark requires a mark name", context)?;
    if is_legacy_timing_name(&name) {
        return Err(dom_exception(
            "The mark name is reserved by the PerformanceTiming interface",
            "SyntaxError",
            context,
        )?);
    }

    let (detail, start_time) = match args.get(1) {
        None => (JsValue::undefined(), None),
        Some(value) if value.is_undefined() => (JsValue::undefined(), None),
        Some(value) => {
            let options = dictionary_object(value, context)?;
            let detail = dictionary_member(&options, "detail", context)?;
            let start_time = dictionary_member(&options, "startTime", context)?
                .map(|value| convert_high_res_timestamp(value, context))
                .transpose()?;
            (detail.unwrap_or_else(JsValue::undefined), start_time)
        }
    };
    let start_time = match start_time {
        Some(start_time) => start_time,
        None => performance_now(this, &[], context)?
            .as_number()
            .ok_or_else(|| {
                JsNativeError::error().with_message("performance.now() did not return a number")
            })?,
    };
    if !start_time.is_finite() || start_time < 0.0 {
        return Err(JsNativeError::typ()
            .with_message("PerformanceMark startTime must be a finite non-negative number")
            .into());
    }
    let detail = crate::structured_clone::clone_serializable_value(&detail, context)?;
    let entry = create_entry(EntryType::Mark, &name, start_time, 0.0, detail, context)?;
    record_entry(name, EntryType::Mark, start_time, entry.clone(), context)?;
    Ok(entry.into())
}

fn performance_clear_marks(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_performance_receiver(this, context)?;
    let name = args
        .first()
        .map(|value| {
            value
                .to_string(context)
                .map(|value| value.to_std_string_escaped())
        })
        .transpose()?;
    let Some(mut state) = context.get_data::<UserTimingState>().cloned() else {
        return Err(invariant_violation());
    };
    state.entries.retain(|entry| {
        entry.entry_type != EntryType::Mark || name.as_ref().is_some_and(|name| &entry.name != name)
    });
    context.insert_data(state);
    Ok(JsValue::undefined())
}

fn performance_measure(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_performance_receiver(this, context)?;
    let name = required_dom_string(args, 0, "performance.measure requires a name", context)?;

    let mut options = MeasureOptions::default();
    let mut positional_start = None;
    if let Some(second) = args.get(1).filter(|value| !value.is_undefined()) {
        if second.as_string().is_some() {
            positional_start = Some(MarkOrTimestamp::Name(
                second.to_string(context)?.to_std_string_escaped(),
            ));
        } else if second.is_null() || second.as_object().is_some() {
            let dictionary = dictionary_object(second, context)?;
            options.detail = dictionary_member(&dictionary, "detail", context)?;
            options.start = convert_mark_member(&dictionary, "start", context)?;
            options.duration = dictionary_member(&dictionary, "duration", context)?
                .map(|value| convert_high_res_timestamp(value, context))
                .transpose()?;
            options.end = convert_mark_member(&dictionary, "end", context)?;
        } else {
            positional_start = Some(MarkOrTimestamp::Name(
                second.to_string(context)?.to_std_string_escaped(),
            ));
        }
    }
    let end_mark = args
        .get(2)
        .map(|value| {
            value
                .to_string(context)
                .map(|value| value.to_std_string_escaped())
        })
        .transpose()?;

    let has_measure_options = options.detail.is_some()
        || options.start.is_some()
        || options.duration.is_some()
        || options.end.is_some();
    if has_measure_options && end_mark.is_some() {
        return Err(JsNativeError::typ()
            .with_message("endMark cannot be combined with PerformanceMeasureOptions")
            .into());
    }
    if has_measure_options && options.start.is_none() && options.end.is_none() {
        return Err(JsNativeError::typ()
            .with_message("PerformanceMeasureOptions requires start or end")
            .into());
    }
    if options.start.is_some() && options.duration.is_some() && options.end.is_some() {
        return Err(JsNativeError::typ()
            .with_message("start, duration, and end cannot all be supplied")
            .into());
    }
    if options.duration.is_some_and(|duration| duration < 0.0) {
        return Err(JsNativeError::typ()
            .with_message("Performance measure duration must not be negative")
            .into());
    }

    let now = performance_now(this, &[], context)?
        .as_number()
        .ok_or_else(|| {
            JsNativeError::error().with_message("performance.now() did not return a number")
        })?;
    let end_time = if let Some(end_mark) = end_mark {
        convert_mark_to_timestamp(MarkOrTimestamp::Name(end_mark), context)?
    } else if let Some(end) = options.end.clone() {
        convert_mark_to_timestamp(end, context)?
    } else if let (Some(start), Some(duration)) = (&options.start, options.duration) {
        convert_mark_to_timestamp(start.clone(), context)? + duration
    } else {
        now
    };

    let start_time = if let Some(start) = options.start {
        convert_mark_to_timestamp(start, context)?
    } else if let (Some(duration), Some(end)) = (options.duration, &options.end) {
        convert_mark_to_timestamp(end.clone(), context)? - duration
    } else if let Some(start) = positional_start {
        convert_mark_to_timestamp(start, context)?
    } else {
        0.0
    };

    let detail = match options.detail {
        Some(detail) => crate::structured_clone::clone_serializable_value(&detail, context)?,
        None => JsValue::null(),
    };
    let duration = end_time - start_time;
    let entry = create_entry(
        EntryType::Measure,
        &name,
        start_time,
        duration,
        detail,
        context,
    )?;
    record_entry(name, EntryType::Measure, start_time, entry.clone(), context)?;
    Ok(entry.into())
}

fn performance_clear_measures(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_performance_receiver(this, context)?;
    let name = args
        .first()
        .map(|value| {
            value
                .to_string(context)
                .map(|value| value.to_std_string_escaped())
        })
        .transpose()?;
    let Some(mut state) = context.get_data::<UserTimingState>().cloned() else {
        return Err(invariant_violation());
    };
    state.entries.retain(|entry| {
        entry.entry_type != EntryType::Measure
            || name.as_ref().is_some_and(|name| &entry.name != name)
    });
    context.insert_data(state);
    Ok(JsValue::undefined())
}

fn performance_get_entries(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_performance_receiver(this, context)?;
    entries_array(None, None, context)
}

fn performance_get_entries_by_type(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_performance_receiver(this, context)?;
    let entry_type =
        required_dom_string(args, 0, "getEntriesByType requires an entry type", context)?;
    entries_array(Some(&entry_type), None, context)
}

fn performance_get_entries_by_name(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    require_performance_receiver(this, context)?;
    let name = required_dom_string(args, 0, "getEntriesByName requires a name", context)?;
    let entry_type = args
        .get(1)
        .map(|value| {
            value
                .to_string(context)
                .map(|value| value.to_std_string_escaped())
        })
        .transpose()?;
    entries_array(entry_type.as_deref(), Some(&name), context)
}

fn entries_array(
    entry_type: Option<&str>,
    name: Option<&str>,
    context: &mut Context,
) -> JsResult<JsValue> {
    let state = context
        .get_data::<UserTimingState>()
        .cloned()
        .ok_or_else(invariant_violation)?;
    let mut entries: Vec<_> = state
        .entries
        .into_iter()
        .filter(|entry| {
            entry_type.is_none_or(|entry_type| entry.entry_type.as_str() == entry_type)
                && name.is_none_or(|name| entry.name == name)
        })
        .collect();
    entries.sort_by(|left, right| left.start_time.total_cmp(&right.start_time));
    Ok(JsArray::from_iter(
        entries.into_iter().map(|entry| entry.object.into()),
        context,
    )
    .into())
}

#[derive(Clone)]
enum MarkOrTimestamp {
    Name(String),
    Timestamp(f64),
}

#[derive(Default)]
struct MeasureOptions {
    detail: Option<JsValue>,
    start: Option<MarkOrTimestamp>,
    duration: Option<f64>,
    end: Option<MarkOrTimestamp>,
}

fn convert_mark_member(
    dictionary: &JsObject,
    name: &str,
    context: &mut Context,
) -> JsResult<Option<MarkOrTimestamp>> {
    dictionary_member(dictionary, name, context)?
        .map(|value| {
            if let Some(timestamp) = value.as_number() {
                convert_high_res_timestamp(JsValue::new(timestamp), context)
                    .map(MarkOrTimestamp::Timestamp)
            } else {
                Ok(MarkOrTimestamp::Name(
                    value.to_string(context)?.to_std_string_escaped(),
                ))
            }
        })
        .transpose()
}

fn convert_mark_to_timestamp(mark: MarkOrTimestamp, context: &mut Context) -> JsResult<f64> {
    match mark {
        MarkOrTimestamp::Timestamp(timestamp) => {
            if !timestamp.is_finite() || timestamp < 0.0 {
                return Err(JsNativeError::typ()
                    .with_message("Performance timestamp must be finite and non-negative")
                    .into());
            }
            Ok(timestamp)
        }
        MarkOrTimestamp::Name(name) if is_legacy_timing_name(&name) => {
            legacy_timing_timestamp(&name, context)
        }
        MarkOrTimestamp::Name(name) => {
            let state = context
                .get_data::<UserTimingState>()
                .ok_or_else(invariant_violation)?;
            if let Some(entry) = state
                .entries
                .iter()
                .rev()
                .find(|entry| entry.entry_type == EntryType::Mark && entry.name == name)
            {
                return Ok(entry.start_time);
            }
            Err(dom_exception(
                "The requested performance mark does not exist",
                "SyntaxError",
                context,
            )?)
        }
    }
}

fn convert_high_res_timestamp(value: JsValue, context: &mut Context) -> JsResult<f64> {
    let timestamp = value.to_number(context)?;
    if !timestamp.is_finite() {
        return Err(JsNativeError::typ()
            .with_message("DOMHighResTimeStamp must be finite")
            .into());
    }
    Ok(timestamp)
}

fn legacy_timing_timestamp(name: &str, context: &mut Context) -> JsResult<f64> {
    if name == "navigationStart" {
        return Ok(0.0);
    }
    let performance = context
        .get_data::<UserTimingBindings>()
        .map(|bindings| bindings.performance.clone())
        .ok_or_else(invariant_violation)?;
    let timing = performance.get(js_string!("timing"), context)?;
    let Some(timing) = timing.as_object() else {
        return Err(dom_exception(
            "The requested legacy performance timing is unavailable",
            "InvalidAccessError",
            context,
        )?);
    };
    let navigation_start = timing
        .get(js_string!("navigationStart"), context)?
        .to_number(context)?;
    let end_time = timing.get(js_string!(name), context)?.to_number(context)?;
    if end_time == 0.0 || end_time.is_nan() {
        return Err(dom_exception(
            "The requested legacy performance timing is unavailable",
            "InvalidAccessError",
            context,
        )?);
    }
    Ok(end_time - navigation_start)
}

fn create_entry(
    entry_type: EntryType,
    name: &str,
    start_time: f64,
    duration: f64,
    detail: JsValue,
    context: &mut Context,
) -> JsResult<JsObject> {
    let factory = context
        .get_data::<UserTimingBindings>()
        .map(|bindings| bindings.create_entry.clone())
        .ok_or_else(invariant_violation)?;
    factory
        .call(
            &JsValue::undefined(),
            &[
                js_string!(entry_type.as_str()).into(),
                JsString::from(name).into(),
                JsValue::new(start_time),
                JsValue::new(duration),
                detail,
            ],
            context,
        )?
        .as_object()
        .ok_or_else(invariant_violation)
}

fn record_entry(
    name: String,
    entry_type: EntryType,
    start_time: f64,
    object: JsObject,
    context: &mut Context,
) -> JsResult<()> {
    let mut state = context
        .get_data::<UserTimingState>()
        .cloned()
        .ok_or_else(invariant_violation)?;
    state.entries.push(PerformanceEntryRecord {
        name,
        entry_type,
        start_time,
        object,
    });
    context.insert_data(state);
    Ok(())
}

fn required_dom_string(
    args: &[JsValue],
    index: usize,
    message: &str,
    context: &mut Context,
) -> JsResult<String> {
    args.get(index)
        .ok_or_else(|| JsNativeError::typ().with_message(message.to_owned()))?
        .to_string(context)
        .map(|value| value.to_std_string_escaped())
}

fn dictionary_object(value: &JsValue, context: &mut Context) -> JsResult<JsObject> {
    if value.is_null_or_undefined() {
        return Ok(JsObject::with_null_proto());
    }
    value.to_object(context)
}

fn dictionary_member(
    dictionary: &JsObject,
    name: &str,
    context: &mut Context,
) -> JsResult<Option<JsValue>> {
    let value = dictionary.get(js_string!(name), context)?;
    Ok((!value.is_undefined()).then_some(value))
}

fn require_performance_receiver(this: &JsValue, context: &Context) -> JsResult<()> {
    let receiver = this
        .as_object()
        .ok_or_else(|| JsNativeError::typ().with_message("Illegal Performance method receiver"))?;
    let performance = context
        .get_data::<UserTimingBindings>()
        .ok_or_else(invariant_violation)?;
    if receiver != performance.performance {
        return Err(JsNativeError::typ()
            .with_message("Illegal Performance method receiver")
            .into());
    }
    Ok(())
}

fn is_legacy_timing_name(name: &str) -> bool {
    LEGACY_TIMING_NAMES.contains(&name)
}

fn dom_exception(message: &str, name: &str, context: &mut Context) -> JsResult<JsError> {
    let constructor = context
        .global_object()
        .get(js_string!("DOMException"), context)?
        .as_object()
        .ok_or_else(|| JsNativeError::error().with_message("DOMException constructor missing"))?;
    let exception = constructor.construct(
        &[JsString::from(message).into(), JsString::from(name).into()],
        None,
        context,
    )?;
    Ok(JsError::from_opaque(exception.into()))
}

fn invariant_violation() -> JsError {
    JsNativeError::error()
        .with_message("User Timing internal invariant violated")
        .into()
}

const INTERFACE_FACTORY: &str = r#"
(function() {
    const token = {};
    const entries = new WeakMap();
    const structuredCloneValue = globalThis.structuredClone;
    const object = Object;
    const isFiniteNumber = Number.isFinite;
    const toNumber = value => +value;
    const legacyNames = new Set([
        'navigationStart', 'unloadEventStart', 'unloadEventEnd', 'redirectStart', 'redirectEnd',
        'fetchStart', 'domainLookupStart', 'domainLookupEnd', 'connectStart', 'connectEnd',
        'secureConnectionStart', 'requestStart', 'responseStart', 'responseEnd', 'domLoading',
        'domInteractive', 'domContentLoadedEventStart', 'domContentLoadedEventEnd', 'domComplete',
        'loadEventStart', 'loadEventEnd'
    ]);
    const own = (object, key) => Object.prototype.hasOwnProperty.call(object, key);
    function slots(receiver) {
        const value = entries.get(receiver);
        if (!value) throw new TypeError('Illegal PerformanceEntry receiver');
        return value;
    }
    function Performance() { throw new TypeError('Illegal constructor'); }
    function PerformanceEntry(secret) {
        if (secret !== token) throw new TypeError('Illegal constructor');
    }
    Object.defineProperties(PerformanceEntry.prototype, {
        name: {get() { return slots(this).name; }, enumerable: true, configurable: true},
        entryType: {get() { return slots(this).entryType; }, enumerable: true, configurable: true},
        startTime: {get() { return slots(this).startTime; }, enumerable: true, configurable: true},
        duration: {get() { return slots(this).duration; }, enumerable: true, configurable: true},
        toJSON: {value: function() {
            const entry = slots(this);
            const json = {name: entry.name, entryType: entry.entryType,
                startTime: entry.startTime, duration: entry.duration};
            if (entry.entryType === 'mark' || entry.entryType === 'measure') json.detail = entry.detail;
            return json;
        }, writable: true, configurable: true},
        [Symbol.toStringTag]: {value: 'PerformanceEntry', configurable: true}
    });
    function PerformanceMark(markName, markOptions = {}) {
        if (!new.target) throw new TypeError("Failed to construct 'PerformanceMark': use 'new'");
        if (arguments.length === 0) throw new TypeError("Failed to construct 'PerformanceMark': markName is required");
        const name = typeof markName === 'symbol' ? (() => { throw new TypeError('Cannot convert a Symbol to a string'); })() : String(markName);
        if (legacyNames.has(name)) throw new DOMException('The mark name is reserved by PerformanceTiming', 'SyntaxError');
        markOptions = object(markOptions);
        const detail = markOptions.detail;
        const startTime = markOptions.startTime;
        const time = startTime === undefined ? performance.now() : toNumber(startTime);
        if (!isFiniteNumber(time) || time < 0) {
            throw new TypeError('PerformanceMark startTime must be a finite non-negative number');
        }
        const clonedDetail = structuredCloneValue(detail);
        entries.set(this, {name, entryType: 'mark', startTime: time, duration: 0, detail: clonedDetail});
    }
    PerformanceMark.prototype = Object.create(PerformanceEntry.prototype);
    Object.defineProperties(PerformanceMark.prototype, {
        constructor: {value: PerformanceMark, writable: true, configurable: true},
        detail: {get() { return slots(this).detail; }, enumerable: true, configurable: true},
        [Symbol.toStringTag]: {value: 'PerformanceMark', configurable: true}
    });
    function PerformanceMeasure() { throw new TypeError('Illegal constructor'); }
    PerformanceMeasure.prototype = Object.create(PerformanceEntry.prototype);
    Object.defineProperties(PerformanceMeasure.prototype, {
        constructor: {value: PerformanceMeasure, writable: true, configurable: true},
        detail: {get() { return slots(this).detail; }, enumerable: true, configurable: true},
        [Symbol.toStringTag]: {value: 'PerformanceMeasure', configurable: true}
    });
    Object.defineProperty(Performance.prototype, Symbol.toStringTag,
        {value: 'Performance', configurable: true});
    const markPrototype = PerformanceMark.prototype;
    const measurePrototype = PerformanceMeasure.prototype;
    function createEntry(type, name, startTime, duration, detail) {
        const entry = Object.create(type === 'mark' ? markPrototype : measurePrototype);
        entries.set(entry, {name, entryType: type, startTime, duration, detail});
        return entry;
    }
    for (const [name, value] of Object.entries({Performance, PerformanceEntry, PerformanceMark, PerformanceMeasure})) {
        Object.defineProperty(globalThis, name, {value, writable: true, configurable: true});
    }
    return {Performance, PerformanceEntry, PerformanceMark, PerformanceMeasure,
        performancePrototype: Performance.prototype, createEntry};
})()
"#;
