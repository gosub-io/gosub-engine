//! Render backend contract types.
//!
//! These moved to `gosub_interface::render::backend` so that `ModuleConfiguration` can
//! reference them. This module re-exports them so existing
//! `gosub_render_pipeline::render::backend::*` paths keep working.

pub use gosub_interface::render::backend::*;
