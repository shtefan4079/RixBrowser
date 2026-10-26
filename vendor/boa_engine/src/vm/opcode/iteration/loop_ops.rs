use crate::error::RuntimeLimitError;
use crate::{Context, JsResult, vm::opcode::Operation};

/// `IncrementLoopIteration` implements the Opcode Operation for `Opcode::IncrementLoopIteration`.
///
/// Operation:
///  - Increment loop iteration count.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IncrementLoopIteration;

impl IncrementLoopIteration {
    #[inline(always)]
    pub(crate) fn operation((): (), context: &mut Context) -> JsResult<()> {
        let max = context.vm.runtime_limits.loop_iteration_limit();
        let frame = context.vm.frame_mut();
        let previous_iteration_count = frame.loop_iteration_count;

        if previous_iteration_count > max {
            return Err(RuntimeLimitError::LoopIteration.into());
        }

        frame.loop_iteration_count = previous_iteration_count.wrapping_add(1);

        // A body that spends thousands of iterations in one interpreted loop
        // is hot even though it was never called a hundred times: ask the VM
        // loop for on-stack replacement so the rest of the loop can run
        // natively. Bodies that are already compiled (`jit_fn() != 0`) or
        // disabled (`JIT_DISABLED`) never ask again.
        if frame.loop_iteration_count >= crate::vm::jit::OSR_LOOP_THRESHOLD
            && frame.code_block.jit_fn() == 0
            && crate::vm::jit::enabled()
        {
            frame.code_block.request_jit_osr();
        }
        Ok(())
    }
}

impl Operation for IncrementLoopIteration {
    const NAME: &'static str = "IncrementLoopIteration";
    const INSTRUCTION: &'static str = "INST - IncrementLoopIteration";
    const COST: u8 = 3;
}
