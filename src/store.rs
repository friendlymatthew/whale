use anyhow::{anyhow, bail, Result};

use crate::binary_grammar::{
    CompositeType, DataSegment, ElementSegment, ExportDescription, Function, FunctionType, Global,
    MemoryType, Module, TableType,
};
use crate::execution_grammar::{
    DataInstance, ElementInstance, ExportInstance, ExternalValue, FunctionInstance, GlobalInstance,
    MemoryInstance, ModuleInstance, Ref, TableInstance, TagInstance, Value,
};

use serde::{Deserialize, Serialize};

pub(crate) const PAGE_SIZE: usize = 65536;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Store {
    pub functions: Vec<FunctionInstance>,
    pub tables: Vec<TableInstance>,
    pub memories: Vec<MemoryInstance>,
    pub globals: Vec<GlobalInstance>,
    pub tags: Vec<TagInstance>,
    pub element_segments: Vec<ElementInstance>,
    pub data_segments: Vec<DataInstance>,
}

impl Store {
    pub const fn new() -> Self {
        Self {
            functions: vec![],
            tables: vec![],
            memories: vec![],
            globals: vec![],
            tags: vec![],
            element_segments: vec![],
            data_segments: vec![],
        }
    }

    fn extract_function_type(
        module_instance: &ModuleInstance,
        type_index: u32,
    ) -> Result<FunctionType> {
        let sub_type = module_instance
            .types
            .get(type_index as usize)
            .ok_or_else(|| {
                anyhow!(
                    "Type index {} too large to index into module instance types. Len: {}",
                    type_index,
                    module_instance.types.len()
                )
            })?;

        match &sub_type.composite_type {
            CompositeType::Func(ft) => Ok(ft.clone()),
            _ => bail!("Type index {} is not a function type", type_index),
        }
    }

    fn allocate_function(
        &mut self,
        f: Function,
        module_instance: &ModuleInstance,
    ) -> Result<usize> {
        let f_address = self.functions.len();

        let function_type = Self::extract_function_type(module_instance, f.type_index)?;

        self.functions.push(FunctionInstance::Local {
            function_type,
            module: Box::new(module_instance.clone()),
            code: f,
        });

        Ok(f_address)
    }

    fn allocate_host_function(
        &mut self,
        h_f: Box<dyn Fn()>,
        f_idx: u32,
        module_instance: &ModuleInstance,
    ) -> Result<usize> {
        let func_address = self.functions.len();

        let function_type = Self::extract_function_type(module_instance, f_idx)?;

        self.functions.push(FunctionInstance::Host {
            function_type,
            code: h_f,
        });

        Ok(func_address)
    }

    fn allocate_table(&mut self, table_type: TableType, initial_ref: Ref) -> usize {
        let n = table_type.limit.min;

        let table_address = self.tables.len();

        self.tables.push(TableInstance {
            table_type,
            elem: vec![initial_ref; n as usize],
        });

        table_address
    }

    fn allocate_memory(&mut self, memory_type: MemoryType) -> usize {
        let memory_address = self.memories.len();
        let n = memory_type.limit.min as usize * PAGE_SIZE;

        self.memories.push(MemoryInstance {
            memory_type,
            data: vec![0u8; n],
        });

        memory_address
    }

    fn allocate_global(&mut self, global: Global, initializer_value: Value) -> usize {
        let global_address = self.globals.len();

        self.globals.push(GlobalInstance {
            global_type: global.global_type,
            value: initializer_value,
        });

        global_address
    }

    fn allocate_element_segment(
        &mut self,
        element_segment: ElementSegment,
        element_segment_ref: Vec<Ref>,
    ) -> usize {
        let element_segment_address = self.element_segments.len();

        self.element_segments.push(ElementInstance {
            ref_type: element_segment.ref_type,
            elem: element_segment_ref,
        });

        element_segment_address
    }

    fn allocate_data_instance(&mut self, data_segment: DataSegment) -> usize {
        let data_address = self.data_segments.len();

        self.data_segments.push(DataInstance {
            data: data_segment.bytes,
        });

        data_address
    }

    fn allocate_tag(&mut self, tag_type: FunctionType) -> usize {
        let addr = self.tags.len();
        self.tags.push(TagInstance { tag_type });

        addr
    }

    pub fn allocate_module(
        &mut self,
        module: Module,
        extern_addrs: Vec<ExternalValue>,
        initial_global_values: Vec<Value>,
        initial_table_refs: Vec<Ref>,
        element_segment_refs: Vec<Vec<Ref>>,
    ) -> Result<ModuleInstance> {
        // step 1
        let mut module_instance = ModuleInstance::new(module.types);

        // step 2-6
        for addr in extern_addrs {
            match addr {
                ExternalValue::Function { addr } => module_instance.function_addrs.push(addr),
                ExternalValue::Table { addr } => module_instance.table_addrs.push(addr),
                ExternalValue::Memory { addr } => module_instance.mem_addrs.push(addr),
                ExternalValue::Global { addr } => module_instance.global_addrs.push(addr),
                ExternalValue::Tag { addr } => module_instance.tag_addrs.push(addr),
            }
        }

        // step 7
        let _function_addresses = (0..module.functions.len()).map(|i| self.functions.len() + i);

        // step 8-24 are just extracting fields

        // step 25-26
        for tag in &module.tags {
            let tag_type = Self::extract_function_type(&module_instance, tag.type_index)?;
            let addr = self.allocate_tag(tag_type);
            module_instance.tag_addrs.push(addr);
        }

        // step 27-28
        module_instance.global_addrs.extend(
            module
                .globals
                .into_iter()
                .zip(initial_global_values)
                .map(|(global, init_val)| self.allocate_global(global, init_val)),
        );

        // step 29-30
        module_instance
            .mem_addrs
            .extend(module.mems.into_iter().map(|m| self.allocate_memory(m)));

        // step 31-32
        module_instance.table_addrs.extend(
            module
                .tables
                .into_iter()
                .zip(initial_table_refs)
                .map(|(td, ref_t)| self.allocate_table(td.table_type, ref_t)),
        );

        // step 35-36
        module_instance.data_addrs.extend(
            module
                .data_segments
                .into_iter()
                .map(|ds| self.allocate_data_instance(ds)),
        );

        // step 37-38
        for (elem, refs) in module
            .element_segments
            .into_iter()
            .zip(element_segment_refs)
        {
            let addr = self.allocate_element_segment(elem, refs);
            module_instance.elem_addrs.push(addr);
        }

        // step 39 no op
        // step 40-42
        // Pre-compute all function addresses so the module instance has the
        // complete function_addrs before any FunctionInstance snapshots it.
        let first_func_addr = self.functions.len();
        let num_funcs = module.functions.len();
        for i in 0..num_funcs {
            module_instance.function_addrs.push(first_func_addr + i);
        }
        for func in module.functions {
            self.allocate_function(func, &module_instance)?;
        }

        // 43: assertion holds bwo construction

        // step 33-34 (after all addrs are populated)
        for export in module.exports {
            let extern_value = match export.description {
                ExportDescription::Func(x) => ExternalValue::Function {
                    addr: *module_instance
                        .function_addrs
                        .get(x as usize)
                        .ok_or_else(|| anyhow!("oob"))?,
                },
                ExportDescription::Table(x) => ExternalValue::Table {
                    addr: *module_instance
                        .table_addrs
                        .get(x as usize)
                        .ok_or_else(|| anyhow!("oob"))?,
                },
                ExportDescription::Mem(x) => ExternalValue::Memory {
                    addr: *module_instance
                        .mem_addrs
                        .get(x as usize)
                        .ok_or_else(|| anyhow!("oob"))?,
                },
                ExportDescription::Global(x) => ExternalValue::Global {
                    addr: *module_instance
                        .global_addrs
                        .get(x as usize)
                        .ok_or_else(|| anyhow!("oob"))?,
                },
                ExportDescription::Tag(x) => ExternalValue::Tag {
                    addr: *module_instance
                        .tag_addrs
                        .get(x as usize)
                        .ok_or_else(|| anyhow!("oob"))?,
                },
            };

            module_instance.exports.push(ExportInstance {
                name: export.name,
                value: extern_value,
            });
        }

        Ok(module_instance)
    }
}
