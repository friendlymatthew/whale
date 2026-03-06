use std::fmt::{Debug, Formatter};
use std::rc::Rc;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::binary_grammar::{
    Function, FunctionType, GlobalType, MemoryType, RefType, SubType, TableType, ValueType,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ref {
    Null,
    FunctionAddr(usize),
    RefExtern(usize),
    I31(i32),
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize, Deserialize)]
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
            ValueType::I32 => Self::I32(0),
            ValueType::I64 => Self::I64(0),
            ValueType::F32 => Self::F32(0.0),
            ValueType::F64 => Self::F64(0.0),
            ValueType::V128 => Self::V128(0),
            ValueType::Ref(_) => Self::Ref(Ref::Null),
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
try_from_value!(Ref, Ref);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInstance {
    pub types: Vec<SubType>,
    pub function_addrs: Vec<usize>,
    pub table_addrs: Vec<usize>,
    pub mem_addrs: Vec<usize>,
    pub global_addrs: Vec<usize>,
    pub tag_addrs: Vec<usize>,
    pub elem_addrs: Vec<usize>,
    pub data_addrs: Vec<usize>,
    pub exports: Vec<ExportInstance>,
}

impl ModuleInstance {
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

impl Default for ModuleInstance {
    fn default() -> Self {
        Self::new(vec![])
    }
}

pub enum FunctionInstance {
    Local {
        function_type: FunctionType,
        module: Rc<ModuleInstance>,
        code: Function,
    },
    Host {
        function_type: FunctionType,
        code: Box<dyn Fn()>,
    },
}

impl Debug for FunctionInstance {
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

impl Serialize for FunctionInstance {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStructVariant;
        match self {
            Self::Local {
                function_type,
                module,
                code,
            } => {
                let mut sv =
                    serializer.serialize_struct_variant("FunctionInstance", 0, "Local", 3)?;
                sv.serialize_field("function_type", function_type)?;
                sv.serialize_field("module", module)?;
                sv.serialize_field("code", code)?;
                sv.end()
            }
            Self::Host { .. } => Err(serde::ser::Error::custom(
                "cannot serialize Host function instance",
            )),
        }
    }
}

impl<'de> Deserialize<'de> for FunctionInstance {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        enum FunctionInstanceData {
            Local {
                function_type: FunctionType,
                module: ModuleInstance,
                code: Function,
            },
        }
        let data = FunctionInstanceData::deserialize(deserializer)?;
        match data {
            FunctionInstanceData::Local {
                function_type,
                module,
                code,
            } => Ok(Self::Local {
                function_type,
                module: Rc::new(module),
                code,
            }),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TableInstance {
    pub table_type: TableType,
    pub elem: Vec<Ref>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryInstance {
    pub memory_type: MemoryType,
    pub data: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GlobalInstance {
    pub global_type: GlobalType,
    pub value: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ElementInstance {
    pub ref_type: RefType,
    pub elem: Vec<Ref>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TagInstance {
    pub tag_type: FunctionType,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DataInstance {
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExternalValue {
    Function { addr: usize },
    Table { addr: usize },
    Memory { addr: usize },
    Global { addr: usize },
    Tag { addr: usize },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportInstance {
    pub name: String,
    pub value: ExternalValue,
}
