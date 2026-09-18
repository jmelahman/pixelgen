//! Renderer for animated pixel-art backgrounds.
//!
//! A photograph is reduced to a small grid of flat colour cells drawn from a
//! palette derived from the image itself, and a handful of declarative layers
//! animate over that static base. The result loops seamlessly by construction:
//! every effect is a pure function of a phase in `[0,1)` and is periodic in it.

pub mod effect;
pub mod encode;
pub mod mask;
pub mod noise;
pub mod palette;
pub mod pixel;
pub mod render;
pub mod resample;
pub mod scene;
pub mod starter;
