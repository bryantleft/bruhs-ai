//! Named numeric conversions for the casts that are lossy on purpose.
//!
//! The workspace denies silent `as` casts that can truncate, wrap, lose sign
//! or lose precision. Lossless conversions use `From`, fallible ones
//! `TryFrom`; what remains are the conversions where rounding or saturating
//! *is* the intended behavior — layout and pixel math, audio samples,
//! averages. Those go through the two traits here so each call site says
//! which edge behavior it accepts, and the one `as` lives in this module.
//!
//! Timestamps have their own type: see [`EpochMs`](crate::EpochMs).

/// Integer → float (and `f64` → `f32`) conversions that round to the nearest
/// representable value.
///
/// For integers, exact for magnitudes up to 2^24 (`f32`) and 2^53 (`f64`) —
/// every count, index, pixel offset and sample the app handles; larger values
/// round to the nearest float rather than failing. For `f64`, `to_f32_lossy`
/// rounds to the nearest `f32` (out-of-range values become infinities) and
/// `to_f64_lossy` is the identity. Epoch-millisecond timestamps go through
/// [`EpochMs`](crate::EpochMs) instead.
pub trait ToFloatLossy: Copy {
    /// `self` as the nearest `f32`.
    fn to_f32_lossy(self) -> f32;
    /// `self` as the nearest `f64`.
    fn to_f64_lossy(self) -> f64;
}

macro_rules! to_float_lossy {
    ($($ty:ty),*) => {$(
        impl ToFloatLossy for $ty {
            #[inline]
            #[expect(
                clippy::cast_precision_loss,
                reason = "rounding to the nearest float is this method's documented contract"
            )]
            fn to_f32_lossy(self) -> f32 {
                self as f32
            }

            #[inline]
            #[expect(
                clippy::cast_precision_loss,
                reason = "rounding to the nearest float is this method's documented contract"
            )]
            fn to_f64_lossy(self) -> f64 {
                self as f64
            }
        }
    )*};
}

to_float_lossy!(usize, isize, u64, i64, u128, i128);

macro_rules! to_float_lossy_small {
    ($($ty:ty),*) => {$(
        impl ToFloatLossy for $ty {
            #[inline]
            #[expect(
                clippy::cast_precision_loss,
                reason = "rounding to the nearest float is this method's documented contract"
            )]
            fn to_f32_lossy(self) -> f32 {
                self as f32
            }

            #[inline]
            fn to_f64_lossy(self) -> f64 {
                f64::from(self)
            }
        }
    )*};
}

to_float_lossy_small!(u32, i32);

impl ToFloatLossy for f64 {
    #[inline]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "rounding to the nearest f32 is this method's documented contract"
    )]
    fn to_f32_lossy(self) -> f32 {
        self as f32
    }

    #[inline]
    fn to_f64_lossy(self) -> f64 {
        self
    }
}

/// Float → integer conversions with Rust's `as` semantics, named.
///
/// Each method truncates toward zero (call `.round()`, `.floor()` or
/// `.ceil()` first to choose otherwise), maps `NaN` to `0`, and clamps
/// infinities and out-of-range values to the target type's bounds — so a
/// negative value becomes `0` for an unsigned target.
pub trait ToIntSaturating: Copy {
    /// Truncated and clamped to `i64`.
    fn to_i64_saturating(self) -> i64;
    /// Truncated and clamped to `u64`.
    fn to_u64_saturating(self) -> u64;
    /// Truncated and clamped to `i32`.
    fn to_i32_saturating(self) -> i32;
    /// Truncated and clamped to `u32`.
    fn to_u32_saturating(self) -> u32;
    /// Truncated and clamped to `i16`.
    fn to_i16_saturating(self) -> i16;
    /// Truncated and clamped to `u16`.
    fn to_u16_saturating(self) -> u16;
    /// Truncated and clamped to `u8`.
    fn to_u8_saturating(self) -> u8;
    /// Truncated and clamped to `usize`.
    fn to_usize_saturating(self) -> usize;
}

macro_rules! to_int_saturating {
    ($($ty:ty),*) => {$(
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "float-to-int `as` truncates and saturates, which is this trait's documented contract"
        )]
        impl ToIntSaturating for $ty {
            #[inline]
            fn to_i64_saturating(self) -> i64 {
                self as i64
            }
            #[inline]
            fn to_u64_saturating(self) -> u64 {
                self as u64
            }
            #[inline]
            fn to_i32_saturating(self) -> i32 {
                self as i32
            }
            #[inline]
            fn to_u32_saturating(self) -> u32 {
                self as u32
            }
            #[inline]
            fn to_i16_saturating(self) -> i16 {
                self as i16
            }
            #[inline]
            fn to_u16_saturating(self) -> u16 {
                self as u16
            }
            #[inline]
            fn to_u8_saturating(self) -> u8 {
                self as u8
            }
            #[inline]
            fn to_usize_saturating(self) -> usize {
                self as usize
            }
        }
    )*};
}

to_int_saturating!(f32, f64);

/// `x` clamped into `i16`: a PCM sample after mixing or averaging in `i32`.
pub fn clamp_to_i16(x: i32) -> i16 {
    i16::try_from(x).unwrap_or(if x < 0 { i16::MIN } else { i16::MAX })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_to_int_truncates_toward_zero_and_saturates() {
        assert_eq!(1.9_f64.to_i64_saturating(), 1);
        assert_eq!((-1.9_f64).to_i64_saturating(), -1);
        assert_eq!(f64::NAN.to_i64_saturating(), 0);
        assert_eq!(f64::INFINITY.to_i64_saturating(), i64::MAX);
        assert_eq!(f64::NEG_INFINITY.to_i64_saturating(), i64::MIN);
        assert_eq!(1e300_f64.to_i64_saturating(), i64::MAX);
        assert_eq!((-3.5_f64).to_u64_saturating(), 0);
        assert_eq!((-3.5_f32).to_usize_saturating(), 0);
        assert_eq!(70_000.0_f32.to_u16_saturating(), u16::MAX);
        assert_eq!(40_000.0_f32.to_i16_saturating(), i16::MAX);
        assert_eq!((-40_000.0_f32).to_i16_saturating(), i16::MIN);
        assert_eq!(300.0_f64.to_u8_saturating(), u8::MAX);
        assert_eq!(5e9_f64.to_u32_saturating(), u32::MAX);
        assert_eq!((-5e9_f64).to_i32_saturating(), i32::MIN);
        assert_eq!(2.5_f32.round().to_i32_saturating(), 3);
    }

    #[test]
    fn clamp_to_i16_saturates_at_both_ends() {
        assert_eq!(clamp_to_i16(0), 0);
        assert_eq!(clamp_to_i16(-12_345), -12_345);
        assert_eq!(clamp_to_i16(40_000), i16::MAX);
        assert_eq!(clamp_to_i16(-40_000), i16::MIN);
        assert_eq!(clamp_to_i16(i32::MIN), i16::MIN);
    }

    #[test]
    fn int_to_float_is_exact_in_range_and_rounds_beyond() {
        assert_eq!(16_777_216_usize.to_f32_lossy(), 16_777_216.0);
        assert_eq!(16_777_217_usize.to_f32_lossy(), 16_777_216.0);
        let max_safe = 9_007_199_254_740_991_i64;
        assert_eq!(max_safe.to_f64_lossy(), 9_007_199_254_740_991.0);
        assert_eq!((-max_safe).to_f64_lossy(), -9_007_199_254_740_991.0);
        assert_eq!(u64::MAX.to_f64_lossy(), 18_446_744_073_709_551_616.0);
        assert_eq!(u32::MAX.to_f64_lossy(), 4_294_967_295.0);
        assert_eq!(i32::MIN.to_f32_lossy(), -2_147_483_648.0);
        assert_eq!(0.1_f64.to_f32_lossy(), 0.1_f32);
    }
}
