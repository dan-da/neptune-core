use std::collections::VecDeque;

use triton_vm::{program::Program, BFieldElement};

pub trait CompiledProgram {
    fn rust_shadow(
        public_input: VecDeque<BFieldElement>,
        secret_input: VecDeque<BFieldElement>,
    ) -> Vec<BFieldElement>;
    fn program() -> Program;
    fn crash_conditions() -> Vec<String>;
}
