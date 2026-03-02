# gabagool

A WebAssembly interpreter written from scratch.

This project aims to build a fully spec-compliant interpreter whose entire execution state can be serialized, suspended, and restored. This enables fast cold starts, cross-machine migrations, deterministic replay, and time-travel debugging.

```sh
# write your module in WAT
cat stair_climb.wat
# compile to wasm (requires wabt: brew install wabt)
wat2wasm stair_climb.wat -o stair_climb.wasm

cargo r -- stair_climb.wasm stair_climb 20
```

# Status

`gabagool` is tested against the [WebAssembly spec test suite](https://github.com/WebAssembly/spec/tree/main/test/core), which generates 32,490 individual assertions from the official `.wast` files. It currently passes 16,353.

```sh
uv run download-spec-tests.py
cargo t --features spec-tests
```

# Reading

https://webassembly.github.io/spec/core/<br>
