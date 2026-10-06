// The scene's programs: the PS Vita's (vita/shaders/*.cg) in WGSL.
//
// Matrices are uploaded in row order, as the Cg programs read them with
// mul(M, v); here that product is written v * M. Each program's draw record
// has a binding of its own in group 1.

struct Scene {
  // The haze colour as the display shows it, and its density.
  fog: vec4<f32>,
  // The screen in pixels, for the marks on the world.
  screen: vec4<f32>,
}
@group(0) @binding(0) var<uniform> scene: Scene;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var texels: sampler;

// U of a vertex spans this many repeats of a strip; the baked light is stored at half scale.
const UV_SCALE: f32 = 8.0;
const COLOR_SCALE: f32 = 2.0;
const BONES: u32 = 19u;

fn haze(depth: f32, density: f32) -> f32 {
  let d = depth * density;
  return 1.0 - exp(-d * d);
}

// ---------------------------------------------------------------- the static world
// Atlas texel × baked light, then haze. A position arrives over its mesh's
// bounds, which are folded into the draw's matrix.

struct WorldDraw {
  mvp: mat4x4<f32>,
}
@group(1) @binding(0) var<uniform> world_draw: WorldDraw;

struct WorldOut {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) color: vec4<f32>,
}

@vertex
fn world_vertex(@location(0) position: vec4<u32>, @location(1) uv: vec2<i32>, @location(2) color: vec4<f32>) -> WorldOut {
  var out: WorldOut;
  let clip = vec4<f32>(vec3<f32>(position.xyz) / 65535.0, 1.0) * world_draw.mvp;
  out.position = clip;
  out.uv = vec2<f32>(uv) / 32767.0 * vec2<f32>(UV_SCALE, 1.0);
  out.color = vec4<f32>(color.rgb * COLOR_SCALE, haze(clip.w, scene.fog.w));
  return out;
}

@fragment
fn world_fragment(in: WorldOut) -> @location(0) vec4<f32> {
  let c = textureSample(picture, texels, in.uv).rgb * in.color.rgb;
  return vec4<f32>(mix(c, scene.fog.rgb, in.color.a), 1.0);
}

// ---------------------------------------------------------------- coloured geometry
// Lit on the processor: the sky's dome, the cloak, the wires, the gas.

struct ColorDraw {
  mvp: mat4x4<f32>,
  // x: the haze density (0 for the sky).
  fog: vec4<f32>,
}
@group(1) @binding(1) var<uniform> color_draw: ColorDraw;

struct ColorOut {
  @builtin(position) position: vec4<f32>,
  @location(0) color: vec4<f32>,
  @location(1) fog: f32,
}

@vertex
fn color_vertex(@location(0) position: vec3<f32>, @location(1) color: vec4<f32>) -> ColorOut {
  var out: ColorOut;
  let clip = vec4<f32>(position, 1.0) * color_draw.mvp;
  out.position = clip;
  out.color = color;
  out.fog = haze(clip.w, color_draw.fog.x);
  return out;
}

@fragment
fn color_fragment(in: ColorOut) -> @location(0) vec4<f32> {
  return vec4<f32>(mix(in.color.rgb, scene.fog.rgb, in.fog), in.color.a);
}

// ---------------------------------------------------------------- skinned models
// The player and the giants: two bones a vertex, three rows a bone, lit by
// the scene's sun and hemisphere per vertex. The tint is in the display's
// encoding, so the light is encoded before it multiplies, as the bake does.

struct SkinDraw {
  mvp: mat4x4<f32>,
  bones: array<vec4<f32>, 57>,
  // The sun's direction and how much of it arrives; the sun, the sky, the bounce.
  light: array<vec4<f32>, 4>,
  fog: vec4<f32>,
}
@group(1) @binding(2) var<uniform> skin_draw: SkinDraw;

@vertex
fn skin_vertex(@location(0) position: vec3<f32>, @location(1) normal_bytes: vec4<i32>, @location(2) color: vec4<f32>, @location(3) joints: vec4<u32>) -> ColorOut {
  var out: ColorOut;
  // (integers as the pack stores them: the normal in 127ths; two bones, then their weights in 255ths)
  let normal = vec3<f32>(normal_bytes.xyz) / 127.0;
  let bones = joints.xy;
  let weights = vec2<f32>(joints.zw) / 255.0;
  let p = vec4<f32>(position, 1.0);
  var world = vec3<f32>(0.0);
  var n = vec3<f32>(0.0);
  for (var k = 0u; k < 2u; k++) {
    let j = min(bones[k], BONES - 1u) * 3u;
    let w = weights[k];
    let r0 = skin_draw.bones[j];
    let r1 = skin_draw.bones[j + 1u];
    let r2 = skin_draw.bones[j + 2u];
    world += vec3<f32>(dot(r0, p), dot(r1, p), dot(r2, p)) * w;
    n += vec3<f32>(dot(r0.xyz, normal), dot(r1.xyz, normal), dot(r2.xyz, normal)) * w;
  }
  n = normalize(n);
  let clip = vec4<f32>(world, 1.0) * skin_draw.mvp;
  out.position = clip;
  let ndl = max(dot(n, skin_draw.light[0].xyz), 0.0) * skin_draw.light[0].w;
  let up = 0.5 + 0.5 * n.y;
  let light = skin_draw.light[1].xyz * ndl + mix(skin_draw.light[3].xyz, skin_draw.light[2].xyz, up) * 0.92;
  let encoded = pow(max(light, vec3<f32>(0.0001)), vec3<f32>(0.4545));
  out.color = vec4<f32>(color.rgb * encoded, 1.0);
  out.fog = haze(clip.w, skin_draw.fog.x);
  return out;
}

// ---------------------------------------------------------------- the marks on the world
// Quads in the screen's pixels, origin at the top left: glyph coverage (or
// the font picture's solid block) × the vertex's colour.

struct MarkOut {
  @builtin(position) position: vec4<f32>,
  @location(0) uv: vec2<f32>,
  @location(1) color: vec4<f32>,
}

@vertex
fn mark_vertex(@location(0) position: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>) -> MarkOut {
  var out: MarkOut;
  out.position = vec4<f32>(position.x / scene.screen.x * 2.0 - 1.0, 1.0 - position.y / scene.screen.y * 2.0, 0.0, 1.0);
  out.uv = uv;
  out.color = color;
  return out;
}

@fragment
fn mark_fragment(in: MarkOut) -> @location(0) vec4<f32> {
  let a = textureSample(picture, texels, in.uv).r;
  return vec4<f32>(in.color.rgb, in.color.a * a);
}
