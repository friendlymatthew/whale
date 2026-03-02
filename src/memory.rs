macro_rules! mem_load {
    ($self:expr, $depth:expr, $mem_arg:expr, $width:literal, |$bytes:ident| $convert:expr) => {{
        let frame_module = &$self.call_stack[$depth].frame.module;
        let mem_addr = frame_module.mem_addrs[$mem_arg.memory as usize];
        let mem = &$self.store.memories[mem_addr];

        let base: u64 = match (mem.memory_type.addr_type, $self.stack.pop_value()?) {
            (AddrType::I32, Value::I32(v)) => v as u64,
            (AddrType::I64, Value::I64(v)) => v as u64,
            (addr_type, val) => bail!("expected {addr_type:?} address, got: {val:?}"),
        };

        let ea = base.checked_add($mem_arg.offset).and_then(|v| usize::try_from(v).ok());
        let Some(ea) = ea.filter(|&ea| ea.saturating_add($width) <= mem.data.len()) else {
            bail!("trap: out of bounds memory access");
        };

        let $bytes: [u8; $width] = mem.data[ea..ea + $width].try_into().unwrap();
        let val = $convert;
        $self.stack.push(val);
    }};
}

macro_rules! mem_store {
    ($self:expr, $depth:expr, $mem_arg:expr, $width:literal, |$val:ident| $to_bytes:expr) => {{
        let $val = $self.stack.pop_value()?;

        let frame_module = &$self.call_stack[$depth].frame.module;
        let mem_addr = frame_module.mem_addrs[$mem_arg.memory as usize];
        let mem = &mut $self.store.memories[mem_addr];

        let base: u64 = match (mem.memory_type.addr_type, $self.stack.pop_value()?) {
            (AddrType::I32, Value::I32(v)) => v as u64,
            (AddrType::I64, Value::I64(v)) => v as u64,
            (addr_type, val) => bail!("expected {addr_type:?} address, got: {val:?}"),
        };

        let ea = base.checked_add($mem_arg.offset).and_then(|v| usize::try_from(v).ok());
        let Some(ea) = ea.filter(|&ea| ea.saturating_add($width) <= mem.data.len()) else {
            bail!("trap: out of bounds memory access");
        };

        let bytes: [u8; $width] = $to_bytes;
        mem.data[ea..ea + $width].copy_from_slice(&bytes);
    }};
}
