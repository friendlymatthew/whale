// use gabagool::{ExecutionState, Interpreter, Value};

// #[test]
// fn pause_resume_snapshot() {
//     let wasm_bytes =
// std::fs::read("stair_climb.wasm").expect("stair_climb.wasm not found");

//     let mut reference = Interpreter::new(&wasm_bytes).unwrap();
//     let full_result = reference
//         .invoke("stair_climb", vec![Value::I32(4)])
//         .unwrap()
//         .into_completed()
//         .unwrap();

//     let mut interp = Interpreter::new(&wasm_bytes).unwrap();
//     interp.set_fuel(50);

//     let state = interp
//         .invoke("stair_climb", vec![Value::I32(4)])
//         .unwrap();
//     assert_eq!(state, ExecutionState::FuelExhausted);

//     let snapshot = interp.snapshot().unwrap();

//     // shutdown!

//     let mut restored = Interpreter::from_snapshot(&snapshot).unwrap();
//     restored.set_fuel(10000);

//     let resumed_result =
// restored.resume().unwrap().into_completed().unwrap();

//     assert_eq!(full_result, resumed_result);
// }
