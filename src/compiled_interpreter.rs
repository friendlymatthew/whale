use crate::binary_grammar::{
    AddrType, CompositeType, DataMode, DataSegment, ElementMode, ElementSegment, ImportDescription,
    Instruction, Mutability, ValueType,
};
use crate::execution_grammar::{
    ExportInstance, ExternalValue, FunctionInstance, GlobalInstance, ModuleInstance, Ref,
};
use crate::ir::{CompiledFunction, CompiledModule, Op};
use crate::parser::Parser;
use crate::store::{Store, PAGE_SIZE};
use crate::value_stack::ValueStack;
use crate::{compiler, RawValue};
use anyhow::{anyhow, bail, ensure, Result};
use std::ops::Neg;

#[derive(Debug, Clone)]
pub enum ExecutionState {
    Completed(Vec<RawValue>),
    FuelExhausted,
}

impl ExecutionState {
    pub fn into_completed(self) -> Result<Vec<RawValue>> {
        match self {
            Self::Completed(v) => Ok(v),
            Self::FuelExhausted => bail!("execution paused: fuel exhausted"),
        }
    }
}

macro_rules! pop_val {
    ($self:expr, I32) => {
        $self.stack.pop().as_i32()
    };
    ($self:expr, I64) => {
        $self.stack.pop().as_i64()
    };
    ($self:expr, F32) => {
        $self.stack.pop().as_f32()
    };
    ($self:expr, F64) => {
        $self.stack.pop().as_f64()
    };
    ($self:expr, Ref) => {
        $self.stack.pop().as_ref()
    };
}

macro_rules! binop {
    ($self:expr, $variant:ident, |$b:ident, $a:ident| $expr:expr) => {{
        let $a = pop_val!($self, $variant);
        let $b = pop_val!($self, $variant);
        $self.stack.push($expr);
    }};
}

macro_rules! cmpop {
    ($self:expr, $variant:ident, |$b:ident, $a:ident| $expr:expr) => {{
        let $a = pop_val!($self, $variant);
        let $b = pop_val!($self, $variant);
        $self.stack.push($expr as i32);
    }};
}

macro_rules! mem_load_c {
    ($self:expr, $offset:expr, $memory:expr, $width:literal, |$bytes:ident| $convert:expr) => {{
        let mem_addr = $self.compiled.mem_addrs[$memory as usize];
        let mem = &$self.store.memories[mem_addr];
        let base = $self.stack.pop_address(mem.memory_type.addr_type) as u64;

        let ea = base
            .checked_add($offset as u64)
            .and_then(|v| usize::try_from(v).ok());
        let Some(ea) = ea.filter(|&ea| ea.saturating_add($width) <= mem.data.len()) else {
            bail!("trap: oob memory access");
        };
        let $bytes: [u8; $width] = mem.data[ea..ea + $width].try_into().unwrap();

        $self.stack.push($convert);
    }};
}

macro_rules! mem_store_c {
    ($self:expr, $offset:expr, $memory:expr, $width:literal, |$val:ident| $to_bytes:expr) => {{
        let $val = $self.stack.pop();
        let mem_addr = $self.compiled.mem_addrs[$memory as usize];
        let addr_type = $self.store.memories[mem_addr].memory_type.addr_type;
        let base = $self.stack.pop_address(addr_type) as u64;
        let ea = base
            .checked_add($offset as u64)
            .and_then(|v| usize::try_from(v).ok());
        let mem = &mut $self.store.memories[mem_addr];
        let Some(ea) = ea.filter(|&ea| ea.saturating_add($width) <= mem.data.len()) else {
            bail!("trap: oob memory access");
        };
        let bytes: [u8; $width] = $to_bytes;
        mem.data[ea..ea + $width].copy_from_slice(&bytes);
    }};
}

struct CallFrame {
    compiled_func_idx: usize,
    pc: usize,
    locals: Vec<RawValue>,
    stack_base: usize,
    arity: usize,
}

enum RunOutcome {
    Completed,
    FuelExhausted,
}

pub struct CompiledInterpreter {
    compiled: CompiledModule,
    store: Store,
    stack: ValueStack,
    call_stack: Vec<CallFrame>,
    fuel: Option<u64>,
    pending_arity: Option<usize>,
    /// mapping from function address -> compiled function index
    /// indexed by store address
    func_addr_to_compiled: Vec<Option<usize>>,
}

const MAX_CALL_DEPTH: usize = 1024;

impl CompiledInterpreter {
    pub fn new(module_data: &[u8]) -> Result<Self> {
        Self::instantiate(Store::new(), module_data, vec![])
    }

    pub fn instantiate(
        mut store: Store,
        module_data: &[u8],
        external_addresses: Vec<ExternalValue>,
    ) -> Result<Self> {
        // step 1-3. todo: validate is a pita
        let module = Parser::new(module_data).parse_module()?;

        // step 4
        ensure!(
            module.import_declarations.len() == external_addresses.len(),
            "Expected {} imports, got {}",
            module.import_declarations.len(),
            external_addresses.len()
        );

        // step 5
        let is_compatible = |external_address: &ExternalValue,
                             import_description: &ImportDescription| {
            matches!(
                (external_address, &import_description),
                (ExternalValue::Function { .. }, ImportDescription::Func(_))
                    | (ExternalValue::Table { .. }, ImportDescription::Table(_))
                    | (ExternalValue::Memory { .. }, ImportDescription::Mem(_))
                    | (ExternalValue::Global { .. }, ImportDescription::Global(_))
                    | (ExternalValue::Tag { .. }, ImportDescription::Tag(_))
            )
        };

        ensure!(
            external_addresses
                .iter()
                .zip(module.import_declarations.iter())
                .all(|(extern_addr, import_decl)| is_compatible(
                    extern_addr,
                    &import_decl.description
                )),
            "import type mismatch"
        );

        // step 6
        let data_instructions = module
            .data_segments
            .iter()
            .enumerate()
            .flat_map(|(i, ds)| run_data(i as u32, ds))
            .collect::<Vec<_>>();

        // step 7
        let element_instructions = module
            .element_segments
            .iter()
            .enumerate()
            .flat_map(|(i, es)| run_elem(i as u32, es))
            .collect::<Vec<_>>();

        // step 8
        let mut module_instance_0 = ModuleInstance::new(module.types.clone());
        module_instance_0.global_addrs = external_addresses
            .iter()
            .filter_map(|addr| match addr {
                ExternalValue::Global { addr } => Some(*addr),
                _ => None,
            })
            .collect();

        module_instance_0.function_addrs = external_addresses
            .iter()
            .filter_map(|addr| match addr {
                ExternalValue::Function { addr } => Some(*addr),
                _ => None,
            })
            .collect();

        let func_base = store.functions.len();
        module_instance_0
            .function_addrs
            .extend((0..module.functions.len()).map(|i| func_base + i));

        // step 9-10
        // todo: read into the 3.0 spec

        // step 11-13: global types/exprs are consumed in step 19 and step 24

        // step 14-15
        let _elem_exprs = module.element_segments.iter().map(|e| &e.expression);

        // step 19: evaluate global init expressions sequentially so that each
        // newly created global is visible to subsequent global.get in const exprs
        let num_imported_globals = store.globals.len();
        let mut initial_global_values = Vec::new();
        for g in &module.globals {
            let value =
                eval_const_expr_with_module(&g.initial_expression, &store, &module_instance_0)?;
            let addr = store.globals.len();
            store.globals.push(GlobalInstance {
                global_type: g.global_type.clone(),
                value,
            });
            module_instance_0.global_addrs.push(addr);
            initial_global_values.push(value);
        }
        // step 20: evaluate table init expressions
        let initial_table_refs = module
            .tables
            .iter()
            .map(|td| {
                let val = eval_const_expr_with_module(&td.init, &store, &module_instance_0)?;
                Ok(val.as_ref())
            })
            .collect::<Result<Vec<_>>>()?;

        // step 21 - evaluate element segment exprs
        let element_segment_refs = module
            .element_segments
            .iter()
            .map(|es| {
                es.expression
                    .iter()
                    .map(|expr| {
                        let val = eval_const_expr_with_module(expr, &store, &module_instance_0)?;
                        Ok(val.as_ref())
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()?;

        // remove temp globals — allocate_module will add them properly
        store.globals.truncate(num_imported_globals);

        // save start function index before module is moved
        let start_func_idx = module.start;
        let num_local_funcs = module.functions.len();

        let mut compiled = compiler::compile(&module);

        // step 24
        let module_instance = store.allocate_module(
            module,
            external_addresses,
            initial_global_values,
            initial_table_refs,
            element_segment_refs,
        )?;

        compiled.function_addrs = module_instance.function_addrs.clone();
        compiled.table_addrs = module_instance.table_addrs.clone();
        compiled.mem_addrs = module_instance.mem_addrs.clone();
        compiled.global_addrs = module_instance.global_addrs.clone();
        compiled.tag_addrs = module_instance.tag_addrs.clone();
        compiled.elem_addrs = module_instance.elem_addrs.clone();
        compiled.data_addrs = module_instance.data_addrs.clone();
        compiled.exports = module_instance.exports.clone();

        // build a mapping from func_addr to compiled func index
        let mut func_addr_to_compiled = vec![None; store.functions.len()];
        let first_compiled_idx = compiled.functions.len() - num_local_funcs;
        for (i, &addr) in module_instance
            .function_addrs
            .iter()
            .rev()
            .take(num_local_funcs)
            .rev()
            .enumerate()
        {
            func_addr_to_compiled[addr] = Some(first_compiled_idx + i);
        }

        // compile any imported local functions not yet in the compiled set
        let types = compiled.types.clone();
        for &addr in &module_instance.function_addrs {
            if func_addr_to_compiled[addr].is_some() {
                continue;
            }
            if let FunctionInstance::Local { code, .. } = &store.functions[addr] {
                let cf = compiler::compile_function_into(&types, code, &mut compiled);
                let idx = compiled.functions.len();
                compiled.functions.push(cf);
                func_addr_to_compiled[addr] = Some(idx);
            }
        }

        let max_func_stack = compiled
            .functions
            .iter()
            .map(|f| f.max_stack_height as usize)
            .max()
            .unwrap_or(1_024);
        let stack_capacity = max_func_stack.saturating_mul(MAX_CALL_DEPTH).max(1024);

        let mut interpreter = Self {
            compiled,
            store,
            stack: ValueStack::with_capacity(stack_capacity),
            call_stack: Vec::with_capacity(MAX_CALL_DEPTH),
            fuel: None,
            pending_arity: None,
            func_addr_to_compiled,
        };

        // step 27 - execute element segment initialization
        // step 28 - execute data segment initialization
        let init_instructions = [element_instructions, data_instructions].concat();
        if !init_instructions.is_empty() {
            interpreter.run_init_instructions(&init_instructions)?;
        }

        // step 29: invoke start function if present
        if let Some(start_idx) = start_func_idx {
            let func_addr = *module_instance
                .function_addrs
                .get(start_idx as usize)
                .ok_or_else(|| anyhow!("start function index {} oob", start_idx))?;
            interpreter.push_function_call(func_addr)?;
            interpreter.run()?;
        }

        // step 31
        Ok(interpreter)
    }

    pub fn invoke(&mut self, name: &str, args: Vec<RawValue>) -> Result<ExecutionState> {
        let addr = self.get_export_func_addr(name)?;
        self.invoke_by_addr(addr, args)
    }

    pub fn resume(&mut self) -> Result<ExecutionState> {
        let arity = self
            .pending_arity
            .ok_or_else(|| anyhow!("no pending execution to resume"))?;

        match self.run() {
            Ok(RunOutcome::Completed) => {
                let results = self.stack.pop_n(arity);
                self.pending_arity = None;
                Ok(ExecutionState::Completed(results.to_vec()))
            }
            Ok(RunOutcome::FuelExhausted) => Ok(ExecutionState::FuelExhausted),
            Err(e) => {
                self.stack.clear();
                self.call_stack.clear();
                self.pending_arity = None;
                Err(e)
            }
        }
    }

    pub fn get_param_types(&self, name: &str) -> Result<Vec<ValueType>> {
        let addr = self.get_export_func_addr(name)?;
        let fi = self
            .store
            .functions
            .get(addr)
            .ok_or_else(|| anyhow!("function addr {} oob", addr))?;

        match fi {
            FunctionInstance::Local { function_type, .. }
            | FunctionInstance::Host { function_type, .. } => Ok(function_type.0 .0.clone()),
        }
    }

    pub const fn is_paused(&self) -> bool {
        self.pending_arity.is_some()
    }

    pub const fn set_fuel(&mut self, fuel: u64) {
        self.fuel = Some(fuel);
    }

    pub const fn fuel(&self) -> Option<u64> {
        self.fuel
    }

    pub const fn store(&self) -> &Store {
        &self.store
    }

    pub const fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }

    pub fn into_store(self) -> Store {
        self.store
    }

    pub fn module_exports(&self) -> &[ExportInstance] {
        &self.compiled.exports
    }

    fn get_export_func_addr(&self, name: &str) -> Result<usize> {
        for export in &self.compiled.exports {
            if export.name == name {
                if let ExternalValue::Function { addr } = export.value {
                    return Ok(addr);
                }

                bail!("export'{}' is not a function", name);
            }
        }
        bail!("export '{}' not found", name)
    }

    fn invoke_by_addr(
        &mut self,
        function_addr: usize,
        args: Vec<RawValue>,
    ) -> Result<ExecutionState> {
        if self.pending_arity.is_some() {
            bail!("cannot invoke while execution is paused; call resume() first");
        }

        let fi = self
            .store
            .functions
            .get(function_addr)
            .ok_or_else(|| anyhow!("function addr {} oob", function_addr))?;

        let (num_args, num_results) = match fi {
            FunctionInstance::Local { function_type, .. } => {
                (function_type.0 .0.len(), function_type.1 .0.len())
            }
            _ => todo!("how does host functions work?"),
        };

        ensure!(num_args == args.len());

        self.stack.extend_from_slice(&args);
        self.push_function_call(function_addr)?;

        match self.run() {
            Ok(RunOutcome::Completed) => {
                let results = self.stack.pop_n(num_results);
                self.pending_arity = None;
                Ok(ExecutionState::Completed(results.to_vec()))
            }
            Ok(RunOutcome::FuelExhausted) => {
                self.pending_arity = Some(num_results);
                Ok(ExecutionState::FuelExhausted)
            }
            Err(e) => {
                self.stack.clear();
                self.call_stack.clear();
                self.pending_arity = None;
                Err(e)
            }
        }
    }

    fn compiled_func_index(&self, func_addr: usize) -> Option<usize> {
        self.func_addr_to_compiled.get(func_addr).copied().flatten()
    }

    fn push_function_call(&mut self, func_addr: usize) -> Result<()> {
        ensure!(
            self.call_stack.len() < MAX_CALL_DEPTH,
            "call stack exhausted"
        );

        let fi = &self.store.functions[func_addr];
        let (num_args, num_results) = match fi {
            FunctionInstance::Local { function_type, .. }
            | FunctionInstance::Host { function_type, .. } => {
                (function_type.0 .0.len(), function_type.1 .0.len())
            }
        };

        let Some(compiled_idx) = self.compiled_func_index(func_addr) else {
            match &self.store.functions[func_addr] {
                FunctionInstance::Host { code, .. } => {
                    ensure!(self.stack.len() >= num_args, "not enough args on stack");
                    let args_start = self.stack.len() - num_args;
                    self.stack.truncate(args_start);
                    code();
                    // this is probably not correct but spec tests are no ops
                    return Ok(());
                }
                _ => bail!("expected host function at addr {}", func_addr),
            }
        };

        ensure!(self.stack.len() >= num_args, "not enough args on stack");
        let args_start = self.stack.len() - num_args;
        let mut locals: Vec<RawValue> = self.stack.slice_from(args_start).to_vec();
        self.stack.truncate(args_start);

        let cf = &self.compiled.functions[compiled_idx];
        for _ in &cf.local_types[num_args..] {
            locals.push(RawValue::default());
        }

        let stack_base = self.stack.len();

        self.call_stack.push(CallFrame {
            compiled_func_idx: compiled_idx,
            pc: 0,
            locals,
            stack_base,
            arity: num_results,
        });

        Ok(())
    }

    fn run(&mut self) -> Result<RunOutcome> {
        loop {
            let depth = match self.call_stack.len() {
                0 => return Ok(RunOutcome::Completed),
                n => n - 1,
            };
            assert!(depth < self.call_stack.len());

            let func_idx = self.call_stack[depth].compiled_func_idx;
            let pc = self.call_stack[depth].pc;
            assert!(
                func_idx < self.compiled.functions.len(),
                "compiler error: compiled function index {func_idx} oob"
            );

            let func_ops = &self.compiled.functions[func_idx].ops;
            assert!(
                pc < func_ops.len(),
                "compiler error: pc {pc} past end of function {func_idx} (len {})",
                func_ops.len()
            );

            let op = func_ops[pc];
            self.call_stack[depth].pc += 1;

            if let Some(ref mut fuel) = self.fuel {
                if *fuel == 0 {
                    self.call_stack[depth].pc -= 1;
                    return Ok(RunOutcome::FuelExhausted);
                }
                *fuel -= 1;
            }

            match op {
                Op::Nop => {}
                Op::Unreachable => bail!("unreachable executed"),
                Op::Return => {
                    let arity = self.call_stack[depth].arity;
                    let base = self.call_stack[depth].stack_base;
                    let len = self.stack.len();
                    if arity > 0 {
                        self.stack.copy_within(len - arity..len, base);
                    }
                    self.stack.truncate(base + arity);
                    self.call_stack.pop();
                }
                Op::Jump { target, keep, drop } => {
                    self.stack.keep_top(keep as usize, drop as usize);
                    self.call_stack[depth].pc = target as usize;
                }
                Op::JumpIf { target, keep, drop } => {
                    let cond = pop_val!(self, I32);
                    if cond != 0 {
                        self.stack.keep_top(keep as usize, drop as usize);
                        self.call_stack[depth].pc = target as usize;
                    }
                }
                Op::JumpIfNot { target, keep, drop } => {
                    let cond = pop_val!(self, I32);
                    if cond == 0 {
                        self.stack.keep_top(keep as usize, drop as usize);
                        self.call_stack[depth].pc = target as usize;
                    }
                }
                Op::JumpTable { index, keep } => {
                    let i = pop_val!(self, I32) as usize;
                    let table = &self.compiled.jump_tables[index as usize];
                    let entry_target = if i < table.len() - 1 {
                        table[i]
                    } else {
                        *table.last().unwrap()
                    };
                    self.stack
                        .keep_top(keep as usize, entry_target.drop as usize);
                    self.call_stack[depth].pc = entry_target.target as usize;
                }
                Op::BrOnNull { target, keep, drop } => {
                    let val = self.stack.pop();
                    if matches!(val.as_ref(), Ref::Null) {
                        self.stack.keep_top(keep as usize, drop as usize);
                        self.call_stack[depth].pc = target as usize;
                    } else {
                        self.stack.push(val);
                    }
                }
                Op::BrOnNonNull { target, keep, drop } => {
                    let val = self.stack.pop();
                    if !matches!(val.as_ref(), Ref::Null) {
                        self.stack.push(val);
                        self.stack.keep_top(keep as usize, drop as usize);
                        self.call_stack[depth].pc = target as usize;
                    }
                }
                Op::Call { func_idx } => {
                    let func_addr = self.compiled.function_addrs[func_idx as usize];
                    self.push_function_call(func_addr)?;
                }
                Op::CallIndirect {
                    type_idx,
                    table_idx,
                } => {
                    let table_addr = self.compiled.table_addrs[table_idx as usize];
                    let addr_type = self.store.tables[table_addr].table_type.addr_type;

                    let i = self.stack.pop_address(addr_type);

                    let elem = self.store.tables[table_addr]
                        .elem
                        .get(i)
                        .ok_or_else(|| anyhow!("trap: undefined element"))?;

                    let Ref::FunctionAddr(func_addr) = elem else {
                        bail!("trap: uninitialized element {}", i);
                    };

                    let expected = match &self.compiled.types[type_idx as usize].composite_type {
                        CompositeType::Func(ft) => ft,
                        _ => bail!("type index {} not a func type", type_idx),
                    };

                    let actual = match &self.store.functions[*func_addr] {
                        FunctionInstance::Local { function_type, .. }
                        | FunctionInstance::Host { function_type, .. } => function_type,
                    };

                    ensure!(
                        expected.0 .0.len() == actual.0 .0.len()
                            && expected.1 .0.len() == actual.1 .0.len(),
                        "trap: indirect call type mismatch"
                    );

                    self.push_function_call(*func_addr)?;
                }
                Op::ReturnCall { func_idx } => {
                    let func_addr = self.compiled.function_addrs[func_idx as usize];
                    let num_args = self.func_num_params(func_addr);

                    let old_base = self.call_stack[depth].stack_base;
                    let len = self.stack.len();

                    self.stack.copy_within(len - num_args..len, old_base);
                    self.stack.truncate(old_base + num_args);
                    self.call_stack.pop();

                    self.push_function_call(func_addr)?;
                }
                Op::ReturnCallIndirect {
                    type_idx,
                    table_idx,
                } => {
                    let table_addr = self.compiled.table_addrs[table_idx as usize];
                    let addr_type = self.store.tables[table_addr].table_type.addr_type;

                    let i = self.stack.pop_address(addr_type);
                    let elem = self.store.tables[table_addr]
                        .elem
                        .get(i)
                        .ok_or_else(|| anyhow!("trap: undefined element"))?;

                    let Ref::FunctionAddr(func_addr) = elem else {
                        bail!("trap: uninitialized element {}", i);
                    };

                    let expected = match &self.compiled.types[type_idx as usize].composite_type {
                        CompositeType::Func(ft) => ft,
                        _ => bail!("type index {} not a func type", type_idx),
                    };

                    let actual = match &self.store.functions[*func_addr] {
                        FunctionInstance::Local { function_type, .. }
                        | FunctionInstance::Host { function_type, .. } => function_type,
                    };

                    ensure!(
                        expected.0 .0.len() == actual.0 .0.len()
                            && expected.1 .0.len() == actual.1 .0.len(),
                        "trap: indirect call type mismatch"
                    );

                    let func_addr = *func_addr;
                    let num_args = expected.0 .0.len();
                    let old_base = self.call_stack[depth].stack_base;
                    let len = self.stack.len();

                    self.stack.copy_within(len - num_args..len, old_base);
                    self.stack.truncate(old_base + num_args);
                    self.call_stack.pop();
                    self.push_function_call(func_addr)?;
                }
                Op::CallRef { .. } => {
                    let func_addr = match self.stack.pop().as_ref() {
                        Ref::Null => bail!("trap: null function ref"),
                        Ref::FunctionAddr(f) => f,
                        _ => bail!("expected function or null ref"),
                    };
                    self.push_function_call(func_addr)?;
                }
                Op::ReturnCallRef { .. } => {
                    let func_addr = match self.stack.pop().as_ref() {
                        Ref::Null => bail!("trap: null function ref"),
                        Ref::FunctionAddr(f) => f,
                        _ => bail!("expected function or null ref"),
                    };

                    let num_args = self.func_num_params(func_addr);
                    let old_base = self.call_stack[depth].stack_base;
                    let len = self.stack.len();
                    self.stack.copy_within(len - num_args..len, old_base);
                    self.stack.truncate(old_base + num_args);
                    self.call_stack.pop();
                    self.push_function_call(func_addr)?;
                }
                Op::I32Const { value } => self.stack.push(value),
                Op::I64Const { value } => self.stack.push(value),
                Op::F32Const { value } => self.stack.push(value),
                Op::F64Const { value } => self.stack.push(value),
                Op::V128Const { table_idx } => {
                    let v = self.compiled.v128_constants[table_idx as usize];
                    self.stack.push_v128(v);
                }
                Op::LocalGet { local_idx } => {
                    let locals = &self.call_stack[depth].locals;
                    assert!(
                        (local_idx as usize) < locals.len(),
                        "compiler error: local index {local_idx} oob (func has {} locals)",
                        locals.len()
                    );
                    let val = locals[local_idx as usize];
                    self.stack.push(val);
                }
                Op::LocalSet { local_idx } => {
                    let val = self.stack.pop();
                    let locals = &mut self.call_stack[depth].locals;
                    assert!(
                        (local_idx as usize) < locals.len(),
                        "compiler error: local index {local_idx} oob (func has {} locals)",
                        locals.len()
                    );
                    locals[local_idx as usize] = val;
                }
                Op::LocalTee { local_idx } => {
                    let val = *self.stack.last();
                    let locals = &mut self.call_stack[depth].locals;
                    assert!(
                        (local_idx as usize) < locals.len(),
                        "compiler error: local index {local_idx} oob (func has {} locals)",
                        locals.len()
                    );
                    locals[local_idx as usize] = val;
                }
                Op::GlobalGet { global_idx } => {
                    let addr = self.compiled.global_addrs[global_idx as usize];
                    self.stack.push(self.store.globals[addr].value);
                }
                Op::GlobalSet { global_idx } => {
                    let addr = self.compiled.global_addrs[global_idx as usize];
                    ensure!(
                        matches!(
                            self.store.globals[addr].global_type.mutability,
                            Mutability::Var
                        ),
                        "cannot set immutable global"
                    );
                    self.store.globals[addr].value = self.stack.pop();
                }
                Op::Drop => {
                    self.stack.pop();
                }
                Op::Select => {
                    let cond = pop_val!(self, I32);
                    let val2 = self.stack.pop();
                    let val1 = self.stack.pop();
                    self.stack.push(if cond != 0 { val1 } else { val2 });
                }
                Op::RefNull(_) => self.stack.push(RawValue::from_ref(Ref::Null)),
                Op::RefIsNull => {
                    let val = self.stack.pop();
                    let is_null = matches!(val.as_ref(), Ref::Null);
                    self.stack.push(is_null as i32);
                }
                Op::RefFunc { func_idx } => {
                    let addr = self.compiled.function_addrs[func_idx as usize];
                    self.stack.push(RawValue::from_ref(Ref::FunctionAddr(addr)));
                }
                Op::RefEq => todo!(),
                Op::RefAsNonNull => {
                    let val = self.stack.pop();
                    ensure!(!matches!(val.as_ref(), Ref::Null), "trap: null reference");
                    self.stack.push(val);
                }
                Op::Throw { .. } => todo!(),
                Op::ThrowRef => todo!(),
                Op::TableGet { table_idx } => {
                    let ta = self.compiled.table_addrs[table_idx as usize];
                    let addr_type = self.store.tables[ta].table_type.addr_type;

                    let i = self.stack.pop_address(addr_type);

                    let elem = self.store.tables[ta]
                        .elem
                        .get(i)
                        .ok_or_else(|| anyhow!("trap: oob table access"))?;

                    self.stack.push(RawValue::from_ref(*elem));
                }
                Op::TableSet { table_idx } => {
                    let ta = self.compiled.table_addrs[table_idx as usize];
                    let r = self.stack.pop().as_ref();

                    let at = self.store.tables[ta].table_type.addr_type;
                    let i = self.stack.pop_address(at);

                    let elem = self.store.tables[ta]
                        .elem
                        .get_mut(i)
                        .ok_or_else(|| anyhow!("trap: oob table access"))?;

                    *elem = r;
                }
                Op::TableInit {
                    elem_idx,
                    table_idx,
                } => {
                    let ta = self.compiled.table_addrs[table_idx as usize];
                    let ea = self.compiled.elem_addrs[elem_idx as usize];

                    let n = pop_val!(self, I32);
                    let s = pop_val!(self, I32);

                    let (n, s) = (n as usize, s as usize);

                    let at = self.store.tables[ta].table_type.addr_type;
                    let d = self.stack.pop_address(at);

                    if s.saturating_add(n) > self.store.element_segments[ea].elem.len() {
                        bail!("trap: oob table access");
                    }

                    if d.saturating_add(n) > self.store.tables[ta].elem.len() {
                        bail!("trap: oob table access");
                    }

                    if n > 0 {
                        let src = self.store.element_segments[ea].elem[s..s + n].to_vec();
                        self.store.tables[ta].elem[d..d + n].copy_from_slice(&src);
                    }
                }
                Op::ElemDrop { elem_idx } => {
                    let ea = self.compiled.elem_addrs[elem_idx as usize];
                    self.store.element_segments[ea].elem.clear();
                }
                Op::TableCopy {
                    dst_table_idx,
                    src_table_idx,
                } => {
                    let dst_a = self.compiled.table_addrs[dst_table_idx as usize];
                    let src_a = self.compiled.table_addrs[src_table_idx as usize];

                    let dst_at = self.store.tables[dst_a].table_type.addr_type;
                    let src_at = self.store.tables[src_a].table_type.addr_type;
                    let n_at = match (dst_at, src_at) {
                        (AddrType::I64, AddrType::I64) => AddrType::I64,
                        _ => AddrType::I32,
                    };

                    let n = self.stack.pop_address(n_at);
                    let s = self.stack.pop_address(src_at);
                    let d = self.stack.pop_address(dst_at);

                    if s.saturating_add(n) > self.store.tables[src_a].elem.len() {
                        bail!("trap: oob table access");
                    }

                    if d.saturating_add(n) > self.store.tables[dst_a].elem.len() {
                        bail!("trap: oob table access");
                    }

                    if n > 0 {
                        if dst_a == src_a {
                            self.store.tables[dst_a].elem.copy_within(s..s + n, d);
                        } else {
                            let src = self.store.tables[src_a].elem[s..s + n].to_vec();
                            self.store.tables[dst_a].elem[d..d + n].copy_from_slice(&src);
                        }
                    }
                }
                Op::TableGrow { table_idx } => {
                    let ta = self.compiled.table_addrs[table_idx as usize];
                    let at = self.store.tables[ta].table_type.addr_type;
                    let n = self.stack.pop_address(at);

                    let r = self.stack.pop().as_ref();

                    let old_size = self.store.tables[ta].elem.len();
                    let new_size = (old_size as u64).checked_add(n as u64);

                    if new_size.is_none_or(|s| s > self.store.tables[ta].table_type.limit.max) {
                        match at {
                            AddrType::I32 => self.stack.push(-1i32),
                            AddrType::I64 => self.stack.push(-1i64),
                        }
                        continue;
                    }

                    let new_size = new_size.unwrap();
                    self.store.tables[ta].elem.resize(new_size as usize, r);
                    self.store.tables[ta].table_type.limit.min = new_size;
                    self.stack.push_address(old_size, at);
                }
                Op::TableSize { table_idx } => {
                    let ta = self.compiled.table_addrs[table_idx as usize];
                    let size = self.store.tables[ta].elem.len();
                    let at = self.store.tables[ta].table_type.addr_type;

                    self.stack.push_address(size, at);
                }
                Op::TableFill { table_idx } => {
                    let ta = self.compiled.table_addrs[table_idx as usize];
                    let at = self.store.tables[ta].table_type.addr_type;
                    let n = self.stack.pop_address(at);

                    let r = self.stack.pop().as_ref();

                    let i = self.stack.pop_address(at);
                    if i.saturating_add(n) > self.store.tables[ta].elem.len() {
                        bail!("trap: oob table access");
                    }

                    if n > 0 {
                        self.store.tables[ta].elem[i..i + n].fill(r);
                    }
                }
                Op::I32Load { offset, memory } => {
                    mem_load_c!(self, offset, memory, 4, |b| i32::from_le_bytes(b))
                }
                Op::I64Load { offset, memory } => {
                    mem_load_c!(self, offset, memory, 8, |b| i64::from_le_bytes(b))
                }
                Op::F32Load { offset, memory } => {
                    mem_load_c!(self, offset, memory, 4, |b| f32::from_le_bytes(b))
                }
                Op::F64Load { offset, memory } => {
                    mem_load_c!(self, offset, memory, 8, |b| f64::from_le_bytes(b))
                }
                Op::I32Load8Signed { offset, memory } => {
                    mem_load_c!(self, offset, memory, 1, |b| b[0] as i8 as i32)
                }
                Op::I32Load8Unsigned { offset, memory } => {
                    mem_load_c!(self, offset, memory, 1, |b| b[0] as i32)
                }
                Op::I32Load16Signed { offset, memory } => {
                    mem_load_c!(self, offset, memory, 2, |b| i16::from_le_bytes(b) as i32)
                }
                Op::I32Load16Unsigned { offset, memory } => {
                    mem_load_c!(self, offset, memory, 2, |b| u16::from_le_bytes(b) as i32)
                }
                Op::I64Load8Signed { offset, memory } => {
                    mem_load_c!(self, offset, memory, 1, |b| b[0] as i8 as i64)
                }
                Op::I64Load8Unsigned { offset, memory } => {
                    mem_load_c!(self, offset, memory, 1, |b| b[0] as i64)
                }
                Op::I64Load16Signed { offset, memory } => {
                    mem_load_c!(self, offset, memory, 2, |b| i16::from_le_bytes(b) as i64)
                }
                Op::I64Load16Unsigned { offset, memory } => {
                    mem_load_c!(self, offset, memory, 2, |b| u16::from_le_bytes(b) as i64)
                }
                Op::I64Load32Signed { offset, memory } => {
                    mem_load_c!(self, offset, memory, 4, |b| i32::from_le_bytes(b) as i64)
                }
                Op::I64Load32Unsigned { offset, memory } => {
                    mem_load_c!(self, offset, memory, 4, |b| u32::from_le_bytes(b) as i64)
                }
                Op::I32Store { offset, memory } => {
                    mem_store_c!(self, offset, memory, 4, |v| v.as_i32().to_le_bytes())
                }
                Op::I64Store { offset, memory } => {
                    mem_store_c!(self, offset, memory, 8, |v| v.as_i64().to_le_bytes())
                }
                Op::F32Store { offset, memory } => {
                    mem_store_c!(self, offset, memory, 4, |v| v.as_f32().to_le_bytes())
                }
                Op::F64Store { offset, memory } => {
                    mem_store_c!(self, offset, memory, 8, |v| v.as_f64().to_le_bytes())
                }
                Op::I32Store8 { offset, memory } => {
                    mem_store_c!(self, offset, memory, 1, |v| (v.as_i32() as u8)
                        .to_le_bytes())
                }
                Op::I32Store16 { offset, memory } => {
                    mem_store_c!(self, offset, memory, 2, |v| (v.as_i32() as u16)
                        .to_le_bytes())
                }
                Op::I64Store8 { offset, memory } => {
                    mem_store_c!(self, offset, memory, 1, |v| (v.as_i64() as u8)
                        .to_le_bytes())
                }
                Op::I64Store16 { offset, memory } => {
                    mem_store_c!(self, offset, memory, 2, |v| (v.as_i64() as u16)
                        .to_le_bytes())
                }
                Op::I64Store32 { offset, memory } => {
                    mem_store_c!(self, offset, memory, 4, |v| (v.as_i64() as u32)
                        .to_le_bytes())
                }
                Op::MemorySize { memory_idx } => {
                    let ma = self.compiled.mem_addrs[memory_idx as usize];
                    let mem = &self.store.memories[ma];
                    let size = mem.data.len() / PAGE_SIZE;

                    self.stack.push_address(size, mem.memory_type.addr_type);
                }
                Op::MemoryGrow { memory_idx } => {
                    let ma = self.compiled.mem_addrs[memory_idx as usize];
                    let mem = &mut self.store.memories[ma];
                    let at = mem.memory_type.addr_type;
                    let page_count = self.stack.pop_address(at);

                    let old_size = mem.data.len() / PAGE_SIZE;
                    let new_size = old_size + page_count;

                    const MAX_PAGES: usize = 65536;

                    if new_size > MAX_PAGES || new_size as u64 > mem.memory_type.limit.max {
                        match at {
                            AddrType::I32 => self.stack.push(-1i32),
                            AddrType::I64 => self.stack.push(-1i64),
                        }
                        continue;
                    }
                    mem.data.resize(new_size * PAGE_SIZE, 0);
                    let at = mem.memory_type.addr_type;
                    mem.memory_type.limit.min = new_size as u64;

                    self.stack.push_address(old_size, at);
                }
                Op::MemoryInit {
                    data_idx,
                    memory_idx,
                } => {
                    let ma = self.compiled.mem_addrs[memory_idx as usize];
                    let da = self.compiled.data_addrs[data_idx as usize];

                    let at = self.store.memories[ma].memory_type.addr_type;

                    let n = pop_val!(self, I32) as usize;
                    let s = pop_val!(self, I32) as usize;
                    let d = self.stack.pop_address(at);

                    if s.saturating_add(n) > self.store.data_segments[da].data.len() {
                        bail!("trap: oob memory access");
                    }

                    if d.saturating_add(n) > self.store.memories[ma].data.len() {
                        bail!("trap: oob memory access");
                    }

                    if n > 0 {
                        let src = self.store.data_segments[da].data[s..s + n].to_vec();
                        self.store.memories[ma].data[d..d + n].copy_from_slice(&src);
                    }
                }
                Op::DataDrop { data_idx } => {
                    let da = self.compiled.data_addrs[data_idx as usize];
                    self.store.data_segments[da].data.clear();
                }
                Op::MemoryCopy {
                    dst_memory_idx,
                    src_memory_idx,
                } => {
                    let m1 = self.compiled.mem_addrs[dst_memory_idx as usize];
                    let m2 = self.compiled.mem_addrs[src_memory_idx as usize];
                    let at = self.store.memories[m1].memory_type.addr_type;

                    let n = self.stack.pop_address(at);
                    let i2 = self.stack.pop_address(at);
                    let i1 = self.stack.pop_address(at);

                    if i1.saturating_add(n) > self.store.memories[m1].data.len() {
                        bail!("trap: oob memory access");
                    }

                    if i2.saturating_add(n) > self.store.memories[m2].data.len() {
                        bail!("trap: oob memory access");
                    }

                    if n > 0 {
                        if m1 == m2 {
                            self.store.memories[m1].data.copy_within(i2..i2 + n, i1);
                        } else {
                            let src = self.store.memories[m2].data[i2..i2 + n].to_vec();
                            self.store.memories[m1].data[i1..i1 + n].copy_from_slice(&src);
                        }
                    }
                }
                Op::MemoryFill { memory_idx } => {
                    let ma = self.compiled.mem_addrs[memory_idx as usize];
                    let at = self.store.memories[ma].memory_type.addr_type;
                    let n = self.stack.pop_address(at);
                    let val = pop_val!(self, I32);
                    let i = self.stack.pop_address(at);

                    if i.saturating_add(n) > self.store.memories[ma].data.len() {
                        bail!("trap: oob memory access");
                    }

                    if n > 0 {
                        self.store.memories[ma].data[i..i + n].fill(val as u8);
                    }
                }
                Op::I32EqZero => {
                    let a = pop_val!(self, I32);
                    self.stack.push((a == 0) as i32);
                }
                Op::I32Eq => cmpop!(self, I32, |b, a| b == a),
                Op::I32Ne => cmpop!(self, I32, |b, a| b != a),
                Op::I32LtSigned => cmpop!(self, I32, |b, a| b < a),
                Op::I32LtUnsigned => cmpop!(self, I32, |b, a| (b as u32) < (a as u32)),
                Op::I32GtSigned => cmpop!(self, I32, |b, a| b > a),
                Op::I32GtUnsigned => cmpop!(self, I32, |b, a| (b as u32) > (a as u32)),
                Op::I32LeSigned => cmpop!(self, I32, |b, a| b <= a),
                Op::I32LeUnsigned => cmpop!(self, I32, |b, a| (b as u32) <= (a as u32)),
                Op::I32GeSigned => cmpop!(self, I32, |b, a| b >= a),
                Op::I32GeUnsigned => cmpop!(self, I32, |b, a| (b as u32) >= (a as u32)),
                Op::I32CountLeadingZeros => {
                    let a = pop_val!(self, I32);
                    self.stack.push(a.leading_zeros() as i32);
                }
                Op::I32CountTrailingZeros => {
                    let a = pop_val!(self, I32);
                    self.stack.push(a.trailing_zeros() as i32);
                }
                Op::I32PopCount => {
                    let a = pop_val!(self, I32);
                    self.stack.push(a.count_ones() as i32);
                }
                Op::I32Add => binop!(self, I32, |b, a| b.wrapping_add(a)),
                Op::I32Sub => binop!(self, I32, |b, a| b.wrapping_sub(a)),
                Op::I32Mul => binop!(self, I32, |b, a| b.wrapping_mul(a)),
                Op::I32DivSigned => {
                    let a = pop_val!(self, I32);
                    let b = pop_val!(self, I32);

                    ensure!(a != 0, "wasm trap: integer divide by zero");
                    ensure!(!(b == i32::MIN && a == -1), "wasm trap: integer overflow");

                    self.stack.push(b.wrapping_div(a));
                }
                Op::I32DivUnsigned => {
                    let a = pop_val!(self, I32);
                    let b = pop_val!(self, I32);

                    ensure!(a != 0, "wasm trap: integer divide by zero");
                    self.stack.push(((b as u32) / (a as u32)) as i32);
                }
                Op::I32RemainderSigned => {
                    let a = pop_val!(self, I32);
                    let b = pop_val!(self, I32);

                    ensure!(a != 0, "wasm trap: integer divide by zero");
                    self.stack.push(b.wrapping_rem(a));
                }
                Op::I32RemainderUnsigned => {
                    let a = pop_val!(self, I32);
                    let b = pop_val!(self, I32);

                    ensure!(a != 0, "wasm trap: integer divide by zero");
                    self.stack.push(((b as u32) % (a as u32)) as i32);
                }
                Op::I32And => binop!(self, I32, |b, a| b & a),
                Op::I32Or => binop!(self, I32, |b, a| b | a),
                Op::I32Xor => binop!(self, I32, |b, a| b ^ a),
                Op::I32Shl => binop!(self, I32, |b, a| b.wrapping_shl(a as u32 % 32)),
                Op::I32ShrSigned => binop!(self, I32, |b, a| b.wrapping_shr(a as u32 % 32)),
                Op::I32ShrUnsigned => {
                    binop!(self, I32, |b, a| ((b as u32).wrapping_shr(a as u32 % 32))
                        as i32)
                }
                Op::I32RotateLeft => binop!(self, I32, |b, a| b.rotate_left(a as u32 % 32)),
                Op::I32RotateRight => binop!(self, I32, |b, a| b.rotate_right(a as u32 % 32)),
                Op::I64EqZero => {
                    let a = pop_val!(self, I64);
                    self.stack.push((a == 0) as i32);
                }
                Op::I64Eq => cmpop!(self, I64, |b, a| b == a),
                Op::I64Ne => cmpop!(self, I64, |b, a| b != a),
                Op::I64LtSigned => cmpop!(self, I64, |b, a| b < a),
                Op::I64LtUnsigned => cmpop!(self, I64, |b, a| (b as u64) < (a as u64)),
                Op::I64GtSigned => cmpop!(self, I64, |b, a| b > a),
                Op::I64GtUnsigned => cmpop!(self, I64, |b, a| (b as u64) > (a as u64)),
                Op::I64LeSigned => cmpop!(self, I64, |b, a| b <= a),
                Op::I64LeUnsigned => cmpop!(self, I64, |b, a| (b as u64) <= (a as u64)),
                Op::I64GeSigned => cmpop!(self, I64, |b, a| b >= a),
                Op::I64GeUnsigned => cmpop!(self, I64, |b, a| (b as u64) >= (a as u64)),
                Op::I64CountLeadingZeros => {
                    let a = pop_val!(self, I64);
                    self.stack.push(a.leading_zeros() as i64);
                }
                Op::I64CountTrailingZeros => {
                    let a = pop_val!(self, I64);
                    self.stack.push(a.trailing_zeros() as i64);
                }
                Op::I64PopCount => {
                    let a = pop_val!(self, I64);
                    self.stack.push(a.count_ones() as i64);
                }
                Op::I64Add => binop!(self, I64, |b, a| b.wrapping_add(a)),
                Op::I64Sub => binop!(self, I64, |b, a| b.wrapping_sub(a)),
                Op::I64Mul => binop!(self, I64, |b, a| b.wrapping_mul(a)),
                Op::I64DivSigned => {
                    let a = pop_val!(self, I64);
                    let b = pop_val!(self, I64);

                    ensure!(a != 0, "wasm trap: integer divide by zero");
                    ensure!(!(b == i64::MIN && a == -1), "wasm trap: integer overflow");

                    self.stack.push(b.wrapping_div(a));
                }
                Op::I64DivUnsigned => {
                    let a = pop_val!(self, I64);
                    let b = pop_val!(self, I64);

                    ensure!(a != 0, "wasm trap: integer divide by zero");

                    self.stack.push(((b as u64) / (a as u64)) as i64);
                }
                Op::I64RemainderSigned => {
                    let a = pop_val!(self, I64);
                    let b = pop_val!(self, I64);

                    ensure!(a != 0, "wasm trap: integer divide by zero");

                    self.stack.push(b.wrapping_rem(a));
                }
                Op::I64RemainderUnsigned => {
                    let a = pop_val!(self, I64);
                    let b = pop_val!(self, I64);

                    ensure!(a != 0, "wasm trap: integer divide by zero");
                    self.stack.push(((b as u64) % (a as u64)) as i64);
                }
                Op::I64And => binop!(self, I64, |b, a| b & a),
                Op::I64Or => binop!(self, I64, |b, a| b | a),
                Op::I64Xor => binop!(self, I64, |b, a| b ^ a),
                Op::I64Shl => binop!(self, I64, |b, a| b.wrapping_shl(a as u32 % 64)),
                Op::I64ShrSigned => binop!(self, I64, |b, a| b.wrapping_shr(a as u32 % 64)),
                Op::I64ShrUnsigned => {
                    binop!(self, I64, |b, a| ((b as u64).wrapping_shr(a as u32 % 64))
                        as i64)
                }
                Op::I64RotateLeft => binop!(self, I64, |b, a| b.rotate_left(a as u32 % 64)),
                Op::I64RotateRight => binop!(self, I64, |b, a| b.rotate_right(a as u32 % 64)),
                Op::F32Eq => cmpop!(self, F32, |b, a| b == a),
                Op::F32Ne => cmpop!(self, F32, |b, a| b != a),
                Op::F32Lt => cmpop!(self, F32, |b, a| b < a),
                Op::F32Gt => cmpop!(self, F32, |b, a| b > a),
                Op::F32Le => cmpop!(self, F32, |b, a| b <= a),
                Op::F32Ge => cmpop!(self, F32, |b, a| b >= a),
                Op::F32Abs => {
                    let a = pop_val!(self, F32);
                    self.stack.push(a.abs());
                }
                Op::F32Neg => {
                    let a = pop_val!(self, F32);
                    self.stack.push(a.neg());
                }
                Op::F32Ceil => {
                    let a = pop_val!(self, F32);
                    self.stack.push(a.ceil());
                }
                Op::F32Floor => {
                    let a = pop_val!(self, F32);
                    self.stack.push(a.floor());
                }
                Op::F32Trunc => {
                    let a = pop_val!(self, F32);
                    self.stack.push(a.trunc());
                }
                Op::F32Nearest => {
                    let a = pop_val!(self, F32);
                    self.stack.push(a.round_ties_even());
                }
                Op::F32Sqrt => {
                    let a = pop_val!(self, F32);
                    self.stack.push(a.sqrt());
                }
                Op::F32Add => binop!(self, F32, |b, a| b + a),
                Op::F32Sub => binop!(self, F32, |b, a| b - a),
                Op::F32Mul => binop!(self, F32, |b, a| b * a),
                Op::F32Div => binop!(self, F32, |b, a| b / a),
                Op::F32Min => {
                    let a = pop_val!(self, F32);
                    let b = pop_val!(self, F32);
                    let r = if a.is_nan() || b.is_nan() {
                        f32::from_bits(0x7FC0_0000)
                    } else if a == b {
                        f32::from_bits(a.to_bits() | b.to_bits())
                    } else {
                        a.min(b)
                    };
                    self.stack.push(r);
                }
                Op::F32Max => {
                    let a = pop_val!(self, F32);
                    let b = pop_val!(self, F32);
                    let r = if a.is_nan() || b.is_nan() {
                        f32::from_bits(0x7FC0_0000)
                    } else if a == b {
                        f32::from_bits(a.to_bits() & b.to_bits())
                    } else {
                        a.max(b)
                    };
                    self.stack.push(r);
                }
                Op::F32CopySign => binop!(self, F32, |b, a| b.copysign(a)),
                Op::F64Eq => cmpop!(self, F64, |b, a| b == a),
                Op::F64Ne => cmpop!(self, F64, |b, a| b != a),
                Op::F64Lt => cmpop!(self, F64, |b, a| b < a),
                Op::F64Gt => cmpop!(self, F64, |b, a| b > a),
                Op::F64Le => cmpop!(self, F64, |b, a| b <= a),
                Op::F64Ge => cmpop!(self, F64, |b, a| b >= a),
                Op::F64Abs => {
                    let a = pop_val!(self, F64);
                    self.stack.push(a.abs());
                }
                Op::F64Neg => {
                    let a = pop_val!(self, F64);
                    self.stack.push(a.neg());
                }
                Op::F64Ceil => {
                    let a = pop_val!(self, F64);
                    self.stack.push(a.ceil());
                }
                Op::F64Floor => {
                    let a = pop_val!(self, F64);
                    self.stack.push(a.floor());
                }
                Op::F64Trunc => {
                    let a = pop_val!(self, F64);
                    self.stack.push(a.trunc());
                }
                Op::F64Nearest => {
                    let a = pop_val!(self, F64);
                    self.stack.push(a.round_ties_even());
                }
                Op::F64Sqrt => {
                    let a = pop_val!(self, F64);
                    self.stack.push(a.sqrt());
                }
                Op::F64Add => binop!(self, F64, |b, a| b + a),
                Op::F64Sub => binop!(self, F64, |b, a| b - a),
                Op::F64Mul => binop!(self, F64, |b, a| b * a),
                Op::F64Div => binop!(self, F64, |b, a| b / a),
                Op::F64Min => {
                    let a = pop_val!(self, F64);
                    let b = pop_val!(self, F64);
                    let r = if a.is_nan() || b.is_nan() {
                        f64::from_bits(0x7FF8_0000_0000_0000)
                    } else if a == b {
                        f64::from_bits(a.to_bits() | b.to_bits())
                    } else {
                        a.min(b)
                    };
                    self.stack.push(r);
                }
                Op::F64Max => {
                    let a = pop_val!(self, F64);
                    let b = pop_val!(self, F64);
                    let r = if a.is_nan() || b.is_nan() {
                        f64::from_bits(0x7FF8_0000_0000_0000)
                    } else if a == b {
                        f64::from_bits(a.to_bits() & b.to_bits())
                    } else {
                        a.max(b)
                    };
                    self.stack.push(r);
                }
                Op::F64CopySign => binop!(self, F64, |b, a| b.copysign(a)),
                Op::I32WrapI64 => {
                    let a = pop_val!(self, I64);
                    self.stack.push(a as i32);
                }
                Op::I32TruncF32Signed => {
                    let a = pop_val!(self, F32);
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let t = a.trunc();
                    ensure!(
                        t >= i32::MIN as f32 && t < i32::MAX as f32,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(t as i32);
                }
                Op::I32TruncF32Unsigned => {
                    let a = pop_val!(self, F32);
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let t = a.trunc();
                    ensure!(
                        t >= 0.0 && t < u32::MAX as f32,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(t as u32 as i32);
                }
                Op::I32TruncF64Signed => {
                    let a = pop_val!(self, F64);
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let t = a.trunc();
                    ensure!(
                        t >= i32::MIN as f64 && t <= i32::MAX as f64,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(t as i32);
                }
                Op::I32TruncF64Unsigned => {
                    let a = pop_val!(self, F64);
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let t = a.trunc();
                    ensure!(
                        t >= 0.0 && t <= u32::MAX as f64,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(t as u32 as i32);
                }
                Op::I64ExtendI32Signed => {
                    let a = pop_val!(self, I32);
                    self.stack.push(a as i64);
                }
                Op::I64ExtendI32Unsigned => {
                    let a = pop_val!(self, I32);
                    self.stack.push(a as u32 as i64);
                }
                Op::I64TruncF32Signed => {
                    let a = pop_val!(self, F32);
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let t = a.trunc();
                    ensure!(
                        t >= i64::MIN as f32 && t < i64::MAX as f32,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(t as i64);
                }
                Op::I64TruncF32Unsigned => {
                    let a = pop_val!(self, F32);
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let t = a.trunc();
                    ensure!(
                        t >= 0.0 && t < u64::MAX as f32,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(t as u64 as i64);
                }
                Op::I64TruncF64Signed => {
                    let a = pop_val!(self, F64);
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let t = a.trunc();
                    ensure!(
                        t >= i64::MIN as f64 && t < i64::MAX as f64,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(t as i64);
                }
                Op::I64TruncF64Unsigned => {
                    let a = pop_val!(self, F64);
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let t = a.trunc();
                    ensure!(
                        t >= 0.0 && t < u64::MAX as f64,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(t as u64 as i64);
                }
                Op::F32ConvertI32Signed => {
                    let a = pop_val!(self, I32);
                    self.stack.push(a as f32);
                }
                Op::F32ConvertI32Unsigned => {
                    let a = pop_val!(self, I32);
                    self.stack.push((a as u32) as f32);
                }
                Op::F32ConvertI64Signed => {
                    let a = pop_val!(self, I64);
                    self.stack.push(a as f32);
                }
                Op::F32ConvertI64Unsigned => {
                    let a = pop_val!(self, I64);
                    self.stack.push((a as u64) as f32);
                }
                Op::F32DemoteF64 => {
                    let a = pop_val!(self, F64);
                    self.stack.push(a as f32);
                }
                Op::F64ConvertI32Signed => {
                    let a = pop_val!(self, I32);
                    self.stack.push(a as f64);
                }
                Op::F64ConvertI32Unsigned => {
                    let a = pop_val!(self, I32);
                    self.stack.push((a as u32) as f64);
                }
                Op::F64ConvertI64Signed => {
                    let a = pop_val!(self, I64);
                    self.stack.push(a as f64);
                }
                Op::F64ConvertI64Unsigned => {
                    let a = pop_val!(self, I64);
                    self.stack.push((a as u64) as f64);
                }
                Op::F64PromoteF32 => {
                    let a = pop_val!(self, F32);
                    self.stack.push(a as f64);
                }
                Op::I32ReinterpretF32 => {
                    let a = pop_val!(self, F32);
                    self.stack.push(a.to_bits() as i32);
                }
                Op::I64ReinterpretF64 => {
                    let a = pop_val!(self, F64);
                    self.stack.push(a.to_bits() as i64);
                }
                Op::F32ReinterpretI32 => {
                    let a = pop_val!(self, I32);
                    self.stack.push(f32::from_bits(a as u32));
                }
                Op::F64ReinterpretI64 => {
                    let a = pop_val!(self, I64);
                    self.stack.push(f64::from_bits(a as u64));
                }
                Op::I32Extend8Signed => {
                    let a = pop_val!(self, I32);
                    self.stack.push((a as i8) as i32);
                }
                Op::I32Extend16Signed => {
                    let a = pop_val!(self, I32);
                    self.stack.push((a as i16) as i32);
                }
                Op::I64Extend8Signed => {
                    let a = pop_val!(self, I64);
                    self.stack.push((a as i8) as i64);
                }
                Op::I64Extend16Signed => {
                    let a = pop_val!(self, I64);
                    self.stack.push((a as i16) as i64);
                }
                Op::I64Extend32Signed => {
                    let a = pop_val!(self, I64);
                    self.stack.push((a as i32) as i64);
                }
                Op::I32TruncSaturatedF32Signed => {
                    let a = pop_val!(self, F32);
                    self.stack.push(if a.is_nan() { 0 } else { a as i32 });
                }
                Op::I32TruncSaturatedF32Unsigned => {
                    let a = pop_val!(self, F32);
                    let r = if a.is_nan() || a < 0.0 {
                        0u32
                    } else {
                        a as u32
                    };
                    self.stack.push(r as i32);
                }
                Op::I32TruncSaturatedF64Signed => {
                    let a = pop_val!(self, F64);
                    let r = if a.is_nan() {
                        0
                    } else if a < i32::MIN as f64 {
                        i32::MIN
                    } else if a >= i32::MAX as f64 + 1.0 {
                        i32::MAX
                    } else {
                        a as i32
                    };
                    self.stack.push(r);
                }
                Op::I32TruncSaturatedF64Unsigned => {
                    let a = pop_val!(self, F64);
                    let r = if a.is_nan() || a < 0.0 {
                        0u32
                    } else if a >= u32::MAX as f64 + 1.0 {
                        u32::MAX
                    } else {
                        a as u32
                    };
                    self.stack.push(r as i32);
                }
                Op::I64TruncSaturatedF32Signed => {
                    let a = pop_val!(self, F32);
                    let r = if a.is_nan() {
                        0i64
                    } else if a < i64::MIN as f32 {
                        i64::MIN
                    } else if a >= i64::MAX as f32 {
                        i64::MAX
                    } else {
                        a as i64
                    };
                    self.stack.push(r);
                }
                Op::I64TruncSaturatedF32Unsigned => {
                    let a = pop_val!(self, F32);
                    let r = if a.is_nan() || a < 0.0 {
                        0u64
                    } else if a >= u64::MAX as f32 {
                        u64::MAX
                    } else {
                        a as u64
                    };
                    self.stack.push(r as i64);
                }
                Op::I64TruncSaturatedF64Signed => {
                    let a = pop_val!(self, F64);
                    let r = if a.is_nan() {
                        0i64
                    } else if a < i64::MIN as f64 {
                        i64::MIN
                    } else if a >= i64::MAX as f64 {
                        i64::MAX
                    } else {
                        a as i64
                    };
                    self.stack.push(r);
                }
                Op::I64TruncSaturatedF64Unsigned => {
                    let a = pop_val!(self, F64);
                    let r = if a.is_nan() || a < 0.0 {
                        0u64
                    } else if a >= u64::MAX as f64 {
                        u64::MAX
                    } else {
                        a as u64
                    };
                    self.stack.push(r as i64);
                }
                _ => todo!(),
            }
        }
    }

    fn func_num_params(&self, func_addr: usize) -> usize {
        match &self.store.functions[func_addr] {
            FunctionInstance::Local { function_type, .. }
            | FunctionInstance::Host { function_type, .. } => function_type.0 .0.len(),
        }
    }

    fn run_init_instructions(&mut self, instructions: &[Instruction]) -> Result<()> {
        let mut ops = Vec::with_capacity(instructions.len() + 1);
        for instr in instructions {
            match instr {
                Instruction::I32Const(v) => ops.push(Op::I32Const { value: *v }),
                Instruction::I64Const(v) => ops.push(Op::I64Const { value: *v }),
                Instruction::MemoryInit(data_idx, mem) => ops.push(Op::MemoryInit {
                    data_idx: *data_idx,
                    memory_idx: *mem,
                }),
                Instruction::DataDrop(idx) => ops.push(Op::DataDrop { data_idx: *idx }),
                Instruction::TableInit(table_idx, elem_idx) => ops.push(Op::TableInit {
                    table_idx: *table_idx,
                    elem_idx: *elem_idx,
                }),
                Instruction::ElemDrop(idx) => ops.push(Op::ElemDrop { elem_idx: *idx }),
                Instruction::RefNull(ht) => ops.push(Op::RefNull(*ht)),
                Instruction::RefFunc(idx) => ops.push(Op::RefFunc { func_idx: *idx }),
                Instruction::GlobalGet(idx) => ops.push(Op::GlobalGet { global_idx: *idx }),
                Instruction::I32Add => ops.push(Op::I32Add),
                Instruction::I32Sub => ops.push(Op::I32Sub),
                Instruction::I32Mul => ops.push(Op::I32Mul),
                Instruction::I64Add => ops.push(Op::I64Add),
                Instruction::I64Sub => ops.push(Op::I64Sub),
                Instruction::I64Mul => ops.push(Op::I64Mul),
                Instruction::F32Const(v) => ops.push(Op::F32Const { value: *v }),
                Instruction::F64Const(v) => ops.push(Op::F64Const { value: *v }),
                other => bail!("unexpected instruction in init sequence: {:?}", other),
            }
        }
        ops.push(Op::Return);

        let max_stack_height = ops.len() as u32;

        let cf = CompiledFunction {
            ops,
            type_index: 0,
            num_args: 0,
            local_types: vec![],
            max_stack_height,
        };

        let compiled_func_idx = self.compiled.functions.len();
        self.compiled.functions.push(cf);

        self.call_stack.push(CallFrame {
            compiled_func_idx,
            pc: 0,
            locals: vec![],
            stack_base: self.stack.len(),
            arity: 0,
        });
        self.run()?;
        Ok(())
    }
}

fn run_data(index: u32, data_segment: &DataSegment) -> Vec<Instruction> {
    match &data_segment.mode {
        DataMode::Passive => vec![],
        DataMode::Active { memory, offset } => {
            let n = data_segment.bytes.len();
            let mut instrs = offset.clone();
            instrs.extend([
                Instruction::I32Const(0),
                Instruction::I32Const(n as i32),
                Instruction::MemoryInit(index, *memory),
                Instruction::DataDrop(index),
            ]);
            instrs
        }
    }
}

fn run_elem(index: u32, element_segment: &ElementSegment) -> Vec<Instruction> {
    match &element_segment.mode {
        ElementMode::Passive => vec![],
        ElementMode::Declarative => vec![Instruction::ElemDrop(index)],
        ElementMode::Active {
            table_index,
            offset,
        } => {
            let n = element_segment.expression.len();
            let mut instrs = offset.clone();
            instrs.extend([
                Instruction::I32Const(0),
                Instruction::I32Const(n as i32),
                Instruction::TableInit(*table_index, index),
                Instruction::ElemDrop(index),
            ]);
            instrs
        }
    }
}

fn eval_const_expr_with_module(
    expr: &[Instruction],
    store: &Store,
    module: &ModuleInstance,
) -> Result<RawValue> {
    let mut stack = Vec::with_capacity(expr.len());
    for instr in expr {
        match instr {
            Instruction::I32Const(v) => stack.push(RawValue::from(*v)),
            Instruction::I64Const(v) => stack.push(RawValue::from(*v)),
            Instruction::F32Const(v) => stack.push(RawValue::from(*v)),
            Instruction::F64Const(v) => stack.push(RawValue::from(*v)),
            Instruction::V128Const(v) => {
                let (hi, lo) = RawValue::from_v128(*v);
                stack.extend([hi, lo]);
            }
            Instruction::RefNull(_) => stack.push(RawValue::from_ref(Ref::Null)),
            Instruction::RefFunc(idx) => {
                let addr = *module
                    .function_addrs
                    .get(*idx as usize)
                    .ok_or_else(|| anyhow!("ref.func index {} oob", idx))?;
                stack.push(RawValue::from_ref(Ref::FunctionAddr(addr)));
            }
            Instruction::GlobalGet(idx) => {
                let store_idx = *module
                    .global_addrs
                    .get(*idx as usize)
                    .ok_or_else(|| anyhow!("global index {} oob in const expr", idx))?;
                let global = store
                    .globals
                    .get(store_idx)
                    .ok_or_else(|| anyhow!("global store index {} oob in const expr", store_idx))?;
                stack.push(global.value);
            }
            Instruction::RefI31 => {
                let v = const_pop_i32(&mut stack)?;
                stack.push(RawValue::from_ref(Ref::I31(v & 0x7FFF_FFFF)));
            }
            Instruction::I32Add => {
                let (b, a) = (const_pop_i32(&mut stack)?, const_pop_i32(&mut stack)?);
                stack.push(RawValue::from(a.wrapping_add(b)));
            }
            Instruction::I32Sub => {
                let (b, a) = (const_pop_i32(&mut stack)?, const_pop_i32(&mut stack)?);
                stack.push(RawValue::from(a.wrapping_sub(b)));
            }
            Instruction::I32Mul => {
                let (b, a) = (const_pop_i32(&mut stack)?, const_pop_i32(&mut stack)?);
                stack.push(RawValue::from(a.wrapping_mul(b)));
            }
            Instruction::I64Add => {
                let (b, a) = (const_pop_i64(&mut stack)?, const_pop_i64(&mut stack)?);
                stack.push(RawValue::from(a.wrapping_add(b)));
            }
            Instruction::I64Sub => {
                let (b, a) = (const_pop_i64(&mut stack)?, const_pop_i64(&mut stack)?);
                stack.push(RawValue::from(a.wrapping_sub(b)));
            }
            Instruction::I64Mul => {
                let (b, a) = (const_pop_i64(&mut stack)?, const_pop_i64(&mut stack)?);
                stack.push(RawValue::from(a.wrapping_mul(b)));
            }
            other => bail!("unexpected instruction in const expr: {:?}", other),
        }
    }
    stack
        .pop()
        .ok_or_else(|| anyhow!("const expr produced no value"))
}

fn const_pop_i32(stack: &mut Vec<RawValue>) -> Result<i32> {
    stack
        .pop()
        .map(|v| v.as_i32())
        .ok_or_else(|| anyhow!("stack underflow in const expr"))
}

fn const_pop_i64(stack: &mut Vec<RawValue>) -> Result<i64> {
    stack
        .pop()
        .map(|v| v.as_i64())
        .ok_or_else(|| anyhow!("stack underflow in const expr"))
}
