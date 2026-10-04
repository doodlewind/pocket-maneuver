//! Scalar f32 math. Transcendentals go through `libm` so the wasm build, the
//! host build and the Vita build produce the same bits for the same inputs.
//!
//! Two features change that for small machines. `hw-sqrt` takes the square
//! root from the FPU (the same bits as `libm`, without its loop).
//! `single-float` swaps the transcendentals for kernels that never use `f64`
//! (`fastmath`): the results differ from `libm` in the last places, so such a
//! build repeats against itself only.

use core::ops::{Add, AddAssign, Mul, MulAssign, Neg, Sub, SubAssign};

pub const PI: f32 = core::f32::consts::PI;
pub const TAU: f32 = core::f32::consts::TAU;

#[cfg(not(feature = "single-float"))]
mod kernel {
    #[inline]
    pub fn sin(x: f32) -> f32 {
        libm::sinf(x)
    }
    #[inline]
    pub fn cos(x: f32) -> f32 {
        libm::cosf(x)
    }
    #[inline]
    pub fn tan(x: f32) -> f32 {
        libm::tanf(x)
    }
    #[inline]
    pub fn atan2(y: f32, x: f32) -> f32 {
        libm::atan2f(y, x)
    }
    #[inline]
    pub fn asin(x: f32) -> f32 {
        libm::asinf(x)
    }
    #[inline]
    pub fn acos(x: f32) -> f32 {
        libm::acosf(x)
    }
    #[inline]
    pub fn exp(x: f32) -> f32 {
        libm::expf(x)
    }
}

#[cfg(feature = "single-float")]
mod kernel {
    pub use crate::fastmath::{atan2, cos, exp, sin, tan};
    #[inline]
    pub fn asin(x: f32) -> f32 {
        crate::fastmath::asin(x, super::sqrt)
    }
    #[inline]
    pub fn acos(x: f32) -> f32 {
        core::f32::consts::FRAC_PI_2 - asin(x)
    }
}

#[inline]
pub fn sin(x: f32) -> f32 {
    kernel::sin(x)
}
#[inline]
pub fn cos(x: f32) -> f32 {
    kernel::cos(x)
}
#[inline]
pub fn tan(x: f32) -> f32 {
    kernel::tan(x)
}
#[inline]
pub fn atan2(y: f32, x: f32) -> f32 {
    kernel::atan2(y, x)
}
#[inline]
pub fn asin(x: f32) -> f32 {
    kernel::asin(clamp(x, -1.0, 1.0))
}
#[cfg(not(feature = "hw-sqrt"))]
#[inline]
pub fn sqrt(x: f32) -> f32 {
    libm::sqrtf(x)
}
#[cfg(feature = "hw-sqrt")]
#[inline]
#[allow(unused_unsafe)]
pub fn sqrt(x: f32) -> f32 {
    // Safe on current compilers, `unsafe` on older ones.
    unsafe { core::intrinsics::sqrtf32(x) }
}
#[inline]
pub fn exp(x: f32) -> f32 {
    kernel::exp(x)
}
#[inline]
pub fn floor(x: f32) -> f32 {
    libm::floorf(x)
}
#[inline]
pub fn abs(x: f32) -> f32 {
    libm::fabsf(x)
}
#[inline]
pub fn clamp(x: f32, lo: f32, hi: f32) -> f32 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}
#[inline]
pub fn min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}
#[inline]
pub fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
#[inline]
pub fn saturate(x: f32) -> f32 {
    clamp(x, 0.0, 1.0)
}
#[inline]
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = saturate((x - e0) / (e1 - e0));
    t * t * (3.0 - 2.0 * t)
}
/// Wraps an angle to (-PI, PI].
#[inline]
pub fn wrap_angle(a: f32) -> f32 {
    let mut a = a - TAU * floor((a + PI) / TAU);
    if a <= -PI {
        a += TAU;
    }
    a
}
/// Moves `cur` toward `target` by at most `step`.
#[inline]
pub fn approach(cur: f32, target: f32, step: f32) -> f32 {
    if cur < target {
        min(cur + step, target)
    } else {
        max(cur - step, target)
    }
}
/// Exponential ease of `cur` toward `target`: `rate` per second, frame-rate independent.
#[inline]
pub fn ease(cur: f32, target: f32, rate: f32, dt: f32) -> f32 {
    target + (cur - target) * exp(-rate * dt)
}
#[inline]
pub fn ease_angle(cur: f32, target: f32, rate: f32, dt: f32) -> f32 {
    let d = wrap_angle(target - cur);
    wrap_angle(cur + d * (1.0 - exp(-rate * dt)))
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[inline]
pub const fn v3(x: f32, y: f32, z: f32) -> V3 {
    V3 { x, y, z }
}

impl V3 {
    pub const ZERO: V3 = v3(0.0, 0.0, 0.0);
    pub const UP: V3 = v3(0.0, 1.0, 0.0);

    #[inline]
    pub fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    #[inline]
    pub fn cross(self, o: V3) -> V3 {
        v3(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }
    #[inline]
    pub fn len2(self) -> f32 {
        self.dot(self)
    }
    #[inline]
    pub fn len(self) -> f32 {
        sqrt(self.len2())
    }
    /// Unit vector, or `fallback` for a (near) zero vector.
    #[inline]
    pub fn norm_or(self, fallback: V3) -> V3 {
        let l = self.len();
        if l > 1e-6 {
            self * (1.0 / l)
        } else {
            fallback
        }
    }
    #[inline]
    pub fn norm(self) -> V3 {
        self.norm_or(V3::ZERO)
    }
    #[inline]
    pub fn lerp(self, o: V3, t: f32) -> V3 {
        self + (o - self) * t
    }
    /// The horizontal part.
    #[inline]
    pub fn flat(self) -> V3 {
        v3(self.x, 0.0, self.z)
    }
    #[inline]
    pub fn ease(self, target: V3, rate: f32, dt: f32) -> V3 {
        target + (self - target) * exp(-rate * dt)
    }
}

impl Add for V3 {
    type Output = V3;
    #[inline]
    fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl Sub for V3 {
    type Output = V3;
    #[inline]
    fn sub(self, o: V3) -> V3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Mul<f32> for V3 {
    type Output = V3;
    #[inline]
    fn mul(self, s: f32) -> V3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
}
impl Neg for V3 {
    type Output = V3;
    #[inline]
    fn neg(self) -> V3 {
        v3(-self.x, -self.y, -self.z)
    }
}
impl AddAssign for V3 {
    #[inline]
    fn add_assign(&mut self, o: V3) {
        *self = *self + o;
    }
}
impl SubAssign for V3 {
    #[inline]
    fn sub_assign(&mut self, o: V3) {
        *self = *self - o;
    }
}
impl MulAssign<f32> for V3 {
    #[inline]
    fn mul_assign(&mut self, s: f32) {
        *self = *self * s;
    }
}

/// Heading convention: yaw 0 looks down -Z, positive yaw turns left (toward -X),
/// matching a three.js camera rotated about +Y.
#[inline]
pub fn heading(yaw: f32) -> V3 {
    v3(-sin(yaw), 0.0, -cos(yaw))
}
/// Unit forward vector for a yaw and a pitch (positive pitch looks up).
#[inline]
pub fn forward(yaw: f32, pitch: f32) -> V3 {
    let c = cos(pitch);
    v3(-sin(yaw) * c, sin(pitch), -cos(yaw) * c)
}
/// The yaw whose heading is the horizontal part of `d`.
#[inline]
pub fn yaw_of(d: V3) -> f32 {
    atan2(-d.x, -d.z)
}

/// Rotation as three column vectors.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct M3 {
    pub x: V3,
    pub y: V3,
    pub z: V3,
}

impl M3 {
    pub const ID: M3 = M3 { x: v3(1.0, 0.0, 0.0), y: v3(0.0, 1.0, 0.0), z: v3(0.0, 0.0, 1.0) };

    pub fn rot_x(a: f32) -> M3 {
        let (s, c) = (sin(a), cos(a));
        M3 { x: v3(1.0, 0.0, 0.0), y: v3(0.0, c, s), z: v3(0.0, -s, c) }
    }
    pub fn rot_y(a: f32) -> M3 {
        let (s, c) = (sin(a), cos(a));
        M3 { x: v3(c, 0.0, -s), y: v3(0.0, 1.0, 0.0), z: v3(s, 0.0, c) }
    }
    pub fn rot_z(a: f32) -> M3 {
        let (s, c) = (sin(a), cos(a));
        M3 { x: v3(c, s, 0.0), y: v3(-s, c, 0.0), z: v3(0.0, 0.0, 1.0) }
    }
    #[inline]
    pub fn apply(&self, v: V3) -> V3 {
        self.x * v.x + self.y * v.y + self.z * v.z
    }
    pub fn mul(&self, o: &M3) -> M3 {
        M3 { x: self.apply(o.x), y: self.apply(o.y), z: self.apply(o.z) }
    }
    /// Basis with `-z` along `fwd` and `y` as close to `up` as possible.
    pub fn look(fwd: V3, up: V3) -> M3 {
        let z = (-fwd).norm_or(v3(0.0, 0.0, 1.0));
        let x = up.cross(z).norm_or(v3(1.0, 0.0, 0.0));
        let y = z.cross(x);
        M3 { x, y, z }
    }
}

/// Rigid transform: rotation and translation.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct M34 {
    pub r: M3,
    pub t: V3,
}

impl M34 {
    pub const ID: M34 = M34 { r: M3::ID, t: V3::ZERO };
    #[inline]
    pub fn new(r: M3, t: V3) -> M34 {
        M34 { r, t }
    }
    #[inline]
    pub fn apply(&self, v: V3) -> V3 {
        self.r.apply(v) + self.t
    }
    /// `self * o`: `o` is applied first.
    pub fn mul(&self, o: &M34) -> M34 {
        M34 { r: self.r.mul(&o.r), t: self.apply(o.t) }
    }
}

impl M3 {
    pub fn transpose(&self) -> M3 {
        M3 { x: v3(self.x.x, self.y.x, self.z.x), y: v3(self.x.y, self.y.y, self.z.y), z: v3(self.x.z, self.y.z, self.z.z) }
    }
    pub fn scaled(&self, s: f32) -> M3 {
        M3 { x: self.x * s, y: self.y * s, z: self.z * s }
    }
}

impl M34 {
    /// Inverse of a rigid transform (rotation and translation, no scale).
    pub fn inverse_rigid(&self) -> M34 {
        let r = self.r.transpose();
        M34 { r, t: -r.apply(self.t) }
    }
}

/// Unit quaternion. Bone rotations blend as quaternions and convert to `M3` for kinematics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const ID: Quat = Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };

    pub fn axis_angle(axis: V3, a: f32) -> Quat {
        let (s, c) = (sin(a * 0.5), cos(a * 0.5));
        Quat { x: axis.x * s, y: axis.y * s, z: axis.z * s, w: c }
    }
    /// `Rz(z) · Rx(x) · Ry(y)`: spread, then swing, then twist.
    pub fn euler(x: f32, y: f32, z: f32) -> Quat {
        Quat::axis_angle(v3(0.0, 0.0, 1.0), z).mul(Quat::axis_angle(v3(1.0, 0.0, 0.0), x)).mul(Quat::axis_angle(v3(0.0, 1.0, 0.0), y))
    }
    /// The shortest rotation that takes unit `a` to unit `b`.
    pub fn from_to(a: V3, b: V3) -> Quat {
        let d = a.dot(b);
        if d < -0.9999 {
            let axis = a.cross(v3(1.0, 0.0, 0.0));
            let axis = if axis.len2() < 1e-6 { a.cross(v3(0.0, 0.0, 1.0)) } else { axis };
            return Quat::axis_angle(axis.norm(), PI);
        }
        let c = a.cross(b);
        Quat { x: c.x, y: c.y, z: c.z, w: 1.0 + d }.normalized()
    }
    #[allow(clippy::should_implement_trait)]
    pub fn mul(self, o: Quat) -> Quat {
        Quat {
            x: self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            y: self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            z: self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
            w: self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
        }
    }
    pub fn normalized(self) -> Quat {
        let l = sqrt(self.x * self.x + self.y * self.y + self.z * self.z + self.w * self.w);
        if l < 1e-9 {
            return Quat::ID;
        }
        let k = 1.0 / l;
        Quat { x: self.x * k, y: self.y * k, z: self.z * k, w: self.w * k }
    }
    /// Normalized linear blend along the shorter arc.
    pub fn nlerp(self, o: Quat, t: f32) -> Quat {
        let s = if self.x * o.x + self.y * o.y + self.z * o.z + self.w * o.w < 0.0 { -1.0 } else { 1.0 };
        Quat { x: lerp(self.x, o.x * s, t), y: lerp(self.y, o.y * s, t), z: lerp(self.z, o.z * s, t), w: lerp(self.w, o.w * s, t) }.normalized()
    }
    pub fn m3(self) -> M3 {
        let (x, y, z, w) = (self.x, self.y, self.z, self.w);
        M3 {
            x: v3(1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + z * w), 2.0 * (x * z - y * w)),
            y: v3(2.0 * (x * y - z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + x * w)),
            z: v3(2.0 * (x * z + y * w), 2.0 * (y * z - x * w), 1.0 - 2.0 * (x * x + y * y)),
        }
    }
    pub fn rotate(self, v: V3) -> V3 {
        self.m3().apply(v)
    }
    pub fn conj(self) -> Quat {
        Quat { x: -self.x, y: -self.y, z: -self.z, w: self.w }
    }
}

/// A damped spring toward zero that takes velocity kicks: follow-through after a jolt.
#[derive(Clone, Copy, Debug, Default)]
pub struct Spring {
    pub x: f32,
    pub v: f32,
}

impl Spring {
    /// Advances `dt` with angular frequency `w` and damping ratio `z`, pulled toward `target`.
    pub fn step(&mut self, target: f32, w: f32, z: f32, dt: f32) {
        let a = (target - self.x) * w * w - self.v * 2.0 * z * w;
        self.v += a * dt;
        self.x += self.v * dt;
    }
}

impl M3 {
    /// The rotation as a quaternion (the matrix must be orthonormal).
    pub fn quat(&self) -> Quat {
        let (m00, m11, m22) = (self.x.x, self.y.y, self.z.z);
        let trace = m00 + m11 + m22;
        let q = if trace > 0.0 {
            let s = sqrt(trace + 1.0) * 2.0;
            Quat { w: 0.25 * s, x: (self.y.z - self.z.y) / s, y: (self.z.x - self.x.z) / s, z: (self.x.y - self.y.x) / s }
        } else if m00 > m11 && m00 > m22 {
            let s = sqrt(1.0 + m00 - m11 - m22) * 2.0;
            Quat { w: (self.y.z - self.z.y) / s, x: 0.25 * s, y: (self.y.x + self.x.y) / s, z: (self.z.x + self.x.z) / s }
        } else if m11 > m22 {
            let s = sqrt(1.0 + m11 - m00 - m22) * 2.0;
            Quat { w: (self.z.x - self.x.z) / s, x: (self.y.x + self.x.y) / s, y: 0.25 * s, z: (self.z.y + self.y.z) / s }
        } else {
            let s = sqrt(1.0 + m22 - m00 - m11) * 2.0;
            Quat { w: (self.x.y - self.y.x) / s, x: (self.z.x + self.x.z) / s, y: (self.z.y + self.y.z) / s, z: 0.25 * s }
        };
        q.normalized()
    }
}

#[inline]
pub fn acos(x: f32) -> f32 {
    kernel::acos(clamp(x, -1.0, 1.0))
}
