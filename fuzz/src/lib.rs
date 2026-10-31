//! Helpers of the fuzz targets.

/// Splits fuzz data into parts: each part is a length byte and then that many bytes (the last
/// part takes what is left). So one input gives a sequence of messages or packets.
pub fn parts(mut data: &[u8]) -> impl Iterator<Item = &[u8]> {
    core::iter::from_fn(move || {
        let (&len, rest) = data.split_first()?;
        let n = (len as usize).min(rest.len());
        let (part, tail) = rest.split_at(n);
        data = tail;
        Some(part)
    })
}

/// Big-endian 32-bit words from bytes (a short tail is left out).
pub fn words(data: &[u8]) -> Vec<u32> {
    data.chunks_exact(4)
        .map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}
