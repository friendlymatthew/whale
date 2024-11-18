use std::fmt::{Debug, Formatter};
use std::rc::Rc;

use crate::binary_grammar::{
    Function, FunctionType, GlobalType, Instruction, MemoryType, RefType, TableType,
};

#[derive(Debug)]
pub enum Ref {
    Null,
    FunctionAddr(usize),
    RefExtern(usize),
}

#[derive(Debug)]
pub enum Value {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    V128(i128),
    Ref(Ref),
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

pub enum FunctionInstance<'a> {
    Local {
        function_type: FunctionType,
        module: ModuleInstance<'a>,
        code: Function,
    },
    Host {
        function_type: FunctionType,
        code: Rc<dyn Fn()>,
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
    pub r#type: TableType,
    pub elem: Vec<Ref>,
}

#[derive(Debug)]
pub struct MemoryInstance<'a> {
    pub r#type: MemoryType,
    pub data: &'a [u8],
}

#[derive(Debug)]
pub struct GlobalInstance {
    pub r#type: GlobalType,
    pub value: Value,
}

#[derive(Debug)]
pub struct ElementInstance {
    pub r#type: RefType,
    pub elem: Vec<Ref>,
}

#[derive(Debug)]
pub struct DataInstance<'a> {
    pub data: &'a [u8],
}

pub enum ImportValue {
    // Wrap in Rc to convert a dynamically sized type into one with a statically known
    // type.
    Func(Rc<dyn Fn()>),
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
    arity: u32,
    target: Vec<Instruction>,
}

#[derive(Debug)]
pub struct FrameState<'a> {
    locals: Vec<Value>,
    module: ModuleInstance<'a>,
}

#[derive(Debug)]
pub struct ActivationFrame<'a> {
    arity: u32,
    state: FrameState<'a>,
}

#[derive(Debug)]
pub enum StackEntry<'a> {
    Value(Value),
    Label(Label),
    Activation(ActivationFrame<'a>),
}

#[derive(Debug)]
pub struct Stack<'a>(Vec<StackEntry<'a>>);

impl<'a> Stack<'a> {
    pub fn new() -> Self {
        Self(vec![])
    }

    pub fn push(&mut self, entry: StackEntry<'a>) {
        self.0.push(entry);
    }

    pub fn pop(&mut self) -> Option<StackEntry<'a>> {
        self.0.pop()
    }
}
