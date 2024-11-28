use alloc::vec;
use alloc::vec::Vec;
use core::fmt;
use core::fmt::{Debug, Display, Formatter};
use core::hash::{Hash, Hasher};
use core::iter::{Product, Sum};
use core::mem::transmute;
use core::ops::{Add, AddAssign, Div, Mul, MulAssign, Neg, Sub, SubAssign};

use num_bigint::BigUint;
use p3_field::integers::QuotientMap;
use p3_field::{
    exp_1717986917, exp_u64_by_squaring, halve_u32, quotient_map_small_int, Field, FieldAlgebra,
    Packable, PrimeField, PrimeField32, PrimeField64,
};
use paste::paste;
use rand::distributions::{Distribution, Standard};
use rand::Rng;
use serde::{Deserialize, Serialize};

/// The Mersenne31 prime
const P: u32 = (1 << 31) - 1;

/// The prime field `F_p` where `p = 2^31 - 1`.
#[derive(Copy, Clone, Default, Serialize, Deserialize)]
#[repr(transparent)] // Packed field implementations rely on this!
pub struct Mersenne31 {
    /// Not necessarily canonical, but must fit in 31 bits.
    pub(crate) value: u32,
}

impl Mersenne31 {
    #[inline]
    pub const fn new(value: u32) -> Self {
        debug_assert!((value >> 31) == 0);
        Self { value }
    }
}

impl PartialEq for Mersenne31 {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_canonical_u32() == other.as_canonical_u32()
    }
}

impl Eq for Mersenne31 {}

impl Packable for Mersenne31 {}

impl Hash for Mersenne31 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u32(self.as_canonical_u32());
    }
}

impl Ord for Mersenne31 {
    #[inline]
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.as_canonical_u32().cmp(&other.as_canonical_u32())
    }
}

impl PartialOrd for Mersenne31 {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Display for Mersenne31 {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.value, f)
    }
}

impl Debug for Mersenne31 {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        Debug::fmt(&self.value, f)
    }
}

impl Distribution<Mersenne31> for Standard {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> Mersenne31 {
        loop {
            let next_u31 = rng.next_u32() >> 1;
            let is_canonical = next_u31 != Mersenne31::ORDER_U32;
            if is_canonical {
                return Mersenne31::new(next_u31);
            }
        }
    }
}

impl FieldAlgebra for Mersenne31 {
    type F = Self;
    type Char = Self;

    const ZERO: Self = Self { value: 0 };
    const ONE: Self = Self { value: 1 };
    const TWO: Self = Self { value: 2 };
    const NEG_ONE: Self = Self {
        value: Self::ORDER_U32 - 1,
    };

    #[inline]
    fn from_f(f: Self::F) -> Self {
        f
    }

    #[inline]
    fn from_char(f: Self::Char) -> Self {
        f
    }

    #[inline]
    fn from_bool(b: bool) -> Self {
        Self::new(b as u32)
    }

    #[inline]
    fn mul_2exp_u64(&self, exp: u64) -> Self {
        // In a Mersenne field, multiplication by 2^k is just a left rotation by k bits.
        let exp = exp % 31;
        let left = (self.value << exp) & ((1 << 31) - 1);
        let right = self.value >> (31 - exp);
        let rotated = left | right;
        Self::new(rotated)
    }

    #[inline]
    fn zero_vec(len: usize) -> Vec<Self> {
        // SAFETY: repr(transparent) ensures transmutation safety.
        unsafe { transmute(vec![0u32; len]) }
    }
}

impl Field for Mersenne31 {
    #[cfg(all(target_arch = "aarch64", target_feature = "neon"))]
    type Packing = crate::PackedMersenne31Neon;
    #[cfg(all(
        target_arch = "x86_64",
        target_feature = "avx2",
        not(all(feature = "nightly-features", target_feature = "avx512f"))
    ))]
    type Packing = crate::PackedMersenne31AVX2;
    #[cfg(all(
        feature = "nightly-features",
        target_arch = "x86_64",
        target_feature = "avx512f"
    ))]
    type Packing = crate::PackedMersenne31AVX512;
    #[cfg(not(any(
        all(target_arch = "aarch64", target_feature = "neon"),
        all(
            target_arch = "x86_64",
            target_feature = "avx2",
            not(all(feature = "nightly-features", target_feature = "avx512f"))
        ),
        all(
            feature = "nightly-features",
            target_arch = "x86_64",
            target_feature = "avx512f"
        ),
    )))]
    type Packing = Self;

    // Sage: GF(2^31 - 1).multiplicative_generator()
    const GENERATOR: Self = Self::new(7);

    #[inline]
    fn is_zero(&self) -> bool {
        self.value == 0 || self.value == Self::ORDER_U32
    }

    #[inline]
    fn div_2exp_u64(&self, exp: u64) -> Self {
        // In a Mersenne field, division by 2^k is just a right rotation by k bits.
        let exp = (exp % 31) as u8;
        let left = self.value >> exp;
        let right = (self.value << (31 - exp)) & ((1 << 31) - 1);
        let rotated = left | right;
        Self::new(rotated)
    }

    #[inline]
    fn exp_u64_generic<FA: FieldAlgebra<F = Self>>(val: FA, power: u64) -> FA {
        match power {
            1717986917 => exp_1717986917(val), // used in x^{1/5}
            _ => exp_u64_by_squaring(val, power),
        }
    }

    fn try_inverse(&self) -> Option<Self> {
        if self.is_zero() {
            return None;
        }

        // From Fermat's little theorem, in a prime field `F_p`, the inverse of `a` is `a^(p-2)`.
        // Here p-2 = 2147483645 = 1111111111111111111111111111101_2.
        // Uses 30 Squares + 7 Multiplications => 37 Operations total.

        let p1 = *self;
        let p101 = p1.exp_power_of_2(2) * p1;
        let p1111 = p101.square() * p101;
        let p11111111 = p1111.exp_power_of_2(4) * p1111;
        let p111111110000 = p11111111.exp_power_of_2(4);
        let p111111111111 = p111111110000 * p1111;
        let p1111111111111111 = p111111110000.exp_power_of_2(4) * p11111111;
        let p1111111111111111111111111111 = p1111111111111111.exp_power_of_2(12) * p111111111111;
        let p1111111111111111111111111111101 =
            p1111111111111111111111111111.exp_power_of_2(3) * p101;
        Some(p1111111111111111111111111111101)
    }

    #[inline]
    fn halve(&self) -> Self {
        Mersenne31::new(halve_u32::<P>(self.value))
    }

    #[inline]
    fn order() -> BigUint {
        P.into()
    }
}

/// This is a simple macro which lets us imply `QuotientMap<Int>`
/// for `Int = u64, u128, usize`.
///
/// In these cases the fastest approach is just to use `%p` to compute
/// a canonical input in the range `[0, p]`
macro_rules! large_u_int_m31 {
    ($($type:ty),* $(,)? ) => {
        $(
        impl QuotientMap<$type> for Mersenne31 {
            /// Convert a given integer into an element of the Mersenne31 field.
            ///
            /// For large integer types, we do a modular reduction.
            #[inline]
            fn from_int(int: $type) -> Mersenne31 {
                // NB: Experiments suggest that it's faster to just use the
                // builtin remainder operator rather than split the input into
                // 32-bit chunks and reduce using 2^32 = 2 (mod Mersenne31).
                let int_canonical = int % (Mersenne31::ORDER_U32 as $type);
                Self::new(int_canonical as u32)
            }

            /// Convert a given integer into an element of the Mersenne31 field.
            ///
            /// Returns None if the input is greater than `p - 1 = 2^31 - 2`.
            #[inline]
            fn from_canonical_checked(int: $type) -> Option<Mersenne31> {
                if int < Mersenne31::ORDER_U32 as $type {
                    Some(Self::new(int as u32))
                } else {
                    None
                }
            }

            /// Convert a given integer into an element of the Mersenne31 field.
            ///
            /// # Safety
            /// The input `int` must be `31` bits i.e. lie between `0` and `2^31 - 1`
            #[inline]
            unsafe fn from_canonical_unchecked(int: $type) -> Self {
                debug_assert!(int <= Mersenne31::ORDER_U32 as $type);
                Self::new(int as u32)
            }
        }
        )*
    };
}

/// This is a simple macro which lets us imply `QuotientMap<Int>`
/// for `Int = i64, i128, isize`.
///
/// In these cases the fastest approach is just to use `%p` to compute
/// an input in the range `[-p, p]` and then correct for the sign.
macro_rules! large_i_int_m31 {
    ($($type:ty),* $(,)? ) => {
        $(
        impl QuotientMap<$type> for Mersenne31 {
            /// For small integer types, the input value is always canonical
            /// once we correct for the sign.
            #[inline]
            fn from_int(int: $type) -> Mersenne31 {
                let int_i32 = (int % (Mersenne31::ORDER_U32 as $type)) as i32;
                // Now as `int_i32` must lie in `[-p, p]` we can safely use `from_canonical_unchecked`.
                unsafe { Self::from_canonical_unchecked(int_i32 as i32) }

            }

            /// Convert a given integer into an element of the Mersenne31 field.
            ///
            /// Returns none if the input does not lie in the range `[-2^30, 2^30]`.
            #[inline]
            fn from_canonical_checked(int: $type) -> Option<Mersenne31> {
                const TWO_EXP_30: $type = 1 << 30;
                const NEG_TWO_EXP_30: $type = -1 << 30;
                match int {
                    0..=TWO_EXP_30 => Some(Self::new(int as u32)),
                    NEG_TWO_EXP_30..0 => Some(Self::new(Mersenne31::ORDER_U32.wrapping_add_signed(int as i32))),
                    _ => None,
                }
            }

            /// Convert a given integer into an element of the Mersenne31 field.
            ///
            /// # Safety
            /// The input `int` must be between `-(2^31 - 1)` and `2^31 - 1`.
            #[inline]
            unsafe fn from_canonical_unchecked(int: $type) -> Mersenne31 {
                Self::from_canonical_unchecked(int as i32)
            }
        }
        )*
    };
}

quotient_map_small_int!(Mersenne31, u32, [u8, u16]);
quotient_map_small_int!(Mersenne31, i32, [i8, i16]);

impl QuotientMap<u32> for Mersenne31 {
    #[inline]
    fn from_int(int: u32) -> Self {
        // To reduce `n` to 31 bits, we clear its MSB, then add it back in its reduced form.
        let msb = int & (1 << 31);
        let msb_reduced = msb >> 31;
        Self::new(int ^ msb) + Self::new(msb_reduced)
    }

    /// Convert a given integer into an element of the Mersenne31 field.
    ///
    /// Returns none if the input is greater than `2^31 - 1`.
    #[inline]
    fn from_canonical_checked(int: u32) -> Option<Mersenne31> {
        if int < Self::ORDER_U32 {
            Some(Self::new(int))
        } else {
            None
        }
    }

    /// Convert a given integer into an element of the Mersenne31 field.
    ///
    /// # Safety
    /// The input `int` must be `31` bits i.e. lie between `0` and `2^31 - 1`
    #[inline]
    unsafe fn from_canonical_unchecked(int: u32) -> Mersenne31 {
        debug_assert!(int < Self::ORDER_U32);
        Self::new(int)
    }
}

impl QuotientMap<i32> for Mersenne31 {
    /// Convert a given integer into an element of the Mersenne31 field.
    #[inline]
    fn from_int(int: i32) -> Self {
        if int >= 0 {
            Self::new(int as u32)
        } else if int > (-1 << 31) {
            Self::new(Mersenne31::ORDER_U32.wrapping_add_signed(int))
        } else {
            // The only other option is int = -(2^31) = -1 mod p.
            Self::NEG_ONE
        }
    }

    /// Convert a given integer into an element of the Mersenne31 field.
    ///
    /// Returns none if the input does not lie in the range `[-2^30, 2^30]`.
    #[inline]
    fn from_canonical_checked(int: i32) -> Option<Mersenne31> {
        const TWO_EXP_30: i32 = 1 << 30;
        const NEG_TWO_EXP_30: i32 = -1 << 30;
        match int {
            0..=TWO_EXP_30 => Some(Self::new(int as u32)),
            NEG_TWO_EXP_30..0 => Some(Self::new(Mersenne31::ORDER_U32.wrapping_add_signed(int))),
            _ => None,
        }
    }

    /// Convert a given integer into an element of the Mersenne31 field.
    ///
    /// # Safety
    /// The input `int` must be between `-(2^31 - 1)` and `2^31 - 1`.
    #[inline]
    unsafe fn from_canonical_unchecked(int: i32) -> Mersenne31 {
        if int >= 0 {
            Self::new(int as u32)
        } else {
            Self::new(Mersenne31::ORDER_U32.wrapping_add_signed(int))
        }
    }
}

large_u_int_m31!(u64, u128, usize);
large_i_int_m31!(i64, i128, isize);

impl PrimeField for Mersenne31 {
    fn as_canonical_biguint(&self) -> BigUint {
        <Self as PrimeField32>::as_canonical_u32(self).into()
    }
}

impl PrimeField32 for Mersenne31 {
    const ORDER_U32: u32 = P;

    #[inline]
    fn as_canonical_u32(&self) -> u32 {
        // Since our invariant guarantees that `value` fits in 31 bits, there is only one possible
        // `value` that is not canonical, namely 2^31 - 1 = p = 0.
        if self.value == Self::ORDER_U32 {
            0
        } else {
            self.value
        }
    }
}

impl PrimeField64 for Mersenne31 {
    const ORDER_U64: u64 = <Self as PrimeField32>::ORDER_U32 as u64;

    #[inline]
    fn as_canonical_u64(&self) -> u64 {
        self.as_canonical_u32().into()
    }
}

impl Add for Mersenne31 {
    type Output = Self;

    #[inline]
    fn add(self, rhs: Self) -> Self {
        // See the following for a way to compute the sum that avoids
        // the conditional which may be preferable on some
        // architectures.
        // https://github.com/Plonky3/Plonky3/blob/6049a30c3b1f5351c3eb0f7c994dc97e8f68d10d/mersenne-31/src/lib.rs#L249

        // Working with i32 means we get a flag which informs us if overflow happened.
        let (sum_i32, over) = (self.value as i32).overflowing_add(rhs.value as i32);
        let sum_u32 = sum_i32 as u32;
        let sum_corr = sum_u32.wrapping_sub(Self::ORDER_U32);

        // If self + rhs did not overflow, return it.
        // If self + rhs overflowed, sum_corr = self + rhs - (2**31 - 1).
        Self::new(if over { sum_corr } else { sum_u32 })
    }
}

impl AddAssign for Mersenne31 {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl Sum for Mersenne31 {
    #[inline]
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        // This is faster than iter.reduce(|x, y| x + y).unwrap_or(Self::ZERO) for iterators of length >= 6.
        // It assumes that iter.len() < 2^31.

        // This sum will not overflow so long as iter.len() < 2^33.
        let sum = iter.map(|x| x.value as u64).sum::<u64>();

        // sum is < 2^62 provided iter.len() < 2^31.
        from_u62(sum)
    }
}

impl Sub for Mersenne31 {
    type Output = Self;

    #[inline]
    fn sub(self, rhs: Self) -> Self {
        let (mut sub, over) = self.value.overflowing_sub(rhs.value);

        // If we didn't overflow we have the correct value.
        // Otherwise we have added 2**32 = 2**31 + 1 mod 2**31 - 1.
        // Hence we need to remove the most significant bit and subtract 1.
        sub -= over as u32;
        Self::new(sub & Self::ORDER_U32)
    }
}

impl SubAssign for Mersenne31 {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl Neg for Mersenne31 {
    type Output = Self;

    #[inline]
    fn neg(self) -> Self::Output {
        // Can't underflow, since self.value is 31-bits and thus can't exceed ORDER.
        Self::new(Self::ORDER_U32 - self.value)
    }
}

impl Mul for Mersenne31 {
    type Output = Self;

    #[inline]
    #[allow(clippy::cast_possible_truncation)]
    fn mul(self, rhs: Self) -> Self {
        let prod = u64::from(self.value) * u64::from(rhs.value);
        from_u62(prod)
    }
}

impl MulAssign for Mersenne31 {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}

impl Product for Mersenne31 {
    #[inline]
    fn product<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.reduce(|x, y| x * y).unwrap_or(Self::ONE)
    }
}

impl Div for Mersenne31 {
    type Output = Self;

    #[inline]
    #[allow(clippy::suspicious_arithmetic_impl)]
    fn div(self, rhs: Self) -> Self {
        self * rhs.inverse()
    }
}

#[inline(always)]
pub(crate) fn from_u62(input: u64) -> Mersenne31 {
    debug_assert!(input < (1 << 62));
    let input_lo = (input & ((1 << 31) - 1)) as u32;
    let input_high = (input >> 31) as u32;
    Mersenne31::new(input_lo) + Mersenne31::new(input_high)
}

/// Convert a constant u32 array into a constant Mersenne31 array.
#[inline]
#[must_use]
pub const fn to_mersenne31_array<const N: usize>(input: [u32; N]) -> [Mersenne31; N] {
    // This is currently used only in the test crates of the vectorized implementations.
    let mut output = [Mersenne31 { value: 0 }; N];
    let mut i = 0;
    loop {
        if i == N {
            break;
        }
        output[i].value = input[i] % P;
        i += 1;
    }
    output
}

#[cfg(test)]
mod tests {
    use p3_field::{Field, FieldAlgebra, PrimeField32};
    use p3_field_testing::test_field;

    use crate::Mersenne31;

    type F = Mersenne31;

    #[test]
    fn add() {
        assert_eq!(F::ONE + F::ONE, F::TWO);
        assert_eq!(F::NEG_ONE + F::ONE, F::ZERO);
        assert_eq!(F::NEG_ONE + F::TWO, F::ONE);
        assert_eq!(F::NEG_ONE + F::NEG_ONE, F::new(F::ORDER_U32 - 2));
    }

    #[test]
    fn sub() {
        assert_eq!(F::ONE - F::ONE, F::ZERO);
        assert_eq!(F::TWO - F::TWO, F::ZERO);
        assert_eq!(F::NEG_ONE - F::NEG_ONE, F::ZERO);
        assert_eq!(F::TWO - F::ONE, F::ONE);
        assert_eq!(F::NEG_ONE - F::ZERO, F::NEG_ONE);
    }

    #[test]
    fn mul_2exp_u64() {
        // 1 * 2^0 = 1.
        assert_eq!(F::ONE.mul_2exp_u64(0), F::ONE);
        // 2 * 2^30 = 2^31 = 1.
        assert_eq!(F::TWO.mul_2exp_u64(30), F::ONE);
        // 5 * 2^2 = 20.
        assert_eq!(F::new(5).mul_2exp_u64(2), F::new(20));
    }

    #[test]
    fn div_2exp_u64() {
        // 1 / 2^0 = 1.
        assert_eq!(F::ONE.div_2exp_u64(0), F::ONE);
        // 2 / 2^0 = 2.
        assert_eq!(F::TWO.div_2exp_u64(0), F::TWO);
        // 32 / 2^5 = 1.
        assert_eq!(F::new(32).div_2exp_u64(5), F::new(1));
    }

    #[test]
    fn exp_root() {
        // Confirm that (x^{1/5})^5 = x

        let m1 = F::from_u32(0x34167c58);
        let m2 = F::from_u32(0x61f3207b);

        assert_eq!(m1.exp_u64(1717986917).exp_const_u64::<5>(), m1);
        assert_eq!(m2.exp_u64(1717986917).exp_const_u64::<5>(), m2);
        assert_eq!(F::TWO.exp_u64(1717986917).exp_const_u64::<5>(), F::TWO);
    }

    test_field!(crate::Mersenne31);
}
