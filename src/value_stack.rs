use std::ops::Range;
use std::ptr;

use crate::binary_grammar::AddrType;
use crate::execution_grammar::RawValue;

/// the value stack operates without any bounds check
///
/// safety:
///     - wasm validation guarantees every instruction sequence is stack-safe
///     - we track the max stack height when we compile so we can never overflow
pub struct ValueStack {
    inner: Box<[RawValue]>,
    cursor: usize,
}

impl ValueStack {
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            inner: vec![RawValue::default(); cap].into_boxed_slice(),
            cursor: 0,
        }
    }

    pub fn push<V: Into<RawValue>>(&mut self, val: V) {
        *self.get_mut(self.cursor) = val.into();

        self.cursor += 1;
    }

    pub fn push_v128(&mut self, v: i128) {
        let (hi, lo) = RawValue::from_v128(v);
        self.extend_from_slice(&<[RawValue; 2]>::from((hi, lo)));
    }

    pub fn pop(&mut self) -> RawValue {
        self.cursor -= 1;
        *self.get(self.cursor)
    }

    pub fn last(&self) -> &RawValue {
        unsafe { self.inner.get_unchecked(self.cursor - 1) }
    }

    pub const fn len(&self) -> usize {
        self.cursor
    }

    pub const fn is_empty(&self) -> bool {
        self.cursor == 0
    }

    pub fn capacity(&self) -> usize {
        self.inner.len()
    }

    pub const fn clear(&mut self) {
        self.cursor = 0;
    }

    pub fn extend_from_slice(&mut self, slice: &[RawValue]) {
        unsafe {
            ptr::copy_nonoverlapping(
                slice.as_ptr(),
                self.inner.as_mut_ptr().add(self.cursor),
                slice.len(),
            );
        }
        self.cursor += slice.len();
    }

    pub const fn truncate(&mut self, len: usize) {
        self.cursor = len;
    }

    pub fn slice_from(&self, start: usize) -> &[RawValue] {
        self.get_slice(start..self.cursor)
    }

    pub fn pop_n(&mut self, n: usize) -> &[RawValue] {
        self.cursor -= n;
        self.get_slice(self.cursor..self.cursor + n)
    }

    pub fn keep_top(&mut self, keep: usize, drop: usize) {
        if drop > 0 {
            unsafe {
                let ptr = self.inner.as_mut_ptr();
                let src = ptr.add(self.cursor - keep);
                let dst = ptr.add(self.cursor - keep - drop);

                ptr::copy(src, dst, keep);
            }
            self.cursor -= drop;
        }
    }

    pub fn copy_within(&mut self, src: Range<usize>, dest: usize) {
        let count = src.end - src.start;
        unsafe {
            let ptr = self.inner.as_mut_ptr();
            ptr::copy(ptr.add(src.start), ptr.add(dest), count);
        }
    }

    pub fn pop_address(&mut self, addr_type: AddrType) -> usize {
        match addr_type {
            AddrType::I32 => self.pop().as_i32() as usize,
            AddrType::I64 => self.pop().as_i64() as usize,
        }
    }

    pub fn push_address(&mut self, value: usize, addr_type: AddrType) {
        match addr_type {
            AddrType::I32 => self.push(value as i32),
            AddrType::I64 => self.push(value as i64),
        }
    }

    fn get(&self, index: usize) -> &RawValue {
        unsafe { self.inner.get_unchecked(index) }
    }

    fn get_mut(&mut self, index: usize) -> &mut RawValue {
        unsafe { self.inner.get_unchecked_mut(index) }
    }

    fn get_slice(&self, range: Range<usize>) -> &[RawValue] {
        unsafe { self.inner.get_unchecked(range) }
    }
}
