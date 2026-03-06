# gabagool

A WebAssembly interpreter written from scratch.

This project aims to build a fully spec-compliant, performant interpreter whose entire execution state can be serialized, suspended, and restored.

```rs
use gabagool::{ExecutionState, Interpreter, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let wasm_bytes = std::fs::read("stair_climb.wasm")?;

    // run to completion as a reference
    let full_result = Interpreter::new(&wasm_bytes)?
        .invoke("stair_climb", vec![Value::I32(4)])?
        .into_completed()?;

    // run with limited fuel
    // execution suspends when fuel runs out
    let mut interp = Interpreter::new(&wasm_bytes)?;
    interp.set_fuel(50);

    let state = interp
        .invoke("stair_climb", vec![Value::I32(4)])?;
    assert_eq!(state, ExecutionState::FuelExhausted);

    // snapshot the suspended interpreter to bytes
    let snapshot = interp.snapshot()?;

    // restored on another machine, in another process, whatever
    let mut restored = Interpreter::from_snapshot(&snapshot)?;
    restored.set_fuel(10_000);

    let resumed_result = restored.resume()?.into_completed()?;

    assert_eq!(full_result, resumed_result);

    Ok(())
}
```

# Status

`gabagool` is slow. It is not yet optimized and no serious profiling/benchmarking has been done. That said, the goal is to make `gabagool` as performant as a pure interpreter can be. The most interesting direction is a translation phase that lowers WASM instructions into a compact intermediate representation, designed for efficient dispatch and serialization.

`gabagool` is tested against the [WebAssembly spec test suite](https://github.com/WebAssembly/spec/tree/main/test/core).

1,686 tests pass out of 2,049 (82%). `gabagool` passes on arithmetic, control flow, memory, tables, globals, function references, and imports/exports. It _currently_ fails on garbage collection, exceptions, and tail calls.

Our testing harness uses modules from the test suite that cover execution, traps, resource exhaustion, and rejection (modules that should fail to parse or instantiate). We omit validation, cross-module invocation, and SIMD modules.

```sh
# run the test suite
uv run download-spec-tests.py
cargo t --features spec-tests

# run an example wasm program
cargo r -- stair_climb.wasm stair_climb 20
```

# Reading

https://webassembly.github.io/spec/core/<br>
https://github.com/bytecodealliance/wasmtime/issues/3017<br>
https://github.com/bytecodealliance/wasmtime/issues/4002<br>
