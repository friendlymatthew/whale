use gabagool::{Module, RawValue, Store};

fn run_program(wasm_path: &str, func: &str, args: Vec<RawValue>) -> i32 {
    let wasm = std::fs::read(wasm_path).unwrap();
    let module = Module::new(&wasm).unwrap();
    let mut store = Store::new();
    let instance = store.instantiate(&module, vec![]).unwrap();
    let result = store
        .invoke(instance, func, args)
        .unwrap()
        .into_completed()
        .unwrap();
    result[0].as_i32()
}

fn snapshot_roundtrip(wasm_path: &str, func: &str, args: Vec<RawValue>, fuel: u64) -> i32 {
    let wasm = std::fs::read(wasm_path).unwrap();
    let module = Module::new(&wasm).unwrap();
    let mut store = Store::new();
    let instance = store.instantiate(&module, vec![]).unwrap();

    store.set_fuel(fuel);
    let state = store.invoke(instance, func, args).unwrap();

    assert!(matches!(state, gabagool::ExecutionState::FuelExhausted));
    assert!(store.is_paused());

    let snapshot = store.snapshot();
    let mut restored = Store::from_snapshot(&snapshot);

    restored.set_fuel(u64::MAX);
    let result = restored.resume().unwrap().into_completed().unwrap();
    result[0].as_i32()
}

#[test]
fn fibonacci() {
    assert_eq!(
        run_program(
            "programs/fibonacci.wasm",
            "fib",
            vec![RawValue::from(30i32)]
        ),
        832040
    );
}

#[test]
fn matrix_multiply() {
    assert_eq!(
        run_program("programs/matrix.wasm", "matrix_bench", vec![]),
        626828219
    );
}

#[test]
fn sieve() {
    assert_eq!(
        run_program("programs/sieve.wasm", "count_primes", vec![]),
        9592
    );
}

#[test]
fn quicksort() {
    assert_eq!(
        run_program("programs/sort.wasm", "sort_bench", vec![]),
        67582043
    );
}

#[test]
fn ackermann() {
    assert_eq!(
        run_program("programs/ackermann.wasm", "ackermann_bench", vec![]),
        253
    );
}

#[test]
fn mandelbrot() {
    assert_eq!(
        run_program("programs/mandelbrot.wasm", "mandelbrot_bench", vec![]),
        429384
    );
}

#[test]
fn nbody() {
    assert_eq!(
        run_program("programs/nbody.wasm", "nbody_bench", vec![]),
        -169079859
    );
}

#[test]
fn sha256() {
    assert_eq!(
        run_program("programs/sha256.wasm", "sha256_bench", vec![]),
        -1206794323
    );
}

#[test]
fn switch_dispatch() {
    assert_eq!(
        run_program("programs/switch_dispatch.wasm", "switch_bench", vec![]),
        1
    );
}

#[test]
fn indirect_call() {
    assert_eq!(
        run_program("programs/indirect_call.wasm", "indirect_call_bench", vec![]),
        1
    );
}

#[test]
fn call_chain() {
    assert_eq!(
        run_program("programs/call_chain.wasm", "call_chain_bench", vec![]),
        706982704
    );
}

#[test]
fn binary_search() {
    assert_eq!(
        run_program("programs/binary_search.wasm", "binary_search_bench", vec![]),
        33298
    );
}

#[test]
fn linked_list() {
    assert_eq!(
        run_program("programs/linked_list.wasm", "linked_list_bench", vec![]),
        1072103424
    );
}

#[test]
fn bulk_memory() {
    assert_eq!(
        run_program("programs/bulk_memory.wasm", "bulk_memory_bench", vec![]),
        8364412
    );
}

#[test]
fn matrix_chain() {
    assert_eq!(
        run_program("programs/matrix_chain.wasm", "matrix_chain_bench", vec![]),
        6885669
    );
}

#[test]
fn snapshot_fibonacci() {
    assert_eq!(
        snapshot_roundtrip(
            "programs/fibonacci.wasm",
            "fib",
            vec![RawValue::from(30i32)],
            1000
        ),
        832040
    );
}

#[test]
fn snapshot_matrix_multiply() {
    assert_eq!(
        snapshot_roundtrip("programs/matrix.wasm", "matrix_bench", vec![], 5000),
        626828219
    );
}

#[test]
fn snapshot_sieve() {
    assert_eq!(
        snapshot_roundtrip("programs/sieve.wasm", "count_primes", vec![], 5000),
        9592
    );
}

#[test]
fn snapshot_quicksort() {
    assert_eq!(
        snapshot_roundtrip("programs/sort.wasm", "sort_bench", vec![], 5000),
        67582043
    );
}

#[test]
fn snapshot_ackermann() {
    assert_eq!(
        snapshot_roundtrip("programs/ackermann.wasm", "ackermann_bench", vec![], 1000),
        253
    );
}

#[test]
fn snapshot_mandelbrot() {
    assert_eq!(
        snapshot_roundtrip("programs/mandelbrot.wasm", "mandelbrot_bench", vec![], 5000),
        429384
    );
}

#[test]
fn snapshot_nbody() {
    assert_eq!(
        snapshot_roundtrip("programs/nbody.wasm", "nbody_bench", vec![], 5000),
        -169079859
    );
}

#[test]
fn snapshot_sha256() {
    assert_eq!(
        snapshot_roundtrip("programs/sha256.wasm", "sha256_bench", vec![], 5000),
        -1206794323
    );
}

#[test]
fn snapshot_switch_dispatch() {
    assert_eq!(
        snapshot_roundtrip(
            "programs/switch_dispatch.wasm",
            "switch_bench",
            vec![],
            5000
        ),
        1
    );
}

#[test]
fn snapshot_indirect_call() {
    assert_eq!(
        snapshot_roundtrip(
            "programs/indirect_call.wasm",
            "indirect_call_bench",
            vec![],
            5000
        ),
        1
    );
}

#[test]
fn snapshot_call_chain() {
    assert_eq!(
        snapshot_roundtrip("programs/call_chain.wasm", "call_chain_bench", vec![], 5000),
        706982704
    );
}

#[test]
fn snapshot_binary_search() {
    assert_eq!(
        snapshot_roundtrip(
            "programs/binary_search.wasm",
            "binary_search_bench",
            vec![],
            5000
        ),
        33298
    );
}

#[test]
fn snapshot_linked_list() {
    assert_eq!(
        snapshot_roundtrip(
            "programs/linked_list.wasm",
            "linked_list_bench",
            vec![],
            5000
        ),
        1072103424
    );
}

#[test]
fn snapshot_bulk_memory() {
    assert_eq!(
        snapshot_roundtrip(
            "programs/bulk_memory.wasm",
            "bulk_memory_bench",
            vec![],
            5000
        ),
        8364412
    );
}

#[test]
fn snapshot_matrix_chain() {
    assert_eq!(
        snapshot_roundtrip(
            "programs/matrix_chain.wasm",
            "matrix_chain_bench",
            vec![],
            5000
        ),
        6885669
    );
}
