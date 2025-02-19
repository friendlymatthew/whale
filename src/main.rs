use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use whale::Interpreter;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);

    let wasm_file = args.next().context("Provide .wasm file.")?;
    let wasm_file = PathBuf::from(&wasm_file);
    let wasm_bytes = fs::read(&wasm_file)?;

    let _ = Interpreter::execute(&wasm_bytes)?;

    Ok(())
}
