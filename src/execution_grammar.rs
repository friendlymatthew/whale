use crate::{Function, FunctionType};

#[derive(Debug)]
pub enum Value {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    V128(i128),
    RefNull,
    RefFunctionAddr(u32),
    RefExtern(u32),
}

#[derive(Debug)]
pub enum Result {
    Values(Vec<Value>),
    Trap,
}

pub enum FunctionInstance {
    Local {
        function_type: FunctionType,
        function: Function,
    },
}

#[derive(Debug)]
pub struct Store {}

#[derive(Debug)]
pub enum StackEntry {
    Value(Value),
    Label,
    Activation,
}
