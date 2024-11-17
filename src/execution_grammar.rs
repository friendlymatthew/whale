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

#[derive(Debug)]
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

#[derive(Debug)]
pub enum FunctionInstance<'a> {
    Local {
        function_type: FunctionType,
        module: ModuleInstance<'a>,
        code: Function,
    },
    Host {
        function_type: FunctionType,
    },
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
    Func(Box<dyn Fn()>),
    Table(Vec<Ref>),
    Memory(Vec<u8>),
    Global(Value),
}

pub struct ExternalImport<'a> {
    pub module: &'a str,
    pub name: &'a str,
    pub value: ImportValue,
}

#[derive(Debug)]
pub enum ExternalValue {
    Function { addr: usize },
    Table { addr: usize },
    Memory { addr: usize },
    Global { addr: usize },
}

#[derive(Debug)]
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
