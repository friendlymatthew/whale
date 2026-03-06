use anyhow::{Context, Result};
use gabagool::{CompiledInterpreter, RawValue, ValueType};
use std::fs;
use std::path::PathBuf;

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

    let mut interpreter = CompiledInterpreter::new(&wasm_bytes)?;

    let param_types = interpreter.get_param_types(&func_name)?;

    let values = param_types
        .iter()
        .zip(&cli_args)
        .map(|(vt, arg)| parse_value(vt, arg))
        .collect::<Result<Vec<_>>>()?;

    let results = interpreter.invoke(&func_name, values)?.into_completed()?;
    println!("{:?}", results);

    Ok(())
}

fn parse_value(value_type: &ValueType, s: &str) -> Result<RawValue> {
    match value_type {
        ValueType::I32 => Ok(RawValue::from(s.parse::<i32>().context("invalid i32")?)),
        ValueType::I64 => Ok(RawValue::from(s.parse::<i64>().context("invalid i64")?)),
        ValueType::F32 => Ok(RawValue::from(s.parse::<f32>().context("invalid f32")?)),
        ValueType::F64 => Ok(RawValue::from(s.parse::<f64>().context("invalid f64")?)),
        _ => anyhow::bail!("unsupported parameter type"),
    }
}
