#![warn(clippy::nursery)]

mod binary_grammar;
pub mod compiled_interpreter;
pub mod compiler;
mod execution_grammar;
pub mod ir;
pub mod leb128;
pub mod parser;
mod store;
pub mod value_stack;

pub use compiled_interpreter::*;

pub use binary_grammar::*;
pub use execution_grammar::*;
pub use store::*;
