#![warn(clippy::nursery)]

pub use parser::*;

pub mod grammar;
pub mod leb128;
mod parser;
