use std::ops::Range;

use anyhow::{anyhow, bail, ensure, Result};

use crate::binary_grammar::AddrType;
use crate::execution_grammar::Value;

/*
note: now that the compiler can keep track of stack height, we could probably track the
max stack height per function

then by the time we execute code, we can refactor the Value stack to skip the bounds check

safety:
    - pop never underflows
    wasm requires us to validate before executing. since validation proves every pop is balanced
    by a prior push and the compiler tracks stack height, if the compiler is correct, underflow
    should be impossible lol

    - push never overflows
    wasm guarantees every function's max stack depth is bounded and computed at validation time
    the compiler can probably record the max stack height per function
*/

pub struct ValueStack {
    inner: Vec<Value>,
}

impl ValueStack {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: Vec::with_capacity(cap),
        }
    }

    pub fn push<V: Into<Value>>(&mut self, val: V) {
        self.inner.push(val.into());
    }

    pub fn pop(&mut self) -> Result<Value> {
        self.inner.pop().ok_or_else(|| anyhow!("overflow"))
    }

    pub fn last(&self) -> Result<&Value> {
        self.inner.last().ok_or_else(|| anyhow!("overflow"))
    }

    pub const fn len(&self) -> usize {
        self.inner.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn clear(&mut self) {
        self.inner.clear();
    }

    pub fn extend(&mut self, iter: impl IntoIterator<Item = Value>) {
        self.inner.extend(iter);
    }

    pub fn truncate(&mut self, len: usize) {
        self.inner.truncate(len);
    }

    pub fn split_off(&mut self, at: usize) -> Vec<Value> {
        self.inner.split_off(at)
    }

    pub fn copy_within(&mut self, src: Range<usize>, dest: usize) {
        self.inner.copy_within(src, dest);
    }

    pub fn slice_from(&self, start: usize) -> &[Value] {
        &self.inner[start..]
    }

    pub fn pop_n(&mut self, n: usize) -> Result<Vec<Value>> {
        ensure!(self.inner.len() >= n, "stack underflow");
        let start = self.inner.len() - n;

        Ok(self.inner.split_off(start))
    }

    pub fn keep_top(&mut self, keep: usize, drop: usize) {
        if drop > 0 {
            let len = self.inner.len();
            assert!(
                len >= keep + drop,
                "compiler error: keep_top(keep={keep}, drop={drop}) but stack len={len}"
            );
            self.inner.copy_within(len - keep..len, len - keep - drop);
            self.inner.truncate(len - drop);
        }
    }

    pub fn pop_address(&mut self, addr_type: AddrType) -> Result<usize> {
        match (addr_type, self.pop()?) {
            (AddrType::I32, Value::I32(v)) => Ok(v as usize),
            (AddrType::I64, Value::I64(v)) => Ok(v as usize),
            (at, _) => bail!("expected {at:?} address"),
        }
    }

    pub fn push_address(&mut self, value: usize, addr_type: AddrType) {
        match addr_type {
            AddrType::I32 => self.inner.push(Value::I32(value as i32)),
            AddrType::I64 => self.inner.push(Value::I64(value as i64)),
        }
    }
}
