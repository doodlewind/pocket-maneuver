//! Scene constants from the pack's `META`, and the light computed on the
//! processor for what is not baked (`Scene` in `vita/src/actors.rs`).

use maneuver_sim::math::*;
use serde_json::Value;

pub struct Scene {
    pub sun_dir: V3,
    pub sun: [f32; 3],
    pub sky: [f32; 3],
    pub bounce: [f32; 3],
    pub fog: [f32; 3],
    pub fog_density: f32,
    pub horizon: [f32; 3],
    pub zenith: [f32; 3],
    pub glow: [f32; 3],
    pub lod_near: f32,
    pub lod_mid: f32,
    pub clip_near: f32,
    pub clip_far: f32,
    pub cell: f32,
    pub super_cell: f32,
    /// Linear 0..2 to the display's bytes.
    lut: Vec<u8>,
}

fn arr3(v: &Value) -> [f32; 3] {
    let f = |i: usize| v.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    [f(0), f(1), f(2)]
}

impl Scene {
    pub fn from_meta(meta: &Value) -> Scene {
        let s = &meta["scene"];
        let d = arr3(&s["sunDir"]);
        let num = |v: &Value, d: f32| v.as_f64().map(|x| x as f32).unwrap_or(d);
        let lut = (0..1024).map(|i| (libm::powf(i as f32 / 511.5, 1.0 / 2.2).min(1.0) * 255.0 + 0.5) as u8).collect();
        Scene {
            sun_dir: v3(d[0], d[1], d[2]).norm(),
            sun: arr3(&s["sun"]),
            sky: arr3(&s["sky"]),
            bounce: arr3(&s["bounce"]),
            fog: arr3(&s["fog"]),
            fog_density: num(&s["fogDensity"], 0.0008),
            horizon: arr3(&s["horizon"]),
            zenith: arr3(&s["zenith"]),
            glow: arr3(&s["glow"]),
            lod_near: num(&s["lod"]["near"], 110.0),
            lod_mid: num(&s["lod"]["mid"], 380.0),
            clip_near: num(&s["clip"]["near"], 0.35),
            clip_far: num(&s["clip"]["far"], 4200.0),
            cell: num(&s["cell"], 64.0),
            super_cell: num(&s["superCell"], 256.0),
            lut,
        }
    }

    #[inline]
    pub fn encode(&self, lin: f32) -> u8 {
        self.lut[((lin * 511.5) as usize).min(1023)]
    }

    /// The haze colour as the display shows it.
    pub fn fog_srgb(&self) -> [f32; 3] {
        [0, 1, 2].map(|i| libm::powf(self.fog[i], 1.0 / 2.2))
    }

    /// Lit colour of a surface with normal `n` and linear tint `tint`; `vis` is how much of the sun reaches it.
    #[inline]
    pub fn shade(&self, n: V3, tint: [f32; 3], vis: f32) -> [u8; 4] {
        let ndl = max(n.dot(self.sun_dir), 0.0) * vis;
        let up = 0.5 + 0.5 * n.y;
        let mut out = [0u8, 0, 0, 255];
        for c in 0..3 {
            let hemi = self.bounce[c] + (self.sky[c] - self.bounce[c]) * up;
            out[c] = self.encode(tint[c] * (self.sun[c] * ndl + hemi * 0.92));
        }
        out
    }

    /// The light table of the skinning program: sun direction and visibility, sun, sky, bounce.
    pub fn light(&self, vis: f32) -> [f32; 16] {
        [self.sun_dir.x, self.sun_dir.y, self.sun_dir.z, vis, self.sun[0], self.sun[1], self.sun[2], 0.0, self.sky[0], self.sky[1], self.sky[2], 0.0, self.bounce[0], self.bounce[1], self.bounce[2], 0.0]
    }

    /// Sky radiance toward `d` (`web/src/render/sky.ts`).
    pub fn sky_color(&self, d: V3) -> [f32; 3] {
        let h = max(d.y, 0.0);
        let k = 1.0 - libm::powf(1.0 - h, 3.2);
        let s = max(d.dot(self.sun_dir), 0.0);
        let glow = 0.42 * libm::powf(s, 6.0) + 0.9 * libm::powf(s, 90.0);
        let below = saturate(-d.y * 6.0);
        [0, 1, 2].map(|i| {
            let sky = self.horizon[i] + (self.zenith[i] - self.horizon[i]) * k + self.glow[i] * glow;
            sky + (self.fog[i] - sky) * below
        })
    }
}
