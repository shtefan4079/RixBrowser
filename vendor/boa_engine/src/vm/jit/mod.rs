//! Just-in-time compilation of hot function bodies.
//!
//! # How it works
//!
//! 1. Every ordinary function call bumps the code block's call counter
//!    (`function_call` in `builtins::function`).
//! 2. At [`HOT_THRESHOLD`] calls the VM loop (`Vm::run`) asks this module to
//!    compile the body when the frame is at its first instruction.
//! 3. [`translate`] lowers the whole bytecode of the body: operations that fit
//!    the native value domain become inline machine code, everything else
//!    becomes an *escape* that hands exactly that instruction back to the
//!    interpreter.
//! 4. Native code keeps a shadow of the frame's registers, value stack and
//!    accumulator in an invocation state array. Before an escape it stores the
//!    shadow there (values it cannot represent are marked `RET_BOXED`, meaning
//!    "the interpreter frame still holds the real value"), the engine
//!    materialises the frame and runs the escaping instruction.
//! 5. Execution can resume natively at *any* instruction boundary: the engine
//!    finds the operation in the body's operation table, rebuilds the shadow
//!    from the frame and calls the native code at that operation. That is what
//!    makes calls and property access cheap-ish: the callee runs in the
//!    interpreter or natively, and the caller continues natively afterwards.
//! 6. Guard failures escape at the failing instruction instead of restarting
//!    the frame, so nothing observable ever executes twice.
//!
//! A body can also become hot inside a single call: an interpreted loop that
//! reaches [`OSR_LOOP_THRESHOLD`] iterations sets an on-stack replacement
//! request on its code block, and the VM loop compiles the body and starts
//! native execution at the frame's current instruction.
//!
//! Entries that keep escaping after a handful of operations are blocked (see
//! [`crate::vm::code_block::CodeBlock::note_short_run`]), which keeps bodies
//! that spend their whole loop in the interpreter from paying the shadow
//! traffic for nothing.

mod translate;

use std::cell::RefCell;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use crate::value::JsVariant;
use crate::vm::completion_record::CompletionRecord;
use crate::{Context, JsValue};
use rix_jit::{
    JIT_DISABLED, JitProgram, NativeFn, RET_BOXED, RET_DEOPT, RET_ESCAPE, RET_FALSE, RET_NULL,
    RET_TRUE, RET_UNDEF,
};

/// Number of calls after which a function body becomes a JIT candidate.
pub const HOT_THRESHOLD: u32 = 8;

/// Number of loop iterations after which a body running in the interpreter
/// asks for on-stack replacement.
///
/// A body can become hot without ever being called a hundred times: one long
/// lived loop (an animation frame, a page init loop) spends all its time in a
/// single call. Once the frame's loop counter reaches this threshold the
/// interpreter asks for the body to be compiled and the rest of the loop runs
/// natively, from the frame's current instruction.
pub const OSR_LOOP_THRESHOLD: u64 = 64;

/// Number of deopts after which a body is permanently disabled.
const MAX_DEOPTS: u32 = 8;

/// Number of arguments that fit in the fixed native argument buffer
/// ([`translate`] rejects functions with more parameters).
pub(crate) const MAX_NATIVE_PARAMETERS: usize = 16;

/// Upper bound on the loop iterations a single native invocation may run
/// before it hands control back to the interpreter. Bounding the native run
/// keeps the interpreter (and with it the loop budget, cooperative yields and
/// the worker watchdog) in the picture even when the engine's own loop limit
/// is disabled, as it is for page workers.
const MAX_NATIVE_LOOP_ITERATIONS: u64 = 10_000_000;

/// A native run that escaped after fewer operations than this counts as
/// "short" for the entry-blocking heuristic.
const SHORT_RUN_OPS: u32 = 16;

/// Toggle state of the JIT: `0` unknown, `1` enabled, `2` disabled.
static ENABLED: AtomicU8 = AtomicU8::new(0);

/// Bodies compiled to native code.
static COMPILED: AtomicU64 = AtomicU64::new(0);

/// Bodies examined but rejected (unsupported bytecode or codegen failure).
static REJECTED: AtomicU64 = AtomicU64::new(0);

/// Native executions abandoned in favour of the interpreter.
static DEOPTS: AtomicU64 = AtomicU64::new(0);

/// Native runs started by an on-stack replacement request.
static OSR_RUNS: AtomicU64 = AtomicU64::new(0);

/// Number of times [`reset_stats`] ran.
///
/// The counters are process-global, so a test that brackets a workload with
/// two [`stats`] snapshots uses this to tell whether a neighbouring test reset
/// the counters inside its measurement window.
static RESET_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Counters describing JIT activity, mostly for tests and diagnostics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JitStats {
    /// Bodies successfully compiled to native code.
    pub compiled: u64,
    /// Bodies examined but rejected.
    pub rejected: u64,
    /// Native executions abandoned in favour of the interpreter.
    pub deopts: u64,
    /// Native runs started because an interpreted loop asked for on-stack
    /// replacement instead of the body being entered at its first instruction.
    pub osr_runs: u64,
}

/// Returns the current JIT activity counters.
#[must_use]
pub fn stats() -> JitStats {
    JitStats {
        compiled: COMPILED.load(Ordering::Relaxed),
        rejected: REJECTED.load(Ordering::Relaxed),
        deopts: DEOPTS.load(Ordering::Relaxed),
        osr_runs: OSR_RUNS.load(Ordering::Relaxed),
    }
}

/// Resets the JIT activity counters.
pub fn reset_stats() {
    COMPILED.store(0, Ordering::Relaxed);
    REJECTED.store(0, Ordering::Relaxed);
    DEOPTS.store(0, Ordering::Relaxed);
    OSR_RUNS.store(0, Ordering::Relaxed);
    RESET_EPOCH.fetch_add(1, Ordering::Relaxed);
}

/// Returns how many times [`reset_stats`] has run in this process.
///
/// Tests compare the value before and after a measured workload: a change
/// means a neighbouring test cleared the counters inside the window, so the
/// window is not a clean observation of the workload's own activity.
#[must_use]
pub fn reset_epoch() -> u64 {
    RESET_EPOCH.load(Ordering::Relaxed)
}

/// Returns whether function bodies may be compiled to native code.
///
/// The JIT is enabled by default; setting the `RIX_JIT_OFF` environment
/// variable to a non-empty value other than `0` disables it for the whole
/// process at first use.
#[must_use]
pub fn enabled() -> bool {
    match ENABLED.load(Ordering::Relaxed) {
        1 => true,
        2 => false,
        _ => {
            let off = std::env::var_os("RIX_JIT_OFF")
                .is_some_and(|value| !value.is_empty() && value != "0");
            let state = if off { 2 } else { 1 };
            ENABLED.store(state, Ordering::Relaxed);
            state == 1
        }
    }
}

/// Enables or disables compilation of function bodies.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(if enabled { 1 } else { 2 }, Ordering::Relaxed);
}

thread_local! {
    /// Invocation state arrays, indexed by frame depth so nested native runs
    /// never share a buffer.
    static SCRATCH: RefCell<Vec<Vec<i64>>> = const { RefCell::new(Vec::new()) };
}

/// How a native run ended.
enum Outcome {
    /// The body returned a value.
    Returned(JsValue),
    /// The body stopped at an instruction the interpreter must execute.
    Escaped,
    /// The body refused to start (no effects). The frame keeps running in the
    /// interpreter from the start.
    Deopt,
    /// The body produced a value outside the native domain (engine bug).
    Invalid,
}

/// Encodes an interpreter value into the native domain.
fn encode(value: &JsValue) -> i64 {
    match value.variant() {
        JsVariant::Integer32(int) => i64::from(int),
        JsVariant::Undefined => RET_UNDEF,
        JsVariant::Null => RET_NULL,
        JsVariant::Boolean(true) => RET_TRUE,
        JsVariant::Boolean(false) => RET_FALSE,
        // Everything else keeps living in the interpreter frame.
        _ => RET_BOXED,
    }
}

/// Decodes a native value back into an interpreter value.
///
/// `RET_BOXED` maps to `None`: the interpreter frame already holds the value
/// and must not be overwritten.
fn decode(encoded: i64) -> Option<JsValue> {
    if encoded == RET_UNDEF {
        Some(JsValue::undefined())
    } else if encoded == RET_NULL {
        Some(JsValue::null())
    } else if encoded == RET_TRUE {
        Some(JsValue::new(true))
    } else if encoded == RET_FALSE {
        Some(JsValue::new(false))
    } else if rix_jit::fits_i32(encoded) {
        Some(JsValue::new(encoded as i32))
    } else {
        None
    }
}

/// Whether a body has native code the VM loop can (re-)enter.
///
/// Bodies that were rejected or deopted carry [`JIT_DISABLED`] in their code
/// block, which is not an entry point and must not make the loop call into the
/// JIT on every instruction boundary.
pub(crate) fn has_native(code: &crate::vm::code_block::CodeBlock) -> bool {
    let ptr = code.jit_fn();
    ptr != 0 && ptr != JIT_DISABLED
}

/// Whether the VM loop should offer the current frame to [`try_run`].
///
/// A frame is offered when its body already has native code the loop can
/// resume at the current instruction, or when the interpreter's loop counter
/// asked for on-stack replacement of a body that is still running interpreted.
pub(crate) fn should_enter(code: &crate::vm::code_block::CodeBlock) -> bool {
    has_native(code) || code.jit_osr_requested()
}

/// Runs a frame natively when possible.
///
/// Returns `Some(completion)` when the frame finished in native code, and
/// `None` when the caller should continue interpreting it (either because the
/// body is not native, or because the interpreter must run the instruction at
/// the frame's program counter first).
pub(crate) fn try_run(context: &mut Context) -> Option<ControlFlow<CompletionRecord>> {
    if !enabled() {
        return None;
    }

    // Compilation usually starts at the first instruction of a hot body, and
    // frames of a body that already has native code can resume it at any
    // instruction boundary. A long interpreted loop asks for on-stack
    // replacement, which allows compiling a body whose frame is in the middle
    // of the loop, so the request is consumed before the early exits: the
    // interpreter keeps asking on later iterations while the loop stays hot.
    let (code, mut program_ptr, osr_request) = {
        let frame = context.vm.frame();
        let osr_request = frame.code_block.take_jit_osr_request();
        if frame.exit_early() || frame.construct() || frame.registers_already_pushed() {
            return None;
        }
        let ptr = frame.code_block.jit_fn();
        if ptr == 0
            && !osr_request
            && (frame.pc != 0 || frame.code_block.call_count() < HOT_THRESHOLD)
        {
            return None;
        }
        (frame.code_block.clone(), ptr, osr_request)
    };

    if program_ptr == 0 {
        match translate::translate(&code).and_then(|desc| rix_jit::compile_program(&desc).ok()) {
            Some(program) => {
                // Leaked on purpose: the code block caches the pointer and the
                // executable mapping lives for the whole process anyway.
                let ptr = Box::into_raw(Box::new(program)) as usize;
                code.set_jit_fn(ptr);
                program_ptr = ptr;
                COMPILED.fetch_add(1, Ordering::Relaxed);
            }
            None => {
                code.set_jit_fn(JIT_DISABLED);
                REJECTED.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        }
    }
    if program_ptr == JIT_DISABLED {
        return None;
    }

    // SAFETY: `program_ptr` is a `Box<JitProgram>` leaked above and never
    // freed, so the reference is valid for the rest of the process.
    let program: &JitProgram = unsafe { &*(program_ptr as *const JitProgram) };

    let (entry_pc, rp, register_count, parameter_count) = {
        let frame = context.vm.frame();
        (
            frame.pc,
            frame.rp as usize,
            code.register_count as usize,
            code.parameter_length as usize,
        )
    };
    if code.jit_entry_blocked(entry_pc) {
        return None;
    }
    let Some(entry) = program.entry_at_pc(entry_pc) else {
        return None;
    };
    let height = entry.height as usize;

    // The interpreter must be exactly at the stack height the operation
    // expects; if it is not, the body keeps running in the interpreter.
    let register_end = rp + register_count;
    if context.vm.stack.len() != register_end + height {
        return None;
    }

    // Guard: native code reads exactly `parameter_count` argument slots.
    let mut args = [0_i64; MAX_NATIVE_PARAMETERS];
    {
        let frame = context.vm.frame();
        if (frame.argument_count as usize) < parameter_count {
            return None;
        }
        let arguments = context.vm.stack.get_arguments(frame);
        for (slot, value) in args.iter_mut().zip(arguments.iter().take(parameter_count)) {
            *slot = encode(value);
        }
    }

    // Native loop iterations are charged to the frame's own budget (the escape
    // path adds them to `loop_iteration_count`), so the guest-visible loop
    // limit stays in charge even though the interpreter does not see every
    // tick.
    let limit = context
        .vm
        .runtime_limits
        .loop_iteration_limit()
        .min(MAX_NATIVE_LOOP_ITERATIONS);
    let (depth, charged) = (
        context.vm.frames.len().saturating_sub(1),
        context.vm.frame().loop_iteration_count,
    );
    let budget = limit.saturating_sub(charged);

    let outcome = SCRATCH.with(|cell| {
        // From here on the native function is really called: an on-stack
        // replacement request has produced a run.
        if osr_request {
            OSR_RUNS.fetch_add(1, Ordering::Relaxed);
        }
        let mut pool = cell.borrow_mut();
        if pool.len() <= depth {
            pool.resize_with(depth + 1, Vec::new);
        }
        let state = &mut pool[depth];
        state.clear();
        state.resize(program.state_len as usize, RET_UNDEF);

        // Encode the current frame into the state array.
        let offsets = program.offsets;
        for index in 0..register_count {
            if let Some(value) = context.vm.stack.get_at(rp + index) {
                state[offsets.registers as usize + index] = encode(value);
            }
        }
        for index in 0..height {
            if let Some(value) = context.vm.stack.get_at(register_end + index) {
                state[offsets.value_stack as usize + index] = encode(value);
            }
        }
        state[offsets.accumulator as usize] = encode(&context.vm.get_return_value());

        // SAFETY: `program.entry` was produced by `rix_jit::compile_program`,
        // which always yields a function with exactly this signature and
        // calling convention. `state` and `args` stay alive and pinned for the
        // duration of the call, and the state array is large enough because
        // the engine sized it with `program.state_len`.
        let function: NativeFn = unsafe { std::mem::transmute(program.entry) };
        let result = unsafe {
            function(
                state.as_mut_ptr(),
                args.as_ptr(),
                parameter_count as u64,
                budget,
                u64::from(entry.op_index),
            )
        };

        if result == RET_ESCAPE {
            let escape_pc = state[offsets.escape_pc as usize] as u32;
            let escape_height = state[offsets.escape_height as usize] as usize;
            let escape_op = state[offsets.escape_op as usize] as u32;
            let iterations = state[offsets.iterations as usize] as u64;

            // Charge the loop iterations the native run consumed: the
            // interpreter's `IncrementLoopIteration` must see the true total
            // for the runtime limit and its cooperative yields to work.
            context.vm.frame_mut().loop_iteration_count = charged.wrapping_add(iterations);

            // Materialise registers, value stack and accumulator. Slots that
            // are `RET_BOXED` keep whatever the interpreter frame holds.
            for index in 0..register_count {
                if let Some(value) = decode(state[offsets.registers as usize + index]) {
                    context.vm.stack.set_at(rp + index, value);
                }
            }
            context.vm.stack.set_len(register_end + escape_height);
            for index in 0..escape_height {
                if let Some(value) = decode(state[offsets.value_stack as usize + index]) {
                    context.vm.stack.set_at(register_end + index, value);
                }
            }
            if let Some(value) = decode(state[offsets.accumulator as usize]) {
                context.vm.set_return_value(value);
            }
            if std::env::var_os("RIX_JIT_DEBUG").is_some() {
                eprintln!(
                    "rix-jit: escape body={} entry_pc={} entry_op={} -> pc={} op={} height={}",
                    code.name().to_std_string_escaped(),
                    entry_pc,
                    entry.op_index,
                    escape_pc,
                    escape_op,
                    escape_height
                );
            }
            context.vm.frame_mut().pc = escape_pc;

            // A run that escaped right away is not paying for its shadow
            // traffic; block the entry after a few of those in a row.
            if escape_op >= entry.op_index && escape_op - entry.op_index < SHORT_RUN_OPS {
                code.note_short_run(entry_pc);
            } else {
                code.note_long_run();
            }
            return Outcome::Escaped;
        }

        if result == RET_DEOPT {
            return Outcome::Deopt;
        }

        let value = if result == RET_UNDEF {
            JsValue::undefined()
        } else if result == RET_NULL {
            JsValue::null()
        } else if result == RET_TRUE {
            JsValue::new(true)
        } else if result == RET_FALSE {
            JsValue::new(false)
        } else if rix_jit::fits_i32(result) {
            JsValue::new(result as i32)
        } else {
            // A well-formed native body can only produce the values above; if
            // it ever did not, the safe reaction is to stop using it.
            return Outcome::Invalid;
        };
        Outcome::Returned(value)
    });

    match outcome {
        Outcome::Returned(value) => {
            code.note_long_run();
            context.vm.set_return_value(value);
            Some(context.handle_return())
        }
        Outcome::Escaped => None,
        Outcome::Deopt => {
            note_deopt(&code);
            None
        }
        Outcome::Invalid => {
            note_deopt(&code);
            None
        }
    }
}

/// Records one deopt, disabling the body after [`MAX_DEOPTS`].
fn note_deopt(code: &crate::gc::Gc<crate::vm::code_block::CodeBlock>) {
    DEOPTS.fetch_add(1, Ordering::Relaxed);
    let deopts = code.increment_jit_deopt_count();
    if deopts >= MAX_DEOPTS {
        code.set_jit_fn(JIT_DISABLED);
    }
}
