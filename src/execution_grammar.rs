use std::fmt::{Debug, Formatter};

use anyhow::ensure;

use crate::binary_grammar::{
    Function, FunctionType, GlobalType, Instruction, MemoryType, RefType, TableType, ValueType,
};

#[derive(Debug, Copy, Clone)]
pub enum Ref {
    Null,
    FunctionAddr(usize),
    RefExtern(usize),
}

#[derive(Debug, Clone)]
pub enum Value {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    V128(i128),
    Ref(Ref),
}

impl Value {
    pub fn default(value_type: ValueType) -> Self {
        match value_type {
            ValueType::I32 => Value::I32(0),
            ValueType::I64 => Value::I64(0),
            ValueType::F32 => Value::F32(0.0),
            ValueType::F64 => Value::F64(0.0),
            ValueType::V128 => Value::V128(0),
            ValueType::Ref(_) => Value::Ref(Ref::Null),
        }
    }
}

#[derive(Debug)]
pub enum Result {
    Values(Vec<Value>),
    Trap,
}

// todo: is there a better way to store ModuleInstances than cloning?
#[derive(Debug, Clone)]
pub struct ModuleInstance<'a> {
    pub types: Vec<FunctionType>,
    pub function_addrs: Vec<usize>,
    pub table_addrs: Vec<usize>,
    pub mem_addrs: Vec<usize>,
    pub global_addrs: Vec<usize>,
    pub elem_addrs: Vec<usize>,
    pub data_addrs: Vec<usize>,
    pub exports: Vec<ExportInstance<'a>>,
}

impl<'a> ModuleInstance<'a> {
    pub fn new(types: Vec<FunctionType>) -> Self {
        Self {
            types,
            function_addrs: vec![],
            table_addrs: vec![],
            mem_addrs: vec![],
            global_addrs: vec![],
            elem_addrs: vec![],
            data_addrs: vec![],
            exports: vec![],
        }
    }
}

impl<'a> Default for ModuleInstance<'a> {
    fn default() -> Self {
        Self::new(vec![])
    }
}

pub enum FunctionInstance<'a> {
    Local {
        function_type: FunctionType,
        module: ModuleInstance<'a>,
        code: Function,
    },
    Host {
        function_type: FunctionType,
        code: Box<dyn Fn()>,
    },
}

impl<'a> Debug for FunctionInstance<'a> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local {
                function_type,
                module,
                code,
            } => f
                .debug_struct("FunctionInstance Local")
                .field("function_type: {:?}", function_type)
                .field("module: {:?}", module)
                .field("code: {:?}", code)
                .finish(),
            Self::Host { function_type, .. } => f
                .debug_struct("FunctionInstance Host")
                .field("function_type {:?}", function_type)
                .finish(),
        }
    }
}

#[derive(Debug)]
pub struct TableInstance {
    pub table_type: TableType,
    pub elem: Vec<Ref>,
}

#[derive(Debug)]
pub struct MemoryInstance {
    pub memory_type: MemoryType,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub struct GlobalInstance {
    pub global_type: GlobalType,
    pub value: Value,
}

#[derive(Debug)]
pub struct ElementInstance {
    pub ref_type: RefType,
    pub elem: Vec<Ref>,
}

#[derive(Debug)]
pub struct DataInstance<'a> {
    pub data: &'a [u8],
}

pub enum ImportValue {
    // Wrap in Box to convert a dynamically sized type into one with a statically known
    // type.
    Func(Box<dyn Fn()>),
    Table(Vec<Ref>),
    Memory(Vec<u8>),
    Global(Value),
}

impl Debug for ImportValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportValue::Func(_) => f.debug_tuple("ImportValue Function").finish(),
            ImportValue::Table(t) => f.debug_tuple("Import Value Table").field(t).finish(),
            ImportValue::Memory(m) => f.debug_tuple("Import Value Memory").field(m).finish(),
            ImportValue::Global(g) => f.debug_tuple("Import Value Global").field(g).finish(),
        }
    }
}

#[derive(Debug)]
pub struct ExternalImport<'a> {
    pub module: &'a str,
    pub name: &'a str,
    pub value: ImportValue,
}

#[derive(Debug, Clone)]
pub enum ExternalValue {
    Function { addr: usize },
    Table { addr: usize },
    Memory { addr: usize },
    Global { addr: usize },
}

#[derive(Debug, Clone)]
pub struct ExportInstance<'a> {
    pub name: &'a str,
    pub value: ExternalValue,
}

#[derive(Debug)]
pub struct Label {
    pub arity: u32,
    pub continuation: Vec<Instruction>,
}

#[derive(Debug, Default, Clone)]
pub struct Frame<'a> {
    pub arity: usize,
    pub locals: Vec<Value>,
    pub module: ModuleInstance<'a>,
}

#[derive(Debug)]
pub enum Entry<'a> {
    Value(Value),
    Label(Label),
    Activation(Frame<'a>),
}

#[derive(Debug)]
pub struct Stack<'a>(Vec<Entry<'a>>);

impl<'a> Stack<'a> {
    pub fn new() -> Self {
        Self(vec![])
    }

    pub fn push(&mut self, entry: Entry<'a>) {
        self.0.push(entry);
    }

    pub fn pop(&mut self) -> Option<Entry<'a>> {
        self.0.pop()
    }

    pub fn pop_n(&mut self, n: usize) -> anyhow::Result<Vec<Entry<'a>>> {
        ensure!(self.len() >= n, "Stack must have at least {} entries", n);
        Ok(self.0.split_off(self.len() - n))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}
