//! Bit scaling between value resolutions (M2-115-U v1.0.2, MIDI 2.0 Bit Scaling and Resolution).
//!
//! - Min-Center-Max Scaling (M2-115 3) is the default for most values: Poly Pressure, Channel
//!   Pressure, Pitch Bend, Control Change, Registered Controllers with Index LSB 32 to 127,
//!   Assignable Controllers and Note On/Off Velocity (3.1). Minimum, center
//!   (`TRUNC((Highest + 1) / 2)`) and maximum map to minimum, center and maximum (3.2).
//!   [`upscale`] uses bit shifting up to the center and bit repeating above it (3.3);
//!   [`downscale`] cuts off the low-order bits (3.4).
//! - Zero-Extension Scaling with Rounding (M2-115 4) is for Registered Controllers with Index LSB
//!   0 to 31: fixed-point and right-open values (4.1). [`zero_extend_upscale`] fills the added low
//!   bits with zero (4.3); [`zero_extend_downscale`] rounds to the nearest value and clamps (4.4).
//! - Stepped values and enumerations (M2-115 5) use the full resolution: [`encode_step`] (5.1)
//!   and [`decode_step`] (5.2).
//! - Downscaling a value that the same method upscaled gives the original value (3.2, 4.2).
//! - Every function is `const`, never panics, and gives [`UmpError::OutOfRange`] for a bit
//!   width or value outside the preconditions of the algorithm.
//! - Where a MIDI-CI specification defines its own scaling for a value, those rules apply
//!   instead (M2-115 1.3); this module does not cover them.

use crate::packet::UmpError;

/// True when `value` fits in `bits` bits (`bits` 1 to 32).
const fn fits(value: u32, bits: u8) -> bool {
    bits >= 32 || value < (1u32 << bits)
}

/// Largest value of a `bits`-bit field (`bits` 1 to 32).
const fn max_value(bits: u8) -> u32 {
    if bits >= 32 {
        u32::MAX
    } else {
        (1u32 << bits) - 1
    }
}

/// Min-Center-Max upscaling of `value` from `src_bits` to `dst_bits` (M2-115 3.3, 3.3.1;
/// M2-104-UM D.1.3).
///
/// - Values from minimum to the center (`1 << (src_bits - 1)`) are bit shifted. Values above the
///   center are bit shifted, and all source bits but the highest are repeated into the added low
///   bits (Figure 1).
/// - A 1-bit value scales to zero or to the destination maximum (3.3, Figure 2).
/// - Results equal M2-115 Tables 5, 6 and 7, for example 7 to 16 bits: 0x40 gives 0x8000 and
///   0x46 gives 0x8C30.
///
/// # Errors
/// [`UmpError::OutOfRange`] unless `1 <= src_bits < dst_bits <= 32` and `value` fits in
/// `src_bits` bits.
pub const fn upscale(value: u32, src_bits: u8, dst_bits: u8) -> Result<u32, UmpError> {
    if src_bits == 0 || dst_bits > 32 || src_bits >= dst_bits || !fits(value, src_bits) {
        return Err(UmpError::OutOfRange);
    }
    if src_bits == 1 {
        return Ok(if value == 0 { 0 } else { max_value(dst_bits) });
    }
    let scale_bits = dst_bits - src_bits;
    let src_center = 1u32 << (src_bits - 1);
    let mut result = value << scale_bits;
    if value <= src_center {
        return Ok(result);
    }
    let repeat_bits = src_bits - 1;
    let mut repeat = value & ((1u32 << repeat_bits) - 1);
    if scale_bits > repeat_bits {
        repeat <<= scale_bits - repeat_bits;
    } else {
        repeat >>= repeat_bits - scale_bits;
    }
    while repeat != 0 {
        result |= repeat;
        repeat >>= repeat_bits;
    }
    Ok(result)
}

/// Min-Center-Max downscaling of `value` from `src_bits` to `dst_bits`: the low-order bits are
/// cut off (M2-115 3.4, 3.4.1; M2-104-UM D.1.4). Results equal M2-115 Table 8, for example
/// 16 to 7 bits: 0xAEBA gives 0x57.
///
/// # Errors
/// [`UmpError::OutOfRange`] unless `1 <= dst_bits < src_bits <= 32` and `value` fits in
/// `src_bits` bits.
pub const fn downscale(value: u32, src_bits: u8, dst_bits: u8) -> Result<u32, UmpError> {
    if dst_bits == 0 || src_bits > 32 || dst_bits >= src_bits || !fits(value, src_bits) {
        return Err(UmpError::OutOfRange);
    }
    Ok(value >> (src_bits - dst_bits))
}

/// Zero-Extension upscaling of `value` from `src_bits` to `dst_bits`: the added low bits are
/// zero (M2-115 4.3, 4.3.1). Results equal M2-115 Table 9, for example 7 to 16 bits: 0x7F gives
/// 0xFE00.
///
/// The added bits are always zero, so no noise is sent in them (M2-115 4).
///
/// # Errors
/// [`UmpError::OutOfRange`] unless `2 <= src_bits < dst_bits <= 32` and `value` fits in
/// `src_bits` bits. A 1-bit value is rejected, because this method should not be used for it
/// (M2-115 4.3).
pub const fn zero_extend_upscale(value: u32, src_bits: u8, dst_bits: u8) -> Result<u32, UmpError> {
    if src_bits < 2 || dst_bits > 32 || src_bits >= dst_bits || !fits(value, src_bits) {
        return Err(UmpError::OutOfRange);
    }
    Ok(value << (dst_bits - src_bits))
}

/// Zero-Extension downscaling of `value` from `src_bits` to `dst_bits`: rounds to the nearest
/// value of the lower resolution, then clamps to the destination maximum (M2-115 4.4, 4.4.1).
/// Results equal M2-115 Table 10, for example 16 to 7 bits: 0x15FF gives 0x0B and 0xFFFF gives
/// 0x7F.
///
/// # Errors
/// [`UmpError::OutOfRange`] unless `1 <= dst_bits < src_bits <= 32` and `value` fits in
/// `src_bits` bits.
pub const fn zero_extend_downscale(
    value: u32,
    src_bits: u8,
    dst_bits: u8,
) -> Result<u32, UmpError> {
    if dst_bits == 0 || src_bits > 32 || dst_bits >= src_bits || !fits(value, src_bits) {
        return Err(UmpError::OutOfRange);
    }
    let scale_bits = src_bits - dst_bits;
    let half = 1u64 << (scale_bits - 1);
    let shifted = (value as u64 + half) >> scale_bits;
    let max = max_value(dst_bits) as u64;
    Ok(if shifted > max { max } else { shifted } as u32)
}

/// Message value for step `step_value` (counted from zero) of `step_count` steps, in a field of
/// `bits` bits (M2-115 5.1).
///
/// - `messageValue = TRUNC((valueRange * stepValue + stepCount - 1) / stepCount) +
///   TRUNC(valueRange / (2 * stepCount))`, with `valueRange = 2^bits`. The value is at the
///   center of its step (Figures 6 and 7).
/// - With 2 steps, the values are 0 and `valueRange - 1` (5.1).
///
/// # Errors
/// [`UmpError::OutOfRange`] unless `1 <= bits <= 32`, `1 <= step_count <= valueRange / 2` and
/// `step_value < step_count`. A `step_count` above half the value range is rejected, because
/// M2-115 5.1 says to avoid it.
pub const fn encode_step(step_value: u32, step_count: u32, bits: u8) -> Result<u32, UmpError> {
    if bits == 0 || bits > 32 {
        return Err(UmpError::OutOfRange);
    }
    let range = 1u64 << bits;
    let count = step_count as u64;
    if count == 0 || count > range / 2 || step_value >= step_count {
        return Err(UmpError::OutOfRange);
    }
    if count == 2 {
        return Ok(if step_value == 0 {
            0
        } else {
            (range - 1) as u32
        });
    }
    // TRUNC((a + stepCount - 1) / stepCount) is the ceiling of a / stepCount.
    let value = (range * step_value as u64).div_ceil(count) + range / (2 * count);
    Ok(value as u32)
}

/// Step selected by a received `message_value` in a field of `bits` bits with `step_count`
/// steps: `stepValue = TRUNC(messageValue * stepCount / valueRange)`, with
/// `valueRange = 2^bits` (M2-115 5.2). Every value in the range of a step selects that step.
///
/// # Errors
/// [`UmpError::OutOfRange`] unless `1 <= bits <= 32`, `message_value` fits in `bits` bits and
/// `1 <= step_count <= 2^bits - 1` (the field maximum, M2-115 5.1).
pub const fn decode_step(message_value: u32, step_count: u32, bits: u8) -> Result<u32, UmpError> {
    if bits == 0 || bits > 32 || !fits(message_value, bits) {
        return Err(UmpError::OutOfRange);
    }
    let range = 1u64 << bits;
    let count = step_count as u64;
    if count == 0 || count > range - 1 {
        return Err(UmpError::OutOfRange);
    }
    Ok((message_value as u64 * count / range) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn table(rows: &[(u32, u32)], f: impl Fn(u32) -> Result<u32, UmpError>) {
        for &(input, expected) in rows {
            assert_eq!(f(input), Ok(expected), "input {input:#X}");
        }
    }

    /// M2-115 Table 5: Min-Center-Max upscale 7 to 16 bits.
    #[test]
    fn upscale_table_5_7_to_16() {
        table(
            &[
                (0x00, 0x0000),
                (0x05, 0x0A00),
                (0x1E, 0x3C00),
                (0x20, 0x4000),
                (0x40, 0x8000),
                (0x46, 0x8C30),
                (0x60, 0xC104),
                (0x78, 0xF1C7),
                (0x7F, 0xFFFF),
            ],
            |v| upscale(v, 7, 16),
        );
    }

    /// M2-115 Table 6: Min-Center-Max upscale 7 to 32 bits.
    #[test]
    fn upscale_table_6_7_to_32() {
        table(
            &[
                (0x00, 0x0000_0000),
                (0x05, 0x0A00_0000),
                (0x1E, 0x3C00_0000),
                (0x20, 0x4000_0000),
                (0x40, 0x8000_0000),
                (0x46, 0x8C30_C30C),
                (0x60, 0xC104_1041),
                (0x78, 0xF1C7_1C71),
                (0x7F, 0xFFFF_FFFF),
            ],
            |v| upscale(v, 7, 32),
        );
    }

    /// M2-115 Table 7: Min-Center-Max upscale 16 to 32 bits.
    #[test]
    fn upscale_table_7_16_to_32() {
        table(
            &[
                (0x0000, 0x0000_0000),
                (0x0005, 0x0005_0000),
                (0x001E, 0x001E_0000),
                (0x4000, 0x4000_0000),
                (0x8000, 0x8000_0000),
                (0x9C40, 0x9C40_3880),
                (0xC000, 0xC000_8001),
                (0xFDE8, 0xFDE8_FBD1),
                (0xFFFF, 0xFFFF_FFFF),
            ],
            |v| upscale(v, 16, 32),
        );
    }

    /// M2-104-UM D.1.3 example values: 7 to 16 bits (REQ-XLATE-004 uses the same values).
    #[test]
    fn upscale_m2_104_d_1_3_examples() {
        table(
            &[
                (0x0A, 0x1400),
                (0x40, 0x8000),
                (0x57, 0xAEBA),
                (0x7F, 0xFFFF),
            ],
            |v| upscale(v, 7, 16),
        );
    }

    /// M2-115 Figure 2: a 1-bit value scales to zero or to the destination maximum (3.3).
    #[test]
    fn upscale_figure_2_one_bit() {
        assert_eq!(upscale(0, 1, 7), Ok(0));
        assert_eq!(upscale(1, 1, 7), Ok(0x7F));
        assert_eq!(upscale(1, 1, 16), Ok(0xFFFF));
        assert_eq!(upscale(1, 1, 32), Ok(u32::MAX));
    }

    /// M2-115 Figure 3: 2 to 7 bits. `ab`: a = 0 gives `0b00000`, a = 1 repeats b.
    #[test]
    fn upscale_figure_3_two_bits() {
        assert_eq!(upscale(0b00, 2, 7), Ok(0b000_0000));
        assert_eq!(upscale(0b01, 2, 7), Ok(0b010_0000));
        assert_eq!(upscale(0b10, 2, 7), Ok(0b100_0000));
        assert_eq!(upscale(0b11, 2, 7), Ok(0b111_1111));
    }

    /// M2-115 Figure 4: 3 to 16 bits. With a = 1, `bc` repeats into all low bits.
    #[test]
    fn upscale_figure_4_three_bits() {
        assert_eq!(upscale(0b011, 3, 16), Ok(0b0110_0000_0000_0000));
        assert_eq!(upscale(0b101, 3, 16), Ok(0b1010_1010_1010_1010));
        assert_eq!(upscale(0b110, 3, 16), Ok(0b1101_0101_0101_0101));
        assert_eq!(upscale(0b111, 3, 16), Ok(0xFFFF));
    }

    /// M2-115 Figure 5: 12 to 32 bits. With a = 1, bits b..m repeat (11 bits) from bit 19 down.
    #[test]
    fn upscale_figure_5_twelve_bits() {
        let src = 0b1011_0110_1001u32; // a=1, b..m = 011_0110_1001
        let rep = src & 0x7FF;
        let expected = (src << 20) | (rep << 9) | (rep >> 2);
        assert_eq!(upscale(src, 12, 32), Ok(expected));
        let low = 0b0101_0101_0101u32;
        assert_eq!(upscale(low, 12, 32), Ok(low << 20));
    }

    /// M2-115 Table 4 and 3.2: center maps to center.
    #[test]
    fn upscale_center_maps_to_center() {
        assert_eq!(upscale(0x40, 7, 16), Ok(0x8000));
        assert_eq!(upscale(0x2000, 14, 32), Ok(0x8000_0000));
        assert_eq!(upscale(0x80, 8, 16), Ok(0x8000));
        assert_eq!(upscale(0x8000, 16, 32), Ok(0x8000_0000));
    }

    #[test]
    fn upscale_rejects_bad_widths_and_values() {
        assert_eq!(upscale(0, 0, 7), Err(UmpError::OutOfRange));
        assert_eq!(upscale(0, 7, 7), Err(UmpError::OutOfRange));
        assert_eq!(upscale(0, 16, 7), Err(UmpError::OutOfRange));
        assert_eq!(upscale(0, 7, 33), Err(UmpError::OutOfRange));
        assert_eq!(upscale(0x80, 7, 16), Err(UmpError::OutOfRange));
        assert_eq!(upscale(2, 1, 16), Err(UmpError::OutOfRange));
    }

    /// M2-115 Table 8 and M2-104-UM D.1.4: Min-Center-Max downscale 16 to 7 bits.
    #[test]
    fn downscale_table_8_16_to_7() {
        table(
            &[
                (0x1400, 0x0A),
                (0x8000, 0x40),
                (0xAEBA, 0x57),
                (0xFFFF, 0x7F),
            ],
            |v| downscale(v, 16, 7),
        );
    }

    #[test]
    fn downscale_rejects_bad_widths_and_values() {
        assert_eq!(downscale(0, 16, 0), Err(UmpError::OutOfRange));
        assert_eq!(downscale(0, 7, 7), Err(UmpError::OutOfRange));
        assert_eq!(downscale(0, 7, 16), Err(UmpError::OutOfRange));
        assert_eq!(downscale(0, 33, 7), Err(UmpError::OutOfRange));
        assert_eq!(downscale(0x1_0000, 16, 7), Err(UmpError::OutOfRange));
    }

    /// M2-115 Table 9: Zero-Extension upscale 7 to 16 bits.
    #[test]
    fn zero_extend_upscale_table_9_7_to_16() {
        table(
            &[
                (0x0A, 0x1400),
                (0x40, 0x8000),
                (0x57, 0xAE00),
                (0x7F, 0xFE00),
            ],
            |v| zero_extend_upscale(v, 7, 16),
        );
    }

    /// M2-115 4: Pitch 7.9 127.0 (0xFE00) to 32 bits gives 0xFE000000, with no noise in the low
    /// bits. Min-Center-Max adds noise to the same value. (The M2-115 4 example value 0xFE01F803
    /// does not agree with its own binary form, so it is not used as a vector.)
    #[test]
    fn zero_extend_upscale_pitch_has_no_noise() {
        assert_eq!(zero_extend_upscale(0xFE00, 16, 32), Ok(0xFE00_0000));
        assert_eq!(upscale(0xFE00, 16, 32), Ok(0xFE00_FC01));
    }

    /// M2-115 4.3: Zero-Extension should not be used for 1-bit values.
    #[test]
    fn zero_extend_upscale_rejects_one_bit_and_bad_widths() {
        assert_eq!(zero_extend_upscale(1, 1, 16), Err(UmpError::OutOfRange));
        assert_eq!(zero_extend_upscale(0, 16, 16), Err(UmpError::OutOfRange));
        assert_eq!(zero_extend_upscale(0, 7, 33), Err(UmpError::OutOfRange));
        assert_eq!(zero_extend_upscale(0x80, 7, 16), Err(UmpError::OutOfRange));
    }

    /// M2-115 Table 10: Zero-Extension downscale 16 to 7 bits, with rounding and clamping.
    #[test]
    fn zero_extend_downscale_table_10_16_to_7() {
        table(
            &[
                (0x1400, 0x0A),
                (0x15FF, 0x0B),
                (0x8000, 0x40),
                (0xAE00, 0x57),
                (0xAEBA, 0x57),
                (0xAF00, 0x58),
                (0xFE00, 0x7F),
                (0xFFFF, 0x7F),
            ],
            |v| zero_extend_downscale(v, 16, 7),
        );
    }

    #[test]
    fn zero_extend_downscale_does_not_overflow_at_32_bits() {
        assert_eq!(zero_extend_downscale(u32::MAX, 32, 16), Ok(0xFFFF));
        assert_eq!(zero_extend_downscale(u32::MAX, 32, 31), Ok(0x7FFF_FFFF));
        assert_eq!(zero_extend_downscale(0, 32, 0), Err(UmpError::OutOfRange));
    }

    /// M2-115 Figure 6: 12 steps in 7-bit resolution.
    #[test]
    fn encode_step_figure_6_twelve_steps_7_bit() {
        let expected = [5, 16, 27, 37, 48, 59, 69, 80, 91, 101, 112, 123];
        for (step, &value) in expected.iter().enumerate() {
            assert_eq!(encode_step(step as u32, 12, 7), Ok(value), "step {step}");
        }
    }

    /// M2-115 Figure 7: 8 steps in 32-bit resolution.
    #[test]
    fn encode_step_figure_7_eight_steps_32_bit() {
        for step in 0..8u32 {
            assert_eq!(
                encode_step(step, 8, 32),
                Ok(0x1000_0000 + step * 0x2000_0000),
                "step {step}"
            );
        }
    }

    /// M2-115 5.1: with 2 steps, use 0 and valueRange - 1.
    #[test]
    fn encode_step_two_steps_uses_extremes() {
        assert_eq!(encode_step(0, 2, 7), Ok(0));
        assert_eq!(encode_step(1, 2, 7), Ok(127));
        assert_eq!(encode_step(1, 2, 32), Ok(u32::MAX));
    }

    /// M2-115 5.1: avoid a stepCount above half the valueRange; stepValue is below stepCount.
    #[test]
    fn encode_step_rejects_bad_counts() {
        assert_eq!(encode_step(0, 0, 7), Err(UmpError::OutOfRange));
        assert_eq!(encode_step(0, 65, 7), Err(UmpError::OutOfRange));
        assert_eq!(encode_step(0, 64, 7), Ok(1));
        assert_eq!(encode_step(12, 12, 7), Err(UmpError::OutOfRange));
        assert_eq!(encode_step(0, 2, 0), Err(UmpError::OutOfRange));
        assert_eq!(encode_step(0, 2, 33), Err(UmpError::OutOfRange));
    }

    /// M2-115 5.2 with Figure 6: every value in a step selects that step. The figure rounds
    /// the bound labels (21, 53, 85, 117); the exact lowest values of those steps are
    /// ceil(step * 128 / 12) = 22, 54, 86, 118.
    #[test]
    fn decode_step_figure_6_bounds() {
        let bounds = [0, 11, 22, 32, 43, 54, 64, 75, 86, 96, 107, 118];
        for (step, &low) in bounds.iter().enumerate() {
            assert_eq!(decode_step(low, 12, 7), Ok(step as u32), "low bound {low}");
            if low > 0 {
                assert_eq!(
                    decode_step(low - 1, 12, 7),
                    Ok(step as u32 - 1),
                    "below {low}"
                );
            }
        }
        let centers = [5, 16, 27, 37, 48, 59, 69, 80, 91, 101, 112, 123];
        for (step, &value) in centers.iter().enumerate() {
            assert_eq!(decode_step(value, 12, 7), Ok(step as u32), "center {value}");
        }
        assert_eq!(decode_step(10, 12, 7), Ok(0));
        assert_eq!(decode_step(127, 12, 7), Ok(11));
    }

    #[test]
    fn decode_step_rejects_bad_input() {
        assert_eq!(decode_step(128, 12, 7), Err(UmpError::OutOfRange));
        assert_eq!(decode_step(0, 0, 7), Err(UmpError::OutOfRange));
        assert_eq!(decode_step(0, 128, 7), Err(UmpError::OutOfRange));
        assert_eq!(decode_step(0, 2, 33), Err(UmpError::OutOfRange));
    }

    fn widths() -> impl Strategy<Value = (u8, u8)> {
        (1u8..=31).prop_flat_map(|src| (Just(src), (src + 1)..=32))
    }

    fn mask(bits: u8) -> u32 {
        max_value(bits)
    }

    /// M2-115 3.2: downscaling an upscaled value gives the original value; minimum, center and
    /// maximum map to minimum, center and maximum.
    #[test]
    fn prop_upscale_round_trip_and_core_rules() {
        proptest!(|((src, dst) in widths(), raw in any::<u32>())| {
            let value = raw & mask(src);
            let up = upscale(value, src, dst).unwrap();
            prop_assert!(up <= mask(dst));
            prop_assert_eq!(downscale(up, dst, src), Ok(value));
            prop_assert_eq!(upscale(0, src, dst), Ok(0));
            prop_assert_eq!(upscale(mask(src), src, dst), Ok(mask(dst)));
            if src > 1 {
                prop_assert_eq!(upscale(1 << (src - 1), src, dst), Ok(1 << (dst - 1)));
            }
        });
    }

    /// M2-115 3.2: upscaled values are distributed smoothly (the mapping is strictly monotonic).
    #[test]
    fn prop_upscale_is_monotonic() {
        proptest!(|((src, dst) in widths(), raw in any::<u32>())| {
            let value = raw & mask(src);
            prop_assume!(value < mask(src));
            prop_assert!(upscale(value, src, dst).unwrap() < upscale(value + 1, src, dst).unwrap());
        });
    }

    /// M2-115 4.2: Zero-Extension round trip, with zero in the added low bits.
    #[test]
    fn prop_zero_extend_round_trip() {
        proptest!(|((src, dst) in widths(), raw in any::<u32>())| {
            prop_assume!(src >= 2);
            let value = raw & mask(src);
            let up = zero_extend_upscale(value, src, dst).unwrap();
            prop_assert_eq!(up & mask(dst - src), 0);
            prop_assert_eq!(zero_extend_downscale(up, dst, src), Ok(value));
        });
    }

    /// M2-115 4.4: the rounded result is the nearest lower-resolution value, clamped.
    #[test]
    fn prop_zero_extend_downscale_rounds_to_nearest() {
        proptest!(|((dst, src) in widths(), raw in any::<u32>())| {
            let value = raw & mask(src);
            let down = zero_extend_downscale(value, src, dst).unwrap() as u64;
            let shift = src - dst;
            let err = (value as i128 - ((down as i128) << shift)).unsigned_abs();
            prop_assert!(down <= mask(dst) as u64);
            if down < mask(dst) as u64 {
                prop_assert!(err <= 1u128 << (shift - 1));
            }
        });
    }

    /// M2-115 5.1, 5.2: each encoded step decodes to the same step.
    #[test]
    fn prop_step_round_trip() {
        proptest!(|(bits in 2u8..=32, count_raw in any::<u32>(), step_raw in any::<u32>())| {
            let half = ((1u64 << bits) / 2) as u32;
            let count = 1 + count_raw % half;
            let step = step_raw % count;
            let value = encode_step(step, count, bits).unwrap();
            prop_assert!(fits(value, bits));
            prop_assert_eq!(decode_step(value, count, bits), Ok(step));
        });
    }

    /// No input makes a scaling function panic.
    #[test]
    fn prop_never_panics() {
        proptest!(|(v in any::<u32>(), a in any::<u8>(), b in any::<u8>(), c in any::<u32>())| {
            let _ = upscale(v, a, b);
            let _ = downscale(v, a, b);
            let _ = zero_extend_upscale(v, a, b);
            let _ = zero_extend_downscale(v, a, b);
            let _ = encode_step(v, c, a);
            let _ = decode_step(v, c, a);
        });
    }
}
