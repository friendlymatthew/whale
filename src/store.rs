use anyhow::{anyhow, bail, Result};

use crate::binary_grammar::{Function, Import, ImportDescription, MemoryType, Module, TableType};
use crate::execution_grammar::{
    DataInstance, ElementInstance, ExternalImport, FunctionInstance, GlobalInstance, ImportValue,
    MemoryInstance, ModuleInstance, Ref, TableInstance, Value,
};

#[derive(Debug)]
pub struct Store<'a> {
    pub functions: Vec<FunctionInstance<'a>>,
    pub tables: Vec<TableInstance>,
    pub memories: Vec<MemoryInstance>,
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

    fn allocate_function(
        &mut self,
        f: Function,
        module_instance: &ModuleInstance<'a>,
    ) -> Result<usize> {
        let f_address = self.functions.len();

        let function_type = module_instance
            .types
            .get(f.type_index as usize)
            .ok_or(anyhow!(
                "Function type index {} too large to index into module instance types. Len: {}",
                f.type_index,
                module_instance.types.len()
            ))?;

        self.functions.push(FunctionInstance::Local {
            function_type: function_type.clone(),
            module: module_instance.clone(),
            code: f,
        });

        Ok(f_address)
    }

    fn allocate_host_function(
        &mut self,
        h_f: Box<dyn Fn()>,
        f_idx: u32,
        module_instance: &ModuleInstance<'a>,
    ) -> Result<usize> {
        let func_address = self.functions.len();

        let function_type = module_instance.types.get(f_idx as usize).ok_or(anyhow!(
            "Function type index {} too large to index
    into module instance types",
            f_idx
        ))?;

        self.functions.push(FunctionInstance::Host {
            function_type: function_type.clone(),
            code: h_f,
        });

        Ok(func_address)
    }

    fn allocate_table(&mut self, table_type: TableType, initial_ref: Ref) -> Result<usize> {
        let n = table_type.limit.min;

        let table_address = self.tables.len();

        self.tables.push(TableInstance {
            r#type: table_type,
            elem: vec![initial_ref; n as usize],
        });

        Ok(table_address)
    }

    fn allocate_memory(&mut self, memory_type: MemoryType) -> Result<usize> {
        let memory_address = self.memories.len();
        let n = memory_type.0.min;

        self.memories.push(MemoryInstance {
            r#type: memory_type,
            data: vec![0u8; n as usize],
        });

        Ok(memory_address)
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

            if let Some(index) = imports.iter().position(
                |ExternalImport {
                     module: extern_module,
                     name: extern_name,
                     ..
                 }| { module == *extern_module && name == *extern_name },
            ) {
                let ExternalImport { value, .. } = imports.remove(index);

                match (value, description) {
                    (ImportValue::Func(f), ImportDescription::Func(f_idx)) => {
                        let host_function_address =
                            self.allocate_host_function(f, f_idx, &module_instance)?;
                    }
                    (ImportValue::Global(g), ImportDescription::Global(g_type)) => {}
                    (ImportValue::Table(t), ImportDescription::Table(t_type)) => {
                        let table_addresses = self.allocate_table(t_type, Ref::Null)?;
                    }
                    (ImportValue::Memory(m), ImportDescription::Mem(m_type)) => {}
                    _ => bail!("Mismatched type. todo! impl Debug for ImportValue."),
                }
            } else {
                bail!(
                    "Unrecognized import: Expected module: {}, name: {}.",
                    module,
                    name
                )
            }
        }

        let function_addresses = module
            .functions
            .into_iter()
            .map(|f| self.allocate_function(f, &module_instance))
            .collect::<Result<Vec<_>>>()?;

        let table_addresses = module
            .tables
            .into_iter()
            .map(|t| self.allocate_table(t, Ref::Null))
            .collect::<Result<Vec<_>>>()?;

        let memory_addresses = module
            .mems
            .into_iter()
            .map(|m| self.allocate_memory(m))
            .collect::<Result<Vec<_>>>()?;

        Ok(module_instance)
    }
}
