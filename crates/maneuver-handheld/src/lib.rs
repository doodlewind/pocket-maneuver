//! The device-independent half of the handheld runtimes (PSP, 3DS).
//!
//! A device crate owns the GPU, the pad, the sound output and storage. This
//! crate owns everything those share: which meshes a frame draws (`world`),
//! the geometry that moves (`actors`), the interface (`hud`), the loop around
//! the simulation (`game`), and the PSP's guard-band clipping (`clip`), kept
//! here so a computer can test it.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod actors;
pub mod clip;
pub mod game;
pub mod hud;
pub mod mat;
pub mod scene;
pub mod world;
