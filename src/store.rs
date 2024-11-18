use anyhow::{anyhow, bail, ensure, Result};

use crate::binary_grammar::{
    DataSegment, ElementSegment, Function, Global, Import, ImportDescription, MemoryType, Module,
    TableType,
};
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
            table_type,
            elem: vec![initial_ref; n as usize],
        });

        Ok(table_address)
    }

    fn allocate_memory(&mut self, memory_type: MemoryType) -> Result<usize> {
        let memory_address = self.memories.len();
        let n = memory_type.0.min;

        self.memories.push(MemoryInstance {
            memory_type,
            data: vec![0u8; n as usize],
        });

        Ok(memory_address)
    }

    fn allocate_global(&mut self, global: Global, initializer_value: Value) -> Result<usize> {
        let global_address = self.globals.len();

        self.globals.push(GlobalInstance {
            global_type: global.global_type,
            value: initializer_value,
        });

        Ok(global_address)
    }

    fn allocate_element_segment(
        &mut self,
        element_segment: ElementSegment,
        element_segment_ref: Vec<Ref>,
    ) -> Result<usize> {
        let element_segment_address = self.element_segments.len();

        self.element_segments.push(ElementInstance {
            ref_type: element_segment.ref_type,
            elem: element_segment_ref,
        });

        Ok(element_segment_address)
    }

    fn allocate_data_instance(&mut self, data_segment: DataSegment<'a>) -> Result<usize> {
        let data_address = self.data_segments.len();

        self.data_segments.push(DataInstance {
            data: data_segment.bytes,
        });

        Ok(data_address)
    }

    pub fn allocate_module(
        &mut self,
        module: Module<'a>,
        mut imports: Vec<ExternalImport>,
        initial_global_values: Vec<Value>,
        element_segment_refs: Vec<Vec<Ref>>,
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
                    (ImportValue::Global(g), ImportDescription::Global(g_type)) => {
                        todo!("Do I need to trim off initial_global_values as well for this?")
                    }
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

        ensure!(
            module.globals.len() == initial_global_values.len(),
            "Expected equal number of elements for globals and global initializer values."
        );

        let global_addresses = module
            .globals
            .into_iter()
            .zip(initial_global_values)
            .map(|(g, initial_global_values)| self.allocate_global(g, initial_global_values))
            .collect::<Result<Vec<_>>>();

        ensure!(
            module.element_segments.len() == element_segment_refs.len(),
            "Expected equal number of element segments for initial element segment refs"
        );

        let element_addresses = module
            .element_segments
            .into_iter()
            .zip(element_segment_refs)
            .map(|(element_segment, element_segment_ref)| {
                self.allocate_element_segment(element_segment, element_segment_ref)
            })
            .collect::<Result<Vec<_>>>();

        let data_addresses = module
            .data_segments
            .into_iter()
            .map(|data_segment| self.allocate_data_instance(data_segment))
            .collect::<Result<Vec<_>>>()?;

        Ok(module_instance)
    }
}
