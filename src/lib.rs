#![warn(clippy::nursery)]

pub use interpreter::*;
pub use parser::*;
pub use store::*;

pub(crate) mod binary_grammar;
pub(crate) mod execution_grammar;
mod interpreter;
pub mod leb128;
mod parser;
mod store;
