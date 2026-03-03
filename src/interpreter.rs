use anyhow::{anyhow, bail, ensure, Result};
use serde::{Deserialize, Serialize};
use std::mem;
use std::ops::Neg;

use crate::binary_grammar::{
    BlockType, DataMode, DataSegment, ElementMode, ElementSegment, ImportDescription, Instruction,
    Mutability, RefType, ValueType,
};
use crate::execution_grammar::{
    Entry, ExportInstance, ExternalValue, Frame, FunctionInstance, GlobalInstance, Label,
    ModuleInstance, Ref, Stack, Value,
};
use crate::{AddrType, Store};
use crate::{Parser, PAGE_SIZE};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExecutionState {
    Completed(Vec<Value>),
    FuelExhausted,
}

impl ExecutionState {
    pub fn into_completed(self) -> Result<Vec<Value>> {
        match self {
            Self::Completed(v) => Ok(v),
            Self::FuelExhausted => bail!("execution paused: fuel exhausted"),
        }
    }
}

enum RunOutcome {
    Completed,
    FuelExhausted,
}

#[derive(Debug, Serialize, Deserialize)]
enum ControlBlockKind {
    Block,
    Loop(Vec<Instruction>),
    IfElse,
}

#[derive(Debug, Serialize, Deserialize)]
struct ControlFrame {
    kind: ControlBlockKind,
    block_type: BlockType,
    saved_instructions: Vec<Instruction>,
    saved_pc: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct CallFrame {
    instructions: Vec<Instruction>,
    pc: usize,
    control_stack: Vec<ControlFrame>,
    frame: Frame,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Interpreter {
    module_instances: Vec<ModuleInstance>,
    stack: Stack,
    store: Store,
    globals: Vec<Value>,
    call_stack: Vec<CallFrame>,
    fuel: Option<u64>,
    pending_arity: Option<usize>,
}

impl Interpreter {
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
        let _element_instructions = module
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

        let mut stack = Stack::default();

        // step 16-18
        stack.push(Entry::Activation(Frame {
            module: module_instance_0,
            ..Default::default()
        }));

        // step 19: evaluate global init expressions sequentially so that each
        // newly created global is visible to subsequent global.get in const exprs
        let num_imported_globals = store.globals.len();
        let mut initial_global_values = Vec::new();
        for g in &module.globals {
            let value = eval_const_expr(&g.initial_expression, &store)?;
            store.globals.push(GlobalInstance {
                global_type: g.global_type.clone(),
                value,
            });
            initial_global_values.push(value);
        }
        // Remove temp globals — allocate_module will add them properly
        store.globals.truncate(num_imported_globals);

        // step 20: evaluate table init expressions
        let initial_table_refs = module
            .tables
            .iter()
            .map(|td| {
                let val = eval_const_expr(&td.init, &store)?;
                match val {
                    Value::Ref(r) => Ok(r),
                    _ => bail!("table init expr must produce a ref"),
                }
            })
            .collect::<Result<Vec<_>>>()?;

        // step 21 - todo: evaluate element segment exprs
        let _element_segment_refs: Vec<Vec<Ref>> = vec![];

        // step 22-23
        let _z = stack.pop()?;

        // save start function index before module is moved
        let start_func_idx = module.start;

        // step 24
        let module_instance = store.allocate_module(
            module,
            external_addresses,
            initial_global_values,
            initial_table_refs,
            _element_segment_refs,
        )?;

        // step 25-26
        stack.push(Entry::Activation(Frame {
            module: module_instance.clone(),
            ..Default::default()
        }));

        // step 27 - todo: execute _element_instructions

        // step 28 - execute data segment initialization
        // step 29 - todo: if module has start function, call it

        // step 30
        stack.pop()?;

        let mut interpreter = Self {
            module_instances: vec![module_instance],
            stack,
            store,
            globals: vec![],
            call_stack: vec![],
            fuel: None,
            pending_arity: None,
        };

        if !data_instructions.is_empty() {
            let frame = Frame {
                module: interpreter.module_instances[0].clone(),
                ..Default::default()
            };
            interpreter.stack.push(Entry::Activation(frame.clone()));
            interpreter.call_stack.push(CallFrame {
                instructions: data_instructions,
                pc: 0,
                control_stack: vec![],
                frame,
            });
            interpreter.run()?;
        }

        // step 29: invoke start function if present
        if let Some(start_idx) = start_func_idx {
            let func_addr = *interpreter.module_instances[0]
                .function_addrs
                .get(start_idx as usize)
                .ok_or_else(|| anyhow!("start function index {} out of bounds", start_idx))?;
            interpreter.push_function_call(func_addr)?;
            interpreter.run()?;
        }

        // step 31
        Ok(interpreter)
    }

    pub fn snapshot(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|e| anyhow!("snapshot failed: {}", e))
    }

    pub fn from_snapshot(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).map_err(|e| anyhow!("restore failed: {}", e))
    }

    pub fn invoke(&mut self, name: &str, args: Vec<Value>) -> Result<ExecutionState> {
        let addr = self.get_export_func_addr(name)?;
        self.invoke_by_addr(addr, args)
    }

    pub fn resume(&mut self) -> Result<ExecutionState> {
        let arity = self
            .pending_arity
            .ok_or_else(|| anyhow!("no pending execution to resume"))?;

        match self.run() {
            Ok(RunOutcome::Completed) => {
                let results = self
                    .stack
                    .pop_n(arity)?
                    .into_iter()
                    .map(|entry| match entry {
                        Entry::Value(v) => Ok(v),
                        _ => Err(anyhow!("Expected value on stack")),
                    })
                    .collect::<Result<Vec<_>>>()?;
                self.pending_arity = None;
                Ok(ExecutionState::Completed(results))
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
        self.get_func_param_types(addr)
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
        &self.module_instances[0].exports
    }

    fn get_func_param_types(&self, function_addr: usize) -> Result<Vec<ValueType>> {
        let function_instance =
            self.store.functions.get(function_addr).ok_or_else(|| {
                anyhow!("Function address {} does not exist in store", function_addr)
            })?;

        match function_instance {
            FunctionInstance::Local { function_type, .. }
            | FunctionInstance::Host { function_type, .. } => Ok(function_type.0 .0.clone()),
        }
    }

    fn get_export_func_addr(&self, name: &str) -> Result<usize> {
        let module = self
            .module_instances
            .first()
            .ok_or_else(|| anyhow!("No module instance"))?;

        for export in &module.exports {
            if export.name == name {
                if let ExternalValue::Function { addr } = export.value {
                    return Ok(addr);
                }
                bail!("Export '{}' is not a function", name);
            }
        }

        bail!("Export '{}' not found", name)
    }

    fn invoke_by_addr(&mut self, function_addr: usize, args: Vec<Value>) -> Result<ExecutionState> {
        if self.pending_arity.is_some() {
            bail!("cannot invoke while execution is paused; call resume() first");
        }

        let function_instance = self.store.functions.get(function_addr).ok_or_else(|| {
            anyhow!(
                "Function address: {} does not exist in store.",
                function_addr
            )
        })?;

        let num_results = match function_instance {
            FunctionInstance::Local { function_type, .. } => {
                let f_num_args = function_type.0 .0.len();
                ensure!(f_num_args == args.len(), "Length of provided argument values is different from the number of expected arguments.");

                // validate that every value correspond with value type
                for (value_type, value) in function_type.0 .0.iter().zip(&args) {
                    match (value, value_type) {
                        (Value::I32(_), ValueType::I32)
                        | (Value::I64(_), ValueType::I64)
                        | (Value::F32(_), ValueType::F32)
                        | (Value::F64(_), ValueType::F64)
                        | (Value::V128(_), ValueType::V128)
                        | (Value::Ref(Ref::FunctionAddr(_)), ValueType::Ref(RefType::FuncRef))
                        | (Value::Ref(Ref::RefExtern(_)), ValueType::Ref(RefType::ExternRef)) => {}
                        _ => bail!("Value does not correspond with value type."),
                    }
                }

                function_type.1 .0.len()
            }
            _ => todo!("Handle host function instance"),
        };

        self.stack.extend(args.into_iter().map(Entry::Value));

        self.push_function_call(function_addr)?;
        match self.run() {
            Ok(RunOutcome::Completed) => {
                let results = self
                    .stack
                    .pop_n(num_results)?
                    .into_iter()
                    .map(|entry| match entry {
                        Entry::Value(v) => Ok(v),
                        _ => Err(anyhow!("Expected value on stack")),
                    })
                    .collect::<Result<Vec<_>>>()?;
                self.pending_arity = None;
                Ok(ExecutionState::Completed(results))
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

    fn handle_branch(&mut self, l: u32, depth: usize) -> Result<()> {
        self.stack.pop_to_label(l)?;
        let num_frames = self.call_stack[depth].control_stack.len();
        if l as usize >= num_frames {
            // Branch past all control frames → trigger function exit
            self.call_stack[depth].control_stack.clear();
            self.call_stack[depth].pc = self.call_stack[depth].instructions.len();
            return Ok(());
        }
        self.call_stack[depth]
            .control_stack
            .truncate(num_frames - l as usize);
        let target = self.call_stack[depth].control_stack.pop().unwrap();
        match target.kind {
            ControlBlockKind::Block | ControlBlockKind::IfElse => {
                self.call_stack[depth].instructions = target.saved_instructions;
                self.call_stack[depth].pc = target.saved_pc;
            }
            ControlBlockKind::Loop(body) => {
                let (m, _) = resolve_block_type(&target.block_type, &self.call_stack[depth].frame);
                let inputs = self.stack.pop_n(m)?;
                self.stack
                    .extend(std::iter::once(Entry::Label(Label { arity: m as u32 })).chain(inputs));
                self.call_stack[depth].instructions = body.clone();
                self.call_stack[depth].pc = 0;
                self.call_stack[depth].control_stack.push(ControlFrame {
                    kind: ControlBlockKind::Loop(body),
                    block_type: target.block_type,
                    saved_instructions: target.saved_instructions,
                    saved_pc: target.saved_pc,
                });
            }
        }
        Ok(())
    }

    fn run(&mut self) -> Result<RunOutcome> {
        loop {
            let depth = match self.call_stack.len() {
                0 => return Ok(RunOutcome::Completed),
                n => n - 1,
            };

            if self.call_stack[depth].pc >= self.call_stack[depth].instructions.len() {
                if !self.call_stack[depth].control_stack.is_empty() {
                    // Block exit
                    let cf = self.call_stack[depth].control_stack.pop().unwrap();
                    let (_, n) = resolve_block_type(&cf.block_type, &self.call_stack[depth].frame);
                    self.stack.pop_to_label_with_arity(0, n)?;
                    self.call_stack[depth].instructions = cf.saved_instructions;
                    self.call_stack[depth].pc = cf.saved_pc;
                } else {
                    // Function return: (frame_m{f} val^m) → val^m
                    // The function-level label may or may not still be on the
                    // value stack depending on whether we got here via normal
                    // completion (label still present) or a branch that already
                    // consumed it. Pop results, strip down to the Activation,
                    // then push results back.
                    let arity = self.call_stack[depth].frame.arity;
                    let results = self.stack.pop_n(arity)?;
                    loop {
                        match self.stack.pop()? {
                            Entry::Activation(_) => break,
                            Entry::Label(_) => continue,
                            other => bail!("Unexpected entry on function exit: {:?}", other),
                        }
                    }
                    self.stack.extend(results);
                    self.call_stack.pop();
                }
                continue;
            }

            // clone to release the borrow on self.call_stack so we can mutate freely in every arm
            // todo: use instruction indices into a shared instruction pool for cheaper clones?
            let instruction =
                self.call_stack[depth].instructions[self.call_stack[depth].pc].clone();
            self.call_stack[depth].pc += 1;

            if let Some(ref mut fuel) = self.fuel {
                if *fuel == 0 {
                    self.call_stack[depth].pc -= 1; // undo so resume re-executes this instruction
                    return Ok(RunOutcome::FuelExhausted);
                }
                *fuel -= 1;
            }

            match instruction {
                Instruction::Unreachable => bail!("unreachable executed"),
                Instruction::Nop => {}
                Instruction::Block(bt, body) => {
                    let (m, n) = resolve_block_type(&bt, &self.call_stack[depth].frame);
                    let inputs = self.stack.pop_n(m)?;
                    self.stack.extend(
                        std::iter::once(Entry::Label(Label { arity: n as u32 })).chain(inputs),
                    );
                    let saved = mem::replace(&mut self.call_stack[depth].instructions, body);
                    let saved_pc = self.call_stack[depth].pc;
                    self.call_stack[depth].pc = 0;
                    self.call_stack[depth].control_stack.push(ControlFrame {
                        kind: ControlBlockKind::Block,
                        block_type: bt,
                        saved_instructions: saved,
                        saved_pc,
                    });
                }
                Instruction::Loop(bt, body) => {
                    let (m, _n) = resolve_block_type(&bt, &self.call_stack[depth].frame);
                    let inputs = self.stack.pop_n(m)?;
                    self.stack.extend(
                        std::iter::once(Entry::Label(Label { arity: m as u32 })).chain(inputs),
                    );
                    let saved =
                        mem::replace(&mut self.call_stack[depth].instructions, body.clone());
                    let saved_pc = self.call_stack[depth].pc;
                    self.call_stack[depth].pc = 0;
                    self.call_stack[depth].control_stack.push(ControlFrame {
                        kind: ControlBlockKind::Loop(body),
                        block_type: bt,
                        saved_instructions: saved,
                        saved_pc,
                    });
                }
                Instruction::IfElse(bt, then_body, else_body) => {
                    let cond: i32 = self.stack.pop_value()?.try_into()?;
                    let (m, n) = resolve_block_type(&bt, &self.call_stack[depth].frame);
                    let inputs = self.stack.pop_n(m)?;
                    self.stack.extend(
                        std::iter::once(Entry::Label(Label { arity: n as u32 })).chain(inputs),
                    );
                    let body = if cond != 0 { then_body } else { else_body };
                    let saved = mem::replace(&mut self.call_stack[depth].instructions, body);
                    let saved_pc = self.call_stack[depth].pc;
                    self.call_stack[depth].pc = 0;
                    self.call_stack[depth].control_stack.push(ControlFrame {
                        kind: ControlBlockKind::IfElse,
                        block_type: bt,
                        saved_instructions: saved,
                        saved_pc,
                    });
                }
                Instruction::Br(l) => {
                    self.handle_branch(l, depth)?;
                }
                Instruction::BrIf(l) => {
                    let cond: i32 = self.stack.pop_value()?.try_into()?;
                    if cond != 0 {
                        self.handle_branch(l, depth)?;
                    }
                }
                Instruction::BrTable(labels, default) => {
                    let i: i32 = self.stack.pop_value()?.try_into()?;
                    let i = i as usize;

                    let l = if i < labels.len() { labels[i] } else { default };
                    self.handle_branch(l, depth)?;
                }
                Instruction::Throw(_) => todo!(),
                Instruction::ThrowRef => todo!(),
                Instruction::Return => {
                    let arity = self.call_stack[depth].frame.arity;
                    let results = self.stack.pop_n(arity)?;
                    loop {
                        match self.stack.pop()? {
                            Entry::Activation(_) => break,
                            _ => continue,
                        }
                    }
                    self.stack.extend(results);
                    self.call_stack[depth].control_stack.clear();
                    self.call_stack.pop();
                }
                Instruction::Call(x) => {
                    let a = *self.call_stack[depth]
                        .frame
                        .module
                        .function_addrs
                        .get(x as usize)
                        .ok_or_else(|| anyhow!("Function index {} out of bounds", x))?;
                    self.push_function_call(a)?;
                }
                Instruction::CallIndirect(_, _) => todo!(),
                Instruction::ReturnCall(_) => todo!(),
                Instruction::ReturnCallIndirect(_, _) => todo!(),
                Instruction::CallRef(_) => todo!(),
                Instruction::ReturnCallRef(_) => todo!(),
                Instruction::TryTable(_, _, _) => todo!(),
                Instruction::BrOnNull(_) => todo!(),
                Instruction::BrOnNonNull(_) => todo!(),
                Instruction::RefNull(_) => {
                    self.stack.push(Value::Ref(Ref::Null));
                }
                Instruction::RefIsNull => {
                    let val = self.stack.pop_value()?;
                    let is_null = matches!(val, Value::Ref(Ref::Null));
                    self.stack.push(is_null as i32);
                }
                Instruction::RefFunc(x) => {
                    let addr = *self.call_stack[depth]
                        .frame
                        .module
                        .function_addrs
                        .get(x as usize)
                        .ok_or_else(|| anyhow!("Function index {} out of bounds", x))?;

                    self.stack.push(Value::Ref(Ref::FunctionAddr(addr)));
                }
                Instruction::Drop => {
                    let _ = self.stack.pop();
                }
                Instruction::Select(_) => {
                    let [Entry::Value(val1), Entry::Value(val2), Entry::Value(Value::I32(cond))] =
                        self.stack.pop_array()?
                    else {
                        bail!("expected values")
                    };

                    if cond != 0 {
                        self.stack.push(val1);
                    } else {
                        self.stack.push(val2);
                    }
                }
                Instruction::LocalGet(idx) => {
                    let val = self.call_stack[depth]
                        .frame
                        .locals
                        .get(idx as usize)
                        .ok_or_else(|| anyhow!("Local index {} out of bounds", idx))?;
                    self.stack.push(Entry::Value(*val));
                }
                Instruction::LocalSet(i) => {
                    let val = self.stack.pop_value()?;
                    self.call_stack[depth].frame.locals[i as usize] = val;
                }
                Instruction::LocalTee(i) => {
                    let val = self.stack.pop_value()?;
                    self.call_stack[depth].frame.locals[i as usize] = val;
                    self.stack.push(val);
                }
                Instruction::GlobalGet(i) => {
                    let &addr = self.call_stack[depth]
                        .frame
                        .module
                        .global_addrs
                        .get(i as usize)
                        .ok_or_else(|| anyhow!("oob"))?;

                    let val = self.store.globals[addr].value;
                    self.stack.push(Entry::Value(val))
                }
                Instruction::GlobalSet(i) => {
                    let addr = *self.call_stack[depth]
                        .frame
                        .module
                        .global_addrs
                        .get(i as usize)
                        .ok_or_else(|| anyhow!("oob"))?;

                    let global = &mut self.store.globals[addr];
                    ensure!(
                        matches!(global.global_type.mutability, Mutability::Var),
                        "cannot set immutable global {}",
                        i
                    );

                    global.value = self.stack.pop_value()?;
                }
                Instruction::TableGet(_) => todo!(),
                Instruction::TableSet(_) => todo!(),
                Instruction::TableInit(_, _) => todo!(),
                Instruction::ElemDrop(_) => todo!(),
                Instruction::TableCopy(_, _) => todo!(),
                Instruction::TableGrow(_) => todo!(),
                Instruction::TableSize(_) => todo!(),
                Instruction::TableFill(_) => todo!(),
                Instruction::I32Load(ma) => {
                    mem_load!(self, depth, ma, 4, |b| i32::from_le_bytes(b))
                }
                Instruction::I64Load(ma) => {
                    mem_load!(self, depth, ma, 8, |b| i64::from_le_bytes(b))
                }
                Instruction::F32Load(ma) => {
                    mem_load!(self, depth, ma, 4, |b| f32::from_le_bytes(b))
                }
                Instruction::F64Load(ma) => {
                    mem_load!(self, depth, ma, 8, |b| f64::from_le_bytes(b))
                }
                Instruction::I32Load8Signed(ma) => {
                    mem_load!(self, depth, ma, 1, |b| b[0] as i8 as i32)
                }
                Instruction::I32Load8Unsigned(ma) => {
                    mem_load!(self, depth, ma, 1, |b| b[0] as u8 as i32)
                }
                Instruction::I32Load16Signed(ma) => {
                    mem_load!(self, depth, ma, 2, |b| i16::from_le_bytes(b) as i32)
                }
                Instruction::I32Load16Unsigned(ma) => {
                    mem_load!(self, depth, ma, 2, |b| u16::from_le_bytes(b) as i32)
                }
                Instruction::I64Load8Signed(ma) => {
                    mem_load!(self, depth, ma, 1, |b| b[0] as i8 as i64)
                }
                Instruction::I64Load8Unsigned(ma) => {
                    mem_load!(self, depth, ma, 1, |b| b[0] as u8 as i64)
                }
                Instruction::I64Load16Signed(ma) => {
                    mem_load!(self, depth, ma, 2, |b| i16::from_le_bytes(b) as i64)
                }
                Instruction::I64Load16Unsigned(ma) => {
                    mem_load!(self, depth, ma, 2, |b| u16::from_le_bytes(b) as i64)
                }
                Instruction::I64Load32Signed(ma) => {
                    mem_load!(self, depth, ma, 4, |b| i32::from_le_bytes(b) as i64)
                }
                Instruction::I64Load32Unsigned(ma) => {
                    mem_load!(self, depth, ma, 4, |b| u32::from_le_bytes(b) as i64)
                }
                Instruction::I32Store(ma) => mem_store!(self, depth, ma, 4, |v| {
                    let Value::I32(c) = v else {
                        bail!("expected i32")
                    };
                    c.to_le_bytes()
                }),
                Instruction::I64Store(ma) => mem_store!(self, depth, ma, 8, |v| {
                    let Value::I64(c) = v else {
                        bail!("expected i64")
                    };
                    c.to_le_bytes()
                }),
                Instruction::F32Store(ma) => mem_store!(self, depth, ma, 4, |v| {
                    let Value::F32(c) = v else {
                        bail!("expected f32")
                    };
                    c.to_le_bytes()
                }),
                Instruction::F64Store(ma) => mem_store!(self, depth, ma, 8, |v| {
                    let Value::F64(c) = v else {
                        bail!("expected f64")
                    };
                    c.to_le_bytes()
                }),
                Instruction::I32Store8(ma) => mem_store!(self, depth, ma, 1, |v| {
                    let Value::I32(c) = v else {
                        bail!("expected i32")
                    };
                    (c as u8).to_le_bytes()
                }),
                Instruction::I32Store16(ma) => mem_store!(self, depth, ma, 2, |v| {
                    let Value::I32(c) = v else {
                        bail!("expected i32")
                    };
                    (c as u16).to_le_bytes()
                }),
                Instruction::I64Store8(ma) => mem_store!(self, depth, ma, 1, |v| {
                    let Value::I64(c) = v else {
                        bail!("expected i64")
                    };
                    (c as u8).to_le_bytes()
                }),
                Instruction::I64Store16(ma) => mem_store!(self, depth, ma, 2, |v| {
                    let Value::I64(c) = v else {
                        bail!("expected i64")
                    };
                    (c as u16).to_le_bytes()
                }),
                Instruction::I64Store32(ma) => mem_store!(self, depth, ma, 4, |v| {
                    let Value::I64(c) = v else {
                        bail!("expected i64")
                    };
                    (c as u32).to_le_bytes()
                }),
                Instruction::MemorySize(i) => {
                    let frame_module = &self.call_stack[depth].frame.module;
                    let mem_addr = frame_module.mem_addrs[i as usize];
                    let mem_instance = &self.store.memories[mem_addr];

                    let size = mem_instance.data.len() / PAGE_SIZE;

                    match mem_instance.memory_type.addr_type {
                        AddrType::I32 => self.stack.push(size as i32),
                        AddrType::I64 => self.stack.push(size as i64),
                    }
                }
                Instruction::MemoryGrow(i) => {
                    let frame_module = &self.call_stack[depth].frame.module;
                    let mem_addr = frame_module.mem_addrs[i as usize];
                    let mem_instance = &mut self.store.memories[mem_addr];

                    let page_count_to_grow =
                        match (mem_instance.memory_type.addr_type, self.stack.pop_value()?) {
                            (AddrType::I32, Value::I32(n)) => n as usize,
                            (AddrType::I64, Value::I64(n)) => n as usize,
                            (addr_type, foreign) => {
                                bail!("expected {addr_type:?} value, got: {foreign:?}")
                            }
                        };

                    let old_size = mem_instance.data.len() / PAGE_SIZE;
                    let new_size = old_size + page_count_to_grow;

                    // 4 GiB — max addressable by i32
                    const MAX_PAGES: usize = 65536;
                    if new_size > MAX_PAGES || new_size as u64 > mem_instance.memory_type.limit.max
                    {
                        match mem_instance.memory_type.addr_type {
                            AddrType::I32 => self.stack.push(-1_i32),
                            AddrType::I64 => self.stack.push(-1_i64),
                        }

                        continue;
                    }

                    mem_instance.data.resize(new_size * PAGE_SIZE, 0);
                    mem_instance.memory_type.limit.min = new_size as u64;

                    match mem_instance.memory_type.addr_type {
                        AddrType::I32 => self.stack.push(old_size as i32),
                        AddrType::I64 => self.stack.push(old_size as i64),
                    }
                }
                Instruction::MemoryInit(data_idx, mem_idx) => {
                    let frame_module = &self.call_stack[depth].frame.module;
                    let mem_addr = frame_module.mem_addrs[mem_idx as usize];
                    let data_addr = frame_module.data_addrs[data_idx as usize];
                    let addr_type = self.store.memories[mem_addr].memory_type.addr_type;

                    let n = match self.stack.pop_value()? {
                        Value::I32(v) => v as usize,
                        v => bail!("expected i32 count, got: {v:?}"),
                    };
                    let s = match self.stack.pop_value()? {
                        Value::I32(v) => v as usize,
                        v => bail!("expected i32 source offset, got: {v:?}"),
                    };
                    let d: usize = match (addr_type, self.stack.pop_value()?) {
                        (AddrType::I32, Value::I32(v)) => v as usize,
                        (AddrType::I64, Value::I64(v)) => v as usize,
                        (at, v) => bail!("expected {at:?} dest offset, got: {v:?}"),
                    };

                    if s.saturating_add(n) > self.store.data_segments[data_addr].data.len() {
                        bail!("trap: out of bounds memory access");
                    }
                    if d.saturating_add(n) > self.store.memories[mem_addr].data.len() {
                        bail!("trap: out of bounds memory access");
                    }

                    if n == 0 {
                        continue;
                    }

                    let src = self.store.data_segments[data_addr].data[s..s + n].to_vec();
                    self.store.memories[mem_addr].data[d..d + n].copy_from_slice(&src);
                }
                Instruction::DataDrop(x) => {
                    let frame_module = &self.call_stack[depth].frame.module;
                    let data_addr = frame_module.data_addrs[x as usize];
                    self.store.data_segments[data_addr].data.clear();
                }
                Instruction::MemoryCopy(x1, x2) => {
                    let frame_module = &self.call_stack[depth].frame.module;
                    let mem_addr1 = frame_module.mem_addrs[x1 as usize];
                    let mem_addr2 = frame_module.mem_addrs[x2 as usize];
                    let addr_type = self.store.memories[mem_addr1].memory_type.addr_type;

                    let n: u64 = match (addr_type, self.stack.pop_value()?) {
                        (AddrType::I32, Value::I32(v)) => v as u64,
                        (AddrType::I64, Value::I64(v)) => v as u64,
                        (at, v) => bail!("expected {at:?} value, got: {v:?}"),
                    };
                    let i2: u64 = match (addr_type, self.stack.pop_value()?) {
                        (AddrType::I32, Value::I32(v)) => v as u64,
                        (AddrType::I64, Value::I64(v)) => v as u64,
                        (at, v) => bail!("expected {at:?} value, got: {v:?}"),
                    };
                    let i1: u64 = match (addr_type, self.stack.pop_value()?) {
                        (AddrType::I32, Value::I32(v)) => v as u64,
                        (AddrType::I64, Value::I64(v)) => v as u64,
                        (at, v) => bail!("expected {at:?} value, got: {v:?}"),
                    };

                    let (n, i1, i2) = (n as usize, i1 as usize, i2 as usize);

                    if i1.saturating_add(n) > self.store.memories[mem_addr1].data.len() {
                        bail!("trap: out of bounds memory access");
                    }
                    if i2.saturating_add(n) > self.store.memories[mem_addr2].data.len() {
                        bail!("trap: out of bounds memory access");
                    }

                    if n == 0 {
                        continue;
                    }

                    if mem_addr1 == mem_addr2 {
                        self.store.memories[mem_addr1]
                            .data
                            .copy_within(i2..i2 + n, i1);
                    } else {
                        let src = self.store.memories[mem_addr2].data[i2..i2 + n].to_vec();
                        self.store.memories[mem_addr1].data[i1..i1 + n].copy_from_slice(&src);
                    }
                }
                Instruction::MemoryFill(i) => {
                    let frame_module = &self.call_stack[depth].frame.module;
                    let mem_addr = frame_module.mem_addrs[i as usize];
                    let addr_type = self.store.memories[mem_addr].memory_type.addr_type;

                    let n = match (addr_type, self.stack.pop_value()?) {
                        (AddrType::I32, Value::I32(v)) => v as u64,
                        (AddrType::I64, Value::I64(v)) => v as u64,
                        (at, v) => bail!("expected {at:?} value, got: {v:?}"),
                    };
                    let val: i32 = self.stack.pop_value()?.try_into()?;
                    let i: u64 = match (addr_type, self.stack.pop_value()?) {
                        (AddrType::I32, Value::I32(v)) => v as u64,
                        (AddrType::I64, Value::I64(v)) => v as u64,
                        (at, v) => bail!("expected {at:?} value, got: {v:?}"),
                    };

                    let (n, i) = (n as usize, i as usize);
                    let mem = &mut self.store.memories[mem_addr];

                    if i.saturating_add(n) > mem.data.len() {
                        bail!("trap: out of bounds memory access");
                    }

                    if n > 0 {
                        mem.data[i..i + n].fill(val as u8);
                    }
                }
                Instruction::I32Const(v) => {
                    self.stack.push(v);
                }
                Instruction::I64Const(v) => {
                    self.stack.push(v);
                }
                Instruction::F32Const(v) => {
                    self.stack.push(v);
                }
                Instruction::F64Const(v) => {
                    self.stack.push(v);
                }
                Instruction::I32EqZero => {
                    let a: i32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push((a == 0) as i32);
                }
                Instruction::I32Eq => cmpop!(self.stack, I32, |b, a| b == a),
                Instruction::I32Ne => cmpop!(self.stack, I32, |b, a| b != a),
                Instruction::I32LtSigned => cmpop!(self.stack, I32, |b, a| b < a),
                Instruction::I32LtUnsigned => {
                    cmpop!(self.stack, I32, |b, a| (b as u32) < (a as u32))
                }
                Instruction::I32GtSigned => cmpop!(self.stack, I32, |b, a| b > a),
                Instruction::I32GtUnsigned => {
                    cmpop!(self.stack, I32, |b, a| (b as u32) > (a as u32))
                }
                Instruction::I32LeSigned => cmpop!(self.stack, I32, |b, a| b <= a),
                Instruction::I32LeUnsigned => {
                    cmpop!(self.stack, I32, |b, a| (b as u32) <= (a as u32))
                }
                Instruction::I32GeSigned => cmpop!(self.stack, I32, |b, a| b >= a),
                Instruction::I32GeUnsigned => {
                    cmpop!(self.stack, I32, |b, a| (b as u32) >= (a as u32))
                }
                Instruction::I64EqZero => {
                    let a: i64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push((a == 0) as i32);
                }
                Instruction::I64Eq => cmpop!(self.stack, I64, |b, a| b == a),
                Instruction::I64Ne => cmpop!(self.stack, I64, |b, a| b != a),
                Instruction::I64LtSigned => cmpop!(self.stack, I64, |b, a| b < a),
                Instruction::I64LtUnsigned => {
                    cmpop!(self.stack, I64, |b, a| (b as u64) < (a as u64))
                }
                Instruction::I64GtSigned => cmpop!(self.stack, I64, |b, a| b > a),
                Instruction::I64GtUnsigned => {
                    cmpop!(self.stack, I64, |b, a| (b as u64) > (a as u64))
                }
                Instruction::I64LeSigned => cmpop!(self.stack, I64, |b, a| b <= a),
                Instruction::I64LeUnsigned => {
                    cmpop!(self.stack, I64, |b, a| (b as u64) <= (a as u64))
                }
                Instruction::I64GeSigned => cmpop!(self.stack, I64, |b, a| b >= a),
                Instruction::I64GeUnsigned => {
                    cmpop!(self.stack, I64, |b, a| (b as u64) >= (a as u64))
                }
                Instruction::F32Eq => cmpop!(self.stack, F32, |b, a| b == a),
                Instruction::F32Ne => cmpop!(self.stack, F32, |b, a| b != a),
                Instruction::F32Lt => cmpop!(self.stack, F32, |b, a| b < a),
                Instruction::F32Gt => cmpop!(self.stack, F32, |b, a| b > a),
                Instruction::F32Le => cmpop!(self.stack, F32, |b, a| b <= a),
                Instruction::F32Ge => cmpop!(self.stack, F32, |b, a| b >= a),
                Instruction::F64Eq => cmpop!(self.stack, F64, |b, a| b == a),
                Instruction::F64Ne => cmpop!(self.stack, F64, |b, a| b != a),
                Instruction::F64Lt => cmpop!(self.stack, F64, |b, a| b < a),
                Instruction::F64Gt => cmpop!(self.stack, F64, |b, a| b > a),
                Instruction::F64Le => cmpop!(self.stack, F64, |b, a| b <= a),
                Instruction::F64Ge => cmpop!(self.stack, F64, |b, a| b >= a),
                Instruction::I32CountLeadingZeros => {
                    let a: i32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.leading_zeros() as i32);
                }
                Instruction::I32CountTrailingZeros => {
                    let a: i32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.trailing_zeros() as i32);
                }
                Instruction::I32PopCount => {
                    let a: i32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.count_ones() as i32);
                }
                Instruction::I32Add => binop!(self.stack, I32, |b, a| b.wrapping_add(a)),
                Instruction::I32Sub => binop!(self.stack, I32, |b, a| b.wrapping_sub(a)),
                Instruction::I32Mul => binop!(self.stack, I32, |b, a| b.wrapping_mul(a)),
                Instruction::I32DivSigned => {
                    let [Entry::Value(Value::I32(b)), Entry::Value(Value::I32(a))] =
                        self.stack.pop_array()?
                    else {
                        bail!("expected i32s")
                    };
                    ensure!(a != 0, "wasm trap: integer divide by zero");
                    ensure!(!(b == i32::MIN && a == -1), "wasm trap: integer overflow");
                    self.stack.push(b.wrapping_div(a));
                }
                Instruction::I32DivUnsigned => {
                    let [Entry::Value(Value::I32(b)), Entry::Value(Value::I32(a))] =
                        self.stack.pop_array()?
                    else {
                        bail!("expected i32s")
                    };
                    ensure!(a != 0, "wasm trap: integer divide by zero");
                    self.stack.push(((b as u32) / (a as u32)) as i32);
                }
                Instruction::I32RemainderSigned => {
                    let [Entry::Value(Value::I32(b)), Entry::Value(Value::I32(a))] =
                        self.stack.pop_array()?
                    else {
                        bail!("expected i32s")
                    };
                    ensure!(a != 0, "wasm trap: integer divide by zero");
                    self.stack.push(b.wrapping_rem(a));
                }
                Instruction::I32RemainderUnsigned => {
                    let [Entry::Value(Value::I32(b)), Entry::Value(Value::I32(a))] =
                        self.stack.pop_array()?
                    else {
                        bail!("expected i32s")
                    };
                    ensure!(a != 0, "wasm trap: integer divide by zero");
                    self.stack.push(((b as u32) % (a as u32)) as i32);
                }
                Instruction::I32And => binop!(self.stack, I32, |b, a| b & a),
                Instruction::I32Or => binop!(self.stack, I32, |b, a| b | a),
                Instruction::I32Xor => binop!(self.stack, I32, |b, a| b ^ a),
                Instruction::I32Shl => {
                    binop!(self.stack, I32, |b, a| b.wrapping_shl(a as u32 % 32))
                }
                Instruction::I32ShrSigned => {
                    binop!(self.stack, I32, |b, a| b.wrapping_shr(a as u32 % 32))
                }
                Instruction::I32ShrUnsigned => {
                    binop!(
                        self.stack,
                        I32,
                        |b, a| ((b as u32).wrapping_shr(a as u32 % 32)) as i32
                    )
                }
                Instruction::I32RotateLeft => {
                    binop!(self.stack, I32, |b, a| b.rotate_left(a as u32 % 32))
                }
                Instruction::I32RotateRight => {
                    binop!(self.stack, I32, |b, a| b.rotate_right(a as u32 % 32))
                }
                Instruction::I64CountLeadingZeros => {
                    let a: i64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.leading_zeros() as i64);
                }
                Instruction::I64CountTrailingZeros => {
                    let a: i64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.trailing_zeros() as i64);
                }
                Instruction::I64PopCount => {
                    let a: i64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.count_ones() as i64);
                }
                Instruction::I64Add => binop!(self.stack, I64, |b, a| b.wrapping_add(a)),
                Instruction::I64Sub => binop!(self.stack, I64, |b, a| b.wrapping_sub(a)),
                Instruction::I64Mul => binop!(self.stack, I64, |b, a| b.wrapping_mul(a)),
                Instruction::I64DivSigned => {
                    let [Entry::Value(Value::I64(b)), Entry::Value(Value::I64(a))] =
                        self.stack.pop_array()?
                    else {
                        bail!("expected i64s")
                    };
                    ensure!(a != 0, "wasm trap: integer divide by zero");
                    ensure!(!(b == i64::MIN && a == -1), "wasm trap: integer overflow");
                    self.stack.push(b.wrapping_div(a));
                }
                Instruction::I64DivUnsigned => {
                    let [Entry::Value(Value::I64(b)), Entry::Value(Value::I64(a))] =
                        self.stack.pop_array()?
                    else {
                        bail!("expected i64s")
                    };
                    ensure!(a != 0, "wasm trap: integer divide by zero");
                    self.stack.push(((b as u64) / (a as u64)) as i64);
                }
                Instruction::I64RemainderSigned => {
                    let [Entry::Value(Value::I64(b)), Entry::Value(Value::I64(a))] =
                        self.stack.pop_array()?
                    else {
                        bail!("expected i64s")
                    };
                    ensure!(a != 0, "wasm trap: integer divide by zero");
                    self.stack.push(b.wrapping_rem(a));
                }
                Instruction::I64RemainderUnsigned => {
                    let [Entry::Value(Value::I64(b)), Entry::Value(Value::I64(a))] =
                        self.stack.pop_array()?
                    else {
                        bail!("expected i64s")
                    };
                    ensure!(a != 0, "wasm trap: integer divide by zero");
                    self.stack.push(((b as u64) % (a as u64)) as i64);
                }
                Instruction::I64And => binop!(self.stack, I64, |b, a| b & a),
                Instruction::I64Or => binop!(self.stack, I64, |b, a| b | a),
                Instruction::I64Xor => binop!(self.stack, I64, |b, a| b ^ a),
                Instruction::I64Shl => {
                    binop!(self.stack, I64, |b, a| b.wrapping_shl(a as u32 % 64))
                }
                Instruction::I64ShrSigned => {
                    binop!(self.stack, I64, |b, a| b.wrapping_shr(a as u32 % 64))
                }
                Instruction::I64ShrUnsigned => {
                    binop!(
                        self.stack,
                        I64,
                        |b, a| ((b as u64).wrapping_shr(a as u32 % 64)) as i64
                    )
                }
                Instruction::I64RotateLeft => {
                    binop!(self.stack, I64, |b, a| b.rotate_left(a as u32 % 64))
                }
                Instruction::I64RotateRight => {
                    binop!(self.stack, I64, |b, a| b.rotate_right(a as u32 % 64))
                }
                Instruction::F32Abs => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.abs());
                }
                Instruction::F32Neg => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.neg());
                }
                Instruction::F32Ceil => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.ceil());
                }
                Instruction::F32Floor => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.floor());
                }
                Instruction::F32Trunc => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.trunc());
                }
                Instruction::F32Nearest => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.round_ties_even());
                }
                Instruction::F32Sqrt => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.sqrt());
                }
                Instruction::F32Add => binop!(self.stack, F32, |b, a| b + a),
                Instruction::F32Sub => binop!(self.stack, F32, |b, a| b - a),
                Instruction::F32Mul => binop!(self.stack, F32, |b, a| b * a),
                Instruction::F32Div => binop!(self.stack, F32, |b, a| b / a),
                Instruction::F32Min => {
                    let [Entry::Value(Value::F32(b)), Entry::Value(Value::F32(a))] =
                        self.stack.pop_array()?
                    else {
                        bail!("expected f32s")
                    };
                    let result = if a.is_nan() || b.is_nan() {
                        f32::from_bits(0x7FC0_0000)
                    } else if a == b {
                        f32::from_bits(a.to_bits() | b.to_bits())
                    } else {
                        a.min(b)
                    };
                    self.stack.push(result);
                }
                Instruction::F32Max => {
                    let [Entry::Value(Value::F32(b)), Entry::Value(Value::F32(a))] =
                        self.stack.pop_array()?
                    else {
                        bail!("expected f32s")
                    };
                    let result = if a.is_nan() || b.is_nan() {
                        f32::from_bits(0x7FC0_0000)
                    } else if a == b {
                        f32::from_bits(a.to_bits() & b.to_bits())
                    } else {
                        a.max(b)
                    };
                    self.stack.push(result);
                }
                Instruction::F32CopySign => binop!(self.stack, F32, |b, a| b.copysign(a)),
                Instruction::F64Abs => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.abs());
                }
                Instruction::F64Neg => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.neg());
                }
                Instruction::F64Ceil => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.ceil());
                }
                Instruction::F64Floor => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.floor());
                }
                Instruction::F64Trunc => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.trunc());
                }
                Instruction::F64Nearest => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.round_ties_even());
                }
                Instruction::F64Sqrt => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.sqrt());
                }
                Instruction::F64Add => binop!(self.stack, F64, |b, a| b + a),
                Instruction::F64Sub => binop!(self.stack, F64, |b, a| b - a),
                Instruction::F64Mul => binop!(self.stack, F64, |b, a| b * a),
                Instruction::F64Div => binop!(self.stack, F64, |b, a| b / a),
                Instruction::F64Min => {
                    let [Entry::Value(Value::F64(b)), Entry::Value(Value::F64(a))] =
                        self.stack.pop_array()?
                    else {
                        bail!("expected f64s")
                    };
                    let result = if a.is_nan() || b.is_nan() {
                        f64::from_bits(0x7FF8_0000_0000_0000)
                    } else if a == b {
                        f64::from_bits(a.to_bits() | b.to_bits())
                    } else {
                        a.min(b)
                    };
                    self.stack.push(result);
                }
                Instruction::F64Max => {
                    let [Entry::Value(Value::F64(b)), Entry::Value(Value::F64(a))] =
                        self.stack.pop_array()?
                    else {
                        bail!("expected f64s")
                    };
                    let result = if a.is_nan() || b.is_nan() {
                        f64::from_bits(0x7FF8_0000_0000_0000)
                    } else if a == b {
                        f64::from_bits(a.to_bits() & b.to_bits())
                    } else {
                        a.max(b)
                    };
                    self.stack.push(result);
                }
                Instruction::F64CopySign => binop!(self.stack, F64, |b, a| b.copysign(a)),
                Instruction::I32WrapI64 => {
                    let a: i64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a as i32);
                }
                Instruction::I32TruncF32Signed => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let truncated = a.trunc();
                    // i32::MAX as f32 rounds up to 2^31, so use strict <
                    ensure!(
                        truncated >= i32::MIN as f32 && truncated < i32::MAX as f32,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(truncated as i32);
                }
                Instruction::I32TruncF32Unsigned => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let truncated = a.trunc();
                    // u32::MAX as f32 rounds up to 2^32, so use strict <
                    ensure!(
                        truncated >= 0.0 && truncated < u32::MAX as f32,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(truncated as u32 as i32);
                }
                Instruction::I32TruncF64Signed => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let truncated = a.trunc();
                    ensure!(
                        truncated >= i32::MIN as f64 && truncated <= i32::MAX as f64,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(truncated as i32);
                }
                Instruction::I32TruncF64Unsigned => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let truncated = a.trunc();
                    ensure!(
                        truncated >= 0.0 && truncated <= u32::MAX as f64,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(truncated as u32 as i32);
                }
                Instruction::I64ExtendI32Signed => {
                    let a: i32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a as i64);
                }
                Instruction::I64ExtendI32Unsigned => {
                    let a: i32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a as u32 as i64);
                }
                Instruction::I64TruncF32Signed => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let truncated = a.trunc();
                    // i64::MAX as f32 rounds up to 2^63, so use strict <
                    ensure!(
                        truncated >= i64::MIN as f32 && truncated < i64::MAX as f32,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(truncated as i64);
                }
                Instruction::I64TruncF32Unsigned => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let truncated = a.trunc();
                    // u64::MAX as f32 rounds up to 2^64, so use strict <
                    ensure!(
                        truncated >= 0.0 && truncated < u64::MAX as f32,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(truncated as u64 as i64);
                }
                Instruction::I64TruncF64Signed => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let truncated = a.trunc();
                    // i64::MAX as f64 rounds up to 2^63, so use strict <
                    ensure!(
                        truncated >= i64::MIN as f64 && truncated < i64::MAX as f64,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(truncated as i64);
                }
                Instruction::I64TruncF64Unsigned => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    ensure!(!a.is_nan(), "wasm trap: invalid conversion to integer");
                    let truncated = a.trunc();
                    // u64::MAX as f64 rounds up to 2^64, so use strict <
                    ensure!(
                        truncated >= 0.0 && truncated < u64::MAX as f64,
                        "wasm trap: integer overflow"
                    );
                    self.stack.push(truncated as u64 as i64);
                }
                Instruction::F32ConvertI32Signed => {
                    let a: i32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a as f32);
                }
                Instruction::F32ConvertI32Unsigned => {
                    let a: i32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push((a as u32) as f32);
                }
                Instruction::F32ConvertI64Signed => {
                    let a: i64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a as f32);
                }
                Instruction::F32ConvertI64Unsigned => {
                    let a: i64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push((a as u64) as f32);
                }
                Instruction::F32DemoteF64 => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a as f32);
                }
                Instruction::F64ConvertI32Signed => {
                    let a: i32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a as f64);
                }
                Instruction::F64ConvertI32Unsigned => {
                    let a: i32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push((a as u32) as f64);
                }
                Instruction::F64ConvertI64Signed => {
                    let a: i64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a as f64);
                }
                Instruction::F64ConvertI64Unsigned => {
                    let a: i64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push((a as u64) as f64);
                }
                Instruction::F64PromoteF32 => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a as f64);
                }
                Instruction::I32ReinterpretF32 => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.to_bits() as i32);
                }
                Instruction::I64ReinterpretF64 => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(a.to_bits() as i64);
                }
                Instruction::F32ReinterpretI32 => {
                    let a: i32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(f32::from_bits(a as u32));
                }
                Instruction::F64ReinterpretI64 => {
                    let a: i64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push(f64::from_bits(a as u64));
                }
                Instruction::I32Extend8Signed => {
                    let a: i32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push((a as i8) as i32);
                }
                Instruction::I32Extend16Signed => {
                    let a: i32 = self.stack.pop_value()?.try_into()?;
                    self.stack.push((a as i16) as i32);
                }
                Instruction::I64Extend8Signed => {
                    let a: i64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push((a as i8) as i64);
                }
                Instruction::I64Extend16Signed => {
                    let a: i64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push((a as i16) as i64);
                }
                Instruction::I64Extend32Signed => {
                    let a: i64 = self.stack.pop_value()?.try_into()?;
                    self.stack.push((a as i32) as i64);
                }
                Instruction::I32TruncSaturatedF32Signed => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    let result = if a.is_nan() { 0 } else { a as i32 };
                    self.stack.push(result);
                }
                Instruction::I32TruncSaturatedF32Unsigned => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    let result = if a.is_nan() || a < 0.0 {
                        0u32
                    } else {
                        a as u32
                    };
                    self.stack.push(result as i32);
                }
                Instruction::I32TruncSaturatedF64Signed => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    let result = if a.is_nan() {
                        0
                    } else if a < i32::MIN as f64 {
                        i32::MIN
                    } else if a >= i32::MAX as f64 + 1.0 {
                        i32::MAX
                    } else {
                        a as i32
                    };
                    self.stack.push(result);
                }
                Instruction::I32TruncSaturatedF64Unsigned => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    let result = if a.is_nan() || a < 0.0 {
                        0u32
                    } else if a >= u32::MAX as f64 + 1.0 {
                        u32::MAX
                    } else {
                        a as u32
                    };
                    self.stack.push(result as i32);
                }
                Instruction::I64TruncSaturatedF32Signed => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    let result = if a.is_nan() {
                        0i64
                    } else if a < i64::MIN as f32 {
                        i64::MIN
                    } else if a >= i64::MAX as f32 {
                        i64::MAX
                    } else {
                        a as i64
                    };
                    self.stack.push(result);
                }
                Instruction::I64TruncSaturatedF32Unsigned => {
                    let a: f32 = self.stack.pop_value()?.try_into()?;
                    let result = if a.is_nan() || a < 0.0 {
                        0u64
                    } else if a >= u64::MAX as f32 {
                        u64::MAX
                    } else {
                        a as u64
                    };
                    self.stack.push(result as i64);
                }
                Instruction::I64TruncSaturatedF64Signed => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    let result = if a.is_nan() {
                        0i64
                    } else if a < i64::MIN as f64 {
                        i64::MIN
                    } else if a >= i64::MAX as f64 {
                        i64::MAX
                    } else {
                        a as i64
                    };
                    self.stack.push(result);
                }
                Instruction::I64TruncSaturatedF64Unsigned => {
                    let a: f64 = self.stack.pop_value()?.try_into()?;
                    let result = if a.is_nan() || a < 0.0 {
                        0u64
                    } else if a >= u64::MAX as f64 {
                        u64::MAX
                    } else {
                        a as u64
                    };
                    self.stack.push(result as i64);
                }
                Instruction::V128Load(_) => todo!(),
                Instruction::V128Load8x8Signed(_) => todo!(),
                Instruction::V128Load8x8Unsigned(_) => todo!(),
                Instruction::V128Load16x4Unsigned(_) => todo!(),
                Instruction::V128Load16x4Signed(_) => todo!(),
                Instruction::V128Load32x2Signed(_) => todo!(),
                Instruction::V128Load32x2Unsigned(_) => todo!(),
                Instruction::V128Load8Splat(_) => todo!(),
                Instruction::V128Load16Splat(_) => todo!(),
                Instruction::V128Load32Splat(_) => todo!(),
                Instruction::V128Load64Splat(_) => todo!(),
                Instruction::V128Load32Zero(_) => todo!(),
                Instruction::V128Load64Zero(_) => todo!(),
                Instruction::V128Store(_) => todo!(),
                Instruction::V128Load8Lane(_, _) => todo!(),
                Instruction::V128Load16Lane(_, _) => todo!(),
                Instruction::V128Load32Lane(_, _) => todo!(),
                Instruction::V128Load64Lane(_, _) => todo!(),
                Instruction::V128Store8Lane(_, _) => todo!(),
                Instruction::V128Store16Lane(_, _) => todo!(),
                Instruction::V128Store32Lane(_, _) => todo!(),
                Instruction::V128Store64Lane(_, _) => todo!(),
                Instruction::V128Const(_) => todo!(),
                Instruction::I8x16Shuffle(_) => todo!(),
                Instruction::I8x16ExtractLaneSigned(_) => todo!(),
                Instruction::I8x16ExtractLaneUnsigned(_) => todo!(),
                Instruction::I8x16ReplaceLane(_) => todo!(),
                Instruction::I16x8ExtractLaneSigned(_) => todo!(),
                Instruction::I16x8ExtractLaneUnsigned(_) => todo!(),
                Instruction::I16x8ReplaceLane(_) => todo!(),
                Instruction::I32x4ExtractLane(_) => todo!(),
                Instruction::I32x4ReplaceLane(_) => todo!(),
                Instruction::I64x2ExtractLane(_) => todo!(),
                Instruction::I64x2ReplaceLane(_) => todo!(),
                Instruction::F32x4ExtractLane(_) => todo!(),
                Instruction::F32x4ReplaceLane(_) => todo!(),
                Instruction::F64x2ExtractLane(_) => todo!(),
                Instruction::F64x2ReplaceLane(_) => todo!(),
                Instruction::I8x16Swizzle => todo!(),
                Instruction::I8x16Splat => todo!(),
                Instruction::I16x8Splat => todo!(),
                Instruction::I32x4Splat => todo!(),
                Instruction::I64x2Splat => todo!(),
                Instruction::F32x4Splat => todo!(),
                Instruction::F64x2Splat => todo!(),
                Instruction::I8x16Eq => todo!(),
                Instruction::I8x16Ne => todo!(),
                Instruction::I8x16LtSigned => todo!(),
                Instruction::I8x16LtUnsigned => todo!(),
                Instruction::I8x16GtSigned => todo!(),
                Instruction::I8x16GtUnsigned => todo!(),
                Instruction::I8x16LeSigned => todo!(),
                Instruction::I8x16LeUnsigned => todo!(),
                Instruction::I8x16GeSigned => todo!(),
                Instruction::I8x16GeUnsigned => todo!(),
                Instruction::I16x8Eq => todo!(),
                Instruction::I16x8Ne => todo!(),
                Instruction::I16x8LtSigned => todo!(),
                Instruction::I16x8LtUnsigned => todo!(),
                Instruction::I16x8GtSigned => todo!(),
                Instruction::I16x8GtUnsigned => todo!(),
                Instruction::I16x8LeSigned => todo!(),
                Instruction::I16x8LeUnsigned => todo!(),
                Instruction::I16x8GeSigned => todo!(),
                Instruction::I16x8GeUnsigned => todo!(),
                Instruction::I32x4Eq => todo!(),
                Instruction::I32x4Ne => todo!(),
                Instruction::I32x4LtSigned => todo!(),
                Instruction::I32x4LtUnsigned => todo!(),
                Instruction::I32x4GtSigned => todo!(),
                Instruction::I32x4GtUnsigned => todo!(),
                Instruction::I32x4LeSigned => todo!(),
                Instruction::I32x4LeUnsigned => todo!(),
                Instruction::I32x4GeSigned => todo!(),
                Instruction::I32x4GeUnsigned => todo!(),
                Instruction::I64x2Eq => todo!(),
                Instruction::I64x2Ne => todo!(),
                Instruction::I64x2LtSigned => todo!(),
                Instruction::I64x2GtSigned => todo!(),
                Instruction::I64x2LeSigned => todo!(),
                Instruction::I64x2GeSigned => todo!(),
                Instruction::F32X4Eq => todo!(),
                Instruction::F32x4Ne => todo!(),
                Instruction::F32x4Lt => todo!(),
                Instruction::F32x4Gt => todo!(),
                Instruction::F32x4Le => todo!(),
                Instruction::F32x4Ge => todo!(),
                Instruction::F64x2Eq => todo!(),
                Instruction::F64x2Ne => todo!(),
                Instruction::F64x2Lt => todo!(),
                Instruction::F64x2Gt => todo!(),
                Instruction::F64x2Le => todo!(),
                Instruction::F64x2Ge => todo!(),
                Instruction::V128Not => todo!(),
                Instruction::V128And => todo!(),
                Instruction::V128AndNot => todo!(),
                Instruction::V128Or => todo!(),
                Instruction::V128Xor => todo!(),
                Instruction::V128BitSelect => todo!(),
                Instruction::V128AnyTrue => todo!(),
                Instruction::I8x16Abs => todo!(),
                Instruction::I8x16Neg => todo!(),
                Instruction::I8x16PopCount => todo!(),
                Instruction::I8x16AllTrue => todo!(),
                Instruction::I8x16BitMask => todo!(),
                Instruction::I8x16NarrowI16x8Signed => todo!(),
                Instruction::I8x16NarrowI16x8Unsigned => todo!(),
                Instruction::I8x16Shl => todo!(),
                Instruction::I8x16ShrSigned => todo!(),
                Instruction::I8x16ShrUnsigned => todo!(),
                Instruction::I8x16Add => todo!(),
                Instruction::I8x16AddSaturatedSigned => todo!(),
                Instruction::I8x16AddSaturatedUnsigned => todo!(),
                Instruction::I8x16Sub => todo!(),
                Instruction::I8x16SubSaturatedSigned => todo!(),
                Instruction::I8x16SubSaturatedUnsigned => todo!(),
                Instruction::I8x16MinSigned => todo!(),
                Instruction::I8x16MinUnsigned => todo!(),
                Instruction::I8x16MaxSigned => todo!(),
                Instruction::I8x16MaxUnsigned => todo!(),
                Instruction::I8x16AvgRangeUnsigned => todo!(),
                Instruction::I16x8ExtAddPairWiseI8x16Signed => todo!(),
                Instruction::I16x8ExtAddPairWiseI8x16Unsigned => todo!(),
                Instruction::I16x8Abs => todo!(),
                Instruction::I16x8Neg => todo!(),
                Instruction::I16xQ15MulRangeSaturatedSigned => todo!(),
                Instruction::I16x8AllTrue => todo!(),
                Instruction::I16x8BitMask => todo!(),
                Instruction::I16x8NarrowI32x4Signed => todo!(),
                Instruction::I16x8NarrowI32x4Unsigned => todo!(),
                Instruction::I16x8ExtendLowI8x16Unsigned => todo!(),
                Instruction::I16x8ExtendHighI8x16Unsigned => todo!(),
                Instruction::I16x8ExtendLowI8x16Signed => todo!(),
                Instruction::I16x8ExtendHighI8x16Signed => todo!(),
                Instruction::I16x8Shl => todo!(),
                Instruction::I16x8ShrSigned => todo!(),
                Instruction::I16x8ShrUnsigned => todo!(),
                Instruction::I16x8Add => todo!(),
                Instruction::I16x8AddSaturatedSigned => todo!(),
                Instruction::I16x8AddSaturatedUnsigned => todo!(),
                Instruction::I16x8Sub => todo!(),
                Instruction::I16x8SubSaturatedSigned => todo!(),
                Instruction::I16x8SubSaturatedUnsigned => todo!(),
                Instruction::I16x8Mul => todo!(),
                Instruction::I16x8MinSigned => todo!(),
                Instruction::I16x8MinUnsigned => todo!(),
                Instruction::I16x8MaxSigned => todo!(),
                Instruction::I16x8MaxUnsigned => todo!(),
                Instruction::I16x8AvgRangeUnsigned => todo!(),
                Instruction::I16x8ExtMulLowI8x16Signed => todo!(),
                Instruction::I16x8ExtMulHighI8x16Signed => todo!(),
                Instruction::I16x8ExtMulLowI8x16Unsigned => todo!(),
                Instruction::I16x8ExtMulHighI8x16Unsigned => todo!(),
                Instruction::I32x4ExtAddPairWiseI16x8Signed => todo!(),
                Instruction::I32x4ExtAddPairWiseI16x8Unsigned => todo!(),
                Instruction::I32x4Abs => todo!(),
                Instruction::I32x4Neg => todo!(),
                Instruction::I32x4AllTrue => todo!(),
                Instruction::I32x4BitMask => todo!(),
                Instruction::I32x4ExtendLowI16x8Signed => todo!(),
                Instruction::I32x4ExtendHighI16x8Signed => todo!(),
                Instruction::I32x4ExtendLowI16x8Unsigned => todo!(),
                Instruction::I32x4ExtendHighI16x8Unsigned => todo!(),
                Instruction::I32x4Shl => todo!(),
                Instruction::I32x4ShrSigned => todo!(),
                Instruction::I32x4ShrUnsigned => todo!(),
                Instruction::I32x4Add => todo!(),
                Instruction::I32x4Sub => todo!(),
                Instruction::I32x4Mul => todo!(),
                Instruction::I32x4MinSigned => todo!(),
                Instruction::I32x4MinUnsigned => todo!(),
                Instruction::I32x4MaxSigned => todo!(),
                Instruction::I32x4MaxUnsigned => todo!(),
                Instruction::I32x4DotI16x8Signed => todo!(),
                Instruction::I32x4ExtMulLowI16x8Signed => todo!(),
                Instruction::I32x4ExtMulHighI16x8Signed => todo!(),
                Instruction::I32x4ExtMulLowI16x8Unsigned => todo!(),
                Instruction::I32x4ExtMulHighI16x8Unsigned => todo!(),
                Instruction::I64x2Abs => todo!(),
                Instruction::I64x2Neg => todo!(),
                Instruction::I64x2AllTrue => todo!(),
                Instruction::I64x2BitMask => todo!(),
                Instruction::I64x2ExtendLowI32x4Signed => todo!(),
                Instruction::I64x2ExtendHighI32x4Signed => todo!(),
                Instruction::I64x2ExtendLowI32x4Unsigned => todo!(),
                Instruction::I64x2ExtendHighI32x4Unsigned => todo!(),
                Instruction::I64x2Shl => todo!(),
                Instruction::I64x2ShrSigned => todo!(),
                Instruction::I64x2ShrUnsigned => todo!(),
                Instruction::I64x2Add => todo!(),
                Instruction::I64x2Sub => todo!(),
                Instruction::I64x2Mul => todo!(),
                Instruction::I64x2ExtMulLowI32x4Signed => todo!(),
                Instruction::I64x2ExtMulHighI32x4Signed => todo!(),
                Instruction::I64x2ExtMulLowI32x4Unsigned => todo!(),
                Instruction::I64x2ExtMulHighI32x4Unsigned => todo!(),
                Instruction::F32x4Ceil => todo!(),
                Instruction::F32x4Floor => todo!(),
                Instruction::F32x4Trunc => todo!(),
                Instruction::F32x4Nearest => todo!(),
                Instruction::F32x4Abs => todo!(),
                Instruction::F32x4Neg => todo!(),
                Instruction::F32x4Sqrt => todo!(),
                Instruction::F32x4Add => todo!(),
                Instruction::F32x4Sub => todo!(),
                Instruction::F32x4Mul => todo!(),
                Instruction::F32x4Div => todo!(),
                Instruction::F32x4Min => todo!(),
                Instruction::F32x4Max => todo!(),
                Instruction::F32x4PMin => todo!(),
                Instruction::F32x4PMax => todo!(),
                Instruction::F64x2Ceil => todo!(),
                Instruction::F64x2Floor => todo!(),
                Instruction::F64x2Trunc => todo!(),
                Instruction::F64x2Nearest => todo!(),
                Instruction::F64x2Abs => todo!(),
                Instruction::F64x2Neg => todo!(),
                Instruction::F64x2Sqrt => todo!(),
                Instruction::F64x2Add => todo!(),
                Instruction::F64x2Sub => todo!(),
                Instruction::F64x2Mul => todo!(),
                Instruction::F64x2Div => todo!(),
                Instruction::F64x2Min => todo!(),
                Instruction::F64x2Max => todo!(),
                Instruction::F64x2PMin => todo!(),
                Instruction::F64x2PMax => todo!(),
                Instruction::I32x4TruncSaturatedF32x4Signed => todo!(),
                Instruction::I32x4TruncSaturatedF32x4Unsigned => todo!(),
                Instruction::F32x4ConvertI32x4Signed => todo!(),
                Instruction::F32x4ConvertI32x4Unsigned => todo!(),
                Instruction::I32x4TruncSaturatedF64x2SignedZero => todo!(),
                Instruction::I32x4TruncSaturatedF64x2UnsignedZero => todo!(),
                Instruction::F64x2ConvertLowI32x4Signed => todo!(),
                Instruction::F64x2ConvertLowI32x4Unsigned => todo!(),
                Instruction::F32x4DemoteF64x2Zero => todo!(),
                Instruction::F64xPromoteLowF32x4 => todo!(),

                // GC instructions
                Instruction::RefEq => todo!("ref.eq"),
                Instruction::RefAsNonNull => todo!("ref.as_non_null"),
                Instruction::StructNew(_) => todo!("struct.new"),
                Instruction::StructNewDefault(_) => todo!("struct.new_default"),
                Instruction::StructGet(_, _) => todo!("struct.get"),
                Instruction::StructGetSigned(_, _) => todo!("struct.get_s"),
                Instruction::StructGetUnsigned(_, _) => todo!("struct.get_u"),
                Instruction::StructSet(_, _) => todo!("struct.set"),
                Instruction::ArrayNew(_) => todo!("array.new"),
                Instruction::ArrayNewDefault(_) => todo!("array.new_default"),
                Instruction::ArrayNewFixed(_, _) => todo!("array.new_fixed"),
                Instruction::ArrayNewData(_, _) => todo!("array.new_data"),
                Instruction::ArrayNewElem(_, _) => todo!("array.new_elem"),
                Instruction::ArrayGet(_) => todo!("array.get"),
                Instruction::ArrayGetSigned(_) => todo!("array.get_s"),
                Instruction::ArrayGetUnsigned(_) => todo!("array.get_u"),
                Instruction::ArraySet(_) => todo!("array.set"),
                Instruction::ArrayLen => todo!("array.len"),
                Instruction::ArrayFill(_) => todo!("array.fill"),
                Instruction::ArrayCopy(_, _) => todo!("array.copy"),
                Instruction::ArrayInitData(_, _) => todo!("array.init_data"),
                Instruction::ArrayInitElem(_, _) => todo!("array.init_elem"),
                Instruction::RefTest(_) => todo!("ref.test"),
                Instruction::RefTestNull(_) => todo!("ref.test null"),
                Instruction::RefCast(_) => todo!("ref.cast"),
                Instruction::RefCastNull(_) => todo!("ref.cast null"),
                Instruction::BrOnCast(_, _, _, _) => todo!("br_on_cast"),
                Instruction::BrOnCastFail(_, _, _, _) => todo!("br_on_cast_fail"),
                Instruction::AnyConvertExtern => todo!("any.convert_extern"),
                Instruction::ExternConvertAny => todo!("extern.convert_any"),
                Instruction::RefI31 => todo!("ref.i31"),
                Instruction::I31GetSigned => todo!("i31.get_s"),
                Instruction::I31GetUnsigned => todo!("i31.get_u"),

                // Relaxed SIMD
                Instruction::I8x16RelaxedSwizzle => todo!("relaxed simd"),
                Instruction::I32x4RelaxedTruncF32x4Signed => todo!("relaxed simd"),
                Instruction::I32x4RelaxedTruncF32x4Unsigned => todo!("relaxed simd"),
                Instruction::I32x4RelaxedTruncF64x2SignedZero => todo!("relaxed simd"),
                Instruction::I32x4RelaxedTruncF64x2UnsignedZero => todo!("relaxed simd"),
                Instruction::F32x4RelaxedMadd => todo!("relaxed simd"),
                Instruction::F32x4RelaxedNmadd => todo!("relaxed simd"),
                Instruction::F64x2RelaxedMadd => todo!("relaxed simd"),
                Instruction::F64x2RelaxedNmadd => todo!("relaxed simd"),
                Instruction::I8x16RelaxedLaneselect => todo!("relaxed simd"),
                Instruction::I16x8RelaxedLaneselect => todo!("relaxed simd"),
                Instruction::I32x4RelaxedLaneselect => todo!("relaxed simd"),
                Instruction::I64x2RelaxedLaneselect => todo!("relaxed simd"),
                Instruction::F32x4RelaxedMin => todo!("relaxed simd"),
                Instruction::F32x4RelaxedMax => todo!("relaxed simd"),
                Instruction::F64x2RelaxedMin => todo!("relaxed simd"),
                Instruction::F64x2RelaxedMax => todo!("relaxed simd"),
                Instruction::I16x8RelaxedQ15mulrSigned => todo!("relaxed simd"),
                Instruction::I16x8RelaxedDotI8x16I7x16Signed => todo!("relaxed simd"),
                Instruction::I32x4RelaxedDotI8x16I7x16AddSigned => todo!("relaxed simd"),
            }
        }
    }

    fn push_function_call(&mut self, function_addr: usize) -> Result<()> {
        let function_instance = &self.store.functions[function_addr];

        match function_instance {
            FunctionInstance::Local {
                function_type,
                code,
                module,
            } => {
                let num_args = function_type.0 .0.len();
                ensure!(
                    self.stack.len() >= num_args,
                    "At least {num_args} values must be on top of the stack"
                );

                let mut locals = self.stack.pop_n(num_args)?;

                code.locals.iter().for_each(|local| {
                    for _ in 0..local.count {
                        locals.push(Entry::Value(Value::default(&local.value_type)));
                    }
                });

                let locals = locals
                    .into_iter()
                    .map(|entry| match entry {
                        Entry::Value(v) => Ok(v),
                        _ => Err(anyhow!("Expected entry off the stack to be a value.")),
                    })
                    .collect::<Result<Vec<_>>>()?;

                let num_ret_args = function_type.1 .0.len();

                let frame = Frame {
                    arity: num_ret_args,
                    locals,
                    module: *module.clone(),
                };

                self.stack.push(Entry::Activation(frame.clone()));

                let l = Label {
                    arity: num_ret_args as u32,
                };

                self.stack.push(Entry::Label(l));

                self.call_stack.push(CallFrame {
                    instructions: code.body.clone(),
                    pc: 0,
                    control_stack: vec![],
                    frame,
                });
            }
            _ => todo!("What does invoking a Host Function look like"),
        }

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

fn eval_const_expr(expr: &[Instruction], store: &Store) -> Result<Value> {
    let mut stack = Vec::new();

    // i really don't love how we're rewriting a lot of the instructions
    // though the set of valid const expr instructions is fixed by the spec...
    for instr in expr {
        match instr {
            Instruction::I32Const(v) => stack.push(Value::I32(*v)),
            Instruction::I64Const(v) => stack.push(Value::I64(*v)),
            Instruction::F32Const(v) => stack.push(Value::F32(*v)),
            Instruction::F64Const(v) => stack.push(Value::F64(*v)),
            Instruction::V128Const(v) => stack.push(Value::V128(*v)),
            Instruction::RefNull(_) => stack.push(Value::Ref(Ref::Null)),
            Instruction::RefFunc(idx) => {
                stack.push(Value::Ref(Ref::FunctionAddr(*idx as usize)));
            }
            Instruction::GlobalGet(idx) => {
                let global = store
                    .globals
                    .get(*idx as usize)
                    .ok_or_else(|| anyhow!("global index {} oob in const expr", idx))?;
                stack.push(global.value);
            }
            Instruction::RefI31 => {
                let v = pop_i32(&mut stack)?;
                stack.push(Value::Ref(Ref::I31(v & 0x7FFF_FFFF)));
            }
            // 3.0
            Instruction::I32Add => {
                let (b, a) = (pop_i32(&mut stack)?, pop_i32(&mut stack)?);
                stack.push(Value::I32(a.wrapping_add(b)));
            }
            Instruction::I32Sub => {
                let (b, a) = (pop_i32(&mut stack)?, pop_i32(&mut stack)?);
                stack.push(Value::I32(a.wrapping_sub(b)));
            }
            Instruction::I32Mul => {
                let (b, a) = (pop_i32(&mut stack)?, pop_i32(&mut stack)?);
                stack.push(Value::I32(a.wrapping_mul(b)));
            }
            Instruction::I64Add => {
                let (b, a) = (pop_i64(&mut stack)?, pop_i64(&mut stack)?);
                stack.push(Value::I64(a.wrapping_add(b)));
            }
            Instruction::I64Sub => {
                let (b, a) = (pop_i64(&mut stack)?, pop_i64(&mut stack)?);
                stack.push(Value::I64(a.wrapping_sub(b)));
            }
            Instruction::I64Mul => {
                let (b, a) = (pop_i64(&mut stack)?, pop_i64(&mut stack)?);
                stack.push(Value::I64(a.wrapping_mul(b)));
            }
            foreign => bail!("foreign instruction in const expr: {:?}", foreign),
        }
    }

    stack
        .pop()
        .ok_or_else(|| anyhow!("const expr produced no value"))
}

fn pop_i32(stack: &mut Vec<Value>) -> Result<i32> {
    match stack
        .pop()
        .ok_or_else(|| anyhow!("stack underflow in const expr"))?
    {
        Value::I32(v) => Ok(v),
        other => bail!("expected i32 in const expr, got {:?}", other),
    }
}

fn pop_i64(stack: &mut Vec<Value>) -> Result<i64> {
    match stack
        .pop()
        .ok_or_else(|| anyhow!("stack underflow in const expr"))?
    {
        Value::I64(v) => Ok(v),
        other => bail!("expected i64 in const expr, got {:?}", other),
    }
}

fn resolve_block_type(bt: &BlockType, frame: &Frame) -> (usize, usize) {
    match bt {
        BlockType::Empty => (0, 0),
        BlockType::SingleValue(_) => (0, 1),
        BlockType::TypeIndex(idx) => {
            let st = &frame.module.types[*idx as usize];
            match &st.composite_type {
                crate::binary_grammar::CompositeType::Func(ft) => (ft.0 .0.len(), ft.1 .0.len()),
                _ => (0, 0),
            }
        }
    }
}
