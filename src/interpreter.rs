use anyhow::Result;

use crate::binary_grammar::Module;
use crate::execution_grammar::{ExternalImport, Ref, Stack, Value};
use crate::Parser;
use crate::Store;

#[derive(Debug)]
pub struct Interpreter<'a> {
    module: Module<'a>,
    stack: Stack<'a>,
    store: Store<'a>,
}

impl<'a> Interpreter<'a> {
    pub fn new(
        module_data: &'a [u8],
        imports: Vec<ExternalImport>,
        initial_global_values: Vec<Value>,
        element_segment_refs: Vec<Ref>,
    ) -> Result<Self> {
        let mut module_parser = Parser::new(module_data);

        let module = module_parser.parse_module()?;

        let mut store = Store::new();
        store.allocate_module(module, imports, initial_global_values, element_segment_refs)?;

        todo!()
    }
}
