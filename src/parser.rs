use std::cmp::min;

use anyhow::{bail, ensure, Result};

use crate::grammar::{
    BlockType, CodeSection, CustomSection, DataMode, DataSection, DataSegment, ElementMode,
    ElementSection, ElementSegment, Export, ExportDescription, ExportSection, Expression, Function,
    FunctionSection, FunctionType, Global, GlobalSection, GlobalType, Import, ImportDescription,
    ImportSection, Instruction, Limit, Local, MAGIC_NUMBER, MemArg, MemorySection, MemoryType,
    Mutability, RefType, ResultType, Section, TableSection, TableType, TERM_ELSE_BYTE, TERM_END_BYTE,
    TypeSection, ValueType,
};
use crate::leb128::{MAX_LEB128_LEN_32, MAX_LEB128_LEN_64, read_i32, read_i64, read_u32};

#[derive(Debug)]
pub struct Parser<'a> {
    cursor: usize,
    buffer: &'a [u8],
}

impl<'a> Parser<'a> {
    pub const fn new(buffer: &'a [u8]) -> Self {
        Self { buffer, cursor: 0 }
    }

    /// Parses a .wasm file in its entirety.
    pub fn parse(&mut self) -> Result<()> {
        let _version = self.parse_preamble()?;

        while self.cursor < self.buffer.len() {
            let id = self.read_u8()?;

            let section = self.parse_section(id)?;
            dbg!(section);
        }

        Ok(())
    }

    fn parse_preamble(&mut self) -> Result<u16> {
        ensure!(
            self.read_slice(4)? == MAGIC_NUMBER,
            "Expected magic number in preamble."
        );

        ensure!(self.read_slice(4)? == [1, 0, 0, 0], "Expected version 1.");

        Ok(1)
    }

    fn eof(&self, bytes: usize) -> Result<()> {
        if bytes == 0 {
            ensure!(self.cursor < self.buffer.len(), "EOF.");
        } else {
            ensure!(self.cursor + bytes <= self.buffer.len(), "EOF.")
        }

        Ok(())
    }

    fn read_u8(&mut self) -> Result<u8> {
        self.eof(0)?;

        let b = self.buffer[self.cursor];
        self.cursor += 1;

        Ok(b)
    }

    fn read_u32(&mut self) -> Result<u32> {
        self.eof(0)?;

        let n = min(self.buffer.len() - self.cursor, MAX_LEB128_LEN_32);
        let (x, n) = read_u32(&self.buffer[self.cursor..self.cursor + n])?;

        self.cursor += n;

        Ok(x)
    }

    fn read_i32(&mut self) -> Result<i32> {
        self.eof(0)?;
        let n = min(self.buffer.len() - self.cursor, MAX_LEB128_LEN_32);
        let (x, n) = read_i32(&self.buffer[self.cursor..self.cursor + n])?;

        self.cursor += n;

        Ok(x)
    }

    fn read_i64(&mut self) -> Result<i64> {
        self.eof(0)?;
        let n = min(self.buffer.len() - self.cursor, MAX_LEB128_LEN_64);
        let (x, n) = read_i64(&self.buffer[self.cursor..self.cursor + n])?;

        self.cursor += n;

        Ok(x)
    }

    fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_le_bytes(self.read_slice(4)?.try_into()?))
    }

    fn read_f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.read_slice(8)?.try_into()?))
    }

    fn read_slice(&mut self, len: usize) -> Result<&'a [u8]> {
        self.eof(len)?;
        let slice = &self.buffer[self.cursor..self.cursor + len];
        self.cursor += len;

        Ok(slice)
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
    fn parse_name(&mut self) -> Result<&'a str> {
        let n = self.read_u32()?;
        let slice = self.read_slice(n as usize)?;

        Ok(std::str::from_utf8(slice)?)
    }

    // 5.3: Types
    fn parse_reference_type(&mut self) -> Result<RefType> {
        let r = match self.read_u8()? {
            0x70 => RefType::FuncRef,
            0x6F => RefType::ExternRef,
            foreign => bail!("Unrecognized reference byte. Got: {}", foreign),
        };

        Ok(r)
    }

    fn parse_value_type(&mut self) -> Result<ValueType> {
        let value_type = match self.read_u8()? {
            0x7F => ValueType::I32,
            0x7E => ValueType::I64,
            0x7D => ValueType::F32,
            0x7C => ValueType::F64,
            0x7B => ValueType::V128,
            0x70 => ValueType::Ref(RefType::FuncRef),
            0x6F => ValueType::Ref(RefType::ExternRef),
            foreign => bail!("Unrecognized type. Got: {}", foreign),
        };

        Ok(value_type)
    }

    fn parse_result_type(&mut self) -> Result<ResultType> {
        Ok(ResultType(self.parse_vec(Self::parse_value_type)?))
    }

    fn parse_function_type(&mut self) -> Result<FunctionType> {
        let b = self.read_u8()?;

        ensure!(
            b == 0x60,
            "Expected {}, got: {} at pos: {}",
            0x60,
            b,
            self.cursor
        );

        let arg_type = self.parse_result_type()?;
        let return_type = self.parse_result_type()?;

        Ok(FunctionType(arg_type, return_type))
    }

    fn parse_limit(&mut self) -> Result<Limit> {
        let flag = self.read_u8()?;

        Ok(Limit {
            min: match flag {
                0x00 | 0x01 => self.read_u32()?,
                _ => bail!("Expected flag to be either 0x00 or 0x01. Got: {}", flag),
            },
            max: if flag == 0x01 {
                self.read_u32()?
            } else {
                u32::MAX
            },
        })
    }

    fn parse_memory_type(&mut self) -> Result<MemoryType> {
        Ok(MemoryType(self.parse_limit()?))
    }

    fn parse_table_type(&mut self) -> Result<TableType> {
        Ok(TableType {
            element_reference_type: self.parse_reference_type()?,
            limit: self.parse_limit()?,
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
        let block_type = if self.buffer[self.cursor] == 0x40 {
            self.cursor += 1;
            BlockType::Empty
        } else if let Ok(value_type) = self.parse_value_type() {
            BlockType::SingleValue(value_type)
        } else {
            BlockType::TypeIndex(self.read_i32()?)
        };

        Ok(block_type)
    }

    fn parse_memarg(&mut self) -> Result<MemArg> {
        Ok(MemArg {
            align: self.read_u32()?,
            offset: self.read_u32()?,
        })
    }

    fn parse_expression(&mut self) -> Result<Expression> {
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
                let (if_exprs, else_exprs) = self.parse_if_else()?;
                Instruction::IfElse(self.parse_block_type()?, if_exprs, else_exprs)
            }
            0x0C => Instruction::Br(self.read_u32()?),
            0x0D => Instruction::BrIf(self.read_u32()?),
            0x0E => Instruction::BrTable(self.parse_vec(Self::read_u32)?, self.read_u32()?),
            0x0F => Instruction::Return,
            0x10 => Instruction::Call(self.read_u32()?),
            0x11 => Instruction::CallIndirect(self.read_u32()?, self.read_u32()?),
            0xD0 => Instruction::RefNull(self.parse_reference_type()?),
            0xD1 => Instruction::RefIsNull,
            0xD2 => Instruction::RefFunc(self.read_u32()?),
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
            0xFC => match self.read_u32()? {
                0 => Instruction::I32TruncSaturatedF32Signed,
                1 => Instruction::I32TruncSaturatedF32Unsigned,
                2 => Instruction::I32TruncSaturatedF64Signed,
                3 => Instruction::I32TruncSaturatedF64Unsigned,
                4 => Instruction::I64TruncSaturatedF32Signed,
                5 => Instruction::I64TruncSaturatedF32Unsigned,
                6 => Instruction::I64TruncSaturatedF64Signed,
                7 => Instruction::I64TruncSaturatedF64Unsigned,
                8 => {
                    let x = self.read_u32()?;

                    ensure!(
                        self.read_u8()? == 0x00,
                        "Expected 0x00 when parsing MemoryInit instr."
                    );

                    Instruction::MemoryInit(x)
                }
                9 => Instruction::DataDrop(self.read_u32()?),
                10 => {
                    ensure!(
                        self.read_u8()? == 0x00,
                        "Expected 0x00 when parsing MemCopy instr."
                    );
                    ensure!(
                        self.read_u8()? == 0x00,
                        "Expected 0x00 when parsing MemCopy instr."
                    );

                    Instruction::MemoryCopy
                }
                11 => {
                    ensure!(
                        self.read_u8()? == 0x00,
                        "Expected 0x00 when parsing MemFill instr."
                    );

                    Instruction::MemoryFill
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
            0x3F => match self.read_u8()? {
                0x00 => Instruction::MemorySize,
                foreign => bail!("Encountered foreign byte after 0x3F. Got: {}", foreign),
            },
            0x40 => match self.read_u8()? {
                0x00 => Instruction::MemoryGrow,
                foreign => bail!("Encountered foreign byte after 0x40. Got: {}", foreign),
            },
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

            0xFD => todo!("Implement the SIMD instructions"),
            foreign => bail!("Encountered unknown opcode: {}", foreign),
        };

        Ok(instr)
    }

    // 5.5: Modules

    fn parse_custom_section(&mut self, size: u32) -> Result<CustomSection> {
        let name = self.parse_name()?;
        let bytes = self.read_slice(size as usize - name.len())?;

        Ok(CustomSection { name, bytes })
    }

    fn parse_type_section(&mut self) -> Result<TypeSection> {
        Ok(TypeSection {
            function_types: self.parse_vec(Self::parse_function_type)?,
        })
    }

    fn parse_import(&mut self) -> Result<Import<'a>> {
        Ok(Import {
            module: self.parse_name()?,
            name: self.parse_name()?,
            description: match self.read_u8()? {
                0x00 => ImportDescription::Func(self.read_u32()?),
                0x01 => ImportDescription::Table(self.parse_table_type()?),
                0x02 => ImportDescription::Mem(self.parse_memory_type()?),
                0x03 => ImportDescription::Global(self.parse_global_type()?),
                foreign => bail!(
                    "Unrecognized import description. Got: {}, at: {}",
                    foreign,
                    self.cursor
                ),
            },
        })
    }

    fn parse_import_section(&mut self) -> Result<ImportSection<'a>> {
        let imports = self.parse_vec(Self::parse_import)?;

        Ok(ImportSection { imports })
    }

    fn parse_function_section(&mut self) -> Result<FunctionSection> {
        Ok(FunctionSection {
            indices: self.parse_vec(Self::read_u32)?,
        })
    }

    fn parse_table_section(&mut self) -> Result<TableSection> {
        Ok(TableSection {
            tables: self.parse_vec(Self::parse_table_type)?,
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

    fn parse_export(&mut self) -> Result<Export<'a>> {
        Ok(Export {
            name: self.parse_name()?,
            description: match self.read_u8()? {
                0x00 => ExportDescription::Func(self.read_u32()?),
                0x01 => ExportDescription::Table(self.read_u32()?),
                0x02 => ExportDescription::Mem(self.read_u32()?),
                0x03 => ExportDescription::Global(self.read_u32()?),
                foreign => bail!(
                    "Encountered foreign byte when parsing export description. Got: {}",
                    foreign
                ),
            },
        })
    }

    fn parse_export_section(&mut self) -> Result<ExportSection<'a>> {
        Ok(ExportSection {
            exports: self.parse_vec(Self::parse_export)?,
        })
    }

    fn parse_element_segement(&mut self) -> Result<ElementSegment> {
        let segment = match self.read_u32()? {
            0 => {
                let offset = self.parse_expression()?;
                let expression = vec![self
                    .parse_vec(Self::read_u32)?
                    .iter()
                    .map(|idx| Instruction::RefFunc(*idx))
                    .collect::<Vec<_>>()];

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

                let expression = vec![self
                    .parse_vec(Self::read_u32)?
                    .iter()
                    .map(|idx| Instruction::RefFunc(*idx))
                    .collect::<Vec<_>>()];

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

                let expression = vec![self
                    .parse_vec(Self::read_u32)?
                    .iter()
                    .map(|idx| Instruction::RefFunc(*idx))
                    .collect::<Vec<_>>()];

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

                let expression = vec![self
                    .parse_vec(Self::read_u32)?
                    .iter()
                    .map(|idx| Instruction::RefFunc(*idx))
                    .collect::<Vec<_>>()];

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
        let _size = self.read_u32()?;

        Ok(Function {
            locals: self.parse_vec(Self::parse_local)?,
            expression: self.parse_expression()?,
        })
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
                let bytes = self.parse_vec(Self::read_u8)?;
                DataSegment {
                    bytes,
                    mode: DataMode::Active { memory: 0, offset },
                }
            }
            1 => DataSegment {
                bytes: self.parse_vec(Self::read_u8)?,
                mode: DataMode::Passive,
            },
            2 => {
                let memory = self.read_u32()?;
                let offset = self.parse_expression()?;
                let bytes = self.parse_vec(Self::read_u8)?;

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
        use crate::grammar::section_id::*;

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
            START_ID => todo!("parse start section"),
            ELEMENT_ID => Section::Element(self.parse_element_section()?),
            CODE_ID => Section::Code(self.parse_code_section()?),
            DATA_ID => Section::Data(self.parse_data_section()?),
            DATA_COUNT_ID => Section::DataCount(self.read_u32()?),
            foreign_id => bail!("Encountered foreign section id: {}", foreign_id),
        };

        Ok(section)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(path: &str) -> Result<()> {
        let bytes = std::fs::read(path)?;

        let mut parser = Parser::new(&bytes);
        parser.parse()?;

        Ok(())
    }

    #[test]
    fn parse_hyper() -> Result<()> {
        parse("./tests/hyper-app-691811a8315a1230_bg.wasm")?;

        assert!(false);

        Ok(())
    }
}
