use criterion::{criterion_group, criterion_main, Criterion};
use gabagool::{Module, RawValue, Store};
use std::fs;

fn load_and_instantiate(wasm_bytes: &[u8]) -> (Store, gabagool::Instance) {
    let module = Module::new(wasm_bytes).unwrap();
    let mut store = Store::new();
    let instance = store.instantiate(&module, vec![]).unwrap();
    (store, instance)
}

fn bench_fibonacci(c: &mut Criterion) {
    let wasm = fs::read("programs/fibonacci.wasm").unwrap();

    c.bench_function("fib(30)", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "fib", vec![RawValue::from(30i32)])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), 832040);
        });
    });
}

fn bench_matrix(c: &mut Criterion) {
    let wasm = fs::read("programs/matrix.wasm").unwrap();

    c.bench_function("matrix_multiply_64x64", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "matrix_bench", vec![])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), 626828219);
        });
    });
}

fn bench_sieve(c: &mut Criterion) {
    let wasm = fs::read("programs/sieve.wasm").unwrap();

    c.bench_function("sieve_100k", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "count_primes", vec![])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), 9592);
        });
    });
}

fn bench_sort(c: &mut Criterion) {
    let wasm = fs::read("programs/sort.wasm").unwrap();

    c.bench_function("quicksort_4096", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "sort_bench", vec![])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), 67582043);
        });
    });
}

fn bench_ackermann(c: &mut Criterion) {
    let wasm = fs::read("programs/ackermann.wasm").unwrap();

    c.bench_function("ackermann(3,5)", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "ackermann_bench", vec![])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), 253);
        });
    });
}

fn bench_mandelbrot(c: &mut Criterion) {
    let wasm = fs::read("programs/mandelbrot.wasm").unwrap();

    c.bench_function("mandelbrot_128x128", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "mandelbrot_bench", vec![])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), 429384);
        });
    });
}

fn bench_nbody(c: &mut Criterion) {
    let wasm = fs::read("programs/nbody.wasm").unwrap();

    c.bench_function("nbody_100k_steps", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "nbody_bench", vec![])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), -169079859);
        });
    });
}

fn bench_sha256(c: &mut Criterion) {
    let wasm = fs::read("programs/sha256.wasm").unwrap();

    c.bench_function("sha256_1kb_x1000", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "sha256_bench", vec![])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), -1206794323);
        });
    });
}

fn bench_switch_dispatch(c: &mut Criterion) {
    let wasm = fs::read("programs/switch_dispatch.wasm").unwrap();

    c.bench_function("switch_dispatch_1m", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "switch_bench", vec![])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), 1);
        });
    });
}

fn bench_indirect_call(c: &mut Criterion) {
    let wasm = fs::read("programs/indirect_call.wasm").unwrap();

    c.bench_function("indirect_call_1m", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "indirect_call_bench", vec![])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), 1);
        });
    });
}

fn bench_call_chain(c: &mut Criterion) {
    let wasm = fs::read("programs/call_chain.wasm").unwrap();

    c.bench_function("call_chain_100k", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "call_chain_bench", vec![])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), 706982704);
        });
    });
}

fn bench_binary_search(c: &mut Criterion) {
    let wasm = fs::read("programs/binary_search.wasm").unwrap();

    c.bench_function("binary_search_100k", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "binary_search_bench", vec![])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), 33298);
        });
    });
}

fn bench_linked_list(c: &mut Criterion) {
    let wasm = fs::read("programs/linked_list.wasm").unwrap();

    c.bench_function("linked_list_16k_x200", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "linked_list_bench", vec![])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), 1072103424);
        });
    });
}

fn bench_bulk_memory(c: &mut Criterion) {
    let wasm = fs::read("programs/bulk_memory.wasm").unwrap();

    c.bench_function("bulk_memory_64kb_x500", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "bulk_memory_bench", vec![])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), 8364412);
        });
    });
}

fn bench_matrix_chain(c: &mut Criterion) {
    let wasm = fs::read("programs/matrix_chain.wasm").unwrap();

    c.bench_function("matrix_chain_dp_200", |b| {
        b.iter(|| {
            let (mut store, instance) = load_and_instantiate(&wasm);
            let result = store
                .invoke(instance, "matrix_chain_bench", vec![])
                .unwrap()
                .into_completed()
                .unwrap();
            assert_eq!(result[0].as_i32(), 6885669);
        });
    });
}

criterion_group!(
    benches,
    // Original
    bench_fibonacci,
    bench_matrix,
    bench_sieve,
    bench_sort,
    // Deep recursion
    bench_ackermann,
    // Floating point
    bench_mandelbrot,
    bench_nbody,
    // Bitwise ops
    bench_sha256,
    // Control flow
    bench_switch_dispatch,
    // Function calls
    bench_indirect_call,
    bench_call_chain,
    // Memory access patterns
    bench_binary_search,
    bench_linked_list,
    bench_bulk_memory,
    // Dynamic programming
    bench_matrix_chain,
);
criterion_main!(benches);
