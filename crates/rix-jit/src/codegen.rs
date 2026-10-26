
use std::collections::VecDeque;

use cranelift_codegen::control::ControlPlane;
use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{
    types, AbiParam, Block, BlockCall, InstBuilder, JumpTableData, MemFlagsData, Value,
};
use cranelift_codegen::isa::{TargetFrontendConfig, TargetIsa};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};

use crate::ir::{
    JitFuncDesc, JitOp, StateOffsets, RET_BOXED, RET_DEOPT, RET_ESCAPE, RET_FALSE, RET_NULL,
    RET_TRUE, RET_UNDEF,
};

const MAX_PARAMETERS: u32 = 1024;

const MAX_REGISTERS: u32 = 1 << 20;

const MAX_STATE_SLOTS: u32 = 1 << 16;

pub(crate) fn create_isa() -> Result<cranelift_codegen::isa::OwnedTargetIsa, String> {
    let mut flags = settings::builder();
    flags
        .set("is_pic", "true")
        .map_err(|e| format!("isa flag: {e}"))?;
    flags
        .set("use_colocated_libcalls", "false")
        .map_err(|e| format!("isa flag: {e}"))?;
    flags
        .set("opt_level", "speed")
        .map_err(|e| format!("isa flag: {e}"))?;
    let builder = cranelift_native::builder().map_err(|e| e.to_string())?;
    builder
        .finish(settings::Flags::new(flags))
        .map_err(|e| e.to_string())
}

pub(crate) struct Compiled {
    pub ptr: *const u8,
    pub len: usize,
    pub state_len: u32,
    pub op_heights: Vec<Option<u32>>,
}

pub(crate) fn compile(desc: &JitFuncDesc, isa: &dyn TargetIsa) -> Result<Compiled, String> {
    desc.validate()?;
    if desc.parameter_count > MAX_PARAMETERS {
        return Err(format!("too many parameters ({})", desc.parameter_count));
    }
    if desc.register_count > MAX_REGISTERS {
        return Err(format!("too many registers ({})", desc.register_count));
    }

    let heights = compute_heights(desc)?;
    let max_height = heights.iter().flatten().copied().max().unwrap_or(0);
    let offsets = desc.state_offsets(max_height);
    if offsets.len > MAX_STATE_SLOTS {
        return Err(format!(
            "invocation state needs {} slots, over the limit of {MAX_STATE_SLOTS}",
            offsets.len
        ));
    }

    let mut ctx = Context::new();
    ctx.func
        .signature
        .params
        .push(AbiParam::new(isa.pointer_type()));
    ctx.func
        .signature
        .params
        .push(AbiParam::new(isa.pointer_type()));
    ctx.func.signature.params.push(AbiParam::new(types::I64));
    ctx.func.signature.params.push(AbiParam::new(types::I64));
    ctx.func.signature.params.push(AbiParam::new(types::I64));
    ctx.func.signature.returns.push(AbiParam::new(types::I64));

    let mut fbctx = FunctionBuilderContext::new();
    let builder = FunctionBuilder::new(&mut ctx.func, &mut fbctx);
    let emitter = Emitter::new(builder, desc, heights.clone(), offsets, max_height)?;
    emitter.run(isa.frontend_config());

    let mut ctrl = ControlPlane::default();
    if let Some(filter) = std::env::var_os("RIX_JIT_CLIF") {
        let filter = filter.to_string_lossy();
        if filter.is_empty() || desc.name.contains(filter.as_ref()) {
            eprintln!("rix-jit: CLIF of `{}`:\n{}", desc.name, ctx.func.display());
        }
    }
    ctx.compile(isa, &mut ctrl).map_err(|e| format!("{e:?}"))?;
    let code = ctx.compiled_code().ok_or("no compiled code")?;
    if !code.buffer.relocs().is_empty() {
        return Err(format!(
            "generated code has {} unexpected relocations",
            code.buffer.relocs().len()
        ));
    }
    let bytes = code.code_buffer();
    let ptr = crate::alloc::alloc_code(bytes).ok_or("executable allocation failed")?;
    Ok(Compiled {
        ptr,
        len: bytes.len(),
        state_len: offsets.len,
        op_heights: heights,
    })
}

fn compute_heights(desc: &JitFuncDesc) -> Result<Vec<Option<u32>>, String> {
    let n = desc.ops.len();
    let mut heights: Vec<Option<u32>> = vec![None; n];
    let mut queue: VecDeque<u32> = VecDeque::new();
    heights[0] = Some(0);
    queue.push_back(0);

    while let Some(index) = queue.pop_front() {
        let height = heights[index as usize].unwrap_or(0);
        let op = desc.ops[index as usize];
        let out = match op {
            JitOp::PushFromRegister { .. } => height + 1,
            JitOp::PopIntoRegister { .. } | JitOp::DropValue => height
                .checked_sub(1)
                .ok_or_else(|| format!("value stack underflow at op {index}"))?,
            _ => height,
        };
        let mut successors: [Option<u32>; 2] = [None, None];
        match op {
            JitOp::Return => {}
            JitOp::Jump { target } => successors[0] = Some(target),
            JitOp::JumpIfTrue { target, .. }
            | JitOp::JumpIfFalse { target, .. }
            | JitOp::JumpIfGe { target, .. }
            | JitOp::JumpIfGt { target, .. }
            | JitOp::JumpIfLe { target, .. }
            | JitOp::JumpIfLt { target, .. }
            | JitOp::JumpIfNe { target, .. }
            | JitOp::JumpIfNullish { target, .. }
            | JitOp::JumpIfNotUndefined { target, .. } => {
                successors[0] = Some(target);
                successors[1] = Some(index + 1);
            }
            _ => successors[0] = Some(index + 1),
        }
        for succ in successors.into_iter().flatten() {
            let s = succ as usize;
            if s >= n {
                return Err(format!("flow past the end of the body at op {index}"));
            }
            match heights[s] {
                Some(existing) if existing != out => {
                    return Err(format!("inconsistent value stack height at op {s}"));
                }
                Some(_) => {}
                None => {
                    heights[s] = Some(out);
                    queue.push_back(succ);
                }
            }
        }
    }
    Ok(heights)
}

struct Emitter<'a, 'b> {
    b: FunctionBuilder<'b>,
    desc: &'a JitFuncDesc,
    heights: Vec<Option<u32>>,
    op_blocks: Vec<Block>,
    deopt: Block,
    escape: Block,
    ret_undef: Block,
    ret_null: Block,
    ret_true: Block,
    ret_false: Block,
    regs: Vec<Variable>,
    vs: Vec<Variable>,
    acc: Variable,
    counter: Variable,
    state_ptr: Value,
    args_ptr: Value,
    loop_limit: Value,
    offsets: StateOffsets,
    op_escape: Vec<Option<(Block, u32, u32)>>,
    resumable: bool,
    current_pc: u32,
    current_op: u32,
    current_height: u32,
}

impl<'a, 'b> Emitter<'a, 'b> {
    fn new(
        mut b: FunctionBuilder<'b>,
        desc: &'a JitFuncDesc,
        heights: Vec<Option<u32>>,
        offsets: StateOffsets,
        max_height: u32,
    ) -> Result<Self, String> {
        let n = desc.ops.len();
        let max_height = max_height as usize;

        let entry = b.create_block();
        b.append_block_params_for_function_params(entry);
        b.switch_to_block(entry);
        let params = b.block_params(entry).to_vec();
        if params.len() < 5 {
            return Err("entry block is missing native parameters".to_owned());
        }

        let regs: Vec<Variable> = (0..desc.register_count)
            .map(|_| b.declare_var(types::I64))
            .collect();
        let vs: Vec<Variable> = (0..max_height).map(|_| b.declare_var(types::I64)).collect();
        let acc = b.declare_var(types::I64);
        let counter = b.declare_var(types::I64);

        let state_ptr = params[0];
        let base = i64::from(offsets.registers) * 8;
        for (index, &reg) in regs.iter().enumerate() {
            let offset = i32::try_from(base + index as i64 * 8).unwrap_or(i32::MAX);
            let value = b
                .ins()
                .load(types::I64, MemFlagsData::trusted(), state_ptr, offset);
            b.def_var(reg, value);
        }
        let base = i64::from(offsets.value_stack) * 8;
        for (index, &slot) in vs.iter().enumerate() {
            let offset = i32::try_from(base + index as i64 * 8).unwrap_or(i32::MAX);
            let value = b
                .ins()
                .load(types::I64, MemFlagsData::trusted(), state_ptr, offset);
            b.def_var(slot, value);
        }
        let acc_offset = i32::try_from(i64::from(offsets.accumulator) * 8).unwrap_or(i32::MAX);
        let acc_value = b
            .ins()
            .load(types::I64, MemFlagsData::trusted(), state_ptr, acc_offset);
        b.def_var(acc, acc_value);
        let zero = b.ins().iconst(types::I64, 0);
        b.def_var(counter, zero);

        let op_blocks: Vec<Block> = (0..n).map(|_| b.create_block()).collect();
        let op_escape: Vec<Option<(Block, u32, u32)>> = (0..n).map(|_| None).collect();
        let deopt = b.create_block();
        b.set_cold_block(deopt);
        let escape = b.create_block();
        b.set_cold_block(escape);
        b.append_block_param(escape, types::I64);
        b.append_block_param(escape, types::I64);
        b.append_block_param(escape, types::I64);
        let ret_undef = b.create_block();
        let ret_null = b.create_block();
        let ret_true = b.create_block();
        let ret_false = b.create_block();

        if op_blocks.is_empty() {
            return Err("empty body".to_owned());
        }
        let dispatch = b.create_block();
        if desc.parameter_count > 0 {
            let needed = b.ins().iconst(types::I64, i64::from(desc.parameter_count));
            let ok = b
                .ins()
                .icmp(IntCC::SignedGreaterThanOrEqual, params[2], needed);
            b.ins().brif(ok, dispatch, &[], deopt, &[]);
        } else {
            b.ins().jump(dispatch, &[]);
        }

        b.switch_to_block(dispatch);
        let entry = params[4];
        let narrowed = b.ins().ireduce(types::I32, entry);
        let widened = b.ins().uextend(types::I64, narrowed);
        let exact = b.ins().icmp(IntCC::Equal, widened, entry);
        let limit = b.ins().iconst(types::I32, n as i64);
        let in_range = b.ins().icmp(IntCC::UnsignedLessThan, narrowed, limit);
        let valid = b.ins().band(exact, in_range);
        let table_block = b.create_block();
        b.ins().brif(valid, table_block, &[], deopt, &[]);
        b.seal_block(table_block);
        b.switch_to_block(table_block);
        let table_entries: Vec<BlockCall> = op_blocks
            .iter()
            .map(|&block| b.func.dfg.block_call(block, &[]))
            .collect();
        let default_call = b.func.dfg.block_call(deopt, &[]);
        let table = b.create_jump_table(JumpTableData::new(default_call, &table_entries));
        b.ins().br_table(narrowed, table);

        Ok(Self {
            b,
            desc,
            heights,
            op_blocks,
            deopt,
            escape,
            ret_undef,
            ret_null,
            ret_true,
            ret_false,
            regs,
            vs,
            acc,
            counter,
            state_ptr,
            args_ptr: params[1],
            loop_limit: params[3],
            offsets,
            op_escape,
            resumable: desc.resumable,
            current_pc: 0,
            current_op: 0,
            current_height: 0,
        })
    }

    fn run(mut self, config: TargetFrontendConfig) {
        let n = self.desc.ops.len();
        for index in 0..n {
            if self.heights[index].is_none() {
                self.b.switch_to_block(self.op_blocks[index]);
                self.b.ins().jump(self.deopt, &[]);
                continue;
            }
            self.current_pc = self.pc_of(index);
            self.current_op = index as u32;
            self.current_height = self.heights[index].unwrap_or(0);
            self.b.switch_to_block(self.op_blocks[index]);
            self.emit_op(index);
        }

        self.b.switch_to_block(self.deopt);
        let deopt = self.b.ins().iconst(types::I64, RET_DEOPT);
        self.b.ins().return_(&[deopt]);

        if self.resumable {
            self.emit_escape_landings();
            self.b.switch_to_block(self.escape);
            let escape_params = self.b.block_params(self.escape).to_vec();
            let (escape_pc, escape_height, escape_op) =
                (escape_params[0], escape_params[1], escape_params[2]);
            let pc_offset = i64::from(self.offsets.escape_pc) * 8;
            let height_offset = i64::from(self.offsets.escape_height) * 8;
            let op_offset = i64::from(self.offsets.escape_op) * 8;
            self.b.ins().store(
                MemFlagsData::trusted(),
                escape_pc,
                self.state_ptr,
                i32::try_from(pc_offset).unwrap_or(i32::MAX),
            );
            self.b.ins().store(
                MemFlagsData::trusted(),
                escape_height,
                self.state_ptr,
                i32::try_from(height_offset).unwrap_or(i32::MAX),
            );
            self.b.ins().store(
                MemFlagsData::trusted(),
                escape_op,
                self.state_ptr,
                i32::try_from(op_offset).unwrap_or(i32::MAX),
            );
            let iterations = self.b.use_var(self.counter);
            let iterations_offset = i64::from(self.offsets.iterations) * 8;
            self.b.ins().store(
                MemFlagsData::trusted(),
                iterations,
                self.state_ptr,
                i32::try_from(iterations_offset).unwrap_or(i32::MAX),
            );
            let base = i64::from(self.offsets.registers) * 8;
            for (index, &reg) in self.regs.iter().enumerate() {
                let value = self.b.use_var(reg);
                let offset = i32::try_from(base + index as i64 * 8).unwrap_or(i32::MAX);
                self.b
                    .ins()
                    .store(MemFlagsData::trusted(), value, self.state_ptr, offset);
            }
            let base = i64::from(self.offsets.value_stack) * 8;
            for (index, &slot) in self.vs.iter().enumerate() {
                let value = self.b.use_var(slot);
                let offset = i32::try_from(base + index as i64 * 8).unwrap_or(i32::MAX);
                self.b
                    .ins()
                    .store(MemFlagsData::trusted(), value, self.state_ptr, offset);
            }
            let acc_offset =
                i32::try_from(i64::from(self.offsets.accumulator) * 8).unwrap_or(i32::MAX);
            let acc = self.b.use_var(self.acc);
            self.b
                .ins()
                .store(MemFlagsData::trusted(), acc, self.state_ptr, acc_offset);
            let escape_code = self.b.ins().iconst(types::I64, RET_ESCAPE);
            self.b.ins().return_(&[escape_code]);
        }

        for (block, value) in [
            (self.ret_undef, RET_UNDEF),
            (self.ret_null, RET_NULL),
            (self.ret_true, RET_TRUE),
            (self.ret_false, RET_FALSE),
        ] {
            self.b.switch_to_block(block);
            let v = self.b.ins().iconst(types::I64, value);
            self.b.ins().return_(&[v]);
        }

        self.b.seal_all_blocks();
        self.b.finalize(config);
    }

    fn pc_of(&self, index: usize) -> u32 {
        self.desc.op_pcs.get(index).copied().unwrap_or(0)
    }

    fn escape_target(&mut self, pc: u32, height: u32) -> Block {
        let index = self.current_op as usize;
        if let Some(Some((block, existing_pc, existing_height))) =
            self.op_escape.get(index).copied()
        {
            debug_assert_eq!(existing_pc, pc, "one escape position per operation");
            debug_assert_eq!(existing_height, height, "one escape height per operation");
            return block;
        }
        let block = self.b.create_block();
        self.b.set_cold_block(block);
        if let Some(slot) = self.op_escape.get_mut(index) {
            *slot = Some((block, pc, height));
        }
        block
    }

    fn escape_at(&mut self, pc: u32, height: u32) {
        debug_assert!(
            self.resumable,
            "a body without escapes must not take the escape path"
        );
        let target = self.escape_target(pc, height);
        self.b.ins().jump(target, &[]);
    }

    fn emit_escape_landings(&mut self) {
        for index in 0..self.op_escape.len() {
            let Some((block, pc, height)) = self.op_escape[index] else {
                continue;
            };
            self.b.switch_to_block(block);
            let pc_value = self.b.ins().iconst(types::I64, i64::from(pc));
            let height_value = self.b.ins().iconst(types::I64, i64::from(height));
            let op_value = self.b.ins().iconst(types::I64, i64::from(index as u32));
            self.b.ins().jump(
                self.escape,
                &[pc_value.into(), height_value.into(), op_value.into()],
            );
            self.b.seal_block(block);
        }
    }

    fn load_int(&mut self, reg: u32) -> Value {
        let value = self.b.use_var(self.regs[reg as usize]);
        self.check_i32(value);
        value
    }

    fn check_i32(&mut self, value: Value) {
        let narrowed = self.b.ins().ireduce(types::I32, value);
        let widened = self.b.ins().sextend(types::I64, narrowed);
        let ok = self.b.ins().icmp(IntCC::Equal, widened, value);
        self.guard(ok);
    }

    fn check_not_boxed(&mut self, value: Value) {
        let boxed = self.b.ins().iconst(types::I64, RET_BOXED);
        let ok = self.b.ins().icmp(IntCC::NotEqual, value, boxed);
        self.guard(ok);
    }

    fn check_copy(&mut self, value: Value) {
        if self.resumable {
            self.check_not_boxed(value);
        }
    }

    fn guard(&mut self, ok: Value) {
        let cont = self.b.create_block();
        let target = if self.resumable {
            self.escape_target(self.current_pc, self.current_height)
        } else {
            self.deopt
        };
        self.b.ins().brif(ok, cont, &[], target, &[]);
        self.b.seal_block(cont);
        self.b.switch_to_block(cont);
    }

    fn truthy(&mut self, value: Value) -> Value {
        let zero = self.b.ins().iconst(types::I64, 0);
        let undef = self.b.ins().iconst(types::I64, RET_UNDEF);
        let null = self.b.ins().iconst(types::I64, RET_NULL);
        let false_ = self.b.ins().iconst(types::I64, RET_FALSE);
        let t0 = self.b.ins().icmp(IntCC::NotEqual, value, zero);
        let t1 = self.b.ins().icmp(IntCC::NotEqual, value, undef);
        let t2 = self.b.ins().icmp(IntCC::NotEqual, value, null);
        let t3 = self.b.ins().icmp(IntCC::NotEqual, value, false_);
        let t01 = self.b.ins().band(t0, t1);
        let t23 = self.b.ins().band(t2, t3);
        self.b.ins().band(t01, t23)
    }

    fn fallthrough_block(&self, index: usize) -> Block {
        let next = index + 1;
        if next < self.desc.ops.len() && self.heights[next].is_some() {
            self.op_blocks[next]
        } else {
            self.deopt
        }
    }

    fn fallthrough(&mut self, index: usize) {
        let block = self.fallthrough_block(index);
        self.b.ins().jump(block, &[]);
    }

    fn emit_return(&mut self) {
        let value = self.b.use_var(self.acc);
        for (sentinel, block) in [
            (RET_UNDEF, self.ret_undef),
            (RET_NULL, self.ret_null),
            (RET_TRUE, self.ret_true),
            (RET_FALSE, self.ret_false),
        ] {
            let constant = self.b.ins().iconst(types::I64, sentinel);
            let matches = self.b.ins().icmp(IntCC::Equal, value, constant);
            let keep_going = self.b.create_block();
            self.b.ins().brif(matches, block, &[], keep_going, &[]);
            self.b.seal_block(keep_going);
            self.b.switch_to_block(keep_going);
        }
        let narrowed = self.b.ins().ireduce(types::I32, value);
        let widened = self.b.ins().sextend(types::I64, narrowed);
        let ok = self.b.ins().icmp(IntCC::Equal, widened, value);
        self.guard(ok);
        self.b.ins().return_(&[value]);
    }

    fn emit_op(&mut self, index: usize) {
        let op = self.desc.ops[index];
        let height = self.heights[index].unwrap_or(0);
        match op {
            JitOp::GetArgument { index: arg, dst } => {
                let offset = i32::try_from(arg).unwrap_or(0).saturating_mul(8);
                let value =
                    self.b
                        .ins()
                        .load(types::I64, MemFlagsData::trusted(), self.args_ptr, offset);
                self.check_copy(value);
                self.b.def_var(self.regs[dst as usize], value);
                self.fallthrough(index);
            }
            JitOp::StoreConst { dst, value } => {
                let constant = self.b.ins().iconst(types::I64, value);
                self.b.def_var(self.regs[dst as usize], constant);
                self.fallthrough(index);
            }
            JitOp::Move { dst, src } => {
                let value = self.b.use_var(self.regs[src as usize]);
                self.check_copy(value);
                self.b.def_var(self.regs[dst as usize], value);
                self.fallthrough(index);
            }
            JitOp::AddInt { dst, lhs, rhs } => {
                self.emit_arithmetic(index, dst, lhs, rhs, Arith::Add);
            }
            JitOp::SubInt { dst, lhs, rhs } => {
                self.emit_arithmetic(index, dst, lhs, rhs, Arith::Sub);
            }
            JitOp::MulInt { dst, lhs, rhs } => {
                self.emit_arithmetic(index, dst, lhs, rhs, Arith::Mul);
            }
            JitOp::ModInt { dst, lhs, rhs } => {
                self.emit_mod(index, dst, lhs, rhs);
            }
            JitOp::BitAndInt { dst, lhs, rhs } => {
                self.emit_bitwise(index, dst, lhs, rhs, Bitwise::And);
            }
            JitOp::BitOrInt { dst, lhs, rhs } => {
                self.emit_bitwise(index, dst, lhs, rhs, Bitwise::Or);
            }
            JitOp::BitXorInt { dst, lhs, rhs } => {
                self.emit_bitwise(index, dst, lhs, rhs, Bitwise::Xor);
            }
            JitOp::ShlInt { dst, lhs, rhs } => {
                self.emit_bitwise(index, dst, lhs, rhs, Bitwise::Shl);
            }
            JitOp::ShrInt { dst, lhs, rhs } => {
                self.emit_bitwise(index, dst, lhs, rhs, Bitwise::Shr);
            }
            JitOp::UShrInt { dst, lhs, rhs } => {
                self.emit_unsigned_shift(index, dst, lhs, rhs);
            }
            JitOp::NegReg { reg } => {
                self.emit_negate(index, reg);
            }
            JitOp::BitNotReg { reg } => {
                self.emit_bit_not(index, reg);
            }
            JitOp::LogicalNotReg { reg } => {
                self.emit_logical_not(index, reg);
            }
            JitOp::IncDec { dst, src, delta } => {
                let value = self.load_int(src);
                let delta = self.b.ins().iconst(types::I64, delta);
                let result = self.b.ins().iadd(value, delta);
                self.check_i32(result);
                self.b.def_var(self.regs[dst as usize], result);
                self.fallthrough(index);
            }
            JitOp::PushFromRegister { src } => {
                let value = self.b.use_var(self.regs[src as usize]);
                self.check_copy(value);
                self.b.def_var(self.vs[height as usize], value);
                self.fallthrough(index);
            }
            JitOp::PopIntoRegister { dst } => {
                let value = self.b.use_var(self.vs[height as usize - 1]);
                self.check_copy(value);
                self.b.def_var(self.regs[dst as usize], value);
                self.fallthrough(index);
            }
            JitOp::DropValue => {
                self.fallthrough(index);
            }
            JitOp::LoopTick => {
                let count = self.b.use_var(self.counter);
                let within =
                    self.b
                        .ins()
                        .icmp(IntCC::UnsignedLessThanOrEqual, count, self.loop_limit);
                self.guard(within);
                let one = self.b.ins().iconst(types::I64, 1);
                let next = self.b.ins().iadd(count, one);
                self.b.def_var(self.counter, next);
                self.fallthrough(index);
            }
            JitOp::Jump { target } => {
                self.b.ins().jump(self.op_blocks[target as usize], &[]);
            }
            JitOp::JumpIfTrue { target, value } => {
                let value = self.b.use_var(self.regs[value as usize]);
                self.check_not_boxed(value);
                let truthy = self.truthy(value);
                self.emit_branch(index, truthy, target, true);
            }
            JitOp::JumpIfFalse { target, value } => {
                let value = self.b.use_var(self.regs[value as usize]);
                self.check_not_boxed(value);
                let truthy = self.truthy(value);
                self.emit_branch(index, truthy, target, false);
            }
            JitOp::JumpIfGe { target, lhs, rhs } => {
                self.emit_int_branch(index, lhs, rhs, IntCC::SignedGreaterThanOrEqual, target);
            }
            JitOp::JumpIfGt { target, lhs, rhs } => {
                self.emit_int_branch(index, lhs, rhs, IntCC::SignedGreaterThan, target);
            }
            JitOp::JumpIfLe { target, lhs, rhs } => {
                self.emit_int_branch(index, lhs, rhs, IntCC::SignedLessThanOrEqual, target);
            }
            JitOp::JumpIfLt { target, lhs, rhs } => {
                self.emit_int_branch(index, lhs, rhs, IntCC::SignedLessThan, target);
            }
            JitOp::JumpIfNe { target, lhs, rhs } => {
                self.emit_int_branch(index, lhs, rhs, IntCC::NotEqual, target);
            }
            JitOp::JumpIfNullish { target, value } => {
                let value = self.b.use_var(self.regs[value as usize]);
                self.check_not_boxed(value);
                let null = self.b.ins().iconst(types::I64, RET_NULL);
                let undef = self.b.ins().iconst(types::I64, RET_UNDEF);
                let is_null = self.b.ins().icmp(IntCC::Equal, value, null);
                let is_undef = self.b.ins().icmp(IntCC::Equal, value, undef);
                let nullish = self.b.ins().bor(is_null, is_undef);
                self.emit_branch(index, nullish, target, true);
            }
            JitOp::JumpIfNotUndefined { target, value } => {
                let value = self.b.use_var(self.regs[value as usize]);
                self.check_not_boxed(value);
                let undef = self.b.ins().iconst(types::I64, RET_UNDEF);
                let defined = self.b.ins().icmp(IntCC::NotEqual, value, undef);
                self.emit_branch(index, defined, target, true);
            }
            JitOp::StrictEq { dst, lhs, rhs } => {
                self.emit_strict_compare(index, dst, lhs, rhs, true);
            }
            JitOp::StrictNotEq { dst, lhs, rhs } => {
                self.emit_strict_compare(index, dst, lhs, rhs, false);
            }
            JitOp::LessThan { dst, lhs, rhs } => {
                self.emit_int_compare(index, dst, lhs, rhs, IntCC::SignedLessThan);
            }
            JitOp::GreaterThan { dst, lhs, rhs } => {
                self.emit_int_compare(index, dst, lhs, rhs, IntCC::SignedGreaterThan);
            }
            JitOp::LessThanOrEqual { dst, lhs, rhs } => {
                self.emit_int_compare(index, dst, lhs, rhs, IntCC::SignedLessThanOrEqual);
            }
            JitOp::GreaterThanOrEqual { dst, lhs, rhs } => {
                self.emit_int_compare(index, dst, lhs, rhs, IntCC::SignedGreaterThanOrEqual);
            }
            JitOp::SetAccumulator { src } => {
                let value = self.b.use_var(self.regs[src as usize]);
                self.check_copy(value);
                self.b.def_var(self.acc, value);
                self.fallthrough(index);
            }
            JitOp::Return => {
                self.emit_return();
            }
            JitOp::NoOp => {
                self.fallthrough(index);
            }
            JitOp::Escape { pc } => {
                self.escape_at(pc, height);
            }
        }
    }

    fn emit_arithmetic(&mut self, index: usize, dst: u32, lhs: u32, rhs: u32, op: Arith) {
        let lhs = self.load_int(lhs);
        let rhs = self.load_int(rhs);
        let result = match op {
            Arith::Add => self.b.ins().iadd(lhs, rhs),
            Arith::Sub => self.b.ins().isub(lhs, rhs),
            Arith::Mul => self.b.ins().imul(lhs, rhs),
        };
        self.check_i32(result);
        if matches!(op, Arith::Mul) {
            let zero = self.b.ins().iconst(types::I64, 0);
            let nonzero = self.b.ins().icmp(IntCC::NotEqual, result, zero);
            let lhs_negative = self.b.ins().icmp(IntCC::SignedLessThan, lhs, zero);
            let rhs_negative = self.b.ins().icmp(IntCC::SignedLessThan, rhs, zero);
            let lhs_nonnegative = self
                .b
                .ins()
                .icmp(IntCC::SignedGreaterThanOrEqual, lhs, zero);
            let rhs_nonnegative = self
                .b
                .ins()
                .icmp(IntCC::SignedGreaterThanOrEqual, rhs, zero);
            let both_negative = self.b.ins().band(lhs_negative, rhs_negative);
            let both_nonnegative = self.b.ins().band(lhs_nonnegative, rhs_nonnegative);
            let same_sign = self.b.ins().bor(both_negative, both_nonnegative);
            let representable = self.b.ins().bor(nonzero, same_sign);
            self.guard(representable);
        }
        self.b.def_var(self.regs[dst as usize], result);
        self.fallthrough(index);
    }

    fn emit_mod(&mut self, index: usize, dst: u32, lhs: u32, rhs: u32) {
        let lhs = self.load_int(lhs);
        let lhs = self.narrow_i32(lhs);
        let rhs = self.load_int(rhs);
        let rhs = self.narrow_i32(rhs);
        let zero = self.b.ins().iconst(types::I32, 0);
        let nonzero = self.b.ins().icmp(IntCC::NotEqual, rhs, zero);
        self.guard(nonzero);
        let min = self.b.ins().iconst(types::I32, i64::from(i32::MIN));
        let minus_one = self.b.ins().iconst(types::I32, -1);
        let not_min = self.b.ins().icmp(IntCC::NotEqual, lhs, min);
        let not_minus_one = self.b.ins().icmp(IntCC::NotEqual, rhs, minus_one);
        let safe = self.b.ins().bor(not_min, not_minus_one);
        self.guard(safe);
        let remainder = self.b.ins().srem(lhs, rhs);
        let non_negative = self
            .b
            .ins()
            .icmp(IntCC::SignedGreaterThanOrEqual, lhs, zero);
        let nonzero_remainder = self.b.ins().icmp(IntCC::NotEqual, remainder, zero);
        let representable = self.b.ins().bor(non_negative, nonzero_remainder);
        self.guard(representable);
        let wide = self.b.ins().sextend(types::I64, remainder);
        self.b.def_var(self.regs[dst as usize], wide);
        self.fallthrough(index);
    }

    fn emit_bitwise(&mut self, index: usize, dst: u32, lhs: u32, rhs: u32, op: Bitwise) {
        let lhs = self.load_int(lhs);
        let lhs = self.narrow_i32(lhs);
        let rhs = self.load_int(rhs);
        let rhs = self.narrow_i32(rhs);
        let result = match op {
            Bitwise::And => self.b.ins().band(lhs, rhs),
            Bitwise::Or => self.b.ins().bor(lhs, rhs),
            Bitwise::Xor => self.b.ins().bxor(lhs, rhs),
            Bitwise::Shl => {
                let count = self.shift_count(rhs);
                self.b.ins().ishl(lhs, count)
            }
            Bitwise::Shr => {
                let count = self.shift_count(rhs);
                self.b.ins().sshr(lhs, count)
            }
        };
        let wide = self.b.ins().sextend(types::I64, result);
        self.b.def_var(self.regs[dst as usize], wide);
        self.fallthrough(index);
    }

    fn emit_unsigned_shift(&mut self, index: usize, dst: u32, lhs: u32, rhs: u32) {
        let lhs = self.load_int(lhs);
        let lhs = self.narrow_i32(lhs);
        let rhs = self.load_int(rhs);
        let rhs = self.narrow_i32(rhs);
        let count = self.shift_count(rhs);
        let shifted = self.b.ins().ushr(lhs, count);
        let wide = self.b.ins().uextend(types::I64, shifted);
        let max = self.b.ins().iconst(types::I64, i64::from(i32::MAX));
        let fits = self.b.ins().icmp(IntCC::SignedLessThanOrEqual, wide, max);
        self.guard(fits);
        self.b.def_var(self.regs[dst as usize], wide);
        self.fallthrough(index);
    }

    fn shift_count(&mut self, rhs: Value) -> Value {
        let thirty_one = self.b.ins().iconst(types::I32, 31);
        self.b.ins().band(rhs, thirty_one)
    }

    fn narrow_i32(&mut self, value: Value) -> Value {
        self.b.ins().ireduce(types::I32, value)
    }

    fn emit_negate(&mut self, index: usize, reg: u32) {
        let value = self.load_int(reg);
        let value = self.narrow_i32(value);
        let zero = self.b.ins().iconst(types::I32, 0);
        let nonzero = self.b.ins().icmp(IntCC::NotEqual, value, zero);
        self.guard(nonzero);
        let min = self.b.ins().iconst(types::I32, i64::from(i32::MIN));
        let not_min = self.b.ins().icmp(IntCC::NotEqual, value, min);
        self.guard(not_min);
        let negated = self.b.ins().ineg(value);
        let wide = self.b.ins().sextend(types::I64, negated);
        self.b.def_var(self.regs[reg as usize], wide);
        self.fallthrough(index);
    }

    fn emit_bit_not(&mut self, index: usize, reg: u32) {
        let value = self.load_int(reg);
        let value = self.narrow_i32(value);
        let ones = self.b.ins().iconst(types::I32, -1);
        let result = self.b.ins().bxor(value, ones);
        let wide = self.b.ins().sextend(types::I64, result);
        self.b.def_var(self.regs[reg as usize], wide);
        self.fallthrough(index);
    }

    fn emit_logical_not(&mut self, index: usize, reg: u32) {
        let value = self.b.use_var(self.regs[reg as usize]);
        self.check_not_boxed(value);
        let truthy = self.truthy(value);
        let true_ = self.b.ins().iconst(types::I64, RET_TRUE);
        let false_ = self.b.ins().iconst(types::I64, RET_FALSE);
        let result = self.b.ins().select(truthy, false_, true_);
        self.b.def_var(self.regs[reg as usize], result);
        self.fallthrough(index);
    }

    fn emit_int_branch(&mut self, index: usize, lhs: u32, rhs: u32, cc: IntCC, target: u32) {
        let lhs = self.load_int(lhs);
        let rhs = self.load_int(rhs);
        let condition = self.b.ins().icmp(cc, lhs, rhs);
        self.emit_branch(index, condition, target, true);
    }

    fn emit_branch(&mut self, index: usize, condition: Value, target: u32, on_true: bool) {
        let target_block = self.op_blocks[target as usize];
        let fallthrough_block = self.fallthrough_block(index);
        if on_true {
            self.b
                .ins()
                .brif(condition, target_block, &[], fallthrough_block, &[]);
        } else {
            self.b
                .ins()
                .brif(condition, fallthrough_block, &[], target_block, &[]);
        }
    }

    fn emit_strict_compare(&mut self, index: usize, dst: u32, lhs: u32, rhs: u32, equal: bool) {
        let lhs_val = self.b.use_var(self.regs[lhs as usize]);
        let rhs_val = self.b.use_var(self.regs[rhs as usize]);
        self.check_not_boxed(lhs_val);
        self.check_not_boxed(rhs_val);
        let cc = if equal { IntCC::Equal } else { IntCC::NotEqual };
        let cond = self.b.ins().icmp(cc, lhs_val, rhs_val);
        let true_ = self.b.ins().iconst(types::I64, RET_TRUE);
        let false_ = self.b.ins().iconst(types::I64, RET_FALSE);
        let result = self.b.ins().select(cond, true_, false_);
        self.b.def_var(self.regs[dst as usize], result);
        self.fallthrough(index);
    }

    fn emit_int_compare(&mut self, index: usize, dst: u32, lhs: u32, rhs: u32, cc: IntCC) {
        let lhs_val = self.load_int(lhs);
        let rhs_val = self.load_int(rhs);
        let cond = self.b.ins().icmp(cc, lhs_val, rhs_val);
        let true_ = self.b.ins().iconst(types::I64, RET_TRUE);
        let false_ = self.b.ins().iconst(types::I64, RET_FALSE);
        let result = self.b.ins().select(cond, true_, false_);
        self.b.def_var(self.regs[dst as usize], result);
        self.fallthrough(index);
    }
}

#[derive(Clone, Copy)]
enum Arith {
    Add,
    Sub,
    Mul,
}

#[derive(Clone, Copy)]
enum Bitwise {
    And,
    Or,
    Xor,
    Shl,
    Shr,
}
