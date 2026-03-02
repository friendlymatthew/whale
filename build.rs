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

                    _ => {}
                }
            }

            for (midx, steps) in &modules {
                if steps.is_empty() {
                    continue;
                }
                let test_name = format!("{}_{}", safe_name, midx);
                let steps_code = steps.join("\n");
                all_tests.push_str(&format!(
                    concat!(
                        "#[test]\n",
                        "fn {test_name}() {{\n",
                        "    let wasm_bytes: &[u8] = include_bytes!(concat!(env!(\"OUT_DIR\"), \"/wasm/{file}_{midx}.wasm\"));\n",
                        "    let mut store = Store::new();\n",
                        "    let imports = setup_spectest_imports(&mut store, wasm_bytes);\n",
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
                _ => None,
            })
            .collect();
        rendered.map(|v| v.join(", "))
    }
}
