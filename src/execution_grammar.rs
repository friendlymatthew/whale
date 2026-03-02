use std::fmt::{Debug, Formatter};

use anyhow::{anyhow, bail, ensure, Result};

use crate::binary_grammar::{
    Function, FunctionType, GlobalType, ImportDescription, MemoryType, RefType,
    SubType, TableType, ValueType,
};

#[derive(Debug, Copy, Clone)]
pub enum Ref {
    Null,
    FunctionAddr(usize),
    RefExtern(usize),
}

#[derive(Debug, Copy, Clone)]
pub enum Value {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    V128(i128),
    Ref(Ref),
}

impl Value {
    pub const fn default(value_type: &ValueType) -> Self {
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

#[macro_export]
macro_rules! try_from_value {
    ($value_variant: ident, $ty:ty) => {
        impl TryFrom<Value> for $ty {
            type Error = anyhow::Error;

            fn try_from(value: Value) -> std::result::Result<Self, Self::Error> {
                match value {
                    Value::$value_variant(v) => Ok(v),
                    _ => bail!(""),
                }
            }
        }

        impl From<$ty> for Value {
            fn from(value: $ty) -> Self {
                Value::$value_variant(value)
            }
        }
    };
}

try_from_value!(I32, i32);
try_from_value!(I64, i64);
try_from_value!(F32, f32);
try_from_value!(F64, f64);
try_from_value!(V128, i128);

#[derive(Debug)]
pub enum ResultKind {
    Values(Vec<Value>),
    Trap,
}

// todo: is there a better way to store ModuleInstances than cloning?
#[derive(Debug, Clone)]
pub struct ModuleInstance<'a> {
    pub types: Vec<SubType>,
    pub function_addrs: Vec<usize>,
    pub table_addrs: Vec<usize>,
    pub mem_addrs: Vec<usize>,
    pub global_addrs: Vec<usize>,
    pub tag_addrs: Vec<usize>,
    pub elem_addrs: Vec<usize>,
    pub data_addrs: Vec<usize>,
    pub exports: Vec<ExportInstance<'a>>,
}

impl<'a> ModuleInstance<'a> {
    pub const fn new(types: Vec<SubType>) -> Self {
        Self {
            types,
            function_addrs: vec![],
            table_addrs: vec![],
            mem_addrs: vec![],
            global_addrs: vec![],
            tag_addrs: vec![],
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
        module: Box<ModuleInstance<'a>>,
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
                .field("function_type: {}", function_type)
                .field("module: {}", module)
                .field("code: {}", code)
                .finish(),
            Self::Host { function_type, .. } => f
                .debug_struct("FunctionInstance Host")
                .field("function_type {}", function_type)
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
pub struct TagInstance {
    pub tag_type: FunctionType,
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
            Self::Func(_) => f.debug_tuple("ImportValue Function").finish(),
            Self::Table(t) => f.debug_tuple("Import Value Table").field(t).finish(),
            Self::Memory(m) => f.debug_tuple("Import Value Memory").field(m).finish(),
            Self::Global(g) => f.debug_tuple("Import Value Global").field(g).finish(),
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
    Tag { addr: usize },
}

#[derive(Debug, Clone)]
pub struct ExportInstance<'a> {
    pub name: &'a str,
    pub value: ExternalValue,
}

#[derive(Debug)]
pub struct Label {
    pub arity: u32,
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

impl<'a> From<Label> for Entry<'a> {
    fn from(value: Label) -> Self {
        Self::Label(value)
    }
}

impl<'a> From<Frame<'a>> for Entry<'a> {
    fn from(value: Frame<'a>) -> Self {
        Self::Activation(value)
    }
}

impl<'a, V: Into<Value>> From<V> for Entry<'a> {
    fn from(value: V) -> Self {
        Self::Value(value.into())
    }
}

#[derive(Debug, Default)]
pub struct Stack<'a>(Vec<Entry<'a>>);

impl<'a> Stack<'a> {
    pub fn push<E: Into<Entry<'a>>>(&mut self, entry: E) {
        self.0.push(entry.into());
    }

    pub fn extend<E: Into<Entry<'a>>, I: IntoIterator<Item = E>>(&mut self, iter: I) {
        self.0.extend(iter.into_iter().map(Into::into));
    }

    pub fn pop(&mut self) -> Result<Entry<'a>> {
        self.0.pop().ok_or_else(|| anyhow!("oob"))
    }

    pub fn pop_value(&mut self) -> Result<Value> {
        match self.pop()? {
            Entry::Value(v) => Ok(v),
            foreign => bail!("not value, got: {:?}", foreign),
        }
    }

    pub fn pop_n(&mut self, len: usize) -> Result<Vec<Entry<'a>>> {
        ensure!(
            self.len() >= len,
            "stack must have at least {} entries",
            len
        );
        Ok(self.0.split_off(self.len() - len))
    }

    pub fn pop_array<const N: usize>(&mut self) -> Result<[Entry<'a>; N]> {
        let out = self.pop_n(N)?;

        Ok(out.try_into().unwrap())
    }

    pub const fn len(&self) -> usize {
        self.0.len()
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn pop_to_label(&mut self, depth: u32) -> Result<u32> {
        let mut label_count = 0;
        let mut label_idx = None;

        for i in (0..self.0.len()).rev() {
            if let Entry::Label(_) = &self.0[i] {
                if label_count == depth {
                    label_idx = Some(i);
                    break;
                }
                label_count += 1;
            }
        }

        let label_idx = label_idx.ok_or_else(|| anyhow!("label depth {} not found", depth))?;
        let arity = match &self.0[label_idx] {
            Entry::Label(l) => l.arity,
            _ => unreachable!(),
        };

        let values = self.pop_n(arity as usize)?;
        self.0.truncate(label_idx);
        self.extend(values);

        Ok(arity)
    }

    pub fn pop_to_label_with_arity(&mut self, depth: u32, arity: usize) -> Result<()> {
        let mut label_count = 0;
        let mut label_idx = None;

        for i in (0..self.0.len()).rev() {
            if let Entry::Label(_) = &self.0[i] {
                if label_count == depth {
                    label_idx = Some(i);
                    break;
                }
                label_count += 1;
            }
        }

        let label_idx = label_idx.ok_or_else(|| anyhow!("label depth {} not found", depth))?;

        let values = self.pop_n(arity)?;
        self.0.truncate(label_idx);
        self.extend(values);

        Ok(())
    }
}
