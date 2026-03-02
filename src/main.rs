use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use gabagool::{ExecutionState, Interpreter, Store, Value, ValueType};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);

    let wasm_file = args
        .next()
        .context("gabagool <file.wasm> <func_name> [args...]")?;
    let func_name = args
        .next()
        .context("gabagool <file.wasm> <func_name> [args...]")?;
    let cli_args: Vec<String> = args.collect();

    let wasm_file = PathBuf::from(&wasm_file);
    let wasm_bytes = fs::read(&wasm_file)?;

    let store = Store::new();
    let mut interpreter = Interpreter::instantiate(store, &wasm_bytes, vec![])?;

    let func_addr = interpreter.get_export_func_addr(&func_name)?;
    let param_types = interpreter.get_func_param_types(func_addr)?;

    let values = param_types
        .iter()
        .zip(&cli_args)
        .map(|(vt, arg)| parse_value(vt, arg))
        .collect::<Result<Vec<_>>>()?;

    match interpreter.invoke_export(&func_name, values)? {
        ExecutionState::Completed(results) => println!("{:?}", results),
        ExecutionState::FuelExhausted => println!("Execution paused: fuel exhausted"),
    }

    Ok(())
}

fn parse_value(value_type: &ValueType, s: &str) -> Result<Value> {
    match value_type {
        ValueType::I32 => Ok(Value::I32(s.parse().context("invalid i32")?)),
        ValueType::I64 => Ok(Value::I64(s.parse().context("invalid i64")?)),
        ValueType::F32 => Ok(Value::F32(s.parse().context("invalid f32")?)),
        ValueType::F64 => Ok(Value::F64(s.parse().context("invalid f64")?)),
        _ => anyhow::bail!("unsupported parameter type"),
    }
}
