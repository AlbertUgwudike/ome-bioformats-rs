//! PackBits (Apple run-length) decoding. Port of `loci.formats.codec.PackbitsCodec`.

pub fn decode(data: &[u8], expected: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(expected);
    let mut i = 0;
    while i < data.len() && out.len() < expected {
        let n = data[i] as i8;
        i += 1;
        if n >= 0 {
            let len = n as usize + 1;
            let end = (i + len).min(data.len());
            out.extend_from_slice(&data[i..end]);
            i = end;
        } else if n != -128 {
            let len = (-(n as i16)) as usize + 1;
            if let Some(&b) = data.get(i) {
                out.extend(std::iter::repeat_n(b, len));
            }
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn decodes_reference_example() {
        // Example from the TIFF 6.0 specification.
        let packed = [
            0xFE, 0xAA, 0x02, 0x80, 0x00, 0x2A, 0xFD, 0xAA, 0x03, 0x80, 0x00, 0x2A, 0x22, 0xF7,
            0xAA,
        ];
        let expected = [
            0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0x22,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
        ];
        assert_eq!(super::decode(&packed, expected.len()), expected);
    }
}
