//! 4 × 4 matrices in row-major order: `m[r * 4 + c]`, applied to column vectors.

use maneuver_sim::math::{cos, sin, sqrt, tan, V3};

pub type Mat4 = [f32; 16];

pub const IDENTITY: Mat4 = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut o = [0.0f32; 16];
    for r in 0..4 {
        for c in 0..4 {
            o[r * 4 + c] = a[r * 4] * b[c] + a[r * 4 + 1] * b[4 + c] + a[r * 4 + 2] * b[8 + c] + a[r * 4 + 3] * b[12 + c];
        }
    }
    o
}

/// Perspective looking down -Z with clip depth 0 at `near` and `w` at `far`.
pub fn perspective(fov_y_deg: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / tan(fov_y_deg * (core::f32::consts::PI / 180.0) * 0.5);
    let a = far / (near - far);
    [f / aspect, 0.0, 0.0, 0.0, 0.0, f, 0.0, 0.0, 0.0, 0.0, a, a * near, 0.0, 0.0, -1.0, 0.0]
}

/// The same frustum with clip depth from `-w` at `near` to `w` at `far` (the PSP's GE).
pub fn perspective_gl(fov_y_deg: f32, aspect: f32, near: f32, far: f32) -> Mat4 {
    let f = 1.0 / tan(fov_y_deg * (core::f32::consts::PI / 180.0) * 0.5);
    [f / aspect, 0.0, 0.0, 0.0, 0.0, f, 0.0, 0.0, 0.0, 0.0, (far + near) / (near - far), 2.0 * far * near / (near - far), 0.0, 0.0, -1.0, 0.0]
}

/// View matrix for an eye looking along the unit vector `look`, rolled about it.
pub fn view(eye: V3, look: V3, roll: f32) -> Mat4 {
    let right = look.cross(V3::UP).norm_or(V3 { x: 1.0, y: 0.0, z: 0.0 });
    let up = right.cross(look);
    let (s, c) = (sin(roll), cos(roll));
    let r = right * c + up * s;
    let u = up * c - right * s;
    [r.x, r.y, r.z, -r.dot(eye), u.x, u.y, u.z, -u.dot(eye), -look.x, -look.y, -look.z, look.dot(eye), 0.0, 0.0, 0.0, 1.0]
}

/// `m × T(t) × S(s)`: folds a mesh's dequantization into a matrix.
pub fn with_bounds(m: &Mat4, t: [f32; 3], s: [f32; 3]) -> Mat4 {
    let mut o = [0.0f32; 16];
    for r in 0..4 {
        let (a, b, c, d) = (m[r * 4], m[r * 4 + 1], m[r * 4 + 2], m[r * 4 + 3]);
        o[r * 4] = a * s[0];
        o[r * 4 + 1] = b * s[1];
        o[r * 4 + 2] = c * s[2];
        o[r * 4 + 3] = a * t[0] + b * t[1] + c * t[2] + d;
    }
    o
}

/// `m × T(t)`.
pub fn translated(m: &Mat4, t: V3) -> Mat4 {
    with_bounds(m, [t.x, t.y, t.z], [1.0, 1.0, 1.0])
}

/// Scale and offset that take a mesh's stored `i16` positions (as the device
/// normalizes them: `q / divisor`) to world space.
pub fn dequant(min: &[f32; 3], max: &[f32; 3], divisor: f32) -> ([f32; 3], [f32; 3]) {
    let mut s = [0.0; 3];
    let mut t = [0.0; 3];
    for a in 0..3 {
        let ext = max[a] - min[a];
        s[a] = ext * (divisor / 65535.0);
        t[a] = min[a] + ext * (32768.0 / 65535.0);
    }
    (s, t)
}

/// Six planes `(n, d)` with `n·p + d >= 0` inside, from `perspective × view`.
pub fn planes(m: &Mat4) -> [[f32; 4]; 6] {
    let row = |i: usize| [m[i * 4], m[i * 4 + 1], m[i * 4 + 2], m[i * 4 + 3]];
    let (x, y, z, w) = (row(0), row(1), row(2), row(3));
    let add = |a: [f32; 4], b: [f32; 4]| [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]];
    let sub = |a: [f32; 4], b: [f32; 4]| [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]];
    [add(w, x), sub(w, x), add(w, y), sub(w, y), z, sub(w, z)]
}

/// Whether a box touches the frustum (conservative).
pub fn visible(planes: &[[f32; 4]; 6], min: &[f32; 3], max: &[f32; 3]) -> bool {
    for p in planes {
        let x = if p[0] >= 0.0 { max[0] } else { min[0] };
        let y = if p[1] >= 0.0 { max[1] } else { min[1] };
        let z = if p[2] >= 0.0 { max[2] } else { min[2] };
        if p[0] * x + p[1] * y + p[2] * z + p[3] < 0.0 {
            return false;
        }
    }
    true
}

/// Distance from `p` to a box (0 inside).
pub fn box_distance(p: V3, min: &[f32; 3], max: &[f32; 3]) -> f32 {
    let d = |v: f32, lo: f32, hi: f32| if v < lo { lo - v } else if v > hi { v - hi } else { 0.0 };
    let (x, y, z) = (d(p.x, min[0], max[0]), d(p.y, min[1], max[1]), d(p.z, min[2], max[2]));
    sqrt(x * x + y * y + z * z)
}

/// Projects a world point to pixels of a `w × h` screen; `None` behind the eye.
pub fn project(m: &Mat4, p: V3, w: f32, h: f32) -> Option<(f32, f32)> {
    let cw = m[12] * p.x + m[13] * p.y + m[14] * p.z + m[15];
    if cw <= 0.01 {
        return None;
    }
    let x = (m[0] * p.x + m[1] * p.y + m[2] * p.z + m[3]) / cw;
    let y = (m[4] * p.x + m[5] * p.y + m[6] * p.z + m[7]) / cw;
    Some(((x * 0.5 + 0.5) * w, (0.5 - y * 0.5) * h))
}
