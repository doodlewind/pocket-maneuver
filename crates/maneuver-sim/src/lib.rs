//! Deterministic simulation core of Pocket Maneuver.
//!
//! The same crate runs in the three.js reference (as wasm), in the world
//! compiler (for baking rays) and on the PS Vita. `abi` is the C interface the
//! wasm host uses; native hosts use `Sim` directly.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(feature = "hw-sqrt", feature(core_intrinsics), allow(internal_features))]

extern crate alloc;

pub mod abi;
pub mod audio;
pub mod auto;
pub mod collide;
pub mod fastmath;
pub mod math;
pub mod pose;
pub mod sim;
pub mod testworld;
pub mod worldfile;

pub use sim::{Input, Sim};
