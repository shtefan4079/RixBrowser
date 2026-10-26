
#![deny(unsafe_op_in_unsafe_fn)]

mod alloc;
mod codegen;
mod ir;

pub use ir::{
    fits_i32, is_boxed, is_sentinel, JitEntry, JitFuncDesc, JitOp, JitProgram, StateOffsets,
    RET_BOXED, RET_DEOPT, RET_ESCAPE, RET_FALSE, RET_NULL, RET_TRUE, RET_UNDEF,
};

use std::sync::OnceLock;

pub const JIT_DISABLED: usize = usize::MAX;

pub type NativeFn = unsafe extern "C" fn(
    state: *mut i64,
    args: *const i64,
    argc: u64,
    loop_limit: u64,
    entry: u64,
) -> i64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JitError {
    UnsupportedHost(String),
    InvalidBody(String),
    Codegen(String),
}

impl std::fmt::Display for JitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedHost(msg) => write!(f, "unsupported host: {msg}"),
            Self::InvalidBody(msg) => write!(f, "invalid JIT body: {msg}"),
            Self::Codegen(msg) => write!(f, "JIT code generation failed: {msg}"),
        }
    }
}

impl std::error::Error for JitError {}

pub struct CompiledFunction {
    ptr: *const u8,
    len: usize,
    offsets: StateOffsets,
    op_heights: Vec<Option<u32>>,
}

// SAFETY: compiled code is immutable once finalized and contains no shared

unsafe impl Send for CompiledFunction {}
// SAFETY: see above.
unsafe impl Sync for CompiledFunction {}

impl CompiledFunction {
    #[must_use]
    pub fn as_ptr(&self) -> usize {
        self.ptr as usize
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[must_use]
    pub fn state_len(&self) -> u32 {
        self.offsets.len
    }

    #[must_use]
    pub fn state_offsets(&self) -> StateOffsets {
        self.offsets
    }

    #[must_use]
    pub fn op_heights(&self) -> &[Option<u32>] {
        &self.op_heights
    }

    #[must_use]
    pub fn call_with_state(
        &self,
        state: &mut [i64],
        args: &[i64],
        loop_limit: i64,
        entry: u64,
    ) -> i64 {
        assert!(
            state.len() as u32 >= self.offsets.len,
            "state array too small"
        );
        // SAFETY: `self.ptr` was produced by `codegen::compile` with exactly
        
        let f: NativeFn = unsafe { std::mem::transmute(self.ptr) };
        unsafe {
            f(
                state.as_mut_ptr(),
                args.as_ptr(),
                args.len() as u64,
                loop_limit as u64,
                entry,
            )
        }
    }

    #[must_use]
    pub fn call(&self, args: &[i64], loop_limit: i64) -> i64 {
        let mut state = vec![RET_UNDEF; self.offsets.len as usize];
        self.call_with_state(&mut state, args, loop_limit, 0)
    }
}

fn isa() -> Result<&'static cranelift_codegen::isa::OwnedTargetIsa, JitError> {
    static ISA: OnceLock<Result<cranelift_codegen::isa::OwnedTargetIsa, String>> = OnceLock::new();
    match ISA.get_or_init(codegen::create_isa) {
        Ok(isa) => Ok(isa),
        Err(err) => Err(JitError::UnsupportedHost(err.clone())),
    }
}

pub fn compile(desc: &JitFuncDesc) -> Result<CompiledFunction, JitError> {
    let isa = isa()?;
    let compiled = codegen::compile(desc, &**isa).map_err(classify)?;
    let max_height = compiled
        .op_heights
        .iter()
        .flatten()
        .copied()
        .max()
        .unwrap_or(0);
    Ok(CompiledFunction {
        ptr: compiled.ptr,
        len: compiled.len,
        offsets: desc.state_offsets(max_height),
        op_heights: compiled.op_heights,
    })
}

pub fn compile_program(desc: &JitFuncDesc) -> Result<JitProgram, JitError> {
    let isa = isa()?;
    let compiled = codegen::compile(desc, &**isa).map_err(classify)?;
    alloc::register_code(compiled.ptr);
    let max_height = compiled
        .op_heights
        .iter()
        .flatten()
        .copied()
        .max()
        .unwrap_or(0);
    let mut is_jump_target = vec![false; desc.ops.len()];
    for op in &desc.ops {
        let target = match *op {
            JitOp::Jump { target }
            | JitOp::JumpIfTrue { target, .. }
            | JitOp::JumpIfFalse { target, .. }
            | JitOp::JumpIfGe { target, .. }
            | JitOp::JumpIfGt { target, .. }
            | JitOp::JumpIfLe { target, .. }
            | JitOp::JumpIfLt { target, .. }
            | JitOp::JumpIfNe { target, .. }
            | JitOp::JumpIfNullish { target, .. }
            | JitOp::JumpIfNotUndefined { target, .. } => Some(target as usize),
            _ => None,
        };
        if let Some(t) = target {
            if t < is_jump_target.len() {
                is_jump_target[t] = true;
            }
        }
    }

    let mut forward_streak = vec![0u32; desc.ops.len()];
    let mut current_streak = 0u32;
    for i in (0..desc.ops.len()).rev() {
        if matches!(desc.ops[i], JitOp::Escape { .. }) {
            current_streak = 0;
        } else {
            current_streak += 1;
        }
        forward_streak[i] = current_streak;
    }

    let op_entry: Vec<bool> = compiled
        .op_heights
        .iter()
        .enumerate()
        .map(|(index, height)| {
            if height.is_none() || matches!(desc.ops.get(index), Some(JitOp::Escape { .. })) {
                return false;
            }
            if index == 0 {
                forward_streak[0] >= 2 || is_jump_target[0]
            } else if is_jump_target[index] {
                forward_streak[index] >= 1
            } else {
                forward_streak[index] >= 3
            }
        })
        .collect();
    Ok(JitProgram {
        entry: compiled.ptr as usize,
        state_len: compiled.state_len,
        offsets: desc.state_offsets(max_height),
        op_pcs: desc.op_pcs.clone(),
        op_heights: compiled
            .op_heights
            .into_iter()
            .map(|height| height.unwrap_or(u32::MAX))
            .collect(),
        op_entry,
    })
}

pub fn compile_to_ptr(desc: &JitFuncDesc) -> Result<usize, JitError> {
    compile_program(desc).map(|program| program.entry)
}

fn classify(err: String) -> JitError {
    if err.contains("isa flag") || err.contains("unsupported") {
        JitError::UnsupportedHost(err)
    } else if err.contains("out of range")
        || err.contains("value domain")
        || err.contains("underflow")
        || err.contains("inconsistent")
        || err.contains("past the end")
        || err.contains("empty body")
        || err.contains("must be")
        || err.contains("too many")
    {
        JitError::InvalidBody(err)
    } else {
        JitError::Codegen(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strict_equality() {
        let desc = JitFuncDesc {
            name: "test_strict_equality".to_string(),
            parameter_count: 2,
            register_count: 3,
            ops: vec![
                JitOp::GetArgument { index: 0, dst: 0 },
                JitOp::GetArgument { index: 1, dst: 1 },
                JitOp::StrictEq { dst: 2, lhs: 0, rhs: 1 },
                JitOp::SetAccumulator { src: 2 },
                JitOp::Return,
            ],
            op_pcs: vec![0, 1, 2, 3, 4],
            resumable: false,
        };
        let f = compile(&desc).expect("compile succeeds");
        assert_eq!(f.call(&[42, 42], 1000), RET_TRUE);
        assert_eq!(f.call(&[42, 43], 1000), RET_FALSE);
        assert_eq!(f.call(&[RET_TRUE, RET_TRUE], 1000), RET_TRUE);
        assert_eq!(f.call(&[RET_TRUE, RET_FALSE], 1000), RET_FALSE);
    }

    #[test]
    fn test_strict_inequality() {
        let desc = JitFuncDesc {
            name: "test_strict_inequality".to_string(),
            parameter_count: 2,
            register_count: 3,
            ops: vec![
                JitOp::GetArgument { index: 0, dst: 0 },
                JitOp::GetArgument { index: 1, dst: 1 },
                JitOp::StrictNotEq { dst: 2, lhs: 0, rhs: 1 },
                JitOp::SetAccumulator { src: 2 },
                JitOp::Return,
            ],
            op_pcs: vec![0, 1, 2, 3, 4],
            resumable: false,
        };
        let f = compile(&desc).expect("compile succeeds");
        assert_eq!(f.call(&[42, 42], 1000), RET_FALSE);
        assert_eq!(f.call(&[42, 43], 1000), RET_TRUE);
    }

    #[test]
    fn test_numeric_comparisons() {
        let desc_lt = JitFuncDesc {
            name: "test_lt".to_string(),
            parameter_count: 2,
            register_count: 3,
            ops: vec![
                JitOp::GetArgument { index: 0, dst: 0 },
                JitOp::GetArgument { index: 1, dst: 1 },
                JitOp::LessThan { dst: 2, lhs: 0, rhs: 1 },
                JitOp::SetAccumulator { src: 2 },
                JitOp::Return,
            ],
            op_pcs: vec![0, 1, 2, 3, 4],
            resumable: false,
        };
        let f_lt = compile(&desc_lt).expect("compile lt succeeds");
        assert_eq!(f_lt.call(&[10, 20], 1000), RET_TRUE);
        assert_eq!(f_lt.call(&[20, 10], 1000), RET_FALSE);
        assert_eq!(f_lt.call(&[10, 10], 1000), RET_FALSE);

        let desc_le = JitFuncDesc {
            name: "test_le".to_string(),
            parameter_count: 2,
            register_count: 3,
            ops: vec![
                JitOp::GetArgument { index: 0, dst: 0 },
                JitOp::GetArgument { index: 1, dst: 1 },
                JitOp::LessThanOrEqual { dst: 2, lhs: 0, rhs: 1 },
                JitOp::SetAccumulator { src: 2 },
                JitOp::Return,
            ],
            op_pcs: vec![0, 1, 2, 3, 4],
            resumable: false,
        };
        let f_le = compile(&desc_le).expect("compile le succeeds");
        assert_eq!(f_le.call(&[10, 20], 1000), RET_TRUE);
        assert_eq!(f_le.call(&[10, 10], 1000), RET_TRUE);
        assert_eq!(f_le.call(&[20, 10], 1000), RET_FALSE);

        let desc_gt = JitFuncDesc {
            name: "test_gt".to_string(),
            parameter_count: 2,
            register_count: 3,
            ops: vec![
                JitOp::GetArgument { index: 0, dst: 0 },
                JitOp::GetArgument { index: 1, dst: 1 },
                JitOp::GreaterThan { dst: 2, lhs: 0, rhs: 1 },
                JitOp::SetAccumulator { src: 2 },
                JitOp::Return,
            ],
            op_pcs: vec![0, 1, 2, 3, 4],
            resumable: false,
        };
        let f_gt = compile(&desc_gt).expect("compile gt succeeds");
        assert_eq!(f_gt.call(&[20, 10], 1000), RET_TRUE);
        assert_eq!(f_gt.call(&[10, 20], 1000), RET_FALSE);
        assert_eq!(f_gt.call(&[10, 10], 1000), RET_FALSE);

        let desc_ge = JitFuncDesc {
            name: "test_ge".to_string(),
            parameter_count: 2,
            register_count: 3,
            ops: vec![
                JitOp::GetArgument { index: 0, dst: 0 },
                JitOp::GetArgument { index: 1, dst: 1 },
                JitOp::GreaterThanOrEqual { dst: 2, lhs: 0, rhs: 1 },
                JitOp::SetAccumulator { src: 2 },
                JitOp::Return,
            ],
            op_pcs: vec![0, 1, 2, 3, 4],
            resumable: false,
        };
        let f_ge = compile(&desc_ge).expect("compile ge succeeds");
        assert_eq!(f_ge.call(&[20, 10], 1000), RET_TRUE);
        assert_eq!(f_ge.call(&[10, 10], 1000), RET_TRUE);
        assert_eq!(f_ge.call(&[10, 20], 1000), RET_FALSE);
    }
}
