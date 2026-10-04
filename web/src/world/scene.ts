// Light, air and level-of-detail distances. The reference renderer lights
// with these at run time; the world compiler bakes the same model into vertex
// colours, so both read one table.
//
// Colours are linear RGB. Radiance leaving a surface is
// `albedo × (sun × max(N·L, 0) × visibility + hemisphere(N) × occlusion)`.

export const SCENE = {
  /** Unit vector toward the sun: a late afternoon in the west. */
  sunDir: [-0.7205, 0.5504, 0.4203] as const,
  sun: [1.02, 0.84, 0.62] as const,
  /** Ambient from straight up and from straight down. */
  sky: [0.36, 0.45, 0.62] as const,
  bounce: [0.3, 0.25, 0.19] as const,
  /** Haze: colour and the density of `1 - exp(-(d × density)²)`. */
  fog: [0.72, 0.8, 0.9] as const,
  fogDensity: 0.00082,
  /** Sky gradient: at the horizon, at the zenith, and the glow around the sun. */
  horizon: [0.78, 0.85, 0.93] as const,
  zenith: [0.24, 0.44, 0.8] as const,
  glow: [1.0, 0.82, 0.58] as const,
  /** A near cell draws its detailed mesh inside `near` metres and its simple one beyond; super-cells past `mid` draw the far mesh. */
  lod: { near: 160, mid: 560 },
  clip: { near: 0.35, far: 4200 },
} as const;
