use anyhow::Result;

use crate::binary_grammar::{Function, Import, ImportDescription, Module};
use crate::execution_grammar::{
    DataInstance, ElementInstance, ExternalImport, FunctionInstance, GlobalInstance,
    MemoryInstance, ModuleInstance, Ref, TableInstance, Value,
};

#[derive(Debug)]
pub struct Store<'a> {
    pub functions: Vec<FunctionInstance<'a>>,
    pub tables: Vec<TableInstance>,
    pub memories: Vec<MemoryInstance<'a>>,
    pub globals: Vec<GlobalInstance>,
    pub element_segments: Vec<ElementInstance>,
    pub data_segments: Vec<DataInstance<'a>>,
}

impl<'a> Store<'a> {
    pub fn new() -> Self {
        Self {
            functions: vec![],
            tables: vec![],
            memories: vec![],
            globals: vec![],
            element_segments: vec![],
            data_segments: vec![],
        }
    }

    fn allocate_function(&mut self, f: &Function) {
        let func_address = self.functions.len();
        let func_type = todo!();
    }

    pub fn allocate_module(
        &mut self,
        module: Module<'a>,
        mut imports: Vec<ExternalImport>,
        initial_global_values: Vec<Value>,
        element_segment_refs: Vec<Ref>,
    ) -> Result<ModuleInstance<'a>> {
        let module_instance = ModuleInstance::new(module.types);

        for import in module.imports {
            let Import {
                module,
                name,
                description,
            } = import;

            match description {
                ImportDescription::Func(_) => {}
                ImportDescription::Table(_) => {}
                ImportDescription::Mem(_) => {}
                ImportDescription::Global(_) => {}
            }
        }

        Ok(module_instance)
    }
}
