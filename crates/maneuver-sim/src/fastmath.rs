//! Transcendentals in single precision only (the `single-float` feature).
//!
//! `libm`'s `sinf`, `cosf` and friends compute in `f64`. The PSP's FPU has no
//! doubles, so there each call runs a software float routine. These kernels
//! (the Cephes single-precision ones) stay in `f32` and are within a few units
//! in the last place of `libm`. A build that uses them is repeatable against
//! itself, not bit-identical to the reference.

// The coefficients are written as published, to more digits than an f32 holds.
#![allow(clippy::excessive_precision)]

use core::f32::consts::FRAC_2_PI;

// π/2 in three parts, each exact in f32 times a small integer.
const DP1: f32 = 1.570_312_5;
const DP2: f32 = 4.837_512_969_970_703e-4;
const DP3: f32 = 7.549_789_948_768_648e-8;

#[inline]
fn reduce(x: f32) -> (i32, f32) {
    let k = x * FRAC_2_PI;
    let n = (if k >= 0.0 { k + 0.5 } else { k - 0.5 }) as i32;
    let f = n as f32;
    (n, ((x - f * DP1) - f * DP2) - f * DP3)
}

#[inline]
fn sin_k(r: f32) -> f32 {
    let z = r * r;
    r + r * z * (-1.666_665_461_1e-1 + z * (8.332_160_873_6e-3 + z * -1.951_529_589_1e-4))
}

#[inline]
fn cos_k(r: f32) -> f32 {
    let z = r * r;
    1.0 - 0.5 * z + z * z * (4.166_664_568_298_827e-2 + z * (-1.388_731_625_493_765e-3 + z * 2.443_315_711_809_948e-5))
}

pub fn sin(x: f32) -> f32 {
    let (n, r) = reduce(x);
    match n & 3 {
        0 => sin_k(r),
        1 => cos_k(r),
        2 => -sin_k(r),
        _ => -cos_k(r),
    }
}

pub fn cos(x: f32) -> f32 {
    let (n, r) = reduce(x);
    match n & 3 {
        0 => cos_k(r),
        1 => -sin_k(r),
        2 => -cos_k(r),
        _ => sin_k(r),
    }
}

pub fn tan(x: f32) -> f32 {
    let (n, r) = reduce(x);
    let (s, c) = (sin_k(r), cos_k(r));
    if n & 1 == 0 {
        s / c
    } else {
        -c / s
    }
}

/// Arc tangent of a non-negative argument.
fn atan_pos(x: f32) -> f32 {
    let (y0, t) = if x > 2.414_213_562_373_095 {
        (core::f32::consts::FRAC_PI_2, -1.0 / x)
    } else if x > 0.414_213_562_373_095_03 {
        (core::f32::consts::FRAC_PI_4, (x - 1.0) / (x + 1.0))
    } else {
        (0.0, x)
    };
    let z = t * t;
    y0 + ((((8.053_744_495_38e-2 * z - 1.387_768_560_32e-1) * z + 1.997_771_064_78e-1) * z - 3.333_294_915_39e-1) * z * t + t)
}

pub fn atan2(y: f32, x: f32) -> f32 {
    use core::f32::consts::{FRAC_PI_2, PI};
    if x == 0.0 {
        return if y > 0.0 {
            FRAC_PI_2
        } else if y < 0.0 {
            -FRAC_PI_2
        } else {
            0.0
        };
    }
    let a = atan_pos(if y / x < 0.0 { -(y / x) } else { y / x });
    match (x > 0.0, y >= 0.0) {
        (true, true) => a,
        (true, false) => -a,
        (false, true) => PI - a,
        (false, false) => a - PI,
    }
}

/// Arc sine of `x` in [-1, 1].
pub fn asin(x: f32, sqrt: fn(f32) -> f32) -> f32 {
    let a = if x < 0.0 { -x } else { x };
    let p = |z: f32| ((((4.216_319_904_8e-2 * z + 2.418_131_104_9e-2) * z + 4.547_002_599_8e-2) * z + 7.495_300_268_6e-2) * z + 1.666_675_242_2e-1) * z;
    let r = if a > 0.5 {
        let z = 0.5 * (1.0 - a);
        let s = sqrt(z);
        core::f32::consts::FRAC_PI_2 - 2.0 * (s + s * p(z))
    } else {
        a + a * p(a * a)
    };
    if x < 0.0 {
        -r
    } else {
        r
    }
}

pub fn exp(x: f32) -> f32 {
    if x > 88.0 {
        return f32::INFINITY;
    }
    if x < -87.0 {
        return 0.0;
    }
    let k = x * core::f32::consts::LOG2_E;
    let n = (if k >= 0.0 { k + 0.5 } else { k - 0.5 }) as i32;
    let f = n as f32;
    let r = (x - f * 0.693_359_375) - f * -2.121_944_400_546_905_8e-4;
    let z = r * r;
    let p = (((((1.987_569_150_0e-4 * r + 1.398_199_950_7e-3) * r + 8.333_451_907_3e-3) * r + 4.166_579_589_4e-2) * r + 1.666_666_545_9e-1) * r + 5.000_000_120_1e-1) * z + r + 1.0;
    p * f32::from_bits(((n + 127) as u32) << 23)
}

#[cfg(test)]
mod tests {
    fn sweep(lo: f32, hi: f32, n: usize, f: impl Fn(f32) -> f32, g: impl Fn(f32) -> f32, tol: f32) {
        for i in 0..=n {
            let x = lo + (hi - lo) * i as f32 / n as f32;
            let (a, b) = (f(x), g(x));
            assert!((a - b).abs() <= tol * b.abs().max(1.0), "x {x}: {a} against {b}");
        }
    }

    #[test]
    fn kernels_agree_with_libm() {
        sweep(-40.0, 40.0, 40_000, super::sin, libm::sinf, 4e-6);
        sweep(-40.0, 40.0, 40_000, super::cos, libm::cosf, 4e-6);
        sweep(-1.5, 1.5, 4_000, super::tan, libm::tanf, 2e-5);
        sweep(-1.0, 1.0, 4_000, |x| super::asin(x, libm::sqrtf), libm::asinf, 4e-6);
        sweep(-20.0, 20.0, 4_000, super::exp, libm::expf, 4e-6);
        for i in 0..360 {
            let a = (i as f32 + 0.25).to_radians() - core::f32::consts::PI;
            let (y, x) = (libm::sinf(a) * 3.0, libm::cosf(a) * 3.0);
            assert!((super::atan2(y, x) - libm::atan2f(y, x)).abs() < 4e-6, "angle {a}");
        }
    }
}
