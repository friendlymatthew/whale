use std::sync::Arc;

use crate::binary_grammar::{
    DataSegment, ElementSegment, Export, Function, Global, ImportDeclaration, MemoryType, SubType,
    TableDef, Tag,
};
use crate::compiler::{self, ModuleCode};
use crate::error::Result;
use crate::parser::Parser;

/// A parsed and compiled WASM module ready to be instantiated
pub struct Module {
    pub(crate) code: Arc<ModuleCode>,

    pub(crate) functions: Vec<Function>,
    pub(crate) tables: Vec<TableDef>,
    pub(crate) mems: Vec<MemoryType>,
    pub(crate) element_segments: Vec<ElementSegment>,
    pub(crate) globals: Vec<Global>,
    pub(crate) data_segments: Vec<DataSegment>,
    pub(crate) start: Option<u32>,
    pub(crate) import_declarations: Vec<ImportDeclaration>,
    pub(crate) exports: Vec<Export>,
    pub(crate) tags: Vec<Tag>,
}

impl Module {
    pub fn new(bytes: &[u8]) -> Result<Self> {
        let parsed = Parser::new(bytes).parse_module()?;
        let code = compiler::compile(&parsed);

        Ok(Self {
            code: Arc::new(code),

            functions: parsed.functions,
            tables: parsed.tables,
            mems: parsed.mems,
            element_segments: parsed.element_segments,
            globals: parsed.globals,
            data_segments: parsed.data_segments,
            start: parsed.start,
            import_declarations: parsed.import_declarations,
            exports: parsed.exports,
            tags: parsed.tags,
        })
    }

    pub fn import_declarations(&self) -> &[ImportDeclaration] {
        &self.import_declarations
    }

    pub fn types(&self) -> &[SubType] {
        &self.code.types
    }
}
