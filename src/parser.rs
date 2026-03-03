use std::cmp::min;
use std::collections::VecDeque;

use anyhow::{anyhow, bail, ensure, Result};

use crate::binary_grammar::{
    AddrType, ArrayType, BlockType, CatchClause, CodeSection, CompositeType, CustomSection,
    DataMode, DataSection, DataSegment, ElementMode, ElementSection, ElementSegment, Export,
    ExportDescription, ExportSection, FieldType, Function, FunctionSection, FunctionType, Global,
    GlobalSection, GlobalType, HeapType, ImportDeclaration, ImportDescription, ImportSection,
    Instruction, Limit, Local, MemArg, MemorySection, MemoryType, Module, Mutability, RefType,
    ResultType, Section, StorageType, StructType, SubType, TableDef, TableSection, TableType, Tag,
    TagSection, TypeSection, ValueType, MAGIC_NUMBER, TERM_ELSE_BYTE, TERM_END_BYTE,
};
use crate::leb128::{self, MAX_LEB128_LEN_32, MAX_LEB128_LEN_64};

#[derive(Debug)]
pub struct Parser<'a> {
    cursor: usize,
    buffer: &'a [u8],
    function_types: VecDeque<u32>,
}

impl<'a> Parser<'a> {
    pub const fn new(buffer: &'a [u8]) -> Self {
        Self {
            buffer,
            cursor: 0,
            function_types: VecDeque::new(),
        }
    }

    /// Parses a .wasm file in its entirety.
    pub fn parse_module(&mut self) -> Result<Module> {
        let mut module = Module::new(self.parse_preamble()?);
        let mut data_count: Option<u32> = None;

        while self.cursor < self.buffer.len() {
            let id = self.read_u8()?;

            match self.parse_section(id)? {
                Section::Custom(custom) => module.customs.push(custom),
                Section::Type(TypeSection { mut types }) => module.types.append(&mut types),
                Section::Import(ImportSection {
                    mut import_declarations,
                }) => module.import_declarations.append(&mut import_declarations),
                Section::Function(FunctionSection { indices }) => {
                    self.function_types.extend(indices)
                }
                Section::Table(TableSection { mut tables }) => module.tables.append(&mut tables),
                Section::Memory(MemorySection { mut memories }) => {
                    module.mems.append(&mut memories)
                }
                Section::Global(GlobalSection { mut globals }) => {
                    module.globals.append(&mut globals)
                }
                Section::Export(ExportSection { mut exports }) => {
                    module.exports.append(&mut exports)
                }
                Section::Start(idx) => module.start = Some(idx),
                Section::Element(ElementSection { mut elements }) => {
                    module.element_segments.append(&mut elements)
                }
                Section::Code(CodeSection { mut codes }) => module.functions.append(&mut codes),
                Section::Data(DataSection { mut data_segments }) => {
                    module.data_segments.append(&mut data_segments)
                }
                Section::DataCount(n) => data_count = Some(n),
                Section::Tag(TagSection { mut tags }) => module.tags.append(&mut tags),
            }

        }

        if let Some(count) = data_count {
            ensure!(
                count as usize == module.data_segments.len(),
                "Data count {} does not match number of data segments {}",
                count,
                module.data_segments.len()
            );
        }

        Ok(module)
    }

    fn parse_preamble(&mut self) -> Result<u8> {
        ensure!(
            self.read_slice(4)? == MAGIC_NUMBER,
            "Expected magic number in preamble."
        );

        ensure!(self.read_slice(4)? == [1, 0, 0, 0], "Expected version 1.");

        Ok(1)
    }

    fn read_u8(&mut self) -> Result<u8> {
        let b = self.read_slice(1)?[0];

        Ok(b)
    }

    fn read_u32(&mut self) -> Result<u32> {
        let buf = self.peek_leb_slice::<MAX_LEB128_LEN_32>()?;

        let (out, seen) = leb128::read_u32(buf)?;
        self.cursor += seen;

        Ok(out)
    }

    fn read_i32(&mut self) -> Result<i32> {
        let buf = self.peek_leb_slice::<MAX_LEB128_LEN_32>()?;

        let (out, seen) = leb128::read_i32(buf)?;
        self.cursor += seen;

        Ok(out)
    }

    fn read_u64(&mut self) -> Result<u64> {
        let buf = self.peek_leb_slice::<MAX_LEB128_LEN_64>()?;

        let (out, seen) = leb128::read_u64(buf)?;
        self.cursor += seen;

        Ok(out)
    }

    fn read_i64(&mut self) -> Result<i64> {
        let buf = self.peek_leb_slice::<MAX_LEB128_LEN_64>()?;

        let (out, seen) = leb128::read_i64(buf)?;
        self.cursor += seen;

        Ok(out)
    }

    fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.read_slice(4)?.try_into()?))
    }

    fn read_f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.read_slice(8)?.try_into()?))
    }

    fn peek_leb_slice<const MAX_LEB128_LEN: usize>(&self) -> Result<&'a [u8]> {
        let max_len = min(self.buffer.len() - self.cursor, MAX_LEB128_LEN);

        self.peek_slice(max_len)
    }

    fn peek_slice(&self, len: usize) -> Result<&'a [u8]> {
        self.buffer
            .get(self.cursor..self.cursor + len)
            .ok_or_else(|| anyhow!("oob"))
    }

    fn read_slice(&mut self, len: usize) -> Result<&'a [u8]> {
        let buf = self.peek_slice(len)?;
        self.cursor += len;

        Ok(buf)
    }

    fn parse_vec<T>(&mut self, parse: impl Fn(&mut Self) -> Result<T>) -> Result<Vec<T>> {
        let len = self.read_u32()?;

        let mut items = Vec::with_capacity(len as usize);

        for _ in 0..len {
            items.push(parse(self)?);
        }

        Ok(items)
    }

    // 2.5.6: Global
    fn parse_global(&mut self) -> Result<Global> {
        Ok(Global {
            global_type: self.parse_global_type()?,
            initial_expression: self.parse_expression()?,
        })
    }

    // 5.2: Values
    fn parse_name(&mut self) -> Result<String> {
        let n = self.read_u32()?;
        let slice = self.read_slice(n as usize)?;

        Ok(std::str::from_utf8(slice)?.to_owned())
    }

    // 5.3: Types
    fn parse_abs_heap_type(&mut self) -> Result<HeapType> {
        let ht = match self.read_u8()? {
            0x70 => HeapType::Func,
            0x6F => HeapType::Extern,
            0x6E => HeapType::Any,
            0x6D => HeapType::Eq,
            0x6C => HeapType::I31,
            0x6B => HeapType::Struct,
            0x6A => HeapType::Array,
            0x69 => HeapType::Exn,
            0x71 => HeapType::None,
            0x72 => HeapType::NoExtern,
            0x73 => HeapType::NoFunc,
            0x74 => HeapType::NoExn,
            foreign => bail!("Unrecognized abstract heap type byte: {}", foreign),
        };
        Ok(ht)
    }

    fn parse_heap_type(&mut self) -> Result<HeapType> {
        let byte = self.buffer[self.cursor];
        match byte {
            0x69..=0x74 => self.parse_abs_heap_type(),
            _ => {
                // Type index encoded as s33 (positive signed integer)
                let idx = self.read_i64()?;
                ensure!(
                    idx >= 0,
                    "heap type index must be non-negative, got {}",
                    idx
                );
                Ok(HeapType::TypeIndex(idx as u32))
            }
        }
    }

    fn parse_reference_type(&mut self) -> Result<RefType> {
        let byte = self.buffer[self.cursor];
        let r = match byte {
            0x70 => {
                self.cursor += 1;
                RefType::FuncRef
            }
            0x6F => {
                self.cursor += 1;
                RefType::ExternRef
            }
            0x63 => {
                self.cursor += 1;
                let ht = self.parse_heap_type()?;
                RefType::Ref {
                    nullable: true,
                    heap_type: ht,
                }
            }
            0x64 => {
                self.cursor += 1;
                let ht = self.parse_heap_type()?;
                RefType::Ref {
                    nullable: false,
                    heap_type: ht,
                }
            }
            // Abstract heap type shorthands → ref null ht
            0x69..=0x6E | 0x71..=0x74 => {
                let ht = self.parse_abs_heap_type()?;
                RefType::Ref {
                    nullable: true,
                    heap_type: ht,
                }
            }
            foreign => bail!("Unrecognized reference byte. Got: {}", foreign),
        };
        Ok(r)
    }

    fn parse_value_type(&mut self) -> Result<ValueType> {
        let byte = self.buffer[self.cursor];
        let value_type = match byte {
            0x7F => {
                self.cursor += 1;
                ValueType::I32
            }
            0x7E => {
                self.cursor += 1;
                ValueType::I64
            }
            0x7D => {
                self.cursor += 1;
                ValueType::F32
            }
            0x7C => {
                self.cursor += 1;
                ValueType::F64
            }
            0x7B => {
                self.cursor += 1;
                ValueType::V128
            }
            // Reference types (includes 0x70, 0x6F, 0x63, 0x64, 0x69-0x6E, 0x71-0x74)
            0x63 | 0x64 | 0x69..=0x74 => ValueType::Ref(self.parse_reference_type()?),
            foreign => bail!("Unrecognized type. Got: {}", foreign),
        };
        Ok(value_type)
    }

    fn parse_result_type(&mut self) -> Result<ResultType> {
        Ok(ResultType(self.parse_vec(Self::parse_value_type)?))
    }

    fn parse_mutability(&mut self) -> Result<Mutability> {
        match self.read_u8()? {
            0x00 => Ok(Mutability::Const),
            0x01 => Ok(Mutability::Var),
            foreign => bail!(
                "Unrecognized mutability byte. Expected 0x00 or 0x01, Got: {}",
                foreign
            ),
        }
    }

    fn parse_storage_type(&mut self) -> Result<StorageType> {
        let byte = self.buffer[self.cursor];
        match byte {
            0x78 => {
                self.cursor += 1;
                Ok(StorageType::I8)
            }
            0x77 => {
                self.cursor += 1;
                Ok(StorageType::I16)
            }
            _ => Ok(StorageType::Val(self.parse_value_type()?)),
        }
    }

    fn parse_field_type(&mut self) -> Result<FieldType> {
        let storage_type = self.parse_storage_type()?;
        let mutability = self.parse_mutability()?;
        Ok(FieldType {
            storage_type,
            mutability,
        })
    }

    fn parse_composite_type(&mut self) -> Result<CompositeType> {
        let b = self.read_u8()?;
        match b {
            0x60 => {
                let arg_type = self.parse_result_type()?;
                let return_type = self.parse_result_type()?;
                Ok(CompositeType::Func(FunctionType(arg_type, return_type)))
            }
            0x5E => {
                let field_type = self.parse_field_type()?;
                Ok(CompositeType::Array(ArrayType { field_type }))
            }
            0x5F => {
                let fields = self.parse_vec(Self::parse_field_type)?;
                Ok(CompositeType::Struct(StructType { fields }))
            }
            _ => bail!(
                "Expected composite type (0x5E/0x5F/0x60), got: 0x{:02X} at pos {}",
                b,
                self.cursor - 1
            ),
        }
    }

    fn parse_sub_type(&mut self) -> Result<SubType> {
        let byte = self.buffer[self.cursor];
        match byte {
            0x4F => {
                self.cursor += 1;
                let supertypes = self.parse_vec(Self::read_u32)?;
                let composite_type = self.parse_composite_type()?;
                Ok(SubType {
                    is_final: true,
                    supertypes,
                    composite_type,
                })
            }
            0x50 => {
                self.cursor += 1;
                let supertypes = self.parse_vec(Self::read_u32)?;
                let composite_type = self.parse_composite_type()?;
                Ok(SubType {
                    is_final: false,
                    supertypes,
                    composite_type,
                })
            }
            _ => {
                let composite_type = self.parse_composite_type()?;
                Ok(SubType {
                    is_final: true,
                    supertypes: vec![],
                    composite_type,
                })
            }
        }
    }

    fn parse_rec_type(&mut self) -> Result<Vec<SubType>> {
        let byte = self.buffer[self.cursor];
        if byte == 0x4E {
            self.cursor += 1;
            Ok(self.parse_vec(Self::parse_sub_type)?)
        } else {
            Ok(vec![self.parse_sub_type()?])
        }
    }

    fn parse_limit(&mut self) -> Result<(AddrType, Limit)> {
        let flag = self.read_u8()?;

        match flag {
            0x00 => Ok((
                AddrType::I32,
                Limit {
                    min: self.read_u32()? as u64,
                    max: u64::MAX,
                },
            )),
            0x01 => Ok((
                AddrType::I32,
                Limit {
                    min: self.read_u32()? as u64,
                    max: self.read_u32()? as u64,
                },
            )),
            0x04 => Ok((
                AddrType::I64,
                Limit {
                    min: self.read_u64()?,
                    max: u64::MAX,
                },
            )),
            0x05 => Ok((
                AddrType::I64,
                Limit {
                    min: self.read_u64()?,
                    max: self.read_u64()?,
                },
            )),
            _ => bail!("Expected limit flag 0x00/0x01/0x04/0x05. Got: 0x{:02X}", flag),
        }
    }

    fn parse_memory_type(&mut self) -> Result<MemoryType> {
        let (addr_type, limit) = self.parse_limit()?;
        Ok(MemoryType { addr_type, limit })
    }

    fn parse_table_type(&mut self) -> Result<TableType> {
        let element_reference_type = self.parse_reference_type()?;
        let (addr_type, limit) = self.parse_limit()?;
        Ok(TableType {
            element_reference_type,
            addr_type,
            limit,
        })
    }

    fn parse_global_type(&mut self) -> Result<GlobalType> {
        Ok(GlobalType {
            value_type: self.parse_value_type()?,
            mutability: match self.read_u8()? {
                0x00 => Mutability::Const,
                0x01 => Mutability::Var,
                foreign => bail!(
                    "Unrecognized mutability byte. Expected 0x00 or 0x01, Got: {}",
                    foreign
                ),
            },
        })
    }

    // 5.4: Instructions

    fn parse_block_type(&mut self) -> Result<BlockType> {
        let byte = self.buffer[self.cursor];
        if byte == 0x40 {
            self.cursor += 1;
            Ok(BlockType::Empty)
        } else if matches!(byte,
            0x7B..=0x7F |                    // numtype + vectype
            0x70 | 0x6F |                    // funcref, externref
            0x63 | 0x64 |                    // ref null ht, ref ht
            0x69..=0x6E | 0x71..=0x74        // abstract heap type shorthands
        ) {
            Ok(BlockType::SingleValue(self.parse_value_type()?))
        } else {
            Ok(BlockType::TypeIndex(self.read_i32()?))
        }
    }

    fn parse_memarg(&mut self) -> Result<MemArg> {
        let align_raw = self.read_u32()?;
        let (align, memory) = if align_raw & (1 << 6) != 0 {
            let align = align_raw & !(1 << 6);
            let mem_idx = self.read_u32()?;
            (align, mem_idx)
        } else {
            (align_raw, 0)
        };
        Ok(MemArg {
            align,
            offset: self.read_u64()?,
            memory,
        })
    }

    fn parse_catch_clause(&mut self) -> Result<CatchClause> {
        let kind = self.read_u8()?;
        match kind {
            0x00 => Ok(CatchClause::Catch {
                tag: self.read_u32()?,
                label: self.read_u32()?,
            }),
            0x01 => Ok(CatchClause::CatchRef {
                tag: self.read_u32()?,
                label: self.read_u32()?,
            }),
            0x02 => Ok(CatchClause::CatchAll {
                label: self.read_u32()?,
            }),
            0x03 => Ok(CatchClause::CatchAllRef {
                label: self.read_u32()?,
            }),
            _ => bail!("Unknown catch clause kind: {}", kind),
        }
    }

    fn parse_expression(&mut self) -> Result<Vec<Instruction>> {
        let mut instructions = vec![];

        loop {
            let opcode = self.read_u8()?;

            if opcode == TERM_END_BYTE {
                break;
            }

            let instruction = self.parse_instruction(opcode)?;

            instructions.push(instruction);
        }

        Ok(instructions)
    }

    fn parse_if_else(&mut self) -> Result<(Vec<Instruction>, Vec<Instruction>)> {
        let mut if_else = (vec![], vec![]);

        let mut else_flag = false;

        loop {
            let opcode = self.read_u8()?;

            if opcode == TERM_ELSE_BYTE {
                else_flag = true;
                continue;
            }

            if opcode == TERM_END_BYTE {
                break;
            }

            let instruction = self.parse_instruction(opcode)?;

            if else_flag {
                if_else.1.push(instruction);
            } else {
                if_else.0.push(instruction);
            }
        }

        Ok(if_else)
    }

    fn parse_instruction(&mut self, opcode: u8) -> Result<Instruction> {
        let instr = match opcode {
            0x00 => Instruction::Unreachable,
            0x01 => Instruction::Nop,
            0x02 => Instruction::Block(self.parse_block_type()?, self.parse_expression()?),
            0x03 => Instruction::Loop(self.parse_block_type()?, self.parse_expression()?),
            0x04 => {
                let bt = self.parse_block_type()?;
                let (if_exprs, else_exprs) = self.parse_if_else()?;
                Instruction::IfElse(bt, if_exprs, else_exprs)
            }
            0x08 => Instruction::Throw(self.read_u32()?),
            0x0A => Instruction::ThrowRef,
            0x0C => Instruction::Br(self.read_u32()?),
            0x0D => Instruction::BrIf(self.read_u32()?),
            0x0E => Instruction::BrTable(self.parse_vec(Self::read_u32)?, self.read_u32()?),
            0x0F => Instruction::Return,
            0x10 => Instruction::Call(self.read_u32()?),
            0x11 => Instruction::CallIndirect(self.read_u32()?, self.read_u32()?),
            0x12 => Instruction::ReturnCall(self.read_u32()?),
            0x13 => Instruction::ReturnCallIndirect(self.read_u32()?, self.read_u32()?),
            0x14 => Instruction::CallRef(self.read_u32()?),
            0x15 => Instruction::ReturnCallRef(self.read_u32()?),
            0x1F => {
                let bt = self.parse_block_type()?;
                let catches = self.parse_vec(Self::parse_catch_clause)?;
                let body = self.parse_expression()?;
                Instruction::TryTable(bt, catches, body)
            }
            0xD0 => Instruction::RefNull(self.parse_heap_type()?),
            0xD1 => Instruction::RefIsNull,
            0xD2 => Instruction::RefFunc(self.read_u32()?),
            0xD3 => Instruction::RefEq,
            0xD4 => Instruction::RefAsNonNull,
            0xD5 => Instruction::BrOnNull(self.read_u32()?),
            0xD6 => Instruction::BrOnNonNull(self.read_u32()?),
            0x1A => Instruction::Drop,
            0x1B => Instruction::Select(vec![]),
            0x1C => Instruction::Select(self.parse_vec(Self::parse_value_type)?),
            0x20 => Instruction::LocalGet(self.read_u32()?),
            0x21 => Instruction::LocalSet(self.read_u32()?),
            0x22 => Instruction::LocalTee(self.read_u32()?),
            0x23 => Instruction::GlobalGet(self.read_u32()?),
            0x24 => Instruction::GlobalSet(self.read_u32()?),
            0x25 => Instruction::TableGet(self.read_u32()?),
            0x26 => Instruction::TableSet(self.read_u32()?),
            0xFB => match self.read_u32()? {
                0x00 => Instruction::StructNew(self.read_u32()?),
                0x01 => Instruction::StructNewDefault(self.read_u32()?),
                0x02 => {
                    let type_idx = self.read_u32()?;
                    let field_idx = self.read_u32()?;
                    Instruction::StructGet(type_idx, field_idx)
                }
                0x03 => {
                    let type_idx = self.read_u32()?;
                    let field_idx = self.read_u32()?;
                    Instruction::StructGetSigned(type_idx, field_idx)
                }
                0x04 => {
                    let type_idx = self.read_u32()?;
                    let field_idx = self.read_u32()?;
                    Instruction::StructGetUnsigned(type_idx, field_idx)
                }
                0x05 => {
                    let type_idx = self.read_u32()?;
                    let field_idx = self.read_u32()?;
                    Instruction::StructSet(type_idx, field_idx)
                }
                0x06 => Instruction::ArrayNew(self.read_u32()?),
                0x07 => Instruction::ArrayNewDefault(self.read_u32()?),
                0x08 => {
                    let type_idx = self.read_u32()?;
                    let size = self.read_u32()?;
                    Instruction::ArrayNewFixed(type_idx, size)
                }
                0x09 => {
                    let type_idx = self.read_u32()?;
                    let data_idx = self.read_u32()?;
                    Instruction::ArrayNewData(type_idx, data_idx)
                }
                0x0A => {
                    let type_idx = self.read_u32()?;
                    let elem_idx = self.read_u32()?;
                    Instruction::ArrayNewElem(type_idx, elem_idx)
                }
                0x0B => Instruction::ArrayGet(self.read_u32()?),
                0x0C => Instruction::ArrayGetSigned(self.read_u32()?),
                0x0D => Instruction::ArrayGetUnsigned(self.read_u32()?),
                0x0E => Instruction::ArraySet(self.read_u32()?),
                0x0F => Instruction::ArrayLen,
                0x10 => Instruction::ArrayFill(self.read_u32()?),
                0x11 => {
                    let dst_type_idx = self.read_u32()?;
                    let src_type_idx = self.read_u32()?;
                    Instruction::ArrayCopy(dst_type_idx, src_type_idx)
                }
                0x12 => {
                    let type_idx = self.read_u32()?;
                    let data_idx = self.read_u32()?;
                    Instruction::ArrayInitData(type_idx, data_idx)
                }
                0x13 => {
                    let type_idx = self.read_u32()?;
                    let elem_idx = self.read_u32()?;
                    Instruction::ArrayInitElem(type_idx, elem_idx)
                }
                0x14 => Instruction::RefTest(self.parse_heap_type()?),
                0x15 => Instruction::RefTestNull(self.parse_heap_type()?),
                0x16 => Instruction::RefCast(self.parse_heap_type()?),
                0x17 => Instruction::RefCastNull(self.parse_heap_type()?),
                0x18 => {
                    let flags = self.read_u8()?;
                    let label = self.read_u32()?;
                    let ht1 = self.parse_heap_type()?;
                    let ht2 = self.parse_heap_type()?;
                    Instruction::BrOnCast(flags, label, ht1, ht2)
                }
                0x19 => {
                    let flags = self.read_u8()?;
                    let label = self.read_u32()?;
                    let ht1 = self.parse_heap_type()?;
                    let ht2 = self.parse_heap_type()?;
                    Instruction::BrOnCastFail(flags, label, ht1, ht2)
                }
                0x1A => Instruction::AnyConvertExtern,
                0x1B => Instruction::ExternConvertAny,
                0x1C => Instruction::RefI31,
                0x1D => Instruction::I31GetSigned,
                0x1E => Instruction::I31GetUnsigned,
                foreign => bail!("Encountered unknown GC opcode: 0xFB 0x{:02X}", foreign),
            },
            0xFC => match self.read_u32()? {
                0 => Instruction::I32TruncSaturatedF32Signed,
                1 => Instruction::I32TruncSaturatedF32Unsigned,
                2 => Instruction::I32TruncSaturatedF64Signed,
                3 => Instruction::I32TruncSaturatedF64Unsigned,
                4 => Instruction::I64TruncSaturatedF32Signed,
                5 => Instruction::I64TruncSaturatedF32Unsigned,
                6 => Instruction::I64TruncSaturatedF64Signed,
                7 => Instruction::I64TruncSaturatedF64Unsigned,
                8 => Instruction::MemoryInit(self.read_u32()?, self.read_u32()?),
                9 => Instruction::DataDrop(self.read_u32()?),
                10 => {
                    let dst_mem = self.read_u32()?;
                    let src_mem = self.read_u32()?;
                    Instruction::MemoryCopy(dst_mem, src_mem)
                }
                11 => {
                    let mem_idx = self.read_u32()?;
                    Instruction::MemoryFill(mem_idx)
                }
                12 => {
                    let y = self.read_u32()?;
                    let x = self.read_u32()?;
                    Instruction::TableInit(x, y)
                }
                13 => Instruction::ElemDrop(self.read_u32()?),
                14 => Instruction::TableCopy(self.read_u32()?, self.read_u32()?),
                15 => Instruction::TableGrow(self.read_u32()?),
                16 => Instruction::TableSize(self.read_u32()?),
                17 => Instruction::TableFill(self.read_u32()?),
                foreign => bail!("Encountered foreign table opcode: {}", foreign),
            },
            0x28 => Instruction::I32Load(self.parse_memarg()?),
            0x29 => Instruction::I64Load(self.parse_memarg()?),
            0x2A => Instruction::F32Load(self.parse_memarg()?),
            0x2B => Instruction::F64Load(self.parse_memarg()?),
            0x2C => Instruction::I32Load8Signed(self.parse_memarg()?),
            0x2D => Instruction::I32Load8Unsigned(self.parse_memarg()?),
            0x2E => Instruction::I32Load16Signed(self.parse_memarg()?),
            0x2F => Instruction::I32Load16Unsigned(self.parse_memarg()?),
            0x30 => Instruction::I64Load8Signed(self.parse_memarg()?),
            0x31 => Instruction::I64Load8Unsigned(self.parse_memarg()?),
            0x32 => Instruction::I64Load16Signed(self.parse_memarg()?),
            0x33 => Instruction::I64Load16Unsigned(self.parse_memarg()?),
            0x34 => Instruction::I64Load32Signed(self.parse_memarg()?),
            0x35 => Instruction::I64Load32Unsigned(self.parse_memarg()?),
            0x36 => Instruction::I32Store(self.parse_memarg()?),
            0x37 => Instruction::I64Store(self.parse_memarg()?),
            0x38 => Instruction::F32Store(self.parse_memarg()?),
            0x39 => Instruction::F64Store(self.parse_memarg()?),
            0x3A => Instruction::I32Store8(self.parse_memarg()?),
            0x3B => Instruction::I32Store16(self.parse_memarg()?),
            0x3C => Instruction::I64Store8(self.parse_memarg()?),
            0x3D => Instruction::I64Store16(self.parse_memarg()?),
            0x3E => Instruction::I64Store32(self.parse_memarg()?),
            0x3F => {
                let mem_idx = self.read_u32()?;
                Instruction::MemorySize(mem_idx)
            }
            0x40 => {
                let mem_idx = self.read_u32()?;
                Instruction::MemoryGrow(mem_idx)
            }
            0x41 => Instruction::I32Const(self.read_i32()?),
            0x42 => Instruction::I64Const(self.read_i64()?),
            0x43 => Instruction::F32Const(self.read_f32()?),
            0x44 => Instruction::F64Const(self.read_f64()?),
            0x45 => Instruction::I32EqZero,
            0x46 => Instruction::I32Eq,
            0x47 => Instruction::I32Ne,
            0x48 => Instruction::I32LtSigned,
            0x49 => Instruction::I32LtUnsigned,
            0x4A => Instruction::I32GtSigned,
            0x4B => Instruction::I32GtUnsigned,
            0x4C => Instruction::I32LeSigned,
            0x4D => Instruction::I32LeUnsigned,
            0x4E => Instruction::I32GeSigned,
            0x4F => Instruction::I32GeUnsigned,
            0x50 => Instruction::I64EqZero,
            0x51 => Instruction::I64Eq,
            0x52 => Instruction::I64Ne,
            0x53 => Instruction::I64LtSigned,
            0x54 => Instruction::I64LtUnsigned,
            0x55 => Instruction::I64GtSigned,
            0x56 => Instruction::I64GtUnsigned,
            0x57 => Instruction::I64LeSigned,
            0x58 => Instruction::I64LeUnsigned,
            0x59 => Instruction::I64GeSigned,
            0x5A => Instruction::I64GeUnsigned,
            0x5B => Instruction::F32Eq,
            0x5C => Instruction::F32Ne,
            0x5D => Instruction::F32Lt,
            0x5E => Instruction::F32Gt,
            0x5F => Instruction::F32Le,
            0x60 => Instruction::F32Ge,
            0x61 => Instruction::F64Eq,
            0x62 => Instruction::F64Ne,
            0x63 => Instruction::F64Lt,
            0x64 => Instruction::F64Gt,
            0x65 => Instruction::F64Le,
            0x66 => Instruction::F64Ge,
            0x67 => Instruction::I32CountLeadingZeros,
            0x68 => Instruction::I32CountTrailingZeros,
            0x69 => Instruction::I32PopCount,
            0x6A => Instruction::I32Add,
            0x6B => Instruction::I32Sub,
            0x6C => Instruction::I32Mul,
            0x6D => Instruction::I32DivSigned,
            0x6E => Instruction::I32DivUnsigned,
            0x6F => Instruction::I32RemainderSigned,
            0x70 => Instruction::I32RemainderUnsigned,
            0x71 => Instruction::I32And,
            0x72 => Instruction::I32Or,
            0x73 => Instruction::I32Xor,
            0x74 => Instruction::I32Shl,
            0x75 => Instruction::I32ShrSigned,
            0x76 => Instruction::I32ShrUnsigned,
            0x77 => Instruction::I32RotateLeft,
            0x78 => Instruction::I32RotateRight,
            0x79 => Instruction::I64CountLeadingZeros,
            0x7A => Instruction::I64CountTrailingZeros,
            0x7B => Instruction::I64PopCount,
            0x7C => Instruction::I64Add,
            0x7D => Instruction::I64Sub,
            0x7E => Instruction::I64Mul,
            0x7F => Instruction::I64DivSigned,
            0x80 => Instruction::I64DivUnsigned,
            0x81 => Instruction::I64RemainderSigned,
            0x82 => Instruction::I64RemainderUnsigned,
            0x83 => Instruction::I64And,
            0x84 => Instruction::I64Or,
            0x85 => Instruction::I64Xor,
            0x86 => Instruction::I64Shl,
            0x87 => Instruction::I64ShrSigned,
            0x88 => Instruction::I64ShrUnsigned,
            0x89 => Instruction::I64RotateLeft,
            0x8A => Instruction::I64RotateRight,
            0x8B => Instruction::F32Abs,
            0x8C => Instruction::F32Neg,
            0x8D => Instruction::F32Ceil,
            0x8E => Instruction::F32Floor,
            0x8F => Instruction::F32Trunc,
            0x90 => Instruction::F32Nearest,
            0x91 => Instruction::F32Sqrt,
            0x92 => Instruction::F32Add,
            0x93 => Instruction::F32Sub,
            0x94 => Instruction::F32Mul,
            0x95 => Instruction::F32Div,
            0x96 => Instruction::F32Min,
            0x97 => Instruction::F32Max,
            0x98 => Instruction::F32CopySign,
            0x99 => Instruction::F64Abs,
            0x9A => Instruction::F64Neg,
            0x9B => Instruction::F64Ceil,
            0x9C => Instruction::F64Floor,
            0x9D => Instruction::F64Trunc,
            0x9E => Instruction::F64Nearest,
            0x9F => Instruction::F64Sqrt,
            0xA0 => Instruction::F64Add,
            0xA1 => Instruction::F64Sub,
            0xA2 => Instruction::F64Mul,
            0xA3 => Instruction::F64Div,
            0xA4 => Instruction::F64Min,
            0xA5 => Instruction::F64Max,
            0xA6 => Instruction::F64CopySign,
            0xA7 => Instruction::I32WrapI64,
            0xA8 => Instruction::I32TruncF32Signed,
            0xA9 => Instruction::I32TruncF32Unsigned,
            0xAA => Instruction::I32TruncF64Signed,
            0xAB => Instruction::I32TruncF64Unsigned,
            0xAC => Instruction::I64ExtendI32Signed,
            0xAD => Instruction::I64ExtendI32Unsigned,
            0xAE => Instruction::I64TruncF32Signed,
            0xAF => Instruction::I64TruncF32Unsigned,
            0xB0 => Instruction::I64TruncF64Signed,
            0xB1 => Instruction::I64TruncF64Unsigned,
            0xB2 => Instruction::F32ConvertI32Signed,
            0xB3 => Instruction::F32ConvertI32Unsigned,
            0xB4 => Instruction::F32ConvertI64Signed,
            0xB5 => Instruction::F32ConvertI64Unsigned,
            0xB6 => Instruction::F32DemoteF64,
            0xB7 => Instruction::F64ConvertI32Signed,
            0xB8 => Instruction::F64ConvertI32Unsigned,
            0xB9 => Instruction::F64ConvertI64Signed,
            0xBA => Instruction::F64ConvertI64Unsigned,
            0xBB => Instruction::F64PromoteF32,
            0xBC => Instruction::I32ReinterpretF32,
            0xBD => Instruction::I64ReinterpretF64,
            0xBE => Instruction::F32ReinterpretI32,
            0xBF => Instruction::F64ReinterpretI64,
            0xC0 => Instruction::I32Extend8Signed,
            0xC1 => Instruction::I32Extend16Signed,
            0xC2 => Instruction::I64Extend8Signed,
            0xC3 => Instruction::I64Extend16Signed,
            0xC4 => Instruction::I64Extend32Signed,

            0xFD => match self.read_u32()? {
                0x00 => Instruction::V128Load(self.parse_memarg()?),
                0x01 => Instruction::V128Load8x8Signed(self.parse_memarg()?),
                0x02 => Instruction::V128Load8x8Unsigned(self.parse_memarg()?),
                0x03 => Instruction::V128Load16x4Signed(self.parse_memarg()?),
                0x04 => Instruction::V128Load16x4Unsigned(self.parse_memarg()?),
                0x05 => Instruction::V128Load32x2Signed(self.parse_memarg()?),
                0x06 => Instruction::V128Load32x2Unsigned(self.parse_memarg()?),
                0x07 => Instruction::V128Load8Splat(self.parse_memarg()?),
                0x08 => Instruction::V128Load16Splat(self.parse_memarg()?),
                0x09 => Instruction::V128Load32Splat(self.parse_memarg()?),
                0x0A => Instruction::V128Load64Splat(self.parse_memarg()?),
                0x0B => Instruction::V128Store(self.parse_memarg()?),
                0x0C => {
                    let bytes = self.read_slice(16)?;
                    Instruction::V128Const(i128::from_le_bytes(bytes.try_into()?))
                }
                0x0D => {
                    let mut lanes = [0u8; 16];
                    lanes.copy_from_slice(self.read_slice(16)?);
                    Instruction::I8x16Shuffle(lanes)
                }
                0x0E => Instruction::I8x16Swizzle,
                0x0F => Instruction::I8x16Splat,
                0x10 => Instruction::I16x8Splat,
                0x11 => Instruction::I32x4Splat,
                0x12 => Instruction::I64x2Splat,
                0x13 => Instruction::F32x4Splat,
                0x14 => Instruction::F64x2Splat,
                0x15 => Instruction::I8x16ExtractLaneSigned(self.read_u8()?),
                0x16 => Instruction::I8x16ExtractLaneUnsigned(self.read_u8()?),
                0x17 => Instruction::I8x16ReplaceLane(self.read_u8()?),
                0x18 => Instruction::I16x8ExtractLaneSigned(self.read_u8()?),
                0x19 => Instruction::I16x8ExtractLaneUnsigned(self.read_u8()?),
                0x1A => Instruction::I16x8ReplaceLane(self.read_u8()?),
                0x1B => Instruction::I32x4ExtractLane(self.read_u8()?),
                0x1C => Instruction::I32x4ReplaceLane(self.read_u8()?),
                0x1D => Instruction::I64x2ExtractLane(self.read_u8()?),
                0x1E => Instruction::I64x2ReplaceLane(self.read_u8()?),
                0x1F => Instruction::F32x4ExtractLane(self.read_u8()?),
                0x20 => Instruction::F32x4ReplaceLane(self.read_u8()?),
                0x21 => Instruction::F64x2ExtractLane(self.read_u8()?),
                0x22 => Instruction::F64x2ReplaceLane(self.read_u8()?),
                0x23 => Instruction::I8x16Eq,
                0x24 => Instruction::I8x16Ne,
                0x25 => Instruction::I8x16LtSigned,
                0x26 => Instruction::I8x16LtUnsigned,
                0x27 => Instruction::I8x16GtSigned,
                0x28 => Instruction::I8x16GtUnsigned,
                0x29 => Instruction::I8x16LeSigned,
                0x2A => Instruction::I8x16LeUnsigned,
                0x2B => Instruction::I8x16GeSigned,
                0x2C => Instruction::I8x16GeUnsigned,
                0x2D => Instruction::I16x8Eq,
                0x2E => Instruction::I16x8Ne,
                0x2F => Instruction::I16x8LtSigned,
                0x30 => Instruction::I16x8LtUnsigned,
                0x31 => Instruction::I16x8GtSigned,
                0x32 => Instruction::I16x8GtUnsigned,
                0x33 => Instruction::I16x8LeSigned,
                0x34 => Instruction::I16x8LeUnsigned,
                0x35 => Instruction::I16x8GeSigned,
                0x36 => Instruction::I16x8GeUnsigned,
                0x37 => Instruction::I32x4Eq,
                0x38 => Instruction::I32x4Ne,
                0x39 => Instruction::I32x4LtSigned,
                0x3A => Instruction::I32x4LtUnsigned,
                0x3B => Instruction::I32x4GtSigned,
                0x3C => Instruction::I32x4GtUnsigned,
                0x3D => Instruction::I32x4LeSigned,
                0x3E => Instruction::I32x4LeUnsigned,
                0x3F => Instruction::I32x4GeSigned,
                0x40 => Instruction::I32x4GeUnsigned,
                0x41 => Instruction::F32X4Eq,
                0x42 => Instruction::F32x4Ne,
                0x43 => Instruction::F32x4Lt,
                0x44 => Instruction::F32x4Gt,
                0x45 => Instruction::F32x4Le,
                0x46 => Instruction::F32x4Ge,
                0x47 => Instruction::F64x2Eq,
                0x48 => Instruction::F64x2Ne,
                0x49 => Instruction::F64x2Lt,
                0x4A => Instruction::F64x2Gt,
                0x4B => Instruction::F64x2Le,
                0x4C => Instruction::F64x2Ge,
                0x4D => Instruction::V128Not,
                0x4E => Instruction::V128And,
                0x4F => Instruction::V128AndNot,
                0x50 => Instruction::V128Or,
                0x51 => Instruction::V128Xor,
                0x52 => Instruction::V128BitSelect,
                0x53 => Instruction::V128AnyTrue,
                0x54 => {
                    let memarg = self.parse_memarg()?;
                    let lane = self.read_u8()?;
                    Instruction::V128Load8Lane(memarg, lane)
                }
                0x55 => {
                    let memarg = self.parse_memarg()?;
                    let lane = self.read_u8()?;
                    Instruction::V128Load16Lane(memarg, lane)
                }
                0x56 => {
                    let memarg = self.parse_memarg()?;
                    let lane = self.read_u8()?;
                    Instruction::V128Load32Lane(memarg, lane)
                }
                0x57 => {
                    let memarg = self.parse_memarg()?;
                    let lane = self.read_u8()?;
                    Instruction::V128Load64Lane(memarg, lane)
                }
                0x58 => {
                    let memarg = self.parse_memarg()?;
                    let lane = self.read_u8()?;
                    Instruction::V128Store8Lane(memarg, lane)
                }
                0x59 => {
                    let memarg = self.parse_memarg()?;
                    let lane = self.read_u8()?;
                    Instruction::V128Store16Lane(memarg, lane)
                }
                0x5A => {
                    let memarg = self.parse_memarg()?;
                    let lane = self.read_u8()?;
                    Instruction::V128Store32Lane(memarg, lane)
                }
                0x5B => {
                    let memarg = self.parse_memarg()?;
                    let lane = self.read_u8()?;
                    Instruction::V128Store64Lane(memarg, lane)
                }
                0x5C => Instruction::V128Load32Zero(self.parse_memarg()?),
                0x5D => Instruction::V128Load64Zero(self.parse_memarg()?),
                0x5E => Instruction::F32x4DemoteF64x2Zero,
                0x5F => Instruction::F64xPromoteLowF32x4,
                0x60 => Instruction::I8x16Abs,
                0x61 => Instruction::I8x16Neg,
                0x62 => Instruction::I8x16PopCount,
                0x63 => Instruction::I8x16AllTrue,
                0x64 => Instruction::I8x16BitMask,
                0x65 => Instruction::I8x16NarrowI16x8Signed,
                0x66 => Instruction::I8x16NarrowI16x8Unsigned,
                0x67 => Instruction::F32x4Ceil,
                0x68 => Instruction::F32x4Floor,
                0x69 => Instruction::F32x4Trunc,
                0x6A => Instruction::F32x4Nearest,
                0x6B => Instruction::I8x16Shl,
                0x6C => Instruction::I8x16ShrSigned,
                0x6D => Instruction::I8x16ShrUnsigned,
                0x6E => Instruction::I8x16Add,
                0x6F => Instruction::I8x16AddSaturatedSigned,
                0x70 => Instruction::I8x16AddSaturatedUnsigned,
                0x71 => Instruction::I8x16Sub,
                0x72 => Instruction::I8x16SubSaturatedSigned,
                0x73 => Instruction::I8x16SubSaturatedUnsigned,
                0x74 => Instruction::F64x2Ceil,
                0x75 => Instruction::F64x2Floor,
                0x76 => Instruction::I8x16MinSigned,
                0x77 => Instruction::I8x16MinUnsigned,
                0x78 => Instruction::I8x16MaxSigned,
                0x79 => Instruction::I8x16MaxUnsigned,
                0x7A => Instruction::F64x2Trunc,
                0x7B => Instruction::I8x16AvgRangeUnsigned,
                0x7C => Instruction::I16x8ExtAddPairWiseI8x16Signed,
                0x7D => Instruction::I16x8ExtAddPairWiseI8x16Unsigned,
                0x7E => Instruction::I32x4ExtAddPairWiseI16x8Signed,
                0x7F => Instruction::I32x4ExtAddPairWiseI16x8Unsigned,
                128 => Instruction::I16x8Abs,
                129 => Instruction::I16x8Neg,
                130 => Instruction::I16xQ15MulRangeSaturatedSigned,
                131 => Instruction::I16x8AllTrue,
                132 => Instruction::I16x8BitMask,
                133 => Instruction::I16x8NarrowI32x4Signed,
                134 => Instruction::I16x8NarrowI32x4Unsigned,
                135 => Instruction::I16x8ExtendLowI8x16Signed,
                136 => Instruction::I16x8ExtendHighI8x16Signed,
                137 => Instruction::I16x8ExtendLowI8x16Unsigned,
                138 => Instruction::I16x8ExtendHighI8x16Unsigned,
                139 => Instruction::I16x8Shl,
                140 => Instruction::I16x8ShrSigned,
                141 => Instruction::I16x8ShrUnsigned,
                142 => Instruction::I16x8Add,
                143 => Instruction::I16x8AddSaturatedSigned,
                144 => Instruction::I16x8AddSaturatedUnsigned,
                145 => Instruction::I16x8Sub,
                146 => Instruction::I16x8SubSaturatedSigned,
                147 => Instruction::I16x8SubSaturatedUnsigned,
                148 => Instruction::F64x2Nearest,
                149 => Instruction::I16x8Mul,
                150 => Instruction::I16x8MinSigned,
                151 => Instruction::I16x8MinUnsigned,
                152 => Instruction::I16x8MaxSigned,
                153 => Instruction::I16x8MaxUnsigned,
                155 => Instruction::I16x8AvgRangeUnsigned,
                156 => Instruction::I16x8ExtMulLowI8x16Signed,
                157 => Instruction::I16x8ExtMulHighI8x16Signed,
                158 => Instruction::I16x8ExtMulLowI8x16Unsigned,
                159 => Instruction::I16x8ExtMulHighI8x16Unsigned,
                160 => Instruction::I32x4Abs,
                161 => Instruction::I32x4Neg,
                163 => Instruction::I32x4AllTrue,
                164 => Instruction::I32x4BitMask,
                167 => Instruction::I32x4ExtendLowI16x8Signed,
                168 => Instruction::I32x4ExtendHighI16x8Signed,
                169 => Instruction::I32x4ExtendLowI16x8Unsigned,
                170 => Instruction::I32x4ExtendHighI16x8Unsigned,
                171 => Instruction::I32x4Shl,
                172 => Instruction::I32x4ShrSigned,
                173 => Instruction::I32x4ShrUnsigned,
                174 => Instruction::I32x4Add,
                177 => Instruction::I32x4Sub,
                181 => Instruction::I32x4Mul,
                182 => Instruction::I32x4MinSigned,
                183 => Instruction::I32x4MinUnsigned,
                184 => Instruction::I32x4MaxSigned,
                185 => Instruction::I32x4MaxUnsigned,
                186 => Instruction::I32x4DotI16x8Signed,
                188 => Instruction::I32x4ExtMulLowI16x8Signed,
                189 => Instruction::I32x4ExtMulHighI16x8Signed,
                190 => Instruction::I32x4ExtMulLowI16x8Unsigned,
                191 => Instruction::I32x4ExtMulHighI16x8Unsigned,
                192 => Instruction::I64x2Abs,
                193 => Instruction::I64x2Neg,
                195 => Instruction::I64x2AllTrue,
                196 => Instruction::I64x2BitMask,
                199 => Instruction::I64x2ExtendLowI32x4Signed,
                200 => Instruction::I64x2ExtendHighI32x4Signed,
                201 => Instruction::I64x2ExtendLowI32x4Unsigned,
                202 => Instruction::I64x2ExtendHighI32x4Unsigned,
                203 => Instruction::I64x2Shl,
                204 => Instruction::I64x2ShrSigned,
                205 => Instruction::I64x2ShrUnsigned,
                206 => Instruction::I64x2Add,
                209 => Instruction::I64x2Sub,
                213 => Instruction::I64x2Mul,
                220 => Instruction::I64x2ExtMulLowI32x4Signed,
                221 => Instruction::I64x2ExtMulHighI32x4Signed,
                222 => Instruction::I64x2ExtMulLowI32x4Unsigned,
                223 => Instruction::I64x2ExtMulHighI32x4Unsigned,
                224 => Instruction::F32x4Abs,
                225 => Instruction::F32x4Neg,
                227 => Instruction::F32x4Sqrt,
                228 => Instruction::F32x4Add,
                229 => Instruction::F32x4Sub,
                230 => Instruction::F32x4Mul,
                231 => Instruction::F32x4Div,
                232 => Instruction::F32x4Min,
                233 => Instruction::F32x4Max,
                234 => Instruction::F32x4PMin,
                235 => Instruction::F32x4PMax,
                236 => Instruction::F64x2Abs,
                237 => Instruction::F64x2Neg,
                239 => Instruction::F64x2Sqrt,
                240 => Instruction::F64x2Add,
                241 => Instruction::F64x2Sub,
                242 => Instruction::F64x2Mul,
                243 => Instruction::F64x2Div,
                244 => Instruction::F64x2Min,
                245 => Instruction::F64x2Max,
                246 => Instruction::F64x2PMin,
                247 => Instruction::F64x2PMax,
                248 => Instruction::I32x4TruncSaturatedF32x4Signed,
                249 => Instruction::I32x4TruncSaturatedF32x4Unsigned,
                250 => Instruction::F32x4ConvertI32x4Signed,
                251 => Instruction::F32x4ConvertI32x4Unsigned,
                252 => Instruction::I32x4TruncSaturatedF64x2SignedZero,
                253 => Instruction::I32x4TruncSaturatedF64x2UnsignedZero,
                254 => Instruction::F64x2ConvertLowI32x4Signed,
                255 => Instruction::F64x2ConvertLowI32x4Unsigned,
                // i64x2 comparisons
                214 => Instruction::I64x2Eq,
                215 => Instruction::I64x2Ne,
                216 => Instruction::I64x2LtSigned,
                217 => Instruction::I64x2GtSigned,
                218 => Instruction::I64x2LeSigned,
                219 => Instruction::I64x2GeSigned,
                // Relaxed SIMD (0x100+)
                0x100 => Instruction::I8x16RelaxedSwizzle,
                0x101 => Instruction::I32x4RelaxedTruncF32x4Signed,
                0x102 => Instruction::I32x4RelaxedTruncF32x4Unsigned,
                0x103 => Instruction::I32x4RelaxedTruncF64x2SignedZero,
                0x104 => Instruction::I32x4RelaxedTruncF64x2UnsignedZero,
                0x105 => Instruction::F32x4RelaxedMadd,
                0x106 => Instruction::F32x4RelaxedNmadd,
                0x107 => Instruction::F64x2RelaxedMadd,
                0x108 => Instruction::F64x2RelaxedNmadd,
                0x109 => Instruction::I8x16RelaxedLaneselect,
                0x10A => Instruction::I16x8RelaxedLaneselect,
                0x10B => Instruction::I32x4RelaxedLaneselect,
                0x10C => Instruction::I64x2RelaxedLaneselect,
                0x10D => Instruction::F32x4RelaxedMin,
                0x10E => Instruction::F32x4RelaxedMax,
                0x10F => Instruction::F64x2RelaxedMin,
                0x110 => Instruction::F64x2RelaxedMax,
                0x111 => Instruction::I16x8RelaxedQ15mulrSigned,
                0x112 => Instruction::I16x8RelaxedDotI8x16I7x16Signed,
                0x113 => Instruction::I32x4RelaxedDotI8x16I7x16AddSigned,
                foreign => bail!("Encountered unknown SIMD opcode: 0xFD 0x{:X}", foreign),
            },
            foreign => bail!("Encountered unknown opcode: {}", foreign),
        };

        Ok(instr)
    }

    // 5.5: Modules

    fn parse_custom_section(&mut self, size: u32) -> Result<CustomSection> {
        let current_pos = self.cursor;

        let name = self.parse_name()?;
        let slice_len = size as usize - (self.cursor - current_pos);

        let bytes = self.read_slice(slice_len)?.to_vec();

        Ok(CustomSection { name, bytes })
    }

    fn parse_type_section(&mut self) -> Result<TypeSection> {
        let rec_types = self.parse_vec(Self::parse_rec_type)?;
        Ok(TypeSection {
            types: rec_types.into_iter().flatten().collect(),
        })
    }

    fn parse_import(&mut self) -> Result<ImportDeclaration> {
        Ok(ImportDeclaration {
            module: self.parse_name()?,
            name: self.parse_name()?,
            description: match self.read_u8()? {
                0x00 => ImportDescription::Func(self.read_u32()?),
                0x01 => ImportDescription::Table(self.parse_table_type()?),
                0x02 => ImportDescription::Mem(self.parse_memory_type()?),
                0x03 => ImportDescription::Global(self.parse_global_type()?),
                0x04 => {
                    let _attribute = self.read_u8()?; // tag attribute (0x00)
                    ImportDescription::Tag(self.read_u32()?)
                }
                foreign => bail!(
                    "Unrecognized import description. Got: {}, at: {}",
                    foreign,
                    self.cursor
                ),
            },
        })
    }

    fn parse_import_section(&mut self) -> Result<ImportSection> {
        let imports = self.parse_vec(Self::parse_import)?;

        Ok(ImportSection {
            import_declarations: imports,
        })
    }

    fn parse_function_section(&mut self) -> Result<FunctionSection> {
        Ok(FunctionSection {
            indices: self.parse_vec(Self::read_u32)?,
        })
    }

    fn parse_table_def(&mut self) -> Result<TableDef> {
        let byte = self.buffer[self.cursor];
        if byte == 0x40 {
            // table with init expression: 0x40 0x00 reftype limit expr
            self.cursor += 1;
            ensure!(
                self.read_u8()? == 0x00,
                "Expected 0x00 after 0x40 in table definition"
            );
            let table_type = self.parse_table_type()?;
            let init = self.parse_expression()?;
            Ok(TableDef {
                table_type,
                init,
            })
        } else {
            let table_type = self.parse_table_type()?;
            let ht = match table_type.element_reference_type {
                RefType::FuncRef => HeapType::Func,
                RefType::ExternRef => HeapType::Extern,
                RefType::Ref { heap_type, .. } => heap_type,
            };
            Ok(TableDef {
                table_type,
                init: vec![Instruction::RefNull(ht)],
            })
        }
    }

    fn parse_table_section(&mut self) -> Result<TableSection> {
        Ok(TableSection {
            tables: self.parse_vec(Self::parse_table_def)?,
        })
    }

    fn parse_memory_section(&mut self) -> Result<MemorySection> {
        Ok(MemorySection {
            memories: self.parse_vec(Self::parse_memory_type)?,
        })
    }

    fn parse_global_section(&mut self) -> Result<GlobalSection> {
        Ok(GlobalSection {
            globals: self.parse_vec(Self::parse_global)?,
        })
    }

    fn parse_tag(&mut self) -> Result<Tag> {
        ensure!(
            self.read_u8()? == 0x00,
            "Expected 0x00 attribute byte for tag."
        );
        Ok(Tag {
            type_index: self.read_u32()?,
        })
    }

    fn parse_tag_section(&mut self) -> Result<TagSection> {
        Ok(TagSection {
            tags: self.parse_vec(Self::parse_tag)?,
        })
    }

    fn parse_export(&mut self) -> Result<Export> {
        Ok(Export {
            name: self.parse_name()?,
            description: match self.read_u8()? {
                0x00 => ExportDescription::Func(self.read_u32()?),
                0x01 => ExportDescription::Table(self.read_u32()?),
                0x02 => ExportDescription::Mem(self.read_u32()?),
                0x03 => ExportDescription::Global(self.read_u32()?),
                0x04 => ExportDescription::Tag(self.read_u32()?),
                foreign => bail!(
                    "Encountered foreign byte when parsing export description. Got: {}",
                    foreign
                ),
            },
        })
    }

    fn parse_export_section(&mut self) -> Result<ExportSection> {
        Ok(ExportSection {
            exports: self.parse_vec(Self::parse_export)?,
        })
    }

    fn parse_element_segement(&mut self) -> Result<ElementSegment> {
        let segment = match self.read_u32()? {
            0 => {
                let offset = self.parse_expression()?;
                let expression = self
                    .parse_vec(Self::read_u32)?
                    .into_iter()
                    .map(|idx| vec![Instruction::RefFunc(idx)])
                    .collect::<Vec<_>>();

                ElementSegment {
                    ref_type: RefType::FuncRef,
                    expression,
                    mode: ElementMode::Active {
                        table_index: 0,
                        offset,
                    },
                }
            }
            1 => {
                ensure!(self.read_u8()? == 0x00, "Expected elemkind 0x00.");

                let expression = self
                    .parse_vec(Self::read_u32)?
                    .into_iter()
                    .map(|idx| vec![Instruction::RefFunc(idx)])
                    .collect::<Vec<_>>();

                ElementSegment {
                    ref_type: RefType::FuncRef,
                    expression,
                    mode: ElementMode::Passive,
                }
            }
            2 => {
                let table_index = self.read_u32()?;
                let offset = self.parse_expression()?;
                ensure!(self.read_u8()? == 0x00, "Expected elemkind 0x00.");

                let expression = self
                    .parse_vec(Self::read_u32)?
                    .into_iter()
                    .map(|idx| vec![Instruction::RefFunc(idx)])
                    .collect::<Vec<_>>();

                ElementSegment {
                    ref_type: RefType::FuncRef,
                    expression,
                    mode: ElementMode::Active {
                        table_index,
                        offset,
                    },
                }
            }
            3 => {
                ensure!(self.read_u8()? == 0x00, "Expected elemkind 0x00.");

                let expression = self
                    .parse_vec(Self::read_u32)?
                    .into_iter()
                    .map(|idx| vec![Instruction::RefFunc(idx)])
                    .collect::<Vec<_>>();

                ElementSegment {
                    ref_type: RefType::FuncRef,
                    expression,
                    mode: ElementMode::Declarative,
                }
            }
            4 => {
                let offset = self.parse_expression()?;
                let expression = self.parse_vec(Self::parse_expression)?;

                ElementSegment {
                    ref_type: RefType::FuncRef,
                    expression,
                    mode: ElementMode::Active {
                        table_index: 0,
                        offset,
                    },
                }
            }
            5 => ElementSegment {
                ref_type: self.parse_reference_type()?,
                expression: self.parse_vec(Self::parse_expression)?,
                mode: ElementMode::Passive,
            },
            6 => {
                let table_index = self.read_u32()?;
                let offset = self.parse_expression()?;
                let ref_type = self.parse_reference_type()?;
                let expression = self.parse_vec(Self::parse_expression)?;

                ElementSegment {
                    ref_type,
                    expression,
                    mode: ElementMode::Active {
                        table_index,
                        offset,
                    },
                }
            }
            7 => ElementSegment {
                ref_type: self.parse_reference_type()?,
                expression: self.parse_vec(Self::parse_expression)?,
                mode: ElementMode::Declarative,
            },
            foreign => bail!("Encountered foreign element segement kind: {}", foreign),
        };

        Ok(segment)
    }

    fn parse_element_section(&mut self) -> Result<ElementSection> {
        Ok(ElementSection {
            elements: self.parse_vec(Self::parse_element_segement)?,
        })
    }

    fn parse_local(&mut self) -> Result<Local> {
        Ok(Local {
            count: self.read_u32()?,
            value_type: self.parse_value_type()?,
        })
    }

    fn parse_code(&mut self) -> Result<Function> {
        let size = self.read_u32()?;
        let start = self.cursor;

        let type_index = self
            .function_types
            .pop_front()
            .ok_or_else(|| anyhow!("Function type list empty"))?;

        let func = Function {
            type_index,
            locals: self.parse_vec(Self::parse_local)?,
            body: self.parse_expression()?,
        };

        let consumed = self.cursor - start;
        if consumed != size as usize {
            bail!(
                "parse_code: expected {} bytes but consumed {} (type_index={}, start=0x{:x})",
                size,
                consumed,
                type_index,
                start
            );
        }

        Ok(func)
    }

    fn parse_code_section(&mut self) -> Result<CodeSection> {
        Ok(CodeSection {
            codes: self.parse_vec(Self::parse_code)?,
        })
    }

    fn parse_data_segment(&mut self) -> Result<DataSegment> {
        let segment = match self.read_u32()? {
            0 => {
                let offset = self.parse_expression()?;

                let len = self.read_u32()? as usize;
                let bytes = self.read_slice(len)?.to_vec();

                DataSegment {
                    bytes,
                    mode: DataMode::Active { memory: 0, offset },
                }
            }
            1 => DataSegment {
                bytes: {
                    let len = self.read_u32()? as usize;
                    self.read_slice(len)?.to_vec()
                },
                mode: DataMode::Passive,
            },
            2 => {
                let memory = self.read_u32()?;
                let offset = self.parse_expression()?;
                let len = self.read_u32()? as usize;
                let bytes = self.read_slice(len)?.to_vec();

                DataSegment {
                    bytes,
                    mode: DataMode::Active { memory, offset },
                }
            }
            foreign => bail!("Encountered foreign data kind. Got: {}", foreign),
        };

        Ok(segment)
    }

    fn parse_data_section(&mut self) -> Result<DataSection> {
        Ok(DataSection {
            data_segments: self.parse_vec(Self::parse_data_segment)?,
        })
    }

    fn parse_section(&mut self, id: u8) -> Result<Section> {
        use crate::binary_grammar::section_id::*;

        let size = self.read_u32()?;

        let section = match id {
            CUSTOM_ID => Section::Custom(self.parse_custom_section(size)?),
            TYPE_ID => Section::Type(self.parse_type_section()?),
            IMPORT_ID => Section::Import(self.parse_import_section()?),
            FUNCTION_ID => Section::Function(self.parse_function_section()?),
            TABLE_ID => Section::Table(self.parse_table_section()?),
            MEMORY_ID => Section::Memory(self.parse_memory_section()?),
            GLOBAL_ID => Section::Global(self.parse_global_section()?),
            EXPORT_ID => Section::Export(self.parse_export_section()?),
            START_ID => Section::Start(self.read_u32()?),
            ELEMENT_ID => Section::Element(self.parse_element_section()?),
            CODE_ID => Section::Code(self.parse_code_section()?),
            DATA_ID => Section::Data(self.parse_data_section()?),
            DATA_COUNT_ID => Section::DataCount(self.read_u32()?),
            TAG_ID => Section::Tag(self.parse_tag_section()?),
            foreign_id => bail!("Encountered foreign section id: {}", foreign_id),
        };

        Ok(section)
    }
}
