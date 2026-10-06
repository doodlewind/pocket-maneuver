// Post-processing: the PS Vita's passes (vita/shaders/post_v.cg and the four
// fragment programs beside it). A pass is one triangle over its target. Every
// texture coordinate a fragment program reads is the screen position scaled
// and offset by a tap, computed in the vertex program.

struct Pass {
  // A scale (xy) and an offset (zw) for each of eight taps.
  tap: array<vec4<f32>, 8>,
  // bright: x the threshold, y the slope above it.
  // composite: x bloom gain, y shaft gain, z vignette, w blur at speed.
  a: vec4<f32>,
  // composite: x contrast, y saturation, z warmth of the lights, w coolness of the shadows.
  b: vec4<f32>,
}
@group(0) @binding(0) var<uniform> taps: Pass;
@group(0) @binding(1) var texels: sampler;
@group(0) @binding(2) var first: texture_2d<f32>;
@group(0) @binding(3) var second: texture_2d<f32>;
@group(0) @binding(4) var third: texture_2d<f32>;

struct Out {
  @builtin(position) position: vec4<f32>,
  @location(0) uv0: vec2<f32>,
  @location(1) uv1: vec2<f32>,
  @location(2) uv2: vec2<f32>,
  @location(3) uv3: vec2<f32>,
  @location(4) uv4: vec2<f32>,
  @location(5) uv5: vec2<f32>,
  @location(6) uv6: vec2<f32>,
  @location(7) uv7: vec2<f32>,
}

@vertex
fn corner(@builtin(vertex_index) index: u32) -> Out {
  var out: Out;
  let p = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u)) * 2.0 - 1.0;
  out.position = vec4<f32>(p, 0.0, 1.0);
  let uv = vec2<f32>(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
  out.uv0 = uv * taps.tap[0].xy + taps.tap[0].zw;
  out.uv1 = uv * taps.tap[1].xy + taps.tap[1].zw;
  out.uv2 = uv * taps.tap[2].xy + taps.tap[2].zw;
  out.uv3 = uv * taps.tap[3].xy + taps.tap[3].zw;
  out.uv4 = uv * taps.tap[4].xy + taps.tap[4].zw;
  out.uv5 = uv * taps.tap[5].xy + taps.tap[5].zw;
  out.uv6 = uv * taps.tap[6].xy + taps.tap[6].zw;
  out.uv7 = uv * taps.tap[7].xy + taps.tap[7].zw;
  return out;
}

fn read(t: texture_2d<f32>, uv: vec2<f32>) -> vec3<f32> {
  return textureSampleLevel(t, texels, uv, 0.0).rgb;
}

const LUMA = vec3<f32>(0.30, 0.59, 0.11);

// A quarter-size copy of the scene keeping what is bright: four taps cover a 4 x 4 block.
@fragment
fn bright(in: Out) -> @location(0) vec4<f32> {
  let c = (read(first, in.uv0) + read(first, in.uv1) + read(first, in.uv2) + read(first, in.uv3)) * 0.25;
  let k = saturate((dot(c, LUMA) - taps.a.x) * taps.a.y);
  return vec4<f32>(c * k, 1.0);
}

// One direction of a Gaussian blur: five taps stand for nine texels.
@fragment
fn blur(in: Out) -> @location(0) vec4<f32> {
  let c = read(first, in.uv0) * 0.2270 + (read(first, in.uv1) + read(first, in.uv2)) * 0.3162 + (read(first, in.uv3) + read(first, in.uv4)) * 0.0703;
  return vec4<f32>(c, 1.0);
}

// Light shafts: the bright picture smeared toward the sun, fading.
@fragment
fn shafts(in: Out) -> @location(0) vec4<f32> {
  let c = read(first, in.uv0) * 0.20 + read(first, in.uv1) * 0.17 + read(first, in.uv2) * 0.15 + read(first, in.uv3) * 0.13
    + read(first, in.uv4) * 0.11 + read(first, in.uv5) * 0.09 + read(first, in.uv6) * 0.08 + read(first, in.uv7) * 0.07;
  return vec4<f32>(c, 1.0);
}

// The frame as shown: the scene (first), plus bloom (second) and light shafts
// (third), graded. Tap 1 is the screen as -1..1 for the vignette; taps 2..4 are
// the scene scaled about the centre, for the blur at speed. A gain of zero
// leaves its part out.
@fragment
fn composite(in: Out) -> @location(0) vec4<f32> {
  var c = read(first, in.uv0);
  let edge = dot(in.uv1, in.uv1);
  let smear = (c + read(first, in.uv2) + read(first, in.uv3) + read(first, in.uv4)) * 0.25;
  c = mix(c, smear, saturate(edge * taps.a.w));
  let glow = read(second, in.uv0) * taps.a.x + read(third, in.uv0) * taps.a.y;
  // Screen: light adds without clipping what is bright.
  c = c + glow * (vec3<f32>(1.0) - c);
  let l = dot(c, LUMA);
  c = mix(vec3<f32>(l), c, taps.b.y);
  c = (c - 0.5) * taps.b.x + 0.5;
  // Split tone: warm where it is light, cool where it is dark.
  let warm = vec3<f32>(1.0 + taps.b.z, 1.0, 1.0 - taps.b.z);
  let cool = vec3<f32>(1.0 - taps.b.w, 1.0, 1.0 + taps.b.w);
  c = c * mix(cool, warm, saturate(l * 1.4));
  c = c * (1.0 - edge * taps.a.z);
  return vec4<f32>(c, 1.0);
}
