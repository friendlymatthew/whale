#![warn(clippy::nursery)]

pub use binary_grammar::{
    AddrType, CompositeType, FunctionType, GlobalType, ImportDescription, Limit, MemoryType,
    Mutability, RefType, TableType, ValueType,
};

pub use execution_grammar::{
    ExternalValue, FunctionInstance, GlobalInstance, MemoryInstance, Ref, TableInstance, Value,
};
pub use interpreter::*;
pub use parser::*;
pub use store::*;

pub(crate) mod binary_grammar;
pub(crate) mod execution_grammar;
#[macro_use]
mod numerics;
mod interpreter;
pub mod leb128;
mod parser;
mod store;
