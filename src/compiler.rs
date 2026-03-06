use crate::binary_grammar::{
    BlockType, CompositeType, Function, Instruction, Module, SubType, ValueType,
};
use crate::ir::{CompiledFunction, CompiledModule, JumpTableEntry, Op};

const UNREACHABLE_DEPTH: i32 = i32::MIN;

#[derive(Debug, Clone, Copy, PartialEq)]
enum BlockKind {
    Function,
    Block,
    Loop,
    IfElse,
}

#[derive(Debug)]
struct BlockContext {
    kind: BlockKind,
    entry_stack_height: i32,
    branch_arity: usize,
    start_pc: usize,
    pending_patches: Vec<usize>,
}

struct Compiler<'a> {
    types: &'a [SubType],
    func_signatures: Vec<(usize, usize)>,
    ops: Vec<Op>,
    block_stack: Vec<BlockContext>,
    stack_height: i32,
    max_stack_height: i32,
    v128_constants: Vec<i128>,
    jump_tables: Vec<Vec<JumpTableEntry>>,
    shuffle_masks: Vec<[u8; 16]>,
}

pub fn compile(module: &Module) -> CompiledModule {
    let mut v128_constants = Vec::new();
    let mut jump_tables = Vec::new();
    let mut shuffle_masks = Vec::new();

    let resolve_sig = |type_idx: u32, types: &[SubType]| -> (usize, usize) {
        match &types[type_idx as usize].composite_type {
            CompositeType::Func(ft) => (ft.0 .0.len(), ft.1 .0.len()),
            _ => (0, 0),
        }
    };
    let mut func_signatures: Vec<(usize, usize)> = module
        .import_declarations
        .iter()
        .filter_map(|imp| match &imp.description {
            crate::binary_grammar::ImportDescription::Func(type_idx) => {
                Some(resolve_sig(*type_idx, &module.types))
            }
            _ => None,
        })
        .collect();
    for f in &module.functions {
        func_signatures.push(resolve_sig(f.type_index, &module.types));
    }

    let functions = module
        .functions
        .iter()
        .map(|f| {
            let mut compiler = Compiler {
                types: &module.types,
                func_signatures: func_signatures.clone(),
                ops: Vec::new(),
                block_stack: Vec::new(),
                stack_height: 0,
                v128_constants: std::mem::take(&mut v128_constants),
                jump_tables: std::mem::take(&mut jump_tables),
                shuffle_masks: std::mem::take(&mut shuffle_masks),
                max_stack_height: 0,
            };
            let cf = compiler.compile_function(f);

            v128_constants = compiler.v128_constants;
            jump_tables = compiler.jump_tables;
            shuffle_masks = compiler.shuffle_masks;
            cf
        })
        .collect();

    CompiledModule {
        functions,
        types: module.types.clone(),
        function_addrs: Vec::new(),
        table_addrs: Vec::new(),
        mem_addrs: Vec::new(),
        global_addrs: Vec::new(),
        tag_addrs: Vec::new(),
        elem_addrs: Vec::new(),
        data_addrs: Vec::new(),
        exports: Vec::new(),
        v128_constants,
        jump_tables,
        shuffle_masks,
    }
}

pub fn compile_function_into(
    types: &[SubType],
    func: &Function,
    module: &mut CompiledModule,
) -> CompiledFunction {
    let mut compiler = Compiler {
        types,
        func_signatures: Vec::new(),
        ops: Vec::new(),
        block_stack: Vec::new(),
        stack_height: 0,
        v128_constants: std::mem::take(&mut module.v128_constants),
        jump_tables: std::mem::take(&mut module.jump_tables),
        shuffle_masks: std::mem::take(&mut module.shuffle_masks),
        max_stack_height: 0,
    };
    let cf = compiler.compile_function(func);
    module.v128_constants = compiler.v128_constants;
    module.jump_tables = compiler.jump_tables;
    module.shuffle_masks = compiler.shuffle_masks;
    cf
}

impl<'a> Compiler<'a> {
    fn resolve_type_sig(&self, type_idx: u32) -> (usize, usize) {
        match &self.types[type_idx as usize].composite_type {
            CompositeType::Func(ft) => (ft.0 .0.len(), ft.1 .0.len()),
            _ => (0, 0),
        }
    }

    fn resolve_block_type(&self, bt: &BlockType) -> (usize, usize) {
        match bt {
            BlockType::Empty => (0, 0),
            BlockType::SingleValue(_) => (0, 1),
            BlockType::TypeIndex(idx) => {
                let st = &self.types[*idx as usize];
                match &st.composite_type {
                    CompositeType::Func(ft) => (ft.0 .0.len(), ft.1 .0.len()),
                    _ => (0, 0),
                }
            }
        }
    }

    fn compile_function(&mut self, func: &Function) -> CompiledFunction {
        let st = &self.types[func.type_index as usize];

        let (num_args, num_results) = if let CompositeType::Func(ft) = &st.composite_type {
            (ft.0 .0.len(), ft.1 .0.len())
        } else {
            (0, 0)
        };

        self.stack_height = num_args as i32;
        self.block_stack.push(BlockContext {
            kind: BlockKind::Function,
            entry_stack_height: num_args as i32,
            branch_arity: num_results,
            start_pc: 0,
            pending_patches: Vec::new(),
        });

        for instr in &func.body {
            self.compile_instruction(instr);
        }

        let ctx = self.block_stack.pop().unwrap();
        let return_pc = self.ops.len() as u32;
        self.patch_pending(&ctx.pending_patches, return_pc);

        self.emit(Op::Return);

        let extra_locals: usize = func.locals.iter().map(|l| l.count as usize).sum();
        let mut local_types: Vec<ValueType> = match &st.composite_type {
            CompositeType::Func(ft) => {
                let mut v = Vec::with_capacity(ft.0 .0.len() + extra_locals);
                v.extend_from_slice(&ft.0 .0);
                v
            }
            _ => Vec::with_capacity(extra_locals),
        };
        for local in &func.locals {
            for _ in 0..local.count {
                local_types.push(local.value_type.clone());
            }
        }

        CompiledFunction {
            ops: std::mem::take(&mut self.ops),
            type_index: func.type_index,
            num_args: num_args as u32,
            local_types,
            max_stack_height: self.max_stack_height as u32,
        }
    }

    fn patch(&mut self, idx: usize, target: u32) {
        match &mut self.ops[idx] {
            Op::Jump {
                target: t,
                keep: _,
                drop: _,
            } => *t = target,
            Op::JumpIf {
                target: t,
                keep: _,
                drop: _,
            } => *t = target,
            Op::JumpIfNot {
                target: t,
                keep: _,
                drop: _,
            } => *t = target,
            Op::BrOnNull {
                target: t,
                keep: _,
                drop: _,
            } => *t = target,
            Op::BrOnNonNull {
                target: t,
                keep: _,
                drop: _,
            } => *t = target,
            _ => panic!("cannot patch op at index {idx}"),
        }
    }

    /*
    the high bit is a tag to distinguish 2 types of pending patches

        if high bit = 0: index into self.ops
        if high bit = 1: the value encodes a jump table entry
            the remaining bits are split (table_idx, entry_idx)
    */
    fn patch_pending(&mut self, pending_patches: &[usize], end_pc: u32) {
        for &patch in pending_patches {
            if patch & (1usize << 63) != 0 {
                let table_idx = (patch >> 32) & 0x7FFF_FFFF;
                let entry_idx = patch & 0xFFFF_FFFF;
                self.jump_tables[table_idx][entry_idx].target = end_pc;
            } else {
                self.patch(patch, end_pc);
            }
        }
    }

    fn emit(&mut self, op: Op) -> usize {
        let idx = self.ops.len();
        self.ops.push(op);
        idx
    }

    fn emit_branch(&mut self, depth: u32, conditional: bool, negate: bool) {
        let idx = self.block_stack.len() - 1 - depth as usize;
        let ctx = &self.block_stack[idx];

        let keep = ctx.branch_arity as u16;
        let drop = (self.stack_height - ctx.entry_stack_height - keep as i32) as u16;

        let is_loop = ctx.kind == BlockKind::Loop;
        let target = if is_loop {
            ctx.start_pc as u32
        } else {
            u32::MAX
        };

        let op_idx = match (conditional, negate) {
            (true, true) => self.emit(Op::JumpIfNot { target, keep, drop }),
            (true, false) => self.emit(Op::JumpIf { target, keep, drop }),
            (false, _) => self.emit(Op::Jump { target, keep, drop }),
        };

        if !is_loop {
            self.block_stack[idx].pending_patches.push(op_idx);
        }
    }

    fn compile_instruction(&mut self, instr: &Instruction) {
        match instr {
            Instruction::Block(..) | Instruction::Loop(..) | Instruction::IfElse(..) => {}
            _ => {
                if self.stack_height == UNREACHABLE_DEPTH {
                    return;
                }
            }
        }

        match instr {
            Instruction::Block(bt, body) => {
                let (m, n) = self.resolve_block_type(bt);

                let entry = ((self.stack_height != UNREACHABLE_DEPTH) as i32)
                    .wrapping_mul(self.stack_height.wrapping_sub(m as i32));

                self.block_stack.push(BlockContext {
                    kind: BlockKind::Block,
                    entry_stack_height: entry,
                    branch_arity: n,
                    start_pc: self.ops.len(),
                    pending_patches: Vec::new(),
                });

                for i in body {
                    self.compile_instruction(i);
                }

                let ctx = self.block_stack.pop().expect("we push right before");
                let end_pc = self.ops.len() as u32;

                self.patch_pending(&ctx.pending_patches, end_pc);
                self.stack_height = ctx.entry_stack_height + n as i32;
            }
            Instruction::Loop(bt, body) => {
                let (m, n) = self.resolve_block_type(bt);

                let entry = ((self.stack_height != UNREACHABLE_DEPTH) as i32)
                    .wrapping_mul(self.stack_height.wrapping_sub(m as i32));

                let loop_start = self.ops.len();

                self.block_stack.push(BlockContext {
                    kind: BlockKind::Loop,
                    entry_stack_height: entry,
                    branch_arity: m,
                    start_pc: loop_start,
                    pending_patches: Vec::new(),
                });

                for i in body {
                    self.compile_instruction(i);
                }

                let ctx = self.block_stack.pop().unwrap();
                let end_pc = self.ops.len() as u32;

                self.patch_pending(&ctx.pending_patches, end_pc);
                self.stack_height = ctx.entry_stack_height + n as i32;
            }
            Instruction::IfElse(bt, then_body, else_body) => {
                if self.stack_height != UNREACHABLE_DEPTH {
                    self.stack_height -= 1;
                }

                let (m, n) = self.resolve_block_type(bt);
                let entry = ((self.stack_height != UNREACHABLE_DEPTH) as i32)
                    .wrapping_mul(self.stack_height.wrapping_sub(m as i32));

                let jump_to_else = self.emit(Op::JumpIfNot {
                    target: u32::MAX,
                    keep: 0,
                    drop: 0,
                });

                self.block_stack.push(BlockContext {
                    kind: BlockKind::IfElse,
                    entry_stack_height: entry,
                    branch_arity: n,
                    start_pc: self.ops.len(),
                    pending_patches: Vec::new(),
                });

                let saved_height = self.stack_height;

                for i in then_body {
                    self.compile_instruction(i);
                }

                if else_body.is_empty() {
                    let end_pc = self.ops.len() as u32;
                    self.patch(jump_to_else, end_pc);
                } else {
                    let jump_to_end = self.emit(Op::Jump {
                        target: u32::MAX,
                        keep: 0,
                        drop: 0,
                    });

                    let else_pc = self.ops.len() as u32;
                    self.patch(jump_to_else, else_pc);

                    self.stack_height = saved_height;
                    for i in else_body {
                        self.compile_instruction(i);
                    }

                    let end_pc = self.ops.len() as u32;
                    self.patch(jump_to_end, end_pc);
                }

                let ctx = self.block_stack.pop().unwrap();
                let end_pc = self.ops.len() as u32;
                self.patch_pending(&ctx.pending_patches, end_pc);
                self.stack_height = ctx.entry_stack_height + n as i32;
            }
            Instruction::Br(depth) => {
                self.emit_branch(*depth, false, false);
                self.stack_height = UNREACHABLE_DEPTH;
            }
            Instruction::BrIf(depth) => {
                self.stack_height -= 1;
                self.emit_branch(*depth, true, false);
            }
            Instruction::BrTable(labels, default) => {
                self.stack_height -= 1;
                let default_idx = self.block_stack.len() - 1 - *default as usize;
                let keep = self.block_stack[default_idx].branch_arity as u16;

                let mut entries = Vec::with_capacity(labels.len() + 1);
                for label in labels.iter().chain(std::iter::once(default)) {
                    let idx = self.block_stack.len() - 1 - *label as usize;
                    let ctx = &self.block_stack[idx];
                    let drop = (self.stack_height - ctx.entry_stack_height - keep as i32) as u16;
                    let is_loop = ctx.kind == BlockKind::Loop;
                    let target = if is_loop {
                        ctx.start_pc as u32
                    } else {
                        u32::MAX
                    };
                    entries.push(JumpTableEntry { target, drop });

                    if !is_loop {
                        self.block_stack[idx].pending_patches.push(usize::MAX);
                    }
                }

                for label in labels.iter().chain(std::iter::once(default)) {
                    let idx = self.block_stack.len() - 1 - *label as usize;
                    let ctx = &mut self.block_stack[idx];
                    if ctx.kind != BlockKind::Loop {
                        if let Some(&last) = ctx.pending_patches.last() {
                            if last == usize::MAX {
                                ctx.pending_patches.pop();
                            }
                        }
                    }
                }

                let table_idx = self.jump_tables.len();
                self.jump_tables.push(entries);

                for (entry_i, label) in labels.iter().chain(std::iter::once(default)).enumerate() {
                    let idx = self.block_stack.len() - 1 - *label as usize;
                    let ctx = &self.block_stack[idx];
                    if ctx.kind != BlockKind::Loop {
                        let encoded = (1usize << 63) | (table_idx << 32) | entry_i;
                        self.block_stack[idx].pending_patches.push(encoded);
                    }
                }

                self.emit(Op::JumpTable {
                    index: table_idx as u32,
                    keep,
                });
                self.stack_height = UNREACHABLE_DEPTH;
            }
            Instruction::Return => {
                self.emit(Op::Return);
                self.stack_height = UNREACHABLE_DEPTH;
            }
            Instruction::Unreachable => {
                self.emit(Op::Unreachable);
                self.stack_height = UNREACHABLE_DEPTH;
            }
            Instruction::Nop => {
                self.emit(Op::Nop);
            }
            Instruction::Call(func_idx) => {
                let (n_params, n_results) = self.func_signatures[*func_idx as usize];
                self.emit(Op::Call {
                    func_idx: *func_idx,
                });
                self.stack_height -= n_params as i32;
                self.stack_height += n_results as i32;
            }
            Instruction::CallIndirect(type_idx, table_idx) => {
                let (n_params, n_results) = self.resolve_type_sig(*type_idx);
                self.stack_height -= 1;
                self.emit(Op::CallIndirect {
                    type_idx: *type_idx,
                    table_idx: *table_idx,
                });
                self.stack_height -= n_params as i32;
                self.stack_height += n_results as i32;
            }
            Instruction::ReturnCall(func_idx) => {
                self.emit(Op::ReturnCall {
                    func_idx: *func_idx,
                });
                self.stack_height = UNREACHABLE_DEPTH;
            }
            Instruction::ReturnCallIndirect(type_idx, table_idx) => {
                self.emit(Op::ReturnCallIndirect {
                    type_idx: *type_idx,
                    table_idx: *table_idx,
                });
                self.stack_height = UNREACHABLE_DEPTH;
            }
            Instruction::CallRef(type_idx) => {
                let (n_params, n_results) = self.resolve_type_sig(*type_idx);
                self.stack_height -= 1;
                self.emit(Op::CallRef {
                    type_idx: *type_idx,
                });
                self.stack_height -= n_params as i32;
                self.stack_height += n_results as i32;
            }
            Instruction::ReturnCallRef(type_idx) => {
                self.emit(Op::ReturnCallRef {
                    type_idx: *type_idx,
                });
                self.stack_height = UNREACHABLE_DEPTH;
            }
            Instruction::I32Const(v) => {
                self.emit(Op::I32Const { value: *v });
                self.stack_height += 1;
            }
            Instruction::I64Const(v) => {
                self.emit(Op::I64Const { value: *v });
                self.stack_height += 1;
            }
            Instruction::F32Const(v) => {
                self.emit(Op::F32Const { value: *v });
                self.stack_height += 1;
            }
            Instruction::F64Const(v) => {
                self.emit(Op::F64Const { value: *v });
                self.stack_height += 1;
            }
            Instruction::V128Const(v) => {
                let table_idx = self.v128_constants.len() as u32;
                self.v128_constants.push(*v);
                self.emit(Op::V128Const { table_idx });
                self.stack_height += 1;
            }
            Instruction::LocalGet(idx) => {
                self.emit(Op::LocalGet { local_idx: *idx });
                self.stack_height += 1;
            }
            Instruction::LocalSet(idx) => {
                self.emit(Op::LocalSet { local_idx: *idx });
                self.stack_height -= 1;
            }
            Instruction::LocalTee(idx) => {
                self.emit(Op::LocalTee { local_idx: *idx });
            }
            Instruction::GlobalGet(idx) => {
                self.emit(Op::GlobalGet { global_idx: *idx });
                self.stack_height += 1;
            }
            Instruction::GlobalSet(idx) => {
                self.emit(Op::GlobalSet { global_idx: *idx });
                self.stack_height -= 1;
            }
            Instruction::Drop => {
                self.emit(Op::Drop);
                self.stack_height -= 1;
            }
            Instruction::Select(_) => {
                self.emit(Op::Select);
                self.stack_height -= 2;
            }
            Instruction::RefNull(ht) => {
                self.emit(Op::RefNull(*ht));
                self.stack_height += 1;
            }
            Instruction::RefIsNull => {
                self.emit(Op::RefIsNull);
            }
            Instruction::RefEq => {
                self.emit(Op::RefEq);
                self.stack_height -= 1;
            }
            Instruction::RefAsNonNull => {
                self.emit(Op::RefAsNonNull);
            }
            Instruction::RefFunc(idx) => {
                self.emit(Op::RefFunc { func_idx: *idx });
                self.stack_height += 1;
            }
            Instruction::Throw(tag_idx) => {
                self.emit(Op::Throw { tag_idx: *tag_idx });
                self.stack_height = UNREACHABLE_DEPTH;
            }
            Instruction::ThrowRef => {
                self.emit(Op::ThrowRef);
                self.stack_height = UNREACHABLE_DEPTH;
            }
            Instruction::BrOnNull(depth) => {
                let idx = self.block_stack.len() - 1 - *depth as usize;
                let ctx = &self.block_stack[idx];
                let keep = ctx.branch_arity as u16;
                let drop = (self.stack_height - ctx.entry_stack_height - keep as i32 - 1) as u16;
                let is_loop = ctx.kind == BlockKind::Loop;
                let target = if is_loop {
                    ctx.start_pc as u32
                } else {
                    u32::MAX
                };
                let op_idx = self.emit(Op::BrOnNull { target, keep, drop });
                if !is_loop {
                    self.block_stack[idx].pending_patches.push(op_idx);
                }
            }
            Instruction::BrOnNonNull(depth) => {
                let idx = self.block_stack.len() - 1 - *depth as usize;
                let ctx = &self.block_stack[idx];
                let keep = ctx.branch_arity as u16;
                let drop = (self.stack_height - ctx.entry_stack_height - keep as i32) as u16;
                let is_loop = ctx.kind == BlockKind::Loop;
                let target = if is_loop {
                    ctx.start_pc as u32
                } else {
                    u32::MAX
                };
                let op_idx = self.emit(Op::BrOnNonNull { target, keep, drop });
                if !is_loop {
                    self.block_stack[idx].pending_patches.push(op_idx);
                }
                self.stack_height -= 1;
            }
            Instruction::TableGet(idx) => {
                self.emit(Op::TableGet { table_idx: *idx });
            }
            Instruction::TableSet(idx) => {
                self.emit(Op::TableSet { table_idx: *idx });
                self.stack_height -= 2;
            }
            Instruction::TableInit(elem_idx, table_idx) => {
                self.emit(Op::TableInit {
                    elem_idx: *elem_idx,
                    table_idx: *table_idx,
                });
                self.stack_height -= 3;
            }
            Instruction::ElemDrop(idx) => {
                self.emit(Op::ElemDrop { elem_idx: *idx });
            }
            Instruction::TableCopy(dst, src) => {
                self.emit(Op::TableCopy {
                    dst_table_idx: *dst,
                    src_table_idx: *src,
                });
                self.stack_height -= 3;
            }
            Instruction::TableGrow(idx) => {
                self.emit(Op::TableGrow { table_idx: *idx });
                self.stack_height -= 1;
            }
            Instruction::TableSize(idx) => {
                self.emit(Op::TableSize { table_idx: *idx });
                self.stack_height += 1;
            }
            Instruction::TableFill(idx) => {
                self.emit(Op::TableFill { table_idx: *idx });
                self.stack_height -= 3;
            }
            Instruction::I32Load(ma) => {
                self.emit(Op::I32Load {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::I64Load(ma) => {
                self.emit(Op::I64Load {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::F32Load(ma) => {
                self.emit(Op::F32Load {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::F64Load(ma) => {
                self.emit(Op::F64Load {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::I32Load8Signed(ma) => {
                self.emit(Op::I32Load8Signed {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::I32Load8Unsigned(ma) => {
                self.emit(Op::I32Load8Unsigned {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::I32Load16Signed(ma) => {
                self.emit(Op::I32Load16Signed {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::I32Load16Unsigned(ma) => {
                self.emit(Op::I32Load16Unsigned {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::I64Load8Signed(ma) => {
                self.emit(Op::I64Load8Signed {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::I64Load8Unsigned(ma) => {
                self.emit(Op::I64Load8Unsigned {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::I64Load16Signed(ma) => {
                self.emit(Op::I64Load16Signed {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::I64Load16Unsigned(ma) => {
                self.emit(Op::I64Load16Unsigned {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::I64Load32Signed(ma) => {
                self.emit(Op::I64Load32Signed {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::I64Load32Unsigned(ma) => {
                self.emit(Op::I64Load32Unsigned {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::I32Store(ma) => {
                self.emit(Op::I32Store {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
                self.stack_height -= 2;
            }
            Instruction::I64Store(ma) => {
                self.emit(Op::I64Store {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
                self.stack_height -= 2;
            }
            Instruction::F32Store(ma) => {
                self.emit(Op::F32Store {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
                self.stack_height -= 2;
            }
            Instruction::F64Store(ma) => {
                self.emit(Op::F64Store {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
                self.stack_height -= 2;
            }
            Instruction::I32Store8(ma) => {
                self.emit(Op::I32Store8 {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
                self.stack_height -= 2;
            }
            Instruction::I32Store16(ma) => {
                self.emit(Op::I32Store16 {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
                self.stack_height -= 2;
            }
            Instruction::I64Store8(ma) => {
                self.emit(Op::I64Store8 {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
                self.stack_height -= 2;
            }
            Instruction::I64Store16(ma) => {
                self.emit(Op::I64Store16 {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
                self.stack_height -= 2;
            }
            Instruction::I64Store32(ma) => {
                self.emit(Op::I64Store32 {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
                self.stack_height -= 2;
            }
            Instruction::MemorySize(idx) => {
                self.emit(Op::MemorySize { memory_idx: *idx });
                self.stack_height += 1;
            }
            Instruction::MemoryGrow(idx) => {
                self.emit(Op::MemoryGrow { memory_idx: *idx });
            }
            Instruction::MemoryInit(data_idx, mem_idx) => {
                self.emit(Op::MemoryInit {
                    data_idx: *data_idx,
                    memory_idx: *mem_idx,
                });
                self.stack_height -= 3;
            }
            Instruction::DataDrop(idx) => {
                self.emit(Op::DataDrop { data_idx: *idx });
            }
            Instruction::MemoryCopy(dst, src) => {
                self.emit(Op::MemoryCopy {
                    dst_memory_idx: *dst,
                    src_memory_idx: *src,
                });
                self.stack_height -= 3;
            }
            Instruction::MemoryFill(idx) => {
                self.emit(Op::MemoryFill { memory_idx: *idx });
                self.stack_height -= 3;
            }
            Instruction::I32EqZero => {
                self.emit(Op::I32EqZero);
            }
            Instruction::I32Eq => {
                self.emit(Op::I32Eq);
                self.stack_height -= 1;
            }
            Instruction::I32Ne => {
                self.emit(Op::I32Ne);
                self.stack_height -= 1;
            }
            Instruction::I32LtSigned => {
                self.emit(Op::I32LtSigned);
                self.stack_height -= 1;
            }
            Instruction::I32LtUnsigned => {
                self.emit(Op::I32LtUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I32GtSigned => {
                self.emit(Op::I32GtSigned);
                self.stack_height -= 1;
            }
            Instruction::I32GtUnsigned => {
                self.emit(Op::I32GtUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I32LeSigned => {
                self.emit(Op::I32LeSigned);
                self.stack_height -= 1;
            }
            Instruction::I32LeUnsigned => {
                self.emit(Op::I32LeUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I32GeSigned => {
                self.emit(Op::I32GeSigned);
                self.stack_height -= 1;
            }
            Instruction::I32GeUnsigned => {
                self.emit(Op::I32GeUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I64EqZero => {
                self.emit(Op::I64EqZero);
            }
            Instruction::I64Eq => {
                self.emit(Op::I64Eq);
                self.stack_height -= 1;
            }
            Instruction::I64Ne => {
                self.emit(Op::I64Ne);
                self.stack_height -= 1;
            }
            Instruction::I64LtSigned => {
                self.emit(Op::I64LtSigned);
                self.stack_height -= 1;
            }
            Instruction::I64LtUnsigned => {
                self.emit(Op::I64LtUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I64GtSigned => {
                self.emit(Op::I64GtSigned);
                self.stack_height -= 1;
            }
            Instruction::I64GtUnsigned => {
                self.emit(Op::I64GtUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I64LeSigned => {
                self.emit(Op::I64LeSigned);
                self.stack_height -= 1;
            }
            Instruction::I64LeUnsigned => {
                self.emit(Op::I64LeUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I64GeSigned => {
                self.emit(Op::I64GeSigned);
                self.stack_height -= 1;
            }
            Instruction::I64GeUnsigned => {
                self.emit(Op::I64GeUnsigned);
                self.stack_height -= 1;
            }
            Instruction::F32Eq => {
                self.emit(Op::F32Eq);
                self.stack_height -= 1;
            }
            Instruction::F32Ne => {
                self.emit(Op::F32Ne);
                self.stack_height -= 1;
            }
            Instruction::F32Lt => {
                self.emit(Op::F32Lt);
                self.stack_height -= 1;
            }
            Instruction::F32Gt => {
                self.emit(Op::F32Gt);
                self.stack_height -= 1;
            }
            Instruction::F32Le => {
                self.emit(Op::F32Le);
                self.stack_height -= 1;
            }
            Instruction::F32Ge => {
                self.emit(Op::F32Ge);
                self.stack_height -= 1;
            }
            Instruction::F64Eq => {
                self.emit(Op::F64Eq);
                self.stack_height -= 1;
            }
            Instruction::F64Ne => {
                self.emit(Op::F64Ne);
                self.stack_height -= 1;
            }
            Instruction::F64Lt => {
                self.emit(Op::F64Lt);
                self.stack_height -= 1;
            }
            Instruction::F64Gt => {
                self.emit(Op::F64Gt);
                self.stack_height -= 1;
            }
            Instruction::F64Le => {
                self.emit(Op::F64Le);
                self.stack_height -= 1;
            }
            Instruction::F64Ge => {
                self.emit(Op::F64Ge);
                self.stack_height -= 1;
            }
            Instruction::I32CountLeadingZeros => {
                self.emit(Op::I32CountLeadingZeros);
            }
            Instruction::I32CountTrailingZeros => {
                self.emit(Op::I32CountTrailingZeros);
            }
            Instruction::I32PopCount => {
                self.emit(Op::I32PopCount);
            }
            Instruction::I32Add => {
                self.emit(Op::I32Add);
                self.stack_height -= 1;
            }
            Instruction::I32Sub => {
                self.emit(Op::I32Sub);
                self.stack_height -= 1;
            }
            Instruction::I32Mul => {
                self.emit(Op::I32Mul);
                self.stack_height -= 1;
            }
            Instruction::I32DivSigned => {
                self.emit(Op::I32DivSigned);
                self.stack_height -= 1;
            }
            Instruction::I32DivUnsigned => {
                self.emit(Op::I32DivUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I32RemainderSigned => {
                self.emit(Op::I32RemainderSigned);
                self.stack_height -= 1;
            }
            Instruction::I32RemainderUnsigned => {
                self.emit(Op::I32RemainderUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I32And => {
                self.emit(Op::I32And);
                self.stack_height -= 1;
            }
            Instruction::I32Or => {
                self.emit(Op::I32Or);
                self.stack_height -= 1;
            }
            Instruction::I32Xor => {
                self.emit(Op::I32Xor);
                self.stack_height -= 1;
            }
            Instruction::I32Shl => {
                self.emit(Op::I32Shl);
                self.stack_height -= 1;
            }
            Instruction::I32ShrSigned => {
                self.emit(Op::I32ShrSigned);
                self.stack_height -= 1;
            }
            Instruction::I32ShrUnsigned => {
                self.emit(Op::I32ShrUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I32RotateLeft => {
                self.emit(Op::I32RotateLeft);
                self.stack_height -= 1;
            }
            Instruction::I32RotateRight => {
                self.emit(Op::I32RotateRight);
                self.stack_height -= 1;
            }
            Instruction::I64CountLeadingZeros => {
                self.emit(Op::I64CountLeadingZeros);
            }
            Instruction::I64CountTrailingZeros => {
                self.emit(Op::I64CountTrailingZeros);
            }
            Instruction::I64PopCount => {
                self.emit(Op::I64PopCount);
            }
            Instruction::I64Add => {
                self.emit(Op::I64Add);
                self.stack_height -= 1;
            }
            Instruction::I64Sub => {
                self.emit(Op::I64Sub);
                self.stack_height -= 1;
            }
            Instruction::I64Mul => {
                self.emit(Op::I64Mul);
                self.stack_height -= 1;
            }
            Instruction::I64DivSigned => {
                self.emit(Op::I64DivSigned);
                self.stack_height -= 1;
            }
            Instruction::I64DivUnsigned => {
                self.emit(Op::I64DivUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I64RemainderSigned => {
                self.emit(Op::I64RemainderSigned);
                self.stack_height -= 1;
            }
            Instruction::I64RemainderUnsigned => {
                self.emit(Op::I64RemainderUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I64And => {
                self.emit(Op::I64And);
                self.stack_height -= 1;
            }
            Instruction::I64Or => {
                self.emit(Op::I64Or);
                self.stack_height -= 1;
            }
            Instruction::I64Xor => {
                self.emit(Op::I64Xor);
                self.stack_height -= 1;
            }
            Instruction::I64Shl => {
                self.emit(Op::I64Shl);
                self.stack_height -= 1;
            }
            Instruction::I64ShrSigned => {
                self.emit(Op::I64ShrSigned);
                self.stack_height -= 1;
            }
            Instruction::I64ShrUnsigned => {
                self.emit(Op::I64ShrUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I64RotateLeft => {
                self.emit(Op::I64RotateLeft);
                self.stack_height -= 1;
            }
            Instruction::I64RotateRight => {
                self.emit(Op::I64RotateRight);
                self.stack_height -= 1;
            }
            Instruction::F32Abs => {
                self.emit(Op::F32Abs);
            }
            Instruction::F32Neg => {
                self.emit(Op::F32Neg);
            }
            Instruction::F32Ceil => {
                self.emit(Op::F32Ceil);
            }
            Instruction::F32Floor => {
                self.emit(Op::F32Floor);
            }
            Instruction::F32Trunc => {
                self.emit(Op::F32Trunc);
            }
            Instruction::F32Nearest => {
                self.emit(Op::F32Nearest);
            }
            Instruction::F32Sqrt => {
                self.emit(Op::F32Sqrt);
            }
            Instruction::F32Add => {
                self.emit(Op::F32Add);
                self.stack_height -= 1;
            }
            Instruction::F32Sub => {
                self.emit(Op::F32Sub);
                self.stack_height -= 1;
            }
            Instruction::F32Mul => {
                self.emit(Op::F32Mul);
                self.stack_height -= 1;
            }
            Instruction::F32Div => {
                self.emit(Op::F32Div);
                self.stack_height -= 1;
            }
            Instruction::F32Min => {
                self.emit(Op::F32Min);
                self.stack_height -= 1;
            }
            Instruction::F32Max => {
                self.emit(Op::F32Max);
                self.stack_height -= 1;
            }
            Instruction::F32CopySign => {
                self.emit(Op::F32CopySign);
                self.stack_height -= 1;
            }
            Instruction::F64Abs => {
                self.emit(Op::F64Abs);
            }
            Instruction::F64Neg => {
                self.emit(Op::F64Neg);
            }
            Instruction::F64Ceil => {
                self.emit(Op::F64Ceil);
            }
            Instruction::F64Floor => {
                self.emit(Op::F64Floor);
            }
            Instruction::F64Trunc => {
                self.emit(Op::F64Trunc);
            }
            Instruction::F64Nearest => {
                self.emit(Op::F64Nearest);
            }
            Instruction::F64Sqrt => {
                self.emit(Op::F64Sqrt);
            }
            Instruction::F64Add => {
                self.emit(Op::F64Add);
                self.stack_height -= 1;
            }
            Instruction::F64Sub => {
                self.emit(Op::F64Sub);
                self.stack_height -= 1;
            }
            Instruction::F64Mul => {
                self.emit(Op::F64Mul);
                self.stack_height -= 1;
            }
            Instruction::F64Div => {
                self.emit(Op::F64Div);
                self.stack_height -= 1;
            }
            Instruction::F64Min => {
                self.emit(Op::F64Min);
                self.stack_height -= 1;
            }
            Instruction::F64Max => {
                self.emit(Op::F64Max);
                self.stack_height -= 1;
            }
            Instruction::F64CopySign => {
                self.emit(Op::F64CopySign);
                self.stack_height -= 1;
            }
            Instruction::I32WrapI64 => {
                self.emit(Op::I32WrapI64);
            }
            Instruction::I32TruncF32Signed => {
                self.emit(Op::I32TruncF32Signed);
            }
            Instruction::I32TruncF32Unsigned => {
                self.emit(Op::I32TruncF32Unsigned);
            }
            Instruction::I32TruncF64Signed => {
                self.emit(Op::I32TruncF64Signed);
            }
            Instruction::I32TruncF64Unsigned => {
                self.emit(Op::I32TruncF64Unsigned);
            }
            Instruction::I64ExtendI32Signed => {
                self.emit(Op::I64ExtendI32Signed);
            }
            Instruction::I64ExtendI32Unsigned => {
                self.emit(Op::I64ExtendI32Unsigned);
            }
            Instruction::I64TruncF32Signed => {
                self.emit(Op::I64TruncF32Signed);
            }
            Instruction::I64TruncF32Unsigned => {
                self.emit(Op::I64TruncF32Unsigned);
            }
            Instruction::I64TruncF64Signed => {
                self.emit(Op::I64TruncF64Signed);
            }
            Instruction::I64TruncF64Unsigned => {
                self.emit(Op::I64TruncF64Unsigned);
            }
            Instruction::F32ConvertI32Signed => {
                self.emit(Op::F32ConvertI32Signed);
            }
            Instruction::F32ConvertI32Unsigned => {
                self.emit(Op::F32ConvertI32Unsigned);
            }
            Instruction::F32ConvertI64Signed => {
                self.emit(Op::F32ConvertI64Signed);
            }
            Instruction::F32ConvertI64Unsigned => {
                self.emit(Op::F32ConvertI64Unsigned);
            }
            Instruction::F32DemoteF64 => {
                self.emit(Op::F32DemoteF64);
            }
            Instruction::F64ConvertI32Signed => {
                self.emit(Op::F64ConvertI32Signed);
            }
            Instruction::F64ConvertI32Unsigned => {
                self.emit(Op::F64ConvertI32Unsigned);
            }
            Instruction::F64ConvertI64Signed => {
                self.emit(Op::F64ConvertI64Signed);
            }
            Instruction::F64ConvertI64Unsigned => {
                self.emit(Op::F64ConvertI64Unsigned);
            }
            Instruction::F64PromoteF32 => {
                self.emit(Op::F64PromoteF32);
            }
            Instruction::I32ReinterpretF32 => {
                self.emit(Op::I32ReinterpretF32);
            }
            Instruction::I64ReinterpretF64 => {
                self.emit(Op::I64ReinterpretF64);
            }
            Instruction::F32ReinterpretI32 => {
                self.emit(Op::F32ReinterpretI32);
            }
            Instruction::F64ReinterpretI64 => {
                self.emit(Op::F64ReinterpretI64);
            }
            Instruction::I32Extend8Signed => {
                self.emit(Op::I32Extend8Signed);
            }
            Instruction::I32Extend16Signed => {
                self.emit(Op::I32Extend16Signed);
            }
            Instruction::I64Extend8Signed => {
                self.emit(Op::I64Extend8Signed);
            }
            Instruction::I64Extend16Signed => {
                self.emit(Op::I64Extend16Signed);
            }
            Instruction::I64Extend32Signed => {
                self.emit(Op::I64Extend32Signed);
            }
            Instruction::I32TruncSaturatedF32Signed => {
                self.emit(Op::I32TruncSaturatedF32Signed);
            }
            Instruction::I32TruncSaturatedF32Unsigned => {
                self.emit(Op::I32TruncSaturatedF32Unsigned);
            }
            Instruction::I32TruncSaturatedF64Signed => {
                self.emit(Op::I32TruncSaturatedF64Signed);
            }
            Instruction::I32TruncSaturatedF64Unsigned => {
                self.emit(Op::I32TruncSaturatedF64Unsigned);
            }
            Instruction::I64TruncSaturatedF32Signed => {
                self.emit(Op::I64TruncSaturatedF32Signed);
            }
            Instruction::I64TruncSaturatedF32Unsigned => {
                self.emit(Op::I64TruncSaturatedF32Unsigned);
            }
            Instruction::I64TruncSaturatedF64Signed => {
                self.emit(Op::I64TruncSaturatedF64Signed);
            }
            Instruction::I64TruncSaturatedF64Unsigned => {
                self.emit(Op::I64TruncSaturatedF64Unsigned);
            }
            Instruction::V128Load(ma) => {
                self.emit(Op::V128Load {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::V128Load8x8Signed(ma) => {
                self.emit(Op::V128Load8x8Signed {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::V128Load8x8Unsigned(ma) => {
                self.emit(Op::V128Load8x8Unsigned {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::V128Load16x4Signed(ma) => {
                self.emit(Op::V128Load16x4Signed {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::V128Load16x4Unsigned(ma) => {
                self.emit(Op::V128Load16x4Unsigned {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::V128Load32x2Signed(ma) => {
                self.emit(Op::V128Load32x2Signed {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::V128Load32x2Unsigned(ma) => {
                self.emit(Op::V128Load32x2Unsigned {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::V128Load8Splat(ma) => {
                self.emit(Op::V128Load8Splat {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::V128Load16Splat(ma) => {
                self.emit(Op::V128Load16Splat {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::V128Load32Splat(ma) => {
                self.emit(Op::V128Load32Splat {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::V128Load64Splat(ma) => {
                self.emit(Op::V128Load64Splat {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::V128Load32Zero(ma) => {
                self.emit(Op::V128Load32Zero {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::V128Load64Zero(ma) => {
                self.emit(Op::V128Load64Zero {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
            }
            Instruction::V128Store(ma) => {
                self.emit(Op::V128Store {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                });
                self.stack_height -= 2;
            }
            Instruction::V128Load8Lane(ma, lane) => {
                self.emit(Op::V128Load8Lane {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                    lane: *lane,
                });
                self.stack_height -= 1;
            }
            Instruction::V128Load16Lane(ma, lane) => {
                self.emit(Op::V128Load16Lane {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                    lane: *lane,
                });
                self.stack_height -= 1;
            }
            Instruction::V128Load32Lane(ma, lane) => {
                self.emit(Op::V128Load32Lane {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                    lane: *lane,
                });
                self.stack_height -= 1;
            }
            Instruction::V128Load64Lane(ma, lane) => {
                self.emit(Op::V128Load64Lane {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                    lane: *lane,
                });
                self.stack_height -= 1;
            }
            Instruction::V128Store8Lane(ma, lane) => {
                self.emit(Op::V128Store8Lane {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                    lane: *lane,
                });
                self.stack_height -= 2;
            }
            Instruction::V128Store16Lane(ma, lane) => {
                self.emit(Op::V128Store16Lane {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                    lane: *lane,
                });
                self.stack_height -= 2;
            }
            Instruction::V128Store32Lane(ma, lane) => {
                self.emit(Op::V128Store32Lane {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                    lane: *lane,
                });
                self.stack_height -= 2;
            }
            Instruction::V128Store64Lane(ma, lane) => {
                self.emit(Op::V128Store64Lane {
                    offset: ma.offset as u32,
                    memory: ma.memory,
                    lane: *lane,
                });
                self.stack_height -= 2;
            }
            Instruction::I8x16Shuffle(mask) => {
                let table_idx = self.shuffle_masks.len() as u32;
                self.shuffle_masks.push(*mask);
                self.emit(Op::I8x16Shuffle { table_idx });
                self.stack_height -= 1;
            }
            Instruction::I8x16ExtractLaneSigned(l) => {
                self.emit(Op::I8x16ExtractLaneSigned(*l));
            }
            Instruction::I8x16ExtractLaneUnsigned(l) => {
                self.emit(Op::I8x16ExtractLaneUnsigned(*l));
            }
            Instruction::I16x8ExtractLaneSigned(l) => {
                self.emit(Op::I16x8ExtractLaneSigned(*l));
            }
            Instruction::I16x8ExtractLaneUnsigned(l) => {
                self.emit(Op::I16x8ExtractLaneUnsigned(*l));
            }
            Instruction::I32x4ExtractLane(l) => {
                self.emit(Op::I32x4ExtractLane(*l));
            }
            Instruction::I64x2ExtractLane(l) => {
                self.emit(Op::I64x2ExtractLane(*l));
            }
            Instruction::F32x4ExtractLane(l) => {
                self.emit(Op::F32x4ExtractLane(*l));
            }
            Instruction::F64x2ExtractLane(l) => {
                self.emit(Op::F64x2ExtractLane(*l));
            }
            Instruction::I8x16ReplaceLane(l) => {
                self.emit(Op::I8x16ReplaceLane(*l));
                self.stack_height -= 1;
            }
            Instruction::I16x8ReplaceLane(l) => {
                self.emit(Op::I16x8ReplaceLane(*l));
                self.stack_height -= 1;
            }
            Instruction::I32x4ReplaceLane(l) => {
                self.emit(Op::I32x4ReplaceLane(*l));
                self.stack_height -= 1;
            }
            Instruction::I64x2ReplaceLane(l) => {
                self.emit(Op::I64x2ReplaceLane(*l));
                self.stack_height -= 1;
            }
            Instruction::F32x4ReplaceLane(l) => {
                self.emit(Op::F32x4ReplaceLane(*l));
                self.stack_height -= 1;
            }
            Instruction::F64x2ReplaceLane(l) => {
                self.emit(Op::F64x2ReplaceLane(*l));
                self.stack_height -= 1;
            }
            Instruction::I8x16Swizzle => {
                self.emit(Op::I8x16Swizzle);
                self.stack_height -= 1;
            }
            Instruction::I8x16Splat => {
                self.emit(Op::I8x16Splat);
            }
            Instruction::I16x8Splat => {
                self.emit(Op::I16x8Splat);
            }
            Instruction::I32x4Splat => {
                self.emit(Op::I32x4Splat);
            }
            Instruction::I64x2Splat => {
                self.emit(Op::I64x2Splat);
            }
            Instruction::F32x4Splat => {
                self.emit(Op::F32x4Splat);
            }
            Instruction::F64x2Splat => {
                self.emit(Op::F64x2Splat);
            }
            Instruction::I8x16Eq => {
                self.emit(Op::I8x16Eq);
                self.stack_height -= 1;
            }
            Instruction::I8x16Ne => {
                self.emit(Op::I8x16Ne);
                self.stack_height -= 1;
            }
            Instruction::I8x16LtSigned => {
                self.emit(Op::I8x16LtSigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16LtUnsigned => {
                self.emit(Op::I8x16LtUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16GtSigned => {
                self.emit(Op::I8x16GtSigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16GtUnsigned => {
                self.emit(Op::I8x16GtUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16LeSigned => {
                self.emit(Op::I8x16LeSigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16LeUnsigned => {
                self.emit(Op::I8x16LeUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16GeSigned => {
                self.emit(Op::I8x16GeSigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16GeUnsigned => {
                self.emit(Op::I8x16GeUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8Eq => {
                self.emit(Op::I16x8Eq);
                self.stack_height -= 1;
            }
            Instruction::I16x8Ne => {
                self.emit(Op::I16x8Ne);
                self.stack_height -= 1;
            }
            Instruction::I16x8LtSigned => {
                self.emit(Op::I16x8LtSigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8LtUnsigned => {
                self.emit(Op::I16x8LtUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8GtSigned => {
                self.emit(Op::I16x8GtSigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8GtUnsigned => {
                self.emit(Op::I16x8GtUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8LeSigned => {
                self.emit(Op::I16x8LeSigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8LeUnsigned => {
                self.emit(Op::I16x8LeUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8GeSigned => {
                self.emit(Op::I16x8GeSigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8GeUnsigned => {
                self.emit(Op::I16x8GeUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4Eq => {
                self.emit(Op::I32x4Eq);
                self.stack_height -= 1;
            }
            Instruction::I32x4Ne => {
                self.emit(Op::I32x4Ne);
                self.stack_height -= 1;
            }
            Instruction::I32x4LtSigned => {
                self.emit(Op::I32x4LtSigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4LtUnsigned => {
                self.emit(Op::I32x4LtUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4GtSigned => {
                self.emit(Op::I32x4GtSigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4GtUnsigned => {
                self.emit(Op::I32x4GtUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4LeSigned => {
                self.emit(Op::I32x4LeSigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4LeUnsigned => {
                self.emit(Op::I32x4LeUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4GeSigned => {
                self.emit(Op::I32x4GeSigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4GeUnsigned => {
                self.emit(Op::I32x4GeUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I64x2Eq => {
                self.emit(Op::I64x2Eq);
                self.stack_height -= 1;
            }
            Instruction::I64x2Ne => {
                self.emit(Op::I64x2Ne);
                self.stack_height -= 1;
            }
            Instruction::I64x2LtSigned => {
                self.emit(Op::I64x2LtSigned);
                self.stack_height -= 1;
            }
            Instruction::I64x2GtSigned => {
                self.emit(Op::I64x2GtSigned);
                self.stack_height -= 1;
            }
            Instruction::I64x2LeSigned => {
                self.emit(Op::I64x2LeSigned);
                self.stack_height -= 1;
            }
            Instruction::I64x2GeSigned => {
                self.emit(Op::I64x2GeSigned);
                self.stack_height -= 1;
            }
            Instruction::F32X4Eq => {
                self.emit(Op::F32x4Eq);
                self.stack_height -= 1;
            }
            Instruction::F32x4Ne => {
                self.emit(Op::F32x4Ne);
                self.stack_height -= 1;
            }
            Instruction::F32x4Lt => {
                self.emit(Op::F32x4Lt);
                self.stack_height -= 1;
            }
            Instruction::F32x4Gt => {
                self.emit(Op::F32x4Gt);
                self.stack_height -= 1;
            }
            Instruction::F32x4Le => {
                self.emit(Op::F32x4Le);
                self.stack_height -= 1;
            }
            Instruction::F32x4Ge => {
                self.emit(Op::F32x4Ge);
                self.stack_height -= 1;
            }
            Instruction::F64x2Eq => {
                self.emit(Op::F64x2Eq);
                self.stack_height -= 1;
            }
            Instruction::F64x2Ne => {
                self.emit(Op::F64x2Ne);
                self.stack_height -= 1;
            }
            Instruction::F64x2Lt => {
                self.emit(Op::F64x2Lt);
                self.stack_height -= 1;
            }
            Instruction::F64x2Gt => {
                self.emit(Op::F64x2Gt);
                self.stack_height -= 1;
            }
            Instruction::F64x2Le => {
                self.emit(Op::F64x2Le);
                self.stack_height -= 1;
            }
            Instruction::F64x2Ge => {
                self.emit(Op::F64x2Ge);
                self.stack_height -= 1;
            }
            Instruction::V128Not => {
                self.emit(Op::V128Not);
            }
            Instruction::V128And => {
                self.emit(Op::V128And);
                self.stack_height -= 1;
            }
            Instruction::V128AndNot => {
                self.emit(Op::V128AndNot);
                self.stack_height -= 1;
            }
            Instruction::V128Or => {
                self.emit(Op::V128Or);
                self.stack_height -= 1;
            }
            Instruction::V128Xor => {
                self.emit(Op::V128Xor);
                self.stack_height -= 1;
            }
            Instruction::V128BitSelect => {
                self.emit(Op::V128BitSelect);
                self.stack_height -= 2;
            }
            Instruction::V128AnyTrue => {
                self.emit(Op::V128AnyTrue);
            }
            Instruction::I8x16Abs => {
                self.emit(Op::I8x16Abs);
            }
            Instruction::I8x16Neg => {
                self.emit(Op::I8x16Neg);
            }
            Instruction::I8x16PopCount => {
                self.emit(Op::I8x16PopCount);
            }
            Instruction::I8x16AllTrue => {
                self.emit(Op::I8x16AllTrue);
            }
            Instruction::I8x16BitMask => {
                self.emit(Op::I8x16BitMask);
            }
            Instruction::I8x16NarrowI16x8Signed => {
                self.emit(Op::I8x16NarrowI16x8Signed);
                self.stack_height -= 1;
            }
            Instruction::I8x16NarrowI16x8Unsigned => {
                self.emit(Op::I8x16NarrowI16x8Unsigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16Shl => {
                self.emit(Op::I8x16Shl);
                self.stack_height -= 1;
            }
            Instruction::I8x16ShrSigned => {
                self.emit(Op::I8x16ShrSigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16ShrUnsigned => {
                self.emit(Op::I8x16ShrUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16Add => {
                self.emit(Op::I8x16Add);
                self.stack_height -= 1;
            }
            Instruction::I8x16AddSaturatedSigned => {
                self.emit(Op::I8x16AddSaturatedSigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16AddSaturatedUnsigned => {
                self.emit(Op::I8x16AddSaturatedUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16Sub => {
                self.emit(Op::I8x16Sub);
                self.stack_height -= 1;
            }
            Instruction::I8x16SubSaturatedSigned => {
                self.emit(Op::I8x16SubSaturatedSigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16SubSaturatedUnsigned => {
                self.emit(Op::I8x16SubSaturatedUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16MinSigned => {
                self.emit(Op::I8x16MinSigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16MinUnsigned => {
                self.emit(Op::I8x16MinUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16MaxSigned => {
                self.emit(Op::I8x16MaxSigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16MaxUnsigned => {
                self.emit(Op::I8x16MaxUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I8x16AvgRangeUnsigned => {
                self.emit(Op::I8x16AvgRangeUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8ExtAddPairWiseI8x16Signed => {
                self.emit(Op::I16x8ExtAddPairWiseI8x16Signed);
            }
            Instruction::I16x8ExtAddPairWiseI8x16Unsigned => {
                self.emit(Op::I16x8ExtAddPairWiseI8x16Unsigned);
            }
            Instruction::I16x8Abs => {
                self.emit(Op::I16x8Abs);
            }
            Instruction::I16x8Neg => {
                self.emit(Op::I16x8Neg);
            }
            Instruction::I16xQ15MulRangeSaturatedSigned => {
                self.emit(Op::I16xQ15MulRangeSaturatedSigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8AllTrue => {
                self.emit(Op::I16x8AllTrue);
            }
            Instruction::I16x8BitMask => {
                self.emit(Op::I16x8BitMask);
            }
            Instruction::I16x8NarrowI32x4Signed => {
                self.emit(Op::I16x8NarrowI32x4Signed);
                self.stack_height -= 1;
            }
            Instruction::I16x8NarrowI32x4Unsigned => {
                self.emit(Op::I16x8NarrowI32x4Unsigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8ExtendLowI8x16Unsigned => {
                self.emit(Op::I16x8ExtendLowI8x16Unsigned);
            }
            Instruction::I16x8ExtendHighI8x16Unsigned => {
                self.emit(Op::I16x8ExtendHighI8x16Unsigned);
            }
            Instruction::I16x8ExtendLowI8x16Signed => {
                self.emit(Op::I16x8ExtendLowI8x16Signed);
            }
            Instruction::I16x8ExtendHighI8x16Signed => {
                self.emit(Op::I16x8ExtendHighI8x16Signed);
            }
            Instruction::I16x8Shl => {
                self.emit(Op::I16x8Shl);
                self.stack_height -= 1;
            }
            Instruction::I16x8ShrSigned => {
                self.emit(Op::I16x8ShrSigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8ShrUnsigned => {
                self.emit(Op::I16x8ShrUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8Add => {
                self.emit(Op::I16x8Add);
                self.stack_height -= 1;
            }
            Instruction::I16x8AddSaturatedSigned => {
                self.emit(Op::I16x8AddSaturatedSigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8AddSaturatedUnsigned => {
                self.emit(Op::I16x8AddSaturatedUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8Sub => {
                self.emit(Op::I16x8Sub);
                self.stack_height -= 1;
            }
            Instruction::I16x8SubSaturatedSigned => {
                self.emit(Op::I16x8SubSaturatedSigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8SubSaturatedUnsigned => {
                self.emit(Op::I16x8SubSaturatedUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8Mul => {
                self.emit(Op::I16x8Mul);
                self.stack_height -= 1;
            }
            Instruction::I16x8MinSigned => {
                self.emit(Op::I16x8MinSigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8MinUnsigned => {
                self.emit(Op::I16x8MinUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8MaxSigned => {
                self.emit(Op::I16x8MaxSigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8MaxUnsigned => {
                self.emit(Op::I16x8MaxUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8AvgRangeUnsigned => {
                self.emit(Op::I16x8AvgRangeUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8ExtMulLowI8x16Signed => {
                self.emit(Op::I16x8ExtMulLowI8x16Signed);
                self.stack_height -= 1;
            }
            Instruction::I16x8ExtMulHighI8x16Signed => {
                self.emit(Op::I16x8ExtMulHighI8x16Signed);
                self.stack_height -= 1;
            }
            Instruction::I16x8ExtMulLowI8x16Unsigned => {
                self.emit(Op::I16x8ExtMulLowI8x16Unsigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8ExtMulHighI8x16Unsigned => {
                self.emit(Op::I16x8ExtMulHighI8x16Unsigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4ExtAddPairWiseI16x8Signed => {
                self.emit(Op::I32x4ExtAddPairWiseI16x8Signed);
            }
            Instruction::I32x4ExtAddPairWiseI16x8Unsigned => {
                self.emit(Op::I32x4ExtAddPairWiseI16x8Unsigned);
            }
            Instruction::I32x4Abs => {
                self.emit(Op::I32x4Abs);
            }
            Instruction::I32x4Neg => {
                self.emit(Op::I32x4Neg);
            }
            Instruction::I32x4AllTrue => {
                self.emit(Op::I32x4AllTrue);
            }
            Instruction::I32x4BitMask => {
                self.emit(Op::I32x4BitMask);
            }
            Instruction::I32x4ExtendLowI16x8Signed => {
                self.emit(Op::I32x4ExtendLowI16x8Signed);
            }
            Instruction::I32x4ExtendHighI16x8Signed => {
                self.emit(Op::I32x4ExtendHighI16x8Signed);
            }
            Instruction::I32x4ExtendLowI16x8Unsigned => {
                self.emit(Op::I32x4ExtendLowI16x8Unsigned);
            }
            Instruction::I32x4ExtendHighI16x8Unsigned => {
                self.emit(Op::I32x4ExtendHighI16x8Unsigned);
            }
            Instruction::I32x4Shl => {
                self.emit(Op::I32x4Shl);
                self.stack_height -= 1;
            }
            Instruction::I32x4ShrSigned => {
                self.emit(Op::I32x4ShrSigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4ShrUnsigned => {
                self.emit(Op::I32x4ShrUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4Add => {
                self.emit(Op::I32x4Add);
                self.stack_height -= 1;
            }
            Instruction::I32x4Sub => {
                self.emit(Op::I32x4Sub);
                self.stack_height -= 1;
            }
            Instruction::I32x4Mul => {
                self.emit(Op::I32x4Mul);
                self.stack_height -= 1;
            }
            Instruction::I32x4MinSigned => {
                self.emit(Op::I32x4MinSigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4MinUnsigned => {
                self.emit(Op::I32x4MinUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4MaxSigned => {
                self.emit(Op::I32x4MaxSigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4MaxUnsigned => {
                self.emit(Op::I32x4MaxUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4DotI16x8Signed => {
                self.emit(Op::I32x4DotI16x8Signed);
                self.stack_height -= 1;
            }
            Instruction::I32x4ExtMulLowI16x8Signed => {
                self.emit(Op::I32x4ExtMulLowI16x8Signed);
                self.stack_height -= 1;
            }
            Instruction::I32x4ExtMulHighI16x8Signed => {
                self.emit(Op::I32x4ExtMulHighI16x8Signed);
                self.stack_height -= 1;
            }
            Instruction::I32x4ExtMulLowI16x8Unsigned => {
                self.emit(Op::I32x4ExtMulLowI16x8Unsigned);
                self.stack_height -= 1;
            }
            Instruction::I32x4ExtMulHighI16x8Unsigned => {
                self.emit(Op::I32x4ExtMulHighI16x8Unsigned);
                self.stack_height -= 1;
            }
            Instruction::I64x2Abs => {
                self.emit(Op::I64x2Abs);
            }
            Instruction::I64x2Neg => {
                self.emit(Op::I64x2Neg);
            }
            Instruction::I64x2AllTrue => {
                self.emit(Op::I64x2AllTrue);
            }
            Instruction::I64x2BitMask => {
                self.emit(Op::I64x2BitMask);
            }
            Instruction::I64x2ExtendLowI32x4Signed => {
                self.emit(Op::I64x2ExtendLowI32x4Signed);
            }
            Instruction::I64x2ExtendHighI32x4Signed => {
                self.emit(Op::I64x2ExtendHighI32x4Signed);
            }
            Instruction::I64x2ExtendLowI32x4Unsigned => {
                self.emit(Op::I64x2ExtendLowI32x4Unsigned);
            }
            Instruction::I64x2ExtendHighI32x4Unsigned => {
                self.emit(Op::I64x2ExtendHighI32x4Unsigned);
            }
            Instruction::I64x2Shl => {
                self.emit(Op::I64x2Shl);
                self.stack_height -= 1;
            }
            Instruction::I64x2ShrSigned => {
                self.emit(Op::I64x2ShrSigned);
                self.stack_height -= 1;
            }
            Instruction::I64x2ShrUnsigned => {
                self.emit(Op::I64x2ShrUnsigned);
                self.stack_height -= 1;
            }
            Instruction::I64x2Add => {
                self.emit(Op::I64x2Add);
                self.stack_height -= 1;
            }
            Instruction::I64x2Sub => {
                self.emit(Op::I64x2Sub);
                self.stack_height -= 1;
            }
            Instruction::I64x2Mul => {
                self.emit(Op::I64x2Mul);
                self.stack_height -= 1;
            }
            Instruction::I64x2ExtMulLowI32x4Signed => {
                self.emit(Op::I64x2ExtMulLowI32x4Signed);
                self.stack_height -= 1;
            }
            Instruction::I64x2ExtMulHighI32x4Signed => {
                self.emit(Op::I64x2ExtMulHighI32x4Signed);
                self.stack_height -= 1;
            }
            Instruction::I64x2ExtMulLowI32x4Unsigned => {
                self.emit(Op::I64x2ExtMulLowI32x4Unsigned);
                self.stack_height -= 1;
            }
            Instruction::I64x2ExtMulHighI32x4Unsigned => {
                self.emit(Op::I64x2ExtMulHighI32x4Unsigned);
                self.stack_height -= 1;
            }
            Instruction::F32x4Ceil => {
                self.emit(Op::F32x4Ceil);
            }
            Instruction::F32x4Floor => {
                self.emit(Op::F32x4Floor);
            }
            Instruction::F32x4Trunc => {
                self.emit(Op::F32x4Trunc);
            }
            Instruction::F32x4Nearest => {
                self.emit(Op::F32x4Nearest);
            }
            Instruction::F32x4Abs => {
                self.emit(Op::F32x4Abs);
            }
            Instruction::F32x4Neg => {
                self.emit(Op::F32x4Neg);
            }
            Instruction::F32x4Sqrt => {
                self.emit(Op::F32x4Sqrt);
            }
            Instruction::F32x4Add => {
                self.emit(Op::F32x4Add);
                self.stack_height -= 1;
            }
            Instruction::F32x4Sub => {
                self.emit(Op::F32x4Sub);
                self.stack_height -= 1;
            }
            Instruction::F32x4Mul => {
                self.emit(Op::F32x4Mul);
                self.stack_height -= 1;
            }
            Instruction::F32x4Div => {
                self.emit(Op::F32x4Div);
                self.stack_height -= 1;
            }
            Instruction::F32x4Min => {
                self.emit(Op::F32x4Min);
                self.stack_height -= 1;
            }
            Instruction::F32x4Max => {
                self.emit(Op::F32x4Max);
                self.stack_height -= 1;
            }
            Instruction::F32x4PMin => {
                self.emit(Op::F32x4PMin);
                self.stack_height -= 1;
            }
            Instruction::F32x4PMax => {
                self.emit(Op::F32x4PMax);
                self.stack_height -= 1;
            }
            Instruction::F64x2Ceil => {
                self.emit(Op::F64x2Ceil);
            }
            Instruction::F64x2Floor => {
                self.emit(Op::F64x2Floor);
            }
            Instruction::F64x2Trunc => {
                self.emit(Op::F64x2Trunc);
            }
            Instruction::F64x2Nearest => {
                self.emit(Op::F64x2Nearest);
            }
            Instruction::F64x2Abs => {
                self.emit(Op::F64x2Abs);
            }
            Instruction::F64x2Neg => {
                self.emit(Op::F64x2Neg);
            }
            Instruction::F64x2Sqrt => {
                self.emit(Op::F64x2Sqrt);
            }
            Instruction::F64x2Add => {
                self.emit(Op::F64x2Add);
                self.stack_height -= 1;
            }
            Instruction::F64x2Sub => {
                self.emit(Op::F64x2Sub);
                self.stack_height -= 1;
            }
            Instruction::F64x2Mul => {
                self.emit(Op::F64x2Mul);
                self.stack_height -= 1;
            }
            Instruction::F64x2Div => {
                self.emit(Op::F64x2Div);
                self.stack_height -= 1;
            }
            Instruction::F64x2Min => {
                self.emit(Op::F64x2Min);
                self.stack_height -= 1;
            }
            Instruction::F64x2Max => {
                self.emit(Op::F64x2Max);
                self.stack_height -= 1;
            }
            Instruction::F64x2PMin => {
                self.emit(Op::F64x2PMin);
                self.stack_height -= 1;
            }
            Instruction::F64x2PMax => {
                self.emit(Op::F64x2PMax);
                self.stack_height -= 1;
            }
            Instruction::I32x4TruncSaturatedF32x4Signed => {
                self.emit(Op::I32x4TruncSaturatedF32x4Signed);
            }
            Instruction::I32x4TruncSaturatedF32x4Unsigned => {
                self.emit(Op::I32x4TruncSaturatedF32x4Unsigned);
            }
            Instruction::F32x4ConvertI32x4Signed => {
                self.emit(Op::F32x4ConvertI32x4Signed);
            }
            Instruction::F32x4ConvertI32x4Unsigned => {
                self.emit(Op::F32x4ConvertI32x4Unsigned);
            }
            Instruction::I32x4TruncSaturatedF64x2SignedZero => {
                self.emit(Op::I32x4TruncSaturatedF64x2SignedZero);
            }
            Instruction::I32x4TruncSaturatedF64x2UnsignedZero => {
                self.emit(Op::I32x4TruncSaturatedF64x2UnsignedZero);
            }
            Instruction::F64x2ConvertLowI32x4Signed => {
                self.emit(Op::F64x2ConvertLowI32x4Signed);
            }
            Instruction::F64x2ConvertLowI32x4Unsigned => {
                self.emit(Op::F64x2ConvertLowI32x4Unsigned);
            }
            Instruction::F32x4DemoteF64x2Zero => {
                self.emit(Op::F32x4DemoteF64x2Zero);
            }
            Instruction::F64xPromoteLowF32x4 => {
                self.emit(Op::F64x2PromoteLowF32x4);
            }
            Instruction::I8x16RelaxedSwizzle => {
                self.emit(Op::I8x16RelaxedSwizzle);
                self.stack_height -= 1;
            }
            Instruction::I32x4RelaxedTruncF32x4Signed => {
                self.emit(Op::I32x4RelaxedTruncF32x4Signed);
            }
            Instruction::I32x4RelaxedTruncF32x4Unsigned => {
                self.emit(Op::I32x4RelaxedTruncF32x4Unsigned);
            }
            Instruction::I32x4RelaxedTruncF64x2SignedZero => {
                self.emit(Op::I32x4RelaxedTruncF64x2SignedZero);
            }
            Instruction::I32x4RelaxedTruncF64x2UnsignedZero => {
                self.emit(Op::I32x4RelaxedTruncF64x2UnsignedZero);
            }
            Instruction::F32x4RelaxedMadd => {
                self.emit(Op::F32x4RelaxedMadd);
                self.stack_height -= 2;
            }
            Instruction::F32x4RelaxedNmadd => {
                self.emit(Op::F32x4RelaxedNmadd);
                self.stack_height -= 2;
            }
            Instruction::F64x2RelaxedMadd => {
                self.emit(Op::F64x2RelaxedMadd);
                self.stack_height -= 2;
            }
            Instruction::F64x2RelaxedNmadd => {
                self.emit(Op::F64x2RelaxedNmadd);
                self.stack_height -= 2;
            }
            Instruction::I8x16RelaxedLaneselect => {
                self.emit(Op::I8x16RelaxedLaneselect);
                self.stack_height -= 2;
            }
            Instruction::I16x8RelaxedLaneselect => {
                self.emit(Op::I16x8RelaxedLaneselect);
                self.stack_height -= 2;
            }
            Instruction::I32x4RelaxedLaneselect => {
                self.emit(Op::I32x4RelaxedLaneselect);
                self.stack_height -= 2;
            }
            Instruction::I64x2RelaxedLaneselect => {
                self.emit(Op::I64x2RelaxedLaneselect);
                self.stack_height -= 2;
            }
            Instruction::F32x4RelaxedMin => {
                self.emit(Op::F32x4RelaxedMin);
                self.stack_height -= 1;
            }
            Instruction::F32x4RelaxedMax => {
                self.emit(Op::F32x4RelaxedMax);
                self.stack_height -= 1;
            }
            Instruction::F64x2RelaxedMin => {
                self.emit(Op::F64x2RelaxedMin);
                self.stack_height -= 1;
            }
            Instruction::F64x2RelaxedMax => {
                self.emit(Op::F64x2RelaxedMax);
                self.stack_height -= 1;
            }
            Instruction::I16x8RelaxedQ15mulrSigned => {
                self.emit(Op::I16x8RelaxedQ15mulrSigned);
                self.stack_height -= 1;
            }
            Instruction::I16x8RelaxedDotI8x16I7x16Signed => {
                self.emit(Op::I16x8RelaxedDotI8x16I7x16Signed);
                self.stack_height -= 1;
            }
            Instruction::I32x4RelaxedDotI8x16I7x16AddSigned => {
                self.emit(Op::I32x4RelaxedDotI8x16I7x16AddSigned);
                self.stack_height -= 2;
            }
            Instruction::TryTable(..) => todo!(),
            Instruction::StructNew(..) => todo!(),
            Instruction::StructNewDefault(..) => todo!(),
            Instruction::StructGet(..) => todo!(),
            Instruction::StructGetSigned(..) => todo!(),
            Instruction::StructGetUnsigned(..) => todo!(),
            Instruction::StructSet(..) => todo!(),
            Instruction::ArrayNew(..) => todo!(),
            Instruction::ArrayNewDefault(..) => todo!(),
            Instruction::ArrayNewFixed(..) => todo!(),
            Instruction::ArrayNewData(..) => todo!(),
            Instruction::ArrayNewElem(..) => todo!(),
            Instruction::ArrayGet(..) => todo!(),
            Instruction::ArrayGetSigned(..) => todo!(),
            Instruction::ArrayGetUnsigned(..) => todo!(),
            Instruction::ArraySet(..) => todo!(),
            Instruction::ArrayLen => todo!(),
            Instruction::ArrayFill(..) => todo!(),
            Instruction::ArrayCopy(..) => todo!(),
            Instruction::ArrayInitData(..) => todo!(),
            Instruction::ArrayInitElem(..) => todo!(),
            Instruction::RefTest(..) => todo!(),
            Instruction::RefTestNull(..) => todo!(),
            Instruction::RefCast(..) => todo!(),
            Instruction::RefCastNull(..) => todo!(),
            Instruction::BrOnCast(..) => todo!(),
            Instruction::BrOnCastFail(..) => todo!(),
            Instruction::AnyConvertExtern => todo!(),
            Instruction::ExternConvertAny => todo!(),
            Instruction::RefI31 => todo!(),
            Instruction::I31GetSigned => todo!(),
            Instruction::I31GetUnsigned => todo!(),
        }

        self.max_stack_height = self.max_stack_height.max(self.stack_height);
    }
}
