
pub const RET_DEOPT: i64 = i64::MIN;

pub const RET_UNDEF: i64 = i64::MIN + 1;

pub const RET_NULL: i64 = i64::MIN + 2;

pub const RET_TRUE: i64 = i64::MIN + 3;

pub const RET_FALSE: i64 = i64::MIN + 4;

pub const RET_BOXED: i64 = i64::MIN + 5;

pub const RET_ESCAPE: i64 = i64::MIN + 6;

#[inline]
pub fn is_sentinel(value: i64) -> bool {
    (RET_DEOPT..=RET_ESCAPE).contains(&value)
}

#[inline]
pub fn is_boxed(value: i64) -> bool {
    value == RET_BOXED
}

#[inline]
pub fn fits_i32(value: i64) -> bool {
    (value as i32 as i64) == value
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JitOp {
    NoOp,
    GetArgument {
        index: u32,
        dst: u32,
    },
    StoreConst {
        dst: u32,
        value: i64,
    },
    Move {
        dst: u32,
        src: u32,
    },
    AddInt {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    SubInt {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    MulInt {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    IncDec {
        dst: u32,
        src: u32,
        delta: i64,
    },
    ModInt {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    BitAndInt {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    BitOrInt {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    BitXorInt {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    ShlInt {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    ShrInt {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    UShrInt {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    NegReg {
        reg: u32,
    },
    BitNotReg {
        reg: u32,
    },
    LogicalNotReg {
        reg: u32,
    },
    PushFromRegister {
        src: u32,
    },
    PopIntoRegister {
        dst: u32,
    },
    DropValue,
    LoopTick,
    Jump {
        target: u32,
    },
    JumpIfTrue {
        target: u32,
        value: u32,
    },
    JumpIfFalse {
        target: u32,
        value: u32,
    },
    JumpIfGe {
        target: u32,
        lhs: u32,
        rhs: u32,
    },
    JumpIfGt {
        target: u32,
        lhs: u32,
        rhs: u32,
    },
    JumpIfLe {
        target: u32,
        lhs: u32,
        rhs: u32,
    },
    JumpIfLt {
        target: u32,
        lhs: u32,
        rhs: u32,
    },
    JumpIfNe {
        target: u32,
        lhs: u32,
        rhs: u32,
    },
    JumpIfNullish {
        target: u32,
        value: u32,
    },
    JumpIfNotUndefined {
        target: u32,
        value: u32,
    },
    StrictEq {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    StrictNotEq {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    LessThan {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    GreaterThan {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    LessThanOrEqual {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    GreaterThanOrEqual {
        dst: u32,
        lhs: u32,
        rhs: u32,
    },
    SetAccumulator {
        src: u32,
    },
    Return,
    Escape {
        pc: u32,
    },
}

#[derive(Debug, Clone)]
pub struct JitFuncDesc {
    pub name: String,
    pub parameter_count: u32,
    pub register_count: u32,
    pub ops: Vec<JitOp>,
    pub op_pcs: Vec<u32>,
    pub resumable: bool,
}

#[derive(Debug, Clone, Default)]
pub struct JitProgram {
    pub entry: usize,
    pub state_len: u32,
    pub offsets: StateOffsets,
    pub op_pcs: Vec<u32>,
    pub op_heights: Vec<u32>,
    pub op_entry: Vec<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JitEntry {
    pub op_index: u32,
    pub height: u32,
}

impl JitProgram {
    #[must_use]
    pub fn op_index_at_pc(&self, pc: u32) -> Option<u32> {
        self.op_pcs
            .binary_search(&pc)
            .ok()
            .map(|index| index as u32)
    }

    #[must_use]
    pub fn entry_at_pc(&self, pc: u32) -> Option<JitEntry> {
        let index = self.op_pcs.binary_search(&pc).ok()?;
        if !self.op_entry.get(index).copied().unwrap_or(false) {
            return None;
        }
        Some(JitEntry {
            op_index: index as u32,
            height: self.op_heights.get(index).copied().unwrap_or(0),
        })
    }

    #[must_use]
    pub fn height_at_op(&self, op_index: u32) -> Option<u32> {
        self.op_heights.get(op_index as usize).copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StateOffsets {
    pub escape_pc: u32,
    pub escape_height: u32,
    pub escape_op: u32,
    pub iterations: u32,
    pub registers: u32,
    pub value_stack: u32,
    pub accumulator: u32,
    pub len: u32,
}

impl JitFuncDesc {
    #[must_use]
    pub fn state_offsets(&self, max_height: u32) -> StateOffsets {
        let registers = 4;
        let value_stack = registers + self.register_count;
        let accumulator = value_stack + max_height;
        StateOffsets {
            escape_pc: 0,
            escape_height: 1,
            escape_op: 2,
            iterations: 3,
            registers,
            value_stack,
            accumulator,
            len: accumulator + 1,
        }
    }
}

impl JitFuncDesc {
    pub fn validate(&self) -> Result<(), String> {
        if self.ops.is_empty() {
            return Err("empty body".to_owned());
        }
        if !self.resumable && self.ops.iter().any(|op| matches!(op, JitOp::Escape { .. })) {
            return Err("body contains escapes but is not resumable".to_owned());
        }
        let n = self.ops.len() as u32;
        let check_reg = |reg: u32, what: &str| -> Result<(), String> {
            if reg < self.register_count {
                Ok(())
            } else {
                Err(format!(
                    "{what} register {reg} out of range (register_count {})",
                    self.register_count
                ))
            }
        };
        let check_target = |target: u32| -> Result<(), String> {
            if target < n {
                Ok(())
            } else {
                Err(format!("jump target {target} out of range (len {n})"))
            }
        };
        for (i, op) in self.ops.iter().enumerate() {
            match *op {
                JitOp::NoOp => {}
                JitOp::GetArgument { index, dst } => {
                    check_reg(dst, "GetArgument dst")?;
                    if index >= self.parameter_count {
                        return Err(format!("GetArgument index {index} >= parameter_count"));
                    }
                }
                JitOp::StoreConst { dst, value } => {
                    check_reg(dst, "StoreConst dst")?;
                    if !fits_i32(value)
                        && !matches!(value, RET_UNDEF | RET_NULL | RET_TRUE | RET_FALSE)
                    {
                        return Err(format!("StoreConst value {value} outside value domain"));
                    }
                }
                JitOp::Move { dst, src } => {
                    check_reg(dst, "Move dst")?;
                    check_reg(src, "Move src")?;
                }
                JitOp::AddInt { dst, lhs, rhs }
                | JitOp::SubInt { dst, lhs, rhs }
                | JitOp::MulInt { dst, lhs, rhs }
                | JitOp::ModInt { dst, lhs, rhs }
                | JitOp::BitAndInt { dst, lhs, rhs }
                | JitOp::BitOrInt { dst, lhs, rhs }
                | JitOp::BitXorInt { dst, lhs, rhs }
                | JitOp::ShlInt { dst, lhs, rhs }
                | JitOp::ShrInt { dst, lhs, rhs }
                | JitOp::UShrInt { dst, lhs, rhs } => {
                    check_reg(dst, "arithmetic dst")?;
                    check_reg(lhs, "arithmetic lhs")?;
                    check_reg(rhs, "arithmetic rhs")?;
                }
                JitOp::NegReg { reg } | JitOp::BitNotReg { reg } | JitOp::LogicalNotReg { reg } => {
                    check_reg(reg, "unary register")?;
                }
                JitOp::IncDec { dst, src, delta } => {
                    check_reg(dst, "IncDec dst")?;
                    check_reg(src, "IncDec src")?;
                    if delta != 1 && delta != -1 {
                        return Err(format!("IncDec delta {delta} must be +1 or -1"));
                    }
                }
                JitOp::PushFromRegister { src } => check_reg(src, "PushFromRegister src")?,
                JitOp::PopIntoRegister { dst } => check_reg(dst, "PopIntoRegister dst")?,
                JitOp::DropValue | JitOp::LoopTick | JitOp::Return | JitOp::Escape { .. } => {}
                JitOp::Jump { target } => check_target(target)?,
                JitOp::JumpIfTrue { target, value }
                | JitOp::JumpIfFalse { target, value }
                | JitOp::JumpIfNullish { target, value }
                | JitOp::JumpIfNotUndefined { target, value } => {
                    check_target(target)?;
                    check_reg(value, "jump condition")?;
                }
                JitOp::JumpIfGe { target, lhs, rhs }
                | JitOp::JumpIfGt { target, lhs, rhs }
                | JitOp::JumpIfLe { target, lhs, rhs }
                | JitOp::JumpIfLt { target, lhs, rhs }
                | JitOp::JumpIfNe { target, lhs, rhs } => {
                    check_target(target)?;
                    check_reg(lhs, "jump lhs")?;
                    check_reg(rhs, "jump rhs")?;
                }
                JitOp::StrictEq { dst, lhs, rhs }
                | JitOp::StrictNotEq { dst, lhs, rhs }
                | JitOp::LessThan { dst, lhs, rhs }
                | JitOp::GreaterThan { dst, lhs, rhs }
                | JitOp::LessThanOrEqual { dst, lhs, rhs }
                | JitOp::GreaterThanOrEqual { dst, lhs, rhs } => {
                    check_reg(dst, "comparison dst")?;
                    check_reg(lhs, "comparison lhs")?;
                    check_reg(rhs, "comparison rhs")?;
                }
                JitOp::SetAccumulator { src } => check_reg(src, "SetAccumulator src")?,
            }
            let _ = i;
        }
        if !self.op_pcs.is_empty() {
            if self.op_pcs.len() != self.ops.len() {
                return Err(format!(
                    "op_pcs length {} does not match ops length {}",
                    self.op_pcs.len(),
                    self.ops.len()
                ));
            }
            if self.op_pcs.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err("op_pcs must be strictly ascending".to_owned());
            }
        }
        Ok(())
    }
}
