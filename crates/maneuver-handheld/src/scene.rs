//! Scene constants of the pack and the lighting rules the reference and the
//! Vita runtime use, for geometry lit on the CPU or by fixed-function lights.

use alloc::vec::Vec;
use maneuver_pack::HandScene;
use maneuver_sim::math::*;

pub struct Scene {
    pub h: HandScene,
    pub sun_dir: V3,
    /// Linear 0..2 to sRGB bytes.
    lut: Vec<u8>,
}

/// Three colours for a fixed-function pipeline (sRGB-encoded, to multiply an
/// sRGB tint): a constant, a light from straight above, and the sun.
#[derive(Clone, Copy, Debug)]
pub struct Lights {
    pub ambient: [f32; 3],
    pub above: [f32; 3],
    pub sun: [f32; 3],
}

pub fn powf(x: f32, y: f32) -> f32 {
    libm::powf(x, y)
}

fn enc(x: f32) -> f32 {
    powf(max(x, 0.0), 1.0 / 2.2)
}

impl Scene {
    pub fn new(h: HandScene) -> Scene {
        let lut = (0..1024).map(|i| (min(enc(i as f32 / 511.5), 1.0) * 255.0 + 0.5) as u8).collect();
        Scene { sun_dir: v3(h.sun_dir[0], h.sun_dir[1], h.sun_dir[2]).norm(), h, lut }
    }

    #[inline]
    pub fn encode(&self, lin: f32) -> u8 {
        self.lut[((max(lin, 0.0) * 511.5) as usize).min(1023)]
    }

    /// sRGB haze colour.
    pub fn fog_srgb(&self) -> [f32; 3] {
        [0, 1, 2].map(|i| enc(self.h.fog[i]))
    }

    fn hemi(&self, c: usize, up: f32) -> f32 {
        (self.h.bounce[c] + (self.h.sky[c] - self.h.bounce[c]) * up) * 0.92
    }

    /// Lit colour of a surface with normal `n` and linear tint `tint`; `vis` is how much of the sun reaches it.
    #[inline]
    pub fn shade(&self, n: V3, tint: [f32; 3], vis: f32) -> [u8; 4] {
        let ndl = max(n.dot(self.sun_dir), 0.0) * vis;
        let up = 0.5 + 0.5 * n.y;
        let mut out = [0u8, 0, 0, 255];
        for c in 0..3 {
            out[c] = self.encode(tint[c] * (self.h.sun[c] * ndl + self.hemi(c, up)));
        }
        out
    }

    /// The light table of a skinning program: sun direction and visibility, sun, sky, bounce.
    pub fn light(&self, vis: f32) -> [f32; 16] {
        let h = &self.h;
        [self.sun_dir.x, self.sun_dir.y, self.sun_dir.z, vis, h.sun[0], h.sun[1], h.sun[2], 0.0, h.sky[0], h.sky[1], h.sky[2], 0.0, h.bounce[0], h.bounce[1], h.bounce[2], 0.0]
    }

    /// The same lighting as three additive terms: exact for a normal facing the
    /// horizon, straight up, and the sun; in between it is close.
    pub fn lights(&self, vis: f32) -> Lights {
        let mut l = Lights { ambient: [0.0; 3], above: [0.0; 3], sun: [0.0; 3] };
        let sy = max(self.sun_dir.y, 0.0);
        // The encoding table instead of three powers per colour: this runs for every model in a frame.
        let enc = |x: f32| self.encode(x) as f32 * (1.0 / 255.0);
        for c in 0..3 {
            let side = enc(self.hemi(c, 0.5));
            let top = enc(self.hemi(c, 1.0));
            let lit = enc(self.h.sun[c] * vis + self.hemi(c, 0.5 + 0.5 * self.sun_dir.y));
            l.ambient[c] = side;
            l.above[c] = max(top - side, 0.0);
            l.sun[c] = max(lit - side - l.above[c] * sy, 0.0);
        }
        l
    }

    /// Sky radiance toward `d` (`web/src/render/sky.ts`).
    pub fn sky_color(&self, d: V3) -> [f32; 3] {
        let h = &self.h;
        let up = max(d.y, 0.0);
        let k = 1.0 - powf(1.0 - up, 3.2);
        let s = max(d.dot(self.sun_dir), 0.0);
        let glow = 0.42 * powf(s, 6.0) + 0.9 * powf(s, 90.0);
        let below = saturate(-d.y * 6.0);
        [0, 1, 2].map(|i| {
            let sky = h.horizon[i] + (h.zenith[i] - h.horizon[i]) * k + h.glow[i] * glow;
            sky + (h.fog[i] - sky) * below
        })
    }
}
