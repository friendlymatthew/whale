use anyhow::{anyhow, bail, ensure, Result};
use std::ops::Neg;

use crate::binary_grammar::{Instruction, RefType, ValueType};
use crate::execution_grammar::{
    Entry, ExternalImport, Frame, FunctionInstance, Label, ModuleInstance, Ref, Stack, Value,
};
use crate::Parser;
use crate::Store;

#[derive(Debug)]
pub struct Interpreter<'a> {
    module_instances: Vec<ModuleInstance<'a>>,
    stack: Stack<'a>,
    store: Store<'a>,
    globals: Vec<Value>,
}

impl<'a> Interpreter<'a> {
    pub fn execute(file_bytes: &'a [u8]) -> Result<()> {
        let _interpreter = Self::new(file_bytes, vec![], vec![], vec![])?;

        Ok(())
    }

    pub fn new(
        module_data: &'a [u8],
        imports: Vec<ExternalImport>,
        initial_global_values: Vec<Value>,
        element_segment_refs: Vec<Vec<Ref>>,
    ) -> Result<Self> {
        let module = Parser::new(module_data).parse_module()?;

        ensure!(
            module.imports.len() == imports.len(),
            "The number of module imports is not equal to the number of provided external values."
        );

        let mut store = Store::new();

        let module_instance =
            store.allocate_module(module, imports, initial_global_values, element_segment_refs)?;

        // store the auxiliary frame
        Ok(Self {
            module_instances: vec![module_instance],
            stack: Stack::new(),
            store,
            globals: vec![],
        })
    }

    pub fn invoke(&mut self, function_addr: usize, args: Vec<Value>) -> Result<()> {
        let function_instance = self.store.functions.get(function_addr).ok_or(anyhow!(
            "Function address: {} does not exist in store.",
            function_addr
        ))?;

        match function_instance {
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

                self.stack.push(Entry::Activation(Frame::default()));

                for arg in args {
                    self.stack.push(Entry::Value(arg));
                }
            }
            _ => todo!("Handle host function instance"),
        }

        Ok(())
    }

    fn invoke_expression(&mut self, expression: Vec<Instruction>, _frame: Frame<'a>) -> Result<()> {
        for instruction in expression {
            match instruction {
                Instruction::Unreachable => {}
                Instruction::Nop => {}
                Instruction::Block(_, _) => {}
                Instruction::Loop(_, _) => {}
                Instruction::IfElse(_, _, _) => {}
                Instruction::Br(_) => {}
                Instruction::BrIf(_) => {}
                Instruction::BrTable(_, _) => {}
                Instruction::Return => {}
                Instruction::Call(_) => {}
                Instruction::CallIndirect(_, _) => {}
                Instruction::RefNull(_) => {}
                Instruction::RefIsNull => {}
                Instruction::RefFunc(_) => {}
                Instruction::Drop => {}
                Instruction::Select(_) => {}
                Instruction::LocalGet(_) => {}
                Instruction::LocalSet(_) => {}
                Instruction::LocalTee(_) => {}
                Instruction::GlobalGet(_) => {}
                Instruction::GlobalSet(_) => {}
                Instruction::TableGet(_) => {}
                Instruction::TableSet(_) => {}
                Instruction::TableInit(_, _) => {}
                Instruction::ElemDrop(_) => {}
                Instruction::TableCopy(_, _) => {}
                Instruction::TableGrow(_) => {}
                Instruction::TableSize(_) => {}
                Instruction::TableFill(_) => {}
                Instruction::I32Load(_) => {}
                Instruction::I64Load(_) => {}
                Instruction::F32Load(_) => {}
                Instruction::F64Load(_) => {}
                Instruction::I32Load8Signed(_) => {}
                Instruction::I32Load8Unsigned(_) => {}
                Instruction::I32Load16Signed(_) => {}
                Instruction::I32Load16Unsigned(_) => {}
                Instruction::I64Load8Signed(_) => {}
                Instruction::I64Load8Unsigned(_) => {}
                Instruction::I64Load16Signed(_) => {}
                Instruction::I64Load16Unsigned(_) => {}
                Instruction::I64Load32Signed(_) => {}
                Instruction::I64Load32Unsigned(_) => {}
                Instruction::I32Store(_) => {}
                Instruction::I64Store(_) => {}
                Instruction::F32Store(_) => {}
                Instruction::F64Store(_) => {}
                Instruction::I32Store8(_) => {}
                Instruction::I32Store16(_) => {}
                Instruction::I64Store8(_) => {}
                Instruction::I64Store16(_) => {}
                Instruction::I64Store32(_) => {}
                Instruction::MemorySize => {}
                Instruction::MemoryGrow => {}
                Instruction::MemoryInit(_) => {}
                Instruction::DataDrop(_) => {}
                Instruction::MemoryCopy => {}
                Instruction::MemoryFill => {}
                Instruction::I32Const(_) => {}
                Instruction::I64Const(_) => {}
                Instruction::F32Const(_) => {}
                Instruction::F64Const(_) => {}
                Instruction::I32EqZero => {}
                Instruction::I32Eq => {
                    let a: i32 = self.stack.pop_as_value()?;
                    let b: i32 = self.stack.pop_as_value()?;
                    self.stack.push_value((a == b) as i32)?;
                }
                Instruction::I32Ne => {
                    let a: i32 = self.stack.pop_as_value()?;
                    let b: i32 = self.stack.pop_as_value()?;
                    self.stack.push_value((a != b) as i32)?;
                }
                Instruction::I32LtSigned => {}
                Instruction::I32LtUnsigned => {}
                Instruction::I32GtSigned => {}
                Instruction::I32GtUnsigned => {}
                Instruction::I32LeSigned => {}
                Instruction::I32LeUnsigned => {}
                Instruction::I32GeSigned => {}
                Instruction::I32GeUnsigned => {}
                Instruction::I64EqZero => {
                    let a: i64 = self.stack.pop_as_value()?;
                    self.stack.push_value((a == 0) as i32)?;
                }
                Instruction::I64Eq => {
                    let a: i64 = self.stack.pop_as_value()?;
                    let b: i64 = self.stack.pop_as_value()?;
                    self.stack.push_value((a == b) as i32)?;
                }
                Instruction::I64Ne => {
                    let a: i64 = self.stack.pop_as_value()?;
                    let b: i64 = self.stack.pop_as_value()?;
                    self.stack.push_value((a != b) as i32)?;
                }
                Instruction::I64LtSigned => {}
                Instruction::I64LtUnsigned => {}
                Instruction::I64GtSigned => {}
                Instruction::I64GtUnsigned => {}
                Instruction::I64LeSigned => {}
                Instruction::I64LeUnsigned => {}
                Instruction::I64GeSigned => {}
                Instruction::I64GeUnsigned => {}
                Instruction::F32Eq => {
                    let a: f32 = self.stack.pop_as_value()?;
                    let b: f32 = self.stack.pop_as_value()?;
                    self.stack.push_value((a == b) as i32)?;
                }
                Instruction::F32Ne => {
                    let a: f32 = self.stack.pop_as_value()?;
                    let b: f32 = self.stack.pop_as_value()?;
                    self.stack.push_value((a != b) as i32)?;
                }
                Instruction::F32Lt => {}
                Instruction::F32Gt => {}
                Instruction::F32Le => {}
                Instruction::F32Ge => {}
                Instruction::F64Eq => {}
                Instruction::F64Ne => {}
                Instruction::F64Lt => {}
                Instruction::F64Gt => {}
                Instruction::F64Le => {}
                Instruction::F64Ge => {}
                Instruction::I32CountLeadingZeros => {
                    let a: i32 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.leading_zeros() as i32)?;
                }
                Instruction::I32CountTrailingZeros => {
                    let a: i32 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.trailing_zeros() as i32)?;
                }
                Instruction::I32PopCount => {}
                Instruction::I32Add => {
                    let a: i32 = self.stack.pop_as_value()?;
                    let b: i32 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.wrapping_add(b))?;
                }
                Instruction::I32Sub => {
                    let a: i32 = self.stack.pop_as_value()?;
                    let b: i32 = self.stack.pop_as_value()?;
                    self.stack.push_value(b.wrapping_sub(a))?;
                }
                Instruction::I32Mul => {
                    let a: i32 = self.stack.pop_as_value()?;
                    let b: i32 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.wrapping_mul(b))?;
                }
                Instruction::I32DivSigned => {}
                Instruction::I32DivUnsigned => {}
                Instruction::I32RemainderSigned => {}
                Instruction::I32RemainderUnsigned => {}
                Instruction::I32And => {}
                Instruction::I32Or => {}
                Instruction::I32Xor => {}
                Instruction::I32Shl => {}
                Instruction::I32ShrSigned => {}
                Instruction::I32ShrUnsigned => {}
                Instruction::I32RotateLeft => {}
                Instruction::I32RotateRight => {}
                Instruction::I64CountLeadingZeros => {
                    let a = self.stack.pop_as_value()?;
                    self.stack.push_value(i64::leading_zeros(a) as i32)?;
                }
                Instruction::I64CountTrailingZeros => {
                    let a = self.stack.pop_as_value()?;
                    self.stack.push_value(i64::trailing_zeros(a) as i32)?;
                }
                Instruction::I64PopCount => {}
                Instruction::I64Add => {
                    let a: i64 = self.stack.pop_as_value()?;
                    let b: i64 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.wrapping_add(b))?;
                }
                Instruction::I64Sub => {
                    let a: i64 = self.stack.pop_as_value()?;
                    let b: i64 = self.stack.pop_as_value()?;
                    self.stack.push_value(b.wrapping_sub(a))?;
                }
                Instruction::I64Mul => {
                    let a: i64 = self.stack.pop_as_value()?;
                    let b: i64 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.wrapping_mul(b))?;
                }
                Instruction::I64DivSigned => {}
                Instruction::I64DivUnsigned => {}
                Instruction::I64RemainderSigned => {}
                Instruction::I64RemainderUnsigned => {}
                Instruction::I64And => {
                    let a: i64 = self.stack.pop_as_value()?;
                    let b: i64 = self.stack.pop_as_value()?;
                    self.stack.push_value(a & b)?;
                }
                Instruction::I64Or => {
                    let a: i64 = self.stack.pop_as_value()?;
                    let b: i64 = self.stack.pop_as_value()?;
                    // self.stack.push_value(a || b)?;
                }
                Instruction::I64Xor => {}
                Instruction::I64Shl => {}
                Instruction::I64ShrSigned => {}
                Instruction::I64ShrUnsigned => {}
                Instruction::I64RotateLeft => {}
                Instruction::I64RotateRight => {}
                Instruction::F32Abs => {
                    let a: f32 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.abs())?;
                }
                Instruction::F32Neg => {
                    let a: f32 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.neg())?;
                }
                Instruction::F32Ceil => {}
                Instruction::F32Floor => {}
                Instruction::F32Trunc => {}
                Instruction::F32Nearest => {}
                Instruction::F32Sqrt => {
                    let a: f32 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.sqrt())?;
                }
                Instruction::F32Add => {
                    let a: f32 = self.stack.pop_as_value()?;
                    let b: f32 = self.stack.pop_as_value()?;
                    self.stack.push_value(a + b)?;
                }
                Instruction::F32Sub => {
                    let a: f32 = self.stack.pop_as_value()?;
                    let b: f32 = self.stack.pop_as_value()?;
                    self.stack.push_value(b - a)?;
                }
                Instruction::F32Mul => {
                    let a: f32 = self.stack.pop_as_value()?;
                    let b: f32 = self.stack.pop_as_value()?;
                    self.stack.push_value(a * b)?;
                }
                Instruction::F32Div => {}
                Instruction::F32Min => {
                    let a: f32 = self.stack.pop_as_value()?;
                    let b: f32 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.min(b))?;
                }
                Instruction::F32Max => {
                    let a: f32 = self.stack.pop_as_value()?;
                    let b: f32 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.max(b))?;
                }
                Instruction::F32CopySign => {}
                Instruction::F64Abs => {
                    let a: f64 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.abs())?;
                }
                Instruction::F64Neg => {
                    let a: f64 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.neg())?;
                }
                Instruction::F64Ceil => {}
                Instruction::F64Floor => {}
                Instruction::F64Trunc => {}
                Instruction::F64Nearest => {}
                Instruction::F64Sqrt => {
                    let a: f64 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.sqrt())?;
                }
                Instruction::F64Add => {
                    let a: f64 = self.stack.pop_as_value()?;
                    let b: f64 = self.stack.pop_as_value()?;
                    self.stack.push_value(a + b)?;
                }
                Instruction::F64Sub => {
                    let a: f64 = self.stack.pop_as_value()?;
                    let b: f64 = self.stack.pop_as_value()?;
                    self.stack.push_value(b - a)?;
                }
                Instruction::F64Mul => {
                    let a: f64 = self.stack.pop_as_value()?;
                    let b: f64 = self.stack.pop_as_value()?;
                    self.stack.push_value(a * b)?;
                }
                Instruction::F64Div => {}
                Instruction::F64Min => {
                    let a: f64 = self.stack.pop_as_value()?;
                    let b: f64 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.min(b))?;
                }
                Instruction::F64Max => {
                    let a: f64 = self.stack.pop_as_value()?;
                    let b: f64 = self.stack.pop_as_value()?;
                    self.stack.push_value(a.max(b))?;
                }
                Instruction::F64CopySign => {}
                Instruction::I32WrapI64 => {}
                Instruction::I32TruncF32Signed => {}
                Instruction::I32TruncF32Unsigned => {}
                Instruction::I32TruncF64Signed => {}
                Instruction::I32TruncF64Unsigned => {}
                Instruction::I64ExtendI32Signed => {}
                Instruction::I64ExtendI32Unsigned => {}
                Instruction::I64TruncF32Signed => {}
                Instruction::I64TruncF32Unsigned => {}
                Instruction::I64TruncF64Signed => {}
                Instruction::I64TruncF64Unsigned => {}
                Instruction::F32ConvertI32Signed => {}
                Instruction::F32ConvertI32Unsigned => {}
                Instruction::F32ConvertI64Signed => {}
                Instruction::F32ConvertI64Unsigned => {}
                Instruction::F32DemoteF64 => {}
                Instruction::F64ConvertI32Signed => {}
                Instruction::F64ConvertI32Unsigned => {}
                Instruction::F64ConvertI64Signed => {}
                Instruction::F64ConvertI64Unsigned => {}
                Instruction::F64PromoteF32 => {}
                Instruction::I32ReinterpretF32 => {}
                Instruction::I64ReinterpretF64 => {}
                Instruction::F32ReinterpretI32 => {}
                Instruction::F64ReinterpretI64 => {}
                Instruction::I32Extend8Signed => {}
                Instruction::I32Extend16Signed => {}
                Instruction::I64Extend8Signed => {}
                Instruction::I64Extend16Signed => {}
                Instruction::I64Extend32Signed => {}
                Instruction::I32TruncSaturatedF32Signed => {}
                Instruction::I32TruncSaturatedF32Unsigned => {}
                Instruction::I32TruncSaturatedF64Signed => {}
                Instruction::I32TruncSaturatedF64Unsigned => {}
                Instruction::I64TruncSaturatedF32Signed => {}
                Instruction::I64TruncSaturatedF32Unsigned => {}
                Instruction::I64TruncSaturatedF64Signed => {}
                Instruction::I64TruncSaturatedF64Unsigned => {}
                Instruction::V128Load(_) => {}
                Instruction::V128Load8x8Signed(_) => {}
                Instruction::V128Load8x8Unsigned(_) => {}
                Instruction::V128Load16x4Unsigned(_) => {}
                Instruction::V128Load16x4Signed(_) => {}
                Instruction::V128Load32x2Signed(_) => {}
                Instruction::V128Load32x2Unsigned(_) => {}
                Instruction::V128Load8Splat(_) => {}
                Instruction::V128Load16Splat(_) => {}
                Instruction::V128Load32Splat(_) => {}
                Instruction::V128Load64Splat(_) => {}
                Instruction::V128Load32Zero(_) => {}
                Instruction::V128Load64Zero(_) => {}
                Instruction::V128Store(_) => {}
                Instruction::V128Load8Lane(_, _) => {}
                Instruction::V128Load16Lane(_, _) => {}
                Instruction::V128Load32Lane(_, _) => {}
                Instruction::V128Load64Lane(_, _) => {}
                Instruction::V128Store8Lane(_, _) => {}
                Instruction::V128Store16Lane(_, _) => {}
                Instruction::V128Store32Lane(_, _) => {}
                Instruction::V128Store64Lane(_, _) => {}
                Instruction::V128Const(_) => {}
                Instruction::I8x16Shuffle(_) => {}
                Instruction::I8x16ExtractLaneSigned(_) => {}
                Instruction::I8x16ExtractLaneUnsigned(_) => {}
                Instruction::I8x16ReplaceLane(_) => {}
                Instruction::I16x8ExtractLaneSigned(_) => {}
                Instruction::I16x8ExtractLaneUnsigned(_) => {}
                Instruction::I16x8ReplaceLane(_) => {}
                Instruction::I32x4ExtractLane(_) => {}
                Instruction::I32x4ReplaceLane(_) => {}
                Instruction::I64x2ExtractLane(_) => {}
                Instruction::I64x2ReplaceLane(_) => {}
                Instruction::F32x4ExtractLane(_) => {}
                Instruction::F32x4ReplaceLane(_) => {}
                Instruction::F64x2ExtractLane(_) => {}
                Instruction::F64x2ReplaceLane(_) => {}
                Instruction::I8x16Swizzle => {}
                Instruction::I8x16Splat => {}
                Instruction::I16x8Splat => {}
                Instruction::I32x4Splat => {}
                Instruction::I64x2Splat => {}
                Instruction::F32x4Splat => {}
                Instruction::F64x2Splat => {}
                Instruction::I8x16Eq => {}
                Instruction::I8x16Ne => {}
                Instruction::I8x16LtSigned => {}
                Instruction::I8x16LtUnsigned => {}
                Instruction::I8x16GtSigned => {}
                Instruction::I8x16GtUnsigned => {}
                Instruction::I8x16LeSigned => {}
                Instruction::I8x16LeUnsigned => {}
                Instruction::I8x16GeSigned => {}
                Instruction::I8x16GeUnsigned => {}
                Instruction::I16x8Eq => {}
                Instruction::I16x8Ne => {}
                Instruction::I16x8LtSigned => {}
                Instruction::I16x8LtUnsigned => {}
                Instruction::I16x8GtSigned => {}
                Instruction::I16x8GtUnsigned => {}
                Instruction::I16x8LeSigned => {}
                Instruction::I16x8LeUnsigned => {}
                Instruction::I16x8GeSigned => {}
                Instruction::I16x8GeUnsigned => {}
                Instruction::I32x4Eq => {}
                Instruction::I32x4Ne => {}
                Instruction::I32x4LtSigned => {}
                Instruction::I32x4LtUnsigned => {}
                Instruction::I32x4GtSigned => {}
                Instruction::I32x4GtUnsigned => {}
                Instruction::I32x4LeSigned => {}
                Instruction::I32x4LeUnsigned => {}
                Instruction::I32x4GeSigned => {}
                Instruction::I32x4GeUnsigned => {}
                Instruction::I64x2Eq => {}
                Instruction::I64x2Ne => {}
                Instruction::I64x2LtSigned => {}
                Instruction::I64x2GtSigned => {}
                Instruction::I64x2LeSigned => {}
                Instruction::I64x2GeSigned => {}
                Instruction::F32X4Eq => {}
                Instruction::F32x4Ne => {}
                Instruction::F32x4Lt => {}
                Instruction::F32x4Gt => {}
                Instruction::F32x4Le => {}
                Instruction::F32x4Ge => {}
                Instruction::F64x2Eq => {}
                Instruction::F64x2Ne => {}
                Instruction::F64x2Lt => {}
                Instruction::F64x2Gt => {}
                Instruction::F64x2Le => {}
                Instruction::F64x2Ge => {}
                Instruction::V128Not => {}
                Instruction::V128And => {}
                Instruction::V128AndNot => {}
                Instruction::V128Or => {}
                Instruction::V128Xor => {}
                Instruction::V128BitSelect => {}
                Instruction::V128AnyTrue => {}
                Instruction::I8x16Abs => {}
                Instruction::I8x16Neg => {}
                Instruction::I8x16PopCount => {}
                Instruction::I8x16AllTrue => {}
                Instruction::I8x16BitMask => {}
                Instruction::I8x16NarrowI16x8Signed => {}
                Instruction::I8x16NarrowI16x8Unsigned => {}
                Instruction::I8x16Shl => {}
                Instruction::I8x16ShrSigned => {}
                Instruction::I8x16ShrUnsigned => {}
                Instruction::I8x16Add => {}
                Instruction::I8x16AddSaturatedSigned => {}
                Instruction::I8x16AddSaturatedUnsigned => {}
                Instruction::I8x16Sub => {}
                Instruction::I8x16SubSaturatedSigned => {}
                Instruction::I8x16SubSaturatedUnsigned => {}
                Instruction::I8x16MinSigned => {}
                Instruction::I8x16MinUnsigned => {}
                Instruction::I8x16MaxSigned => {}
                Instruction::I8x16MaxUnsigned => {}
                Instruction::I8x16AvgRangeUnsigned => {}
                Instruction::I16x8ExtAddPairWiseI8x16Signed => {}
                Instruction::I16x8ExtAddPairWiseI8x16Unsigned => {}
                Instruction::I16x8Abs => {}
                Instruction::I16x8Neg => {}
                Instruction::I16xQ15MulRangeSaturatedSigned => {}
                Instruction::I16x8AllTrue => {}
                Instruction::I16x8BitMask => {}
                Instruction::I16x8NarrowI32x4Signed => {}
                Instruction::I16x8NarrowI32x4Unsigned => {}
                Instruction::I16x8ExtendLowI8x16Unsigned => {}
                Instruction::I16x8ExtendHighI8x16Unsigned => {}
                Instruction::I16x8ExtendLowI8x16Signed => {}
                Instruction::I16x8ExtendHighI8x16Signed => {}
                Instruction::I16x8Shl => {}
                Instruction::I16x8ShrSigned => {}
                Instruction::I16x8ShrUnsigned => {}
                Instruction::I16x8Add => {}
                Instruction::I16x8AddSaturatedSigned => {}
                Instruction::I16x8AddSaturatedUnsigned => {}
                Instruction::I16x8Sub => {}
                Instruction::I16x8SubSaturatedSigned => {}
                Instruction::I16x8SubSaturatedUnsigned => {}
                Instruction::I16x8Mul => {}
                Instruction::I16x8MinSigned => {}
                Instruction::I16x8MinUnsigned => {}
                Instruction::I16x8MaxSigned => {}
                Instruction::I16x8MaxUnsigned => {}
                Instruction::I16x8AvgRangeUnsigned => {}
                Instruction::I16x8ExtMulLowI8x16Signed => {}
                Instruction::I16x8ExtMulHighI8x16Signed => {}
                Instruction::I16x8ExtMulLowI8x16Unsigned => {}
                Instruction::I16x8ExtMulHighI8x16Unsigned => {}
                Instruction::I32x4ExtAddPairWiseI16x8Signed => {}
                Instruction::I32x4ExtAddPairWiseI16x8Unsigned => {}
                Instruction::I32x4Abs => {}
                Instruction::I32x4Neg => {}
                Instruction::I32x4AllTrue => {}
                Instruction::I32x4BitMask => {}
                Instruction::I32x4ExtendLowI16x8Signed => {}
                Instruction::I32x4ExtendHighI16x8Signed => {}
                Instruction::I32x4ExtendLowI16x8Unsigned => {}
                Instruction::I32x4ExtendHighI16x8Unsigned => {}
                Instruction::I32x4Shl => {}
                Instruction::I32x4ShrSigned => {}
                Instruction::I32x4ShrUnsigned => {}
                Instruction::I32x4Add => {}
                Instruction::I32x4Sub => {}
                Instruction::I32x4Mul => {}
                Instruction::I32x4MinSigned => {}
                Instruction::I32x4MinUnsigned => {}
                Instruction::I32x4MaxSigned => {}
                Instruction::I32x4MaxUnsigned => {}
                Instruction::I32x4DotI16x8Signed => {}
                Instruction::I32x4ExtMulLowI16x8Signed => {}
                Instruction::I32x4ExtMulHighI16x8Signed => {}
                Instruction::I32x4ExtMulLowI16x8Unsigned => {}
                Instruction::I32x4ExtMulHighI16x8Unsigned => {}
                Instruction::I64x2Abs => {}
                Instruction::I64x2Neg => {}
                Instruction::I64x2AllTrue => {}
                Instruction::I64x2BitMask => {}
                Instruction::I64x2ExtendLowI32x4Signed => {}
                Instruction::I64x2ExtendHighI32x4Signed => {}
                Instruction::I64x2ExtendLowI32x4Unsigned => {}
                Instruction::I64x2ExtendHighI32x4Unsigned => {}
                Instruction::I64x2Shl => {}
                Instruction::I64x2ShrSigned => {}
                Instruction::I64x2ShrUnsigned => {}
                Instruction::I64x2Add => {}
                Instruction::I64x2Sub => {}
                Instruction::I64x2Mul => {}
                Instruction::I64x2ExtMulLowI32x4Signed => {}
                Instruction::I64x2ExtMulHighI32x4Signed => {}
                Instruction::I64x2ExtMulLowI32x4Unsigned => {}
                Instruction::I64x2ExtMulHighI32x4Unsigned => {}
                Instruction::F32x4Ceil => {}
                Instruction::F32x4Floor => {}
                Instruction::F32x4Trunc => {}
                Instruction::F32x4Nearest => {}
                Instruction::F32x4Abs => {}
                Instruction::F32x4Neg => {}
                Instruction::F32x4Sqrt => {}
                Instruction::F32x4Add => {}
                Instruction::F32x4Sub => {}
                Instruction::F32x4Mul => {}
                Instruction::F32x4Div => {}
                Instruction::F32x4Min => {}
                Instruction::F32x4Max => {}
                Instruction::F32x4PMin => {}
                Instruction::F32x4PMax => {}
                Instruction::F64x2Ceil => {}
                Instruction::F64x2Floor => {}
                Instruction::F64x2Trunc => {}
                Instruction::F64x2Nearest => {}
                Instruction::F64x2Abs => {}
                Instruction::F64x2Neg => {}
                Instruction::F64x2Sqrt => {}
                Instruction::F64x2Add => {}
                Instruction::F64x2Sub => {}
                Instruction::F64x2Mul => {}
                Instruction::F64x2Div => {}
                Instruction::F64x2Min => {}
                Instruction::F64x2Max => {}
                Instruction::F64x2PMin => {}
                Instruction::F64x2PMax => {}
                Instruction::I32x4TruncSaturatedF32x4Signed => {}
                Instruction::I32x4TruncSaturatedF32x4Unsigned => {}
                Instruction::F32x4ConvertI32x4Signed => {}
                Instruction::F32x4ConvertI32x4Unsigned => {}
                Instruction::I32x4TruncSaturatedF64x2SignedZero => {}
                Instruction::I32x4TruncSaturatedF64x2UnsignedZero => {}
                Instruction::F64x2ConvertLowI32x4Signed => {}
                Instruction::F64x2ConvertLowI32x4Unsigned => {}
                Instruction::F32x4DemoteF64x2Zero => {}
                Instruction::F64xPromoteLowF32x4 => {}
            }
        }

        Ok(())
    }

    fn invoke_function_call(&mut self, function_addr: usize) -> Result<()> {
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

                code.locals
                    .iter()
                    .for_each(|local| locals.push(Entry::Value(Value::default(&local.value_type))));

                let locals = locals
                    .into_iter()
                    .map(|entry| match entry {
                        Entry::Value(v) => Ok(v),
                        _ => Err(anyhow!("Expected entry off the stack to be a value.")),
                    })
                    .collect::<Result<Vec<_>>>()?;

                let num_ret_args = function_type.1 .0.len();

                let f = Frame {
                    arity: num_ret_args,
                    locals,
                    module: module.clone(),
                };

                self.stack.push(Entry::Activation(f.clone()));

                let l = Label {
                    arity: num_ret_args as u32,
                    continuation: code.body.clone(),
                };

                self.stack.push(Entry::Label(l));

                self.invoke_expression(code.body.clone(), f)?;

                if num_ret_args == 0 {
                    // no return
                }
            }
            _ => todo!("What does invoking a Host Function look like"),
        }

        Ok(())
    }
}
