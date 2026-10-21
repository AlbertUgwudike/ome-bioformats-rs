//! TIFF horizontal-differencing predictor (Predictor = 2).
//!
//! Operates per row on chunky (interleaved) data: each sample is replaced by
//! its difference from the same sample of the previous pixel.

/// Undoes differencing in place. `row_len` is the row length in bytes.
pub fn undo(buf: &mut [u8], row_len: usize, samples: usize, bytes: usize, little_endian: bool) {
    for row in buf.chunks_mut(row_len) {
        match bytes {
            1 => {
                for i in samples..row.len() {
                    row[i] = row[i].wrapping_add(row[i - samples]);
                }
            }
            2 => accumulate::<2>(row, samples, little_endian),
            4 => accumulate::<4>(row, samples, little_endian),
            8 => accumulate::<8>(row, samples, little_endian),
            _ => {}
        }
    }
}

/// Applies differencing in place to little-endian data.
pub fn apply(buf: &mut [u8], row_len: usize, samples: usize, bytes: usize) {
    for row in buf.chunks_mut(row_len) {
        match bytes {
            1 => {
                for i in (samples..row.len()).rev() {
                    row[i] = row[i].wrapping_sub(row[i - samples]);
                }
            }
            2 => difference::<2>(row, samples),
            4 => difference::<4>(row, samples),
            8 => difference::<8>(row, samples),
            _ => {}
        }
    }
}

fn get<const N: usize>(row: &[u8], i: usize, le: bool) -> u64 {
    let mut v = 0u64;
    for b in 0..N {
        let byte = if le {
            row[i * N + N - 1 - b]
        } else {
            row[i * N + b]
        };
        v = (v << 8) | byte as u64;
    }
    v
}

fn put<const N: usize>(row: &mut [u8], i: usize, v: u64, le: bool) {
    for b in 0..N {
        let byte = (v >> (8 * b)) as u8;
        if le {
            row[i * N + b] = byte;
        } else {
            row[i * N + N - 1 - b] = byte;
        }
    }
}

fn accumulate<const N: usize>(row: &mut [u8], samples: usize, le: bool) {
    for i in samples..row.len() / N {
        let v = get::<N>(row, i, le).wrapping_add(get::<N>(row, i - samples, le));
        put::<N>(row, i, v, le);
    }
}

fn difference<const N: usize>(row: &mut [u8], samples: usize) {
    for i in (samples..row.len() / N).rev() {
        let v = get::<N>(row, i, true).wrapping_sub(get::<N>(row, i - samples, true));
        put::<N>(row, i, v, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_16bit_rgb() {
        let orig: Vec<u8> = (0..2 * 3 * 2 * 5u32)
            .map(|i| (i * 37 % 256) as u8)
            .collect();
        let mut buf = orig.clone();
        apply(&mut buf, 2 * 3 * 5, 3, 2);
        assert_ne!(buf, orig);
        undo(&mut buf, 2 * 3 * 5, 3, 2, true);
        assert_eq!(buf, orig);
    }
}
