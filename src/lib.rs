#![warn(clippy::nursery)]

pub use binary_grammar::*;
pub use interpreter::*;
pub use leb128::*;
pub use parser::*;

mod binary_grammar;
mod execution_grammar;
mod interpreter;
mod leb128;
mod parser;
