#![warn(clippy::nursery)]

mod binary_grammar;
pub mod compiler;
mod error;
mod execution_grammar;
pub mod ir;
pub mod leb128;
mod module;
pub mod parser;
mod store;
pub mod value_stack;

pub use binary_grammar::*;
pub use error::*;
pub use execution_grammar::*;
pub use module::*;
pub use store::*;
