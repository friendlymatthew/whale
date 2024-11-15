use anyhow::{anyhow, bail, ensure, Result};

/// The maximum length of a leb128-encoded 32-bit integer
pub const MAX_LEB128_LEN_32: usize = 5;
pub const MAX_LEB128_LEN_64: usize = 10;

#[inline]
const fn zig_zag(x: i32) -> u32 {
    ((x << 1) ^ (x >> 31)) as u32
}

#[inline]
const fn decode_zig_zag_u32(x: u32) -> i32 {
    ((x >> 1) as i32) ^ -((x & 1) as i32)
}

#[inline]
const fn decode_zig_zag_u64(x: u64) -> i64 {
    ((x >> 1) as i64) ^ -((x & 1) as i64)
}

/// Returns the encoded size of an unsigned leb128-encoded integer.
#[inline]
fn size_u32(x: u32) -> usize {
    let bits = x.max(1).ilog2() + 1;
    ((9 * bits + 64) / 64) as usize
}

#[inline]
pub fn write_u32(buf: &mut [u8], mut x: u32) -> Result<usize> {
    ensure!(
        size_u32(x) <= buf.len(),
        "The number being read is too large for the provided buffer."
    );

    for (i, curr_byte) in buf.iter_mut().enumerate().take(MAX_LEB128_LEN_32) {
        let byte = (x & 0x7F) as u8;
        x >>= 7;

        let more = (x != 0) as u8;
        *curr_byte = byte | (more << 7);

        if more == 0 {
            return Ok(i + 1);
        }
    }

    Err(anyhow!(
        "The number being encoded exceeds the maximum representable length."
    ))
}

#[inline]
pub fn read_u32(buf: &[u8]) -> Result<(u32, usize)> {
    let mut x = 0u32;
    let mut s: usize = 0;

    for (i, &b) in buf.iter().enumerate() {
        ensure!(
            i < MAX_LEB128_LEN_32,
            "The number being decoded exceeds the maximum length of a leb128-encoded 32-bit integer."
        );

        if b < 0x80 {
            ensure!(
                i != MAX_LEB128_LEN_32 || b <= 1,
                "Invalid final byte for 32-bit leb128 decoding."
            );

            return Ok((x | (b as u32) << s, i + 1));
        }

        x |= ((b & 0x7f) as u32) << s;
        s += 7
    }

    bail!("The number being decoded exceeds the maximum length of a leb128-encoded 32-bit integer.")
}

#[inline]
pub fn write_i32(buf: &mut [u8], x: i32) -> Result<usize> {
    write_u32(buf, zig_zag(x))
}

#[inline]
pub fn read_i32(buf: &[u8]) -> Result<(i32, usize)> {
    let (ux, n) = read_u32(buf)?;
    Ok((decode_zig_zag_u32(ux), n))
}

#[inline]
pub fn read_u64(buf: &[u8]) -> Result<(u64, usize)> {
    let mut x = 0u64;
    let mut s: usize = 0;

    for (i, &b) in buf.iter().enumerate() {
        ensure!(
            i < MAX_LEB128_LEN_64,
            "The number being decoded exceeds the maximum length of a leb128-encoded 32-bit integer."
        );

        if b < 0x80 {
            ensure!(
                i != MAX_LEB128_LEN_64 || b <= 1,
                "Invalid final byte for 32-bit leb128 decoding."
            );

            return Ok((x | (b as u64) << s, i + 1));
        }

        x |= ((b & 0x7f) as u64) << s;
        s += 7
    }

    bail!("The number being decoded exceeds the maximum length of a leb128-encoded 64-bit integer.")
}

#[inline]
pub fn read_i64(buf: &[u8]) -> Result<(i64, usize)> {
    let (ux, n) = read_u64(buf)?;
    Ok((decode_zig_zag_u64(ux), n))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The following tests follow the test cases in the Golang implementation of
    // leb128.
    //
    // https://go.dev/src/encoding/binary/varint_test.go

    const TEST_VALUES: [i32; 18] = [
        i32::MIN,
        i32::MIN + 1,
        -1,
        0,
        1,
        2,
        10,
        20,
        63,
        64,
        65,
        127,
        128,
        129,
        255,
        256,
        257,
        i32::MAX,
    ];

    fn test_i32(x: i32) -> Result<()> {
        let mut buf = [0u8; MAX_LEB128_LEN_32];

        let n = write_i32(&mut buf, x)?;
        let (y, m) = read_i32(&buf[0..n])?;

        assert_eq!(x, y, "Expected {}, got: {}", x, y);
        assert_eq!(n, m, "For {}, got {}, want: {}", x, m, n);

        Ok(())
    }

    fn test_u32(x: u32) -> Result<()> {
        let mut buf = [0u8; MAX_LEB128_LEN_32];

        let n = write_u32(&mut buf, x)?;
        let (y, m) = read_u32(&buf[0..n])?;

        assert_eq!(x, y, "Expected {}, got: {}", x, y);
        assert_eq!(n, m, "For {}, got {}, want: {}", x, m, n);

        Ok(())
    }

    #[test]
    fn valid_signed_integers() -> Result<()> {
        for x in TEST_VALUES {
            test_i32(x)?;
        }

        Ok(())
    }

    #[test]
    fn valid_unsigned_integers() -> Result<()> {
        for x in TEST_VALUES {
            if x >= 0 {
                test_u32(x as u32)?;
            }
        }

        Ok(())
    }

    #[test]
    fn non_canonical_zero() -> Result<()> {
        let buf = [0x80, 0x80, 0x80, 0];
        let (x, n) = read_u32(&buf)?;

        assert_eq!(x, 0, "Expected x to be 0. Got: {}", x);
        assert_eq!(n, 4, "Expected n to be 4. Got: {}", n);

        Ok(())
    }

    #[test]
    fn overflow() -> Result<()> {
        let overflow_cases = [
            vec![0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
            vec![0x80, 0x80, 0x80, 0x80, 0x80, 0x80],
        ];

        for buffer in overflow_cases {
            assert!(
                read_u32(&buffer).is_err(),
                "Expected overflow for buffer: {:?}",
                buffer
            );
        }

        Ok(())
    }
}
