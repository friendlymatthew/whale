use gabagool::{
    parser::Parser, AddrType, CompositeType, ExportInstance, ExternalValue,
    FunctionInstance, GlobalInstance, GlobalType, ImportDescription, Instance, Limit,
    MemoryInstance, MemoryType, Module, RawValue, Ref, Store, ValueType,
};

#[derive(Debug)]
enum NanPat<T> {
    CanonicalNan,
    ArithmeticNan,
    Value(T),
}

#[derive(Debug)]
enum ExpectedRef {
    Null,
    Extern(Option<u32>),
    Func,
    NonNull,
    I31,
}

#[derive(Debug)]
enum ExpectedValue {
    I32(i32),
    I64(i64),
    F32(NanPat<u32>),
    F64(NanPat<u64>),
    Ref(ExpectedRef),
}

/// Create a spectest-style memory: 1 page initial, 2 pages max
/// (matching the standard WebAssembly spectest module)
fn create_spectest_memory(store: &mut Store, mt: &MemoryType) -> ExternalValue {
    let addr = store.memories.len();
    // spectest module always provides memory with 1 page initial, 2 pages max
    store.memories.push(MemoryInstance {
        memory_type: MemoryType {
            addr_type: mt.addr_type,
            limit: Limit { min: 1, max: 2 },
        },
        data: vec![0u8; 65536],
    });
    ExternalValue::Memory { addr }
}

fn setup_spectest_imports(store: &mut Store, module: &Module) -> Vec<ExternalValue> {
    module
        .import_declarations()
        .iter()
        .map(|import| match &import.description {
            ImportDescription::Global(gt) => {
                let value = match gt.value_type {
                    ValueType::I32 => RawValue::from(666i32),
                    ValueType::I64 => RawValue::from(666i64),
                    ValueType::F32 => RawValue::from(666.6f32),
                    ValueType::F64 => RawValue::from(666.6f64),
                    _ => RawValue::from(0i32),
                };
                let addr = store.globals.len();
                store.globals.push(GlobalInstance {
                    global_type: GlobalType {
                        value_type: gt.value_type.clone(),
                        mutability: gt.mutability.clone(),
                    },
                    value,
                });
                ExternalValue::Global { addr }
            }
            ImportDescription::Mem(mt) => create_spectest_memory(store, mt),
            ImportDescription::Table(_) => {
                let addr = store.tables.len();
                store.tables.push(gabagool::TableInstance {
                    table_type: gabagool::TableType {
                        element_reference_type: gabagool::RefType::FuncRef,
                        addr_type: AddrType::I32,
                        limit: Limit { min: 10, max: 20 },
                    },
                    elem: vec![Ref::Null; 10],
                });
                ExternalValue::Table { addr }
            }
            ImportDescription::Func(type_idx) => {
                let addr = store.functions.len();
                let function_type = match &module.types()[*type_idx as usize].composite_type {
                    CompositeType::Func(ft) => ft.clone(),
                    _ => panic!("expected function type at index {}", type_idx),
                };
                store.functions.push(FunctionInstance::Host {
                    function_type,
                    code: Box::new(|| {}),
                });
                ExternalValue::Function { addr }
            }
            ImportDescription::Tag(_) => {
                let addr = store.tags.len();
                ExternalValue::Tag { addr }
            }
        })
        .collect()
}

fn spec_step_assert_return(
    store: &mut Store,
    instance: Instance,
    name: &str,
    args: &[RawValue],
    expected: &[ExpectedValue],
    step: usize,
    failures: &mut Vec<String>,
) {
    match store
        .invoke(instance, name, args.to_vec())
        .and_then(|s| s.into_completed())
    {
        Ok(actual) => {
            if !values_match(expected, &actual) {
                failures.push(format!(
                    "step {} assert_return(\"{}\", {:?}): expected {:?}, got {:?}",
                    step, name, args, expected, actual
                ));
            }
        }
        Err(e) => {
            failures.push(format!(
                "step {} assert_return(\"{}\", {:?}): unexpected error: {}",
                step, name, args, e
            ));
        }
    }
}

fn spec_step_assert_trap(
    store: &mut Store,
    instance: Instance,
    name: &str,
    args: &[RawValue],
    step: usize,
    failures: &mut Vec<String>,
) {
    if let Ok(results) = store
        .invoke(instance, name, args.to_vec())
        .and_then(|s| s.into_completed())
    {
        failures.push(format!(
            "step {} assert_trap(\"{}\", {:?}): expected trap, got {:?}",
            step, name, args, results
        ));
    }
}

fn spec_step_invoke(store: &mut Store, instance: Instance, name: &str, args: &[RawValue]) {
    let _ = store.invoke(instance, name, args.to_vec());
}

fn values_match(expected: &[ExpectedValue], actual: &[RawValue]) -> bool {
    if expected.len() != actual.len() {
        return false;
    }

    expected
        .iter()
        .zip(actual.iter())
        .all(|(exp, act)| match exp {
            ExpectedValue::I32(e) => *e == act.as_i32(),
            ExpectedValue::I64(e) => *e == act.as_i64(),
            ExpectedValue::F32(pat) => {
                let a = act.as_f32();
                match pat {
                    NanPat::CanonicalNan => a.is_nan() && (a.to_bits() & 0x003F_FFFF == 0),
                    NanPat::ArithmeticNan => a.is_nan(),
                    NanPat::Value(e) => a.to_bits() == *e,
                }
            }
            ExpectedValue::F64(pat) => {
                let a = act.as_f64();
                match pat {
                    NanPat::CanonicalNan => a.is_nan() && (a.to_bits() & 0x0007_FFFF_FFFF_FFFF == 0),
                    NanPat::ArithmeticNan => a.is_nan(),
                    NanPat::Value(e) => a.to_bits() == *e,
                }
            }
            ExpectedValue::Ref(exp_ref) => {
                let act_ref = act.as_ref();
                match (exp_ref, act_ref) {
                    (ExpectedRef::Null, Ref::Null) => true,
                    (ExpectedRef::Extern(Some(n)), Ref::RefExtern(m)) => *n as usize == m,
                    (ExpectedRef::Extern(None), Ref::RefExtern(_)) => true,
                    (ExpectedRef::Func, Ref::FunctionAddr(_)) => true,
                    (ExpectedRef::NonNull, r) => r != Ref::Null,
                    (ExpectedRef::I31, Ref::I31(_)) => true,
                    _ => false,
                }
            }
        })
}

fn resolve_imports_with_registered(
    store: &mut Store,
    module: &Module,
    registered_exports: &[(&str, &[ExportInstance])],
) -> Vec<ExternalValue> {
    module
        .import_declarations()
        .iter()
        .map(|import| {
            // Try to find the import in registered modules
            for &(reg_name, exports) in registered_exports {
                if import.module == reg_name {
                    for export in exports {
                        if export.name == import.name {
                            return export.value.clone();
                        }
                    }
                }
            }
            // Fall back to spectest-style import
            match &import.description {
                ImportDescription::Global(gt) => {
                    let value = match gt.value_type {
                        ValueType::I32 => RawValue::from(666i32),
                        ValueType::I64 => RawValue::from(666i64),
                        ValueType::F32 => RawValue::from(666.6f32),
                        ValueType::F64 => RawValue::from(666.6f64),
                        _ => RawValue::from(0i32),
                    };
                    let addr = store.globals.len();
                    store.globals.push(GlobalInstance {
                        global_type: GlobalType {
                            value_type: gt.value_type.clone(),
                            mutability: gt.mutability.clone(),
                        },
                        value,
                    });
                    ExternalValue::Global { addr }
                }
                ImportDescription::Mem(mt) => create_spectest_memory(store, mt),
                ImportDescription::Table(_) => {
                    let addr = store.tables.len();
                    store.tables.push(gabagool::TableInstance {
                        table_type: gabagool::TableType {
                            element_reference_type: gabagool::RefType::FuncRef,
                            addr_type: AddrType::I32,
                            limit: Limit { min: 10, max: 20 },
                        },
                        elem: vec![Ref::Null; 10],
                    });
                    ExternalValue::Table { addr }
                }
                ImportDescription::Func(type_idx) => {
                    let addr = store.functions.len();
                    let function_type = match &module.types()[*type_idx as usize].composite_type {
                        CompositeType::Func(ft) => ft.clone(),
                        _ => panic!("expected function type at index {}", type_idx),
                    };
                    store.functions.push(FunctionInstance::Host {
                        function_type,
                        code: Box::new(|| {}),
                    });
                    ExternalValue::Function { addr }
                }
                ImportDescription::Tag(_) => {
                    let addr = store.tags.len();
                    ExternalValue::Tag { addr }
                }
            }
        })
        .collect()
}

include!(concat!(env!("OUT_DIR"), "/spec_tests_generated.rs"));
