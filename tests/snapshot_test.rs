use gabagool::{ExecutionState, Interpreter, Store, Value};

fn unwrap_completed(state: ExecutionState) -> Vec<Value> {
    match state {
        ExecutionState::Completed(v) => v,
        ExecutionState::FuelExhausted => panic!("expected Completed, got FuelExhausted"),
    }
}

#[test]
fn pause_resume_snapshot() {
    let wasm_bytes = std::fs::read("stair_climb.wasm").expect("stair_climb.wasm not found");

    let store = Store::new();
    let mut reference = Interpreter::instantiate(store, &wasm_bytes, vec![]).unwrap();
    let full_result = unwrap_completed(
        reference
            .invoke_export("stair_climb", vec![Value::I32(4)])
            .unwrap(),
    );

    let store = Store::new();
    let mut interp = Interpreter::instantiate(store, &wasm_bytes, vec![]).unwrap();
    interp.set_fuel(50);

    let state = interp
        .invoke_export("stair_climb", vec![Value::I32(4)])
        .unwrap();
    assert_eq!(state, ExecutionState::FuelExhausted);

    let snapshot = interp.snapshot().unwrap();

    // shutdown!

    let mut restored = Interpreter::from_snapshot(&snapshot).unwrap();
    restored.set_fuel(10000);

    let resumed_result = unwrap_completed(restored.resume().unwrap());

    assert_eq!(full_result, resumed_result);
}
