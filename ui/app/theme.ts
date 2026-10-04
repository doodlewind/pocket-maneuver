// The look shared by every device: warm white type over the town, dark
// glass behind anything that must stay legible, amber for what is selected
// or held, and the orange of the nearest-target mark for what is urgent.
export const INK = "#f4f1e8";
export const DIM = "#f4f1e8c0";
export const FAINT = "#f4f1e880";
export const GLASS = "#0c1016d9";
export const PANEL = "#10141cf2";
export const HAIRLINE = "#ffffff26";
export const WASH = "#ffffff1c";
export const AMBER = "#ffb347";
export const ALERT = "#ff7a3c";
export const TARGET = "#ff6048";
export const NIGHT = "#0a0d12";

/** "#rrggbb" with an alpha in 0…1. */
export function tint(color: string, alpha: number): string {
  return color.slice(0, 7) + Math.round(Math.max(0, Math.min(1, alpha)) * 255).toString(16).padStart(2, "0");
}
