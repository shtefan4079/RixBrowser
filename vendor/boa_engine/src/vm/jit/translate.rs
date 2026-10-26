//! Translation of function bytecode into the JIT IR.
//!
//! Every instruction of a body is translated: the ones that fit the native
//! value domain (`i32`-range integers plus `undefined`, `null` and booleans)
//! become inline operations, and everything else — heap access, calls,
//! environments, iteration, exceptions — becomes [`JitOp::Escape`], which
//! stops the native run so the interpreter executes exactly that instruction.
//! `op_pcs` records the bytecode offset of every operation so the engine can
//! resume native execution at any instruction.
//!
//! # Value domain invariants
//!
//! * Every register and value-stack slot holds either an `i32`-range integer,
//!   one of the sentinel encodings from [`rix_jit`], or `RET_BOXED` (the value
//!   only exists in the interpreter frame). Arithmetic verifies its operands
//!   and results before committing them.
//! * `undefined`, `null`, `true` and `false` are representable, so stores
//!   of those literals never need a guard.
//! * No instruction makes a body ineligible: it either runs natively or
//!   escapes, so translation only fails for structural reasons.

use std::collections::HashMap;

use crate::vm::code_block::CodeBlock;
use crate::vm::opcode::Instruction;
use rix_jit::{JitFuncDesc, JitOp, RET_FALSE, RET_NULL, RET_TRUE, RET_UNDEF};

/// Translates `code` into JIT IR.
///
/// Returns `None` when the body cannot run natively; callers cache that
/// decision and keep interpreting the function.
pub(super) fn translate(code: &CodeBlock) -> Option<JitFuncDesc> {
    if !eligible(code) {
        return None;
    }
    let parameter_count = code.parameter_length;

    let bytecode_len = code.bytecode.bytes.len();
    let mut ops: Vec<JitOp> = Vec::new();
    // Byte offset of an instruction start -> index in `ops`.
    let mut starts: HashMap<u32, u32> = HashMap::new();
    // Byte offset of every operation's instruction start (parallel to `ops`).
    let mut op_pcs: Vec<u32> = Vec::new();
    // Operations whose jump target must be resolved after the walk.
    let mut jumps: Vec<(usize, u32)> = Vec::new();

    let mut pc = 0_usize;
    while pc < bytecode_len {
        starts.insert(pc as u32, ops.len() as u32);
        op_pcs.push(pc as u32);
        let (instruction, next) = code.bytecode.next_instruction(pc);
        if next <= pc {
            return None;
        }
        match instruction {
            Instruction::GetArgument { index, dst } => {
                let index = u32::from(index);
                if index >= parameter_count {
                    return None;
                }
                ops.push(JitOp::GetArgument {
                    index,
                    dst: u32::from(dst),
                });
            }
            Instruction::StoreZero { dst } => ops.push(JitOp::StoreConst {
                dst: u32::from(dst),
                value: 0,
            }),
            Instruction::StoreOne { dst } => ops.push(JitOp::StoreConst {
                dst: u32::from(dst),
                value: 1,
            }),
            Instruction::StoreInt8 { dst, value } => ops.push(JitOp::StoreConst {
                dst: u32::from(dst),
                value: i64::from(value),
            }),
            Instruction::StoreInt16 { dst, value } => ops.push(JitOp::StoreConst {
                dst: u32::from(dst),
                value: i64::from(value),
            }),
            Instruction::StoreInt32 { dst, value } => ops.push(JitOp::StoreConst {
                dst: u32::from(dst),
                value: i64::from(value),
            }),
            Instruction::StoreNull { dst } => ops.push(JitOp::StoreConst {
                dst: u32::from(dst),
                value: RET_NULL,
            }),
            Instruction::StoreTrue { dst } => ops.push(JitOp::StoreConst {
                dst: u32::from(dst),
                value: RET_TRUE,
            }),
            Instruction::StoreFalse { dst } => ops.push(JitOp::StoreConst {
                dst: u32::from(dst),
                value: RET_FALSE,
            }),
            Instruction::StoreUndefined { dst } => ops.push(JitOp::StoreConst {
                dst: u32::from(dst),
                value: RET_UNDEF,
            }),
            Instruction::Move { dst, src } => ops.push(JitOp::Move {
                dst: u32::from(dst),
                src: u32::from(src),
            }),
            Instruction::Add { dst, lhs, rhs } => ops.push(JitOp::AddInt {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::Sub { dst, lhs, rhs } => ops.push(JitOp::SubInt {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::Mul { dst, lhs, rhs } => ops.push(JitOp::MulInt {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::Mod { dst, lhs, rhs } => ops.push(JitOp::ModInt {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::BitAnd { dst, lhs, rhs } => ops.push(JitOp::BitAndInt {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::BitOr { dst, lhs, rhs } => ops.push(JitOp::BitOrInt {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::BitXor { dst, lhs, rhs } => ops.push(JitOp::BitXorInt {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::ShiftLeft { dst, lhs, rhs } => ops.push(JitOp::ShlInt {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::ShiftRight { dst, lhs, rhs } => ops.push(JitOp::ShrInt {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::UnsignedShiftRight { dst, lhs, rhs } => ops.push(JitOp::UShrInt {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::StrictEq { dst, lhs, rhs } => ops.push(JitOp::StrictEq {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::StrictNotEq { dst, lhs, rhs } => ops.push(JitOp::StrictNotEq {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::LessThan { dst, lhs, rhs } => ops.push(JitOp::LessThan {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::GreaterThan { dst, lhs, rhs } => ops.push(JitOp::GreaterThan {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::LessThanOrEq { dst, lhs, rhs } => ops.push(JitOp::LessThanOrEqual {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::GreaterThanOrEq { dst, lhs, rhs } => ops.push(JitOp::GreaterThanOrEqual {
                dst: u32::from(dst),
                lhs: u32::from(lhs),
                rhs: u32::from(rhs),
            }),
            Instruction::Neg { value } => ops.push(JitOp::NegReg {
                reg: u32::from(value),
            }),
            Instruction::BitNot { value } => ops.push(JitOp::BitNotReg {
                reg: u32::from(value),
            }),
            Instruction::LogicalNot { value } => ops.push(JitOp::LogicalNotReg {
                reg: u32::from(value),
            }),
            Instruction::Inc { dst, src } => ops.push(JitOp::IncDec {
                dst: u32::from(dst),
                src: u32::from(src),
                delta: 1,
            }),
            Instruction::Dec { dst, src } => ops.push(JitOp::IncDec {
                dst: u32::from(dst),
                src: u32::from(src),
                delta: -1,
            }),
            Instruction::PushFromRegister { src } => ops.push(JitOp::PushFromRegister {
                src: u32::from(src),
            }),
            Instruction::PopIntoRegister { dst } => ops.push(JitOp::PopIntoRegister {
                dst: u32::from(dst),
            }),
            Instruction::Pop => ops.push(JitOp::DropValue),
            Instruction::IncrementLoopIteration => ops.push(JitOp::LoopTick),
            Instruction::SetAccumulator { src } => ops.push(JitOp::SetAccumulator {
                src: u32::from(src),
            }),
            Instruction::Return => ops.push(JitOp::Return),
            Instruction::CheckReturn => {
                // Only meaningful in construct calls, and those never run
                // natively (the VM refuses frames with the `construct` flag),
                // so for a native frame this instruction does nothing. It
                // still needs an operation to keep every instruction boundary
                // an entry point.
                ops.push(JitOp::NoOp);
            }
            Instruction::Jump { address } => {
                ops.push(JitOp::Jump { target: 0 });
                jumps.push((ops.len() - 1, u32::from(address)));
            }
            Instruction::JumpIfTrue { address, value } => {
                ops.push(JitOp::JumpIfTrue {
                    target: 0,
                    value: u32::from(value),
                });
                jumps.push((ops.len() - 1, u32::from(address)));
            }
            Instruction::JumpIfFalse { address, value } => {
                ops.push(JitOp::JumpIfFalse {
                    target: 0,
                    value: u32::from(value),
                });
                jumps.push((ops.len() - 1, u32::from(address)));
            }
            Instruction::JumpIfNotLessThan { address, lhs, rhs } => {
                ops.push(JitOp::JumpIfGe {
                    target: 0,
                    lhs: u32::from(lhs),
                    rhs: u32::from(rhs),
                });
                jumps.push((ops.len() - 1, u32::from(address)));
            }
            Instruction::JumpIfNotLessThanOrEqual { address, lhs, rhs } => {
                ops.push(JitOp::JumpIfGt {
                    target: 0,
                    lhs: u32::from(lhs),
                    rhs: u32::from(rhs),
                });
                jumps.push((ops.len() - 1, u32::from(address)));
            }
            Instruction::JumpIfNotGreaterThan { address, lhs, rhs } => {
                ops.push(JitOp::JumpIfLe {
                    target: 0,
                    lhs: u32::from(lhs),
                    rhs: u32::from(rhs),
                });
                jumps.push((ops.len() - 1, u32::from(address)));
            }
            Instruction::JumpIfNotGreaterThanOrEqual { address, lhs, rhs } => {
                ops.push(JitOp::JumpIfLt {
                    target: 0,
                    lhs: u32::from(lhs),
                    rhs: u32::from(rhs),
                });
                jumps.push((ops.len() - 1, u32::from(address)));
            }
            Instruction::JumpIfNotEqual { address, lhs, rhs } => {
                ops.push(JitOp::JumpIfNe {
                    target: 0,
                    lhs: u32::from(lhs),
                    rhs: u32::from(rhs),
                });
                jumps.push((ops.len() - 1, u32::from(address)));
            }
            Instruction::JumpIfNullOrUndefined { address, value } => {
                ops.push(JitOp::JumpIfNullish {
                    target: 0,
                    value: u32::from(value),
                });
                jumps.push((ops.len() - 1, u32::from(address)));
            }
            Instruction::JumpIfNotUndefined { address, value } => {
                ops.push(JitOp::JumpIfNotUndefined {
                    target: 0,
                    value: u32::from(value),
                });
                jumps.push((ops.len() - 1, u32::from(address)));
            }
            _ => {
                // Everything else runs in the interpreter at this instruction:
                // the native body stops here and the engine may resume it at
                // the next instruction boundary.
                ops.push(JitOp::Escape { pc: pc as u32 });
            }
        }
        pc = next;
    }

    // Resolve jump targets; a target that is not an instruction start means
    // the body is not supported.
    for &(op_index, address) in &jumps {
        let Some(target) = starts.get(&address).copied() else {
            debug_reject(
                code,
                address as usize,
                "jump target is not an instruction start",
            );
            return None;
        };
        match &mut ops[op_index] {
            JitOp::Jump { target: slot }
            | JitOp::JumpIfTrue { target: slot, .. }
            | JitOp::JumpIfFalse { target: slot, .. }
            | JitOp::JumpIfGe { target: slot, .. }
            | JitOp::JumpIfGt { target: slot, .. }
            | JitOp::JumpIfLe { target: slot, .. }
            | JitOp::JumpIfLt { target: slot, .. }
            | JitOp::JumpIfNe { target: slot, .. }
            | JitOp::JumpIfNullish { target: slot, .. }
            | JitOp::JumpIfNotUndefined { target: slot, .. } => *slot = target,
            _ => return None,
        }
    }

    if op_pcs.len() != ops.len() {
        debug_reject(code, 0, "operation table is inconsistent");
        return None;
    }

    let total_ops = ops.len();
    let escape_count = ops
        .iter()
        .filter(|op| matches!(op, JitOp::Escape { .. }))
        .count();
    let has_loop = jumps.iter().any(|&(src_op, address)| {
        starts
            .get(&address)
            .copied()
            .is_some_and(|target_op| target_op <= src_op as u32)
    });

    let mut max_native_streak = 0;
    let mut current_streak = 0;
    for op in &ops {
        if matches!(op, JitOp::Escape { .. }) {
            current_streak = 0;
        } else {
            current_streak += 1;
            if current_streak > max_native_streak {
                max_native_streak = current_streak;
            }
        }
    }

    if !has_loop && max_native_streak < 4 {
        debug_reject(code, 0, "no loops and native streak too short");
        return None;
    }
    if total_ops > 4 && escape_count * 2 > total_ops && max_native_streak < 6 {
        debug_reject(code, 0, "high escape density without long native streak");
        return None;
    }

    // Escapes are observable effects: only a body without them may report
    // `RET_DEOPT` on a guard failure, because a deopt lets the interpreter
    // re-run the frame from its current instruction.
    let resumable = ops.iter().any(|op| matches!(op, JitOp::Escape { .. }));
    let desc = JitFuncDesc {
        name: code.name().to_std_string_escaped(),
        parameter_count,
        register_count: code.register_count,
        ops,
        op_pcs,
        resumable,
    };
    desc.validate().ok()?;
    debug_dump(&desc);
    Some(desc)
}

/// Function kind filters applied before the bytecode walk.
///
/// The walk itself rejects everything it does not understand, so these only
/// cover the cases where running the body natively would need interpreter
/// state that the JIT does not model at all.
fn eligible(code: &CodeBlock) -> bool {
    if !code.is_ordinary() || code.is_class_constructor() || code.is_derived_constructor() {
        return false;
    }
    if code.parameter_length as usize > super::MAX_NATIVE_PARAMETERS {
        return false;
    }
    if code.register_count as usize > 10_000_000 {
        return false;
    }
    true
}

/// Prints why a body was rejected when `RIX_JIT_DEBUG` is set.
fn debug_reject(code: &CodeBlock, pc: usize, reason: &str) {
    if std::env::var_os("RIX_JIT_DEBUG").is_some_and(|value| !value.is_empty() && value != "0") {
        eprintln!(
            "rix-jit: rejecting `{}`: {reason} at pc {pc}",
            code.name().to_std_string_escaped()
        );
    }
}

/// Prints the translated operation set when `RIX_JIT_DUMP` is set.
fn debug_dump(desc: &JitFuncDesc) {
    if std::env::var_os("RIX_JIT_DUMP").is_some_and(|value| !value.is_empty() && value != "0") {
        let escapes = desc
            .ops
            .iter()
            .filter(|op| matches!(op, JitOp::Escape { .. }))
            .count();
        eprintln!(
            "rix-jit: body `{}`: {} operations ({escapes} escaping), {} registers, {} parameters",
            desc.name,
            desc.ops.len(),
            desc.register_count,
            desc.parameter_count
        );
    }
}
