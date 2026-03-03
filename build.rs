fn main() {
    #[cfg(feature = "spec-tests")]
    spec_tests::generate();

    #[cfg(not(feature = "spec-tests"))]
    {
        let out_dir = std::env::var("OUT_DIR").unwrap();
        std::fs::write(
            std::path::Path::new(&out_dir).join("spec_tests_generated.rs"),
            "",
        )
        .unwrap();
    }
}

#[cfg(feature = "spec-tests")]
mod spec_tests {
    use std::env;
    use std::fs;
    use std::path::Path;

    use wast::core::{NanPattern, WastArgCore, WastRetCore};
    use wast::parser::ParseBuffer;
    use wast::{Wast, WastArg, WastDirective, WastExecute, WastRet};

    pub fn generate() {
        println!("cargo::rerun-if-changed=tests/spec");

        let out_dir = env::var("OUT_DIR").unwrap();
        let wasm_dir = Path::new(&out_dir).join("wasm");
        fs::create_dir_all(&wasm_dir).unwrap();

        let spec_dir = Path::new("tests/spec");
        if !spec_dir.exists() {
            fs::write(Path::new(&out_dir).join("spec_tests_generated.rs"), "").unwrap();
            return;
        }

        let mut all_tests = String::new();

        let entries = fs::read_dir(spec_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "wast"));

        for entry in entries {
            let path = entry.path();
            let file_stem = path.file_stem().unwrap().to_str().unwrap();
            let safe_name = file_stem.replace('-', "_");

            let Ok(contents) = fs::read_to_string(&path) else {
                println!("cargo::warning=skipping {}: failed to read", path.display());
                continue;
            };
            let Ok(buf) = ParseBuffer::new(&contents) else {
                println!("cargo::warning=skipping {}: failed to lex", path.display());
                continue;
            };
            let Ok(wast) = wast::parser::parse::<Wast>(&buf) else {
                println!(
                    "cargo::warning=skipping {}: failed to parse",
                    path.display()
                );
                continue;
            };

            let mut module_idx: i32 = -1;
            let mut modules = Vec::new();
            // Track (register ...) directives: maps registered name -> module index
            let mut registered: Vec<(String, i32)> = Vec::new();
            let mut malformed_idx: u32 = 0;
            let mut unlinkable_idx: u32 = 0;
            let mut trap_module_idx: u32 = 0;

            for directive in wast.directives {
                match directive {
                    WastDirective::Module(mut wat) => {
                        module_idx += 1;
                        if let Ok(bytes) = wat.encode() {
                            let wasm_path =
                                wasm_dir.join(format!("{}_{}.wasm", safe_name, module_idx));
                            fs::write(&wasm_path, bytes).unwrap();
                        }
                        modules.push((module_idx, Vec::new()));
                    }

                    WastDirective::Register { name, .. } => {
                        if module_idx >= 0 {
                            registered.push((name.to_string(), module_idx));
                        }
                    }

                    WastDirective::AssertReturn { exec, results, .. } => {
                        if module_idx < 0 {
                            continue;
                        }
                        let WastExecute::Invoke(ref invoke) = exec else {
                            continue;
                        };
                        if invoke.module.is_some() {
                            continue;
                        }

                        let Some(args_code) = render_args(&invoke.args) else {
                            continue;
                        };
                        let Some(expected_code) = render_expected(&results) else {
                            continue;
                        };

                        let steps = &mut modules.last_mut().unwrap().1;
                        let step_idx = steps.len();
                        steps.push(format!(
                            "    spec_step_assert_return(&mut interp, \"{}\", &[{}], &[{}], {}, &mut failures);",
                            invoke.name, args_code, expected_code, step_idx
                        ));
                    }

                    WastDirective::AssertTrap { exec, .. } => {
                        match exec {
                            WastExecute::Invoke(ref invoke) => {
                                if module_idx < 0 {
                                    continue;
                                }
                                if invoke.module.is_some() {
                                    continue;
                                }

                                let Some(args_code) = render_args(&invoke.args) else {
                                    continue;
                                };

                                let steps = &mut modules.last_mut().unwrap().1;
                                let step_idx = steps.len();
                                steps.push(format!(
                                    "    spec_step_assert_trap(&mut interp, \"{}\", &[{}], {}, &mut failures);",
                                    invoke.name, args_code, step_idx
                                ));
                            }
                            WastExecute::Wat(mut wat) => {
                                let Ok(bytes) = wat.encode() else {
                                    trap_module_idx += 1;
                                    continue;
                                };
                                let wasm_path = wasm_dir.join(format!(
                                    "trap_module_{}_{}.wasm",
                                    safe_name, trap_module_idx
                                ));
                                fs::write(&wasm_path, bytes).unwrap();
                                let test_name =
                                    format!("trap_module_{}_{}", safe_name, trap_module_idx);
                                all_tests.push_str(&format!(
                                    concat!(
                                        "#[test]\n",
                                        "fn {test_name}() {{\n",
                                        "    let wasm_bytes: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/wasm/trap_module_{file}_{idx}.wasm\"));\n",
                                        "    let mut store = Store::new();\n",
                                        "    let imports = setup_spectest_imports(&mut store, wasm_bytes);\n",
                                        "    let result = Interpreter::instantiate(store, wasm_bytes, imports);\n",
                                        "    assert!(result.is_err(), \"expected module instantiation to trap, but it succeeded\");\n",
                                        "}}\n",
                                    ),
                                    test_name = test_name,
                                    file = safe_name,
                                    idx = trap_module_idx,
                                ));
                                trap_module_idx += 1;
                            }
                            _ => {}
                        }
                    }

                    WastDirective::AssertExhaustion { call: ref invoke, .. } => {
                        if module_idx < 0 {
                            continue;
                        }
                        if invoke.module.is_some() {
                            continue;
                        }

                        let Some(args_code) = render_args(&invoke.args) else {
                            continue;
                        };

                        let steps = &mut modules.last_mut().unwrap().1;
                        let step_idx = steps.len();
                        steps.push(format!(
                            "    spec_step_assert_trap(&mut interp, \"{}\", &[{}], {}, &mut failures);",
                            invoke.name, args_code, step_idx
                        ));
                    }

                    WastDirective::Invoke(ref invoke) => {
                        if module_idx < 0 {
                            continue;
                        }
                        if invoke.module.is_some() {
                            continue;
                        }

                        let Some(args_code) = render_args(&invoke.args) else {
                            continue;
                        };

                        let steps = &mut modules.last_mut().unwrap().1;
                        steps.push(format!(
                            "    spec_step_invoke(&mut interp, \"{}\", &[{}]);",
                            invoke.name, args_code
                        ));
                    }

                    WastDirective::AssertMalformed { mut module, .. } => {
                        let Ok(bytes) = module.encode() else {
                            malformed_idx += 1;
                            continue;
                        };
                        let wasm_path = wasm_dir.join(format!(
                            "malformed_{}_{}.wasm",
                            safe_name, malformed_idx
                        ));
                        fs::write(&wasm_path, bytes).unwrap();
                        let test_name =
                            format!("malformed_{}_{}", safe_name, malformed_idx);
                        all_tests.push_str(&format!(
                            concat!(
                                "#[test]\n",
                                "fn {test_name}() {{\n",
                                "    let wasm_bytes: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/wasm/malformed_{file}_{idx}.wasm\"));\n",
                                "    let result = Parser::new(wasm_bytes).parse_module();\n",
                                "    assert!(result.is_err(), \"expected malformed module to fail parsing, but it succeeded\");\n",
                                "}}\n",
                            ),
                            test_name = test_name,
                            file = safe_name,
                            idx = malformed_idx,
                        ));
                        malformed_idx += 1;
                    }

                    WastDirective::AssertUnlinkable { mut module, .. } => {
                        let Ok(bytes) = module.encode() else {
                            unlinkable_idx += 1;
                            continue;
                        };
                        let wasm_path = wasm_dir.join(format!(
                            "unlinkable_{}_{}.wasm",
                            safe_name, unlinkable_idx
                        ));
                        fs::write(&wasm_path, bytes).unwrap();
                        let test_name =
                            format!("unlinkable_{}_{}", safe_name, unlinkable_idx);
                        all_tests.push_str(&format!(
                            concat!(
                                "#[test]\n",
                                "fn {test_name}() {{\n",
                                "    let wasm_bytes: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/wasm/unlinkable_{file}_{idx}.wasm\"));\n",
                                "    let mut store = Store::new();\n",
                                "    let imports = setup_spectest_imports(&mut store, wasm_bytes);\n",
                                "    let result = Interpreter::instantiate(store, wasm_bytes, imports);\n",
                                "    assert!(result.is_err(), \"expected unlinkable module to fail instantiation, but it succeeded\");\n",
                                "}}\n",
                            ),
                            test_name = test_name,
                            file = safe_name,
                            idx = unlinkable_idx,
                        ));
                        unlinkable_idx += 1;
                    }

                    _ => {}
                }
            }

            // Build a map: module_idx -> list of registered modules that precede it
            let mut registered_before: std::collections::BTreeMap<i32, Vec<(String, i32)>> =
                std::collections::BTreeMap::new();
            for &(midx, ref _steps) in &modules {
                let deps: Vec<(String, i32)> = registered
                    .iter()
                    .filter(|(_, ridx)| *ridx < midx)
                    .cloned()
                    .collect();
                if !deps.is_empty() {
                    registered_before.insert(midx, deps);
                }
            }

            for (midx, steps) in &modules {
                if steps.is_empty() {
                    continue;
                }
                let test_name = format!("{}_{}", safe_name, midx);
                let steps_code = steps.join("\n");

                // Generate prerequisite setup code for registered modules
                let deps = registered_before.get(midx);
                let has_deps = deps.is_some_and(|d| !d.is_empty());

                let setup_code = if has_deps {
                    let deps = deps.unwrap();
                    // Collect unique prerequisite module indices (in order)
                    let mut prereq_indices: Vec<i32> = Vec::new();
                    for (_, dep_idx) in deps {
                        if !prereq_indices.contains(dep_idx) {
                            prereq_indices.push(*dep_idx);
                        }
                    }
                    prereq_indices.sort();

                    let mut setup = String::new();
                    // Instantiate each prerequisite module
                    for pidx in &prereq_indices {
                        setup.push_str(&format!(
                            concat!(
                                "    let prereq_wasm_{pidx}: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/wasm/{file}_{pidx}.wasm\"));\n",
                                "    let prereq_imports_{pidx} = setup_spectest_imports(&mut store, prereq_wasm_{pidx});\n",
                                "    let prereq_interp_{pidx} = Interpreter::instantiate(store, prereq_wasm_{pidx}, prereq_imports_{pidx}).unwrap();\n",
                                "    let prereq_exports_{pidx}: Vec<ExportInstance> = prereq_interp_{pidx}.module_exports().to_vec();\n",
                                "    store = prereq_interp_{pidx}.into_store();\n",
                            ),
                            pidx = pidx,
                            file = safe_name,
                        ));
                    }

                    // Build the registered_exports vec
                    setup.push_str("    let registered_exports: Vec<(&str, &[ExportInstance])> = vec![");
                    for (name, dep_idx) in deps {
                        setup.push_str(&format!(
                            "(\"{}\", &prereq_exports_{}), ",
                            name, dep_idx
                        ));
                    }
                    setup.push_str("];\n");

                    // Resolve imports using registered modules
                    setup.push_str(&format!(
                        concat!(
                            "    let imports = resolve_imports_with_registered(&mut store, wasm_bytes, &registered_exports);\n",
                        ),
                    ));
                    setup
                } else {
                    "    let imports = setup_spectest_imports(&mut store, wasm_bytes);\n"
                        .to_string()
                };

                all_tests.push_str(&format!(
                    concat!(
                        "#[test]\n",
                        "fn {test_name}() {{\n",
                        "    let wasm_bytes: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/wasm/{file}_{midx}.wasm\"));\n",
                        "    let mut store = Store::new();\n",
                        "{setup}",
                        "    let mut interp = Interpreter::instantiate(store, wasm_bytes, imports).unwrap();\n",
                        "    let mut failures: Vec<String> = Vec::new();\n",
                        "{steps}\n",
                        "    if !failures.is_empty() {{\n",
                        "        panic!(\"{{}} assertion(s) failed in {test_name}:\\n{{}}\", failures.len(), failures.join(\"\\n\"));\n",
                        "    }}\n",
                        "}}\n",
                    ),
                    test_name = test_name,
                    file = safe_name,
                    midx = midx,
                    setup = setup_code,
                    steps = steps_code,
                ));
            }
        }

        fs::write(
            Path::new(&out_dir).join("spec_tests_generated.rs"),
            all_tests,
        )
        .unwrap();
    }

    fn render_i32(v: i32) -> String {
        if v == i32::MIN {
            "i32::MIN".to_string()
        } else {
            format!("{}i32", v)
        }
    }

    fn render_i64(v: i64) -> String {
        if v == i64::MIN {
            "i64::MIN".to_string()
        } else {
            format!("{}i64", v)
        }
    }

    fn render_args(args: &[WastArg]) -> Option<String> {
        let rendered: Option<Vec<String>> = args
            .iter()
            .map(|arg| match arg {
                WastArg::Core(WastArgCore::I32(v)) => {
                    Some(format!("Value::I32({})", render_i32(*v)))
                }
                WastArg::Core(WastArgCore::I64(v)) => {
                    Some(format!("Value::I64({})", render_i64(*v)))
                }
                WastArg::Core(WastArgCore::F32(v)) => {
                    Some(format!("Value::F32(f32::from_bits({}))", v.bits))
                }
                WastArg::Core(WastArgCore::F64(v)) => {
                    Some(format!("Value::F64(f64::from_bits({}))", v.bits))
                }
                WastArg::Core(WastArgCore::RefNull(_)) => {
                    Some("Value::Ref(Ref::Null)".to_string())
                }
                WastArg::Core(WastArgCore::RefExtern(n)) => {
                    Some(format!("Value::Ref(Ref::RefExtern({} as usize))", n))
                }
                WastArg::Core(WastArgCore::RefHost(n)) => {
                    Some(format!("Value::Ref(Ref::RefExtern({} as usize))", n))
                }
                _ => None,
            })
            .collect();
        rendered.map(|v| v.join(", "))
    }

    fn render_expected(results: &[WastRet]) -> Option<String> {
        let rendered: Option<Vec<String>> = results
            .iter()
            .map(|ret| match ret {
                WastRet::Core(WastRetCore::I32(v)) => {
                    Some(format!("ExpectedValue::I32({})", render_i32(*v)))
                }
                WastRet::Core(WastRetCore::I64(v)) => {
                    Some(format!("ExpectedValue::I64({})", render_i64(*v)))
                }
                WastRet::Core(WastRetCore::F32(np)) => Some(match np {
                    NanPattern::CanonicalNan => {
                        "ExpectedValue::F32(NanPat::CanonicalNan)".to_string()
                    }
                    NanPattern::ArithmeticNan => {
                        "ExpectedValue::F32(NanPat::ArithmeticNan)".to_string()
                    }
                    NanPattern::Value(v) => {
                        format!("ExpectedValue::F32(NanPat::Value({}))", v.bits)
                    }
                }),
                WastRet::Core(WastRetCore::F64(np)) => Some(match np {
                    NanPattern::CanonicalNan => {
                        "ExpectedValue::F64(NanPat::CanonicalNan)".to_string()
                    }
                    NanPattern::ArithmeticNan => {
                        "ExpectedValue::F64(NanPat::ArithmeticNan)".to_string()
                    }
                    NanPattern::Value(v) => {
                        format!("ExpectedValue::F64(NanPat::Value({}))", v.bits)
                    }
                }),
                WastRet::Core(WastRetCore::RefNull(_)) => {
                    Some("ExpectedValue::Ref(ExpectedRef::Null)".to_string())
                }
                WastRet::Core(WastRetCore::RefExtern(Some(n))) => {
                    Some(format!("ExpectedValue::Ref(ExpectedRef::Extern(Some({})))", n))
                }
                WastRet::Core(WastRetCore::RefExtern(None)) => {
                    Some("ExpectedValue::Ref(ExpectedRef::Extern(None))".to_string())
                }
                WastRet::Core(WastRetCore::RefHost(n)) => {
                    Some(format!("ExpectedValue::Ref(ExpectedRef::Extern(Some({})))", n))
                }
                WastRet::Core(WastRetCore::RefFunc(_)) => {
                    Some("ExpectedValue::Ref(ExpectedRef::Func)".to_string())
                }
                WastRet::Core(WastRetCore::RefAny
                    | WastRetCore::RefEq
                    | WastRetCore::RefStruct
                    | WastRetCore::RefArray) => {
                    Some("ExpectedValue::Ref(ExpectedRef::NonNull)".to_string())
                }
                WastRet::Core(WastRetCore::RefI31 | WastRetCore::RefI31Shared) => {
                    Some("ExpectedValue::Ref(ExpectedRef::I31)".to_string())
                }
                WastRet::Core(WastRetCore::Either(_)) => None,
                _ => None,
            })
            .collect();
        rendered.map(|v| v.join(", "))
    }
}
