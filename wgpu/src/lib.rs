//! Pocket Maneuver drawn with wgpu: the game of the handheld builds in a
//! browser tab over WebGPU, and on the build machine, where frames go to
//! files.
//!
//! The game is `maneuver_sim` and the flow of `maneuver_interface`, as every
//! device links them. The pack is the PS Vita's (`profiles/vita60.json`),
//! read over HTTP ([`pack`]); the passes are the PS Vita's (`vita/src`), in
//! WGSL ([`render`], [`world`], [`actors`], [`marks`]). [`app`] is the shell
//! around them. What is not this game's is PocketJS's browser kernel,
//! `pocket_web_wgpu` (`vendor/pocketjs/devices/web/pocket-web-wgpu`).

pub mod actors;
pub mod app;
pub mod marks;
pub mod mat;
pub mod pack;
pub mod render;
pub mod scene;
#[cfg(target_arch = "wasm32")]
mod web;
pub mod world;
