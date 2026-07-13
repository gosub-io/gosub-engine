//! Engine configuration.
//!
//! The [`EngineSettings`] struct holds the engine-wide, set-once configuration:
//! values that shape how the engine is constructed and that cannot change while
//! it is running. It is passed to `GosubEngine::new()` and frozen from then on.
//!
//! This is deliberately small. Anything that can be re-read at the point of use
//! (network timeouts, renderer knobs, feature toggles) lives in the dynamic
//! settings store instead (see [`crate::engine::settings_store`]), where it can
//! change on the fly. The rule: if changing a value at runtime would require
//! reconstructing an engine component, it belongs here; otherwise it belongs in
//! the settings store. New fields are added here only once the engine actually
//! consumes them.
//!
//! Use [`EngineSettings::default()`] for sensible defaults, or
//! [`EngineSettings::builder()`] for a fluent builder API with validation.
//!
//! # Examples
//!
//! ## Default engine configuration
//! ```rust
//! use gosub_engine::EngineSettings;
//!
//! let engine_cfg = EngineSettings::default();
//! assert_eq!(engine_cfg.max_zones, 8);
//! ```
//!
//! ## Customized configuration with builder
//! ```rust
//! use gosub_engine::EngineSettings;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let cfg = EngineSettings::builder()
//!     .max_zones(12)
//!     .build()?;
//! # Ok(()) }
//! ```
//!
//! # Errors
//!
//! Builder validation returns [`EngineSettingsError`] if values are nonsensical
//! (e.g. `max_zones == 0`).
//!
//! # See also
//!
//! - [`ZoneConfig`] for per-zone settings.

use std::fmt;

use crate::zone::ZoneConfig;

/// Overall engine configuration (engine-wide, set-once knobs).
///
/// Use [`EngineSettings::default()`] for sensible defaults, or
/// [`EngineSettings::builder()`] to customize with validation.
#[derive(Debug, Clone)]
pub struct EngineSettings {
    /// Maximum number of zones that can be created within this engine.
    /// Enforced by `GosubEngine::create_zone`.
    pub max_zones: usize,
    /// Default zone configuration used when creating zones without an explicit config.
    pub default_zone_config: ZoneConfig,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            max_zones: 8,
            default_zone_config: ZoneConfig::default(),
        }
    }
}

impl EngineSettings {
    /// Start building an `EngineSettings` from defaults using a fluent builder.
    pub fn builder() -> EngineSettingsBuilder {
        EngineSettingsBuilder::default()
    }
}

/// Fluent builder for [`EngineSettings`] with validation.
#[derive(Debug, Clone, Default)]
pub struct EngineSettingsBuilder {
    inner: EngineSettings,
}

impl EngineSettingsBuilder {
    #[inline]
    fn map(mut self, f: impl FnOnce(&mut EngineSettings)) -> Self {
        f(&mut self.inner);
        self
    }

    pub fn max_zones(self, n: usize) -> Self {
        self.map(|c| c.max_zones = n)
    }
    pub fn default_zone_config(self, z: ZoneConfig) -> Self {
        self.map(|c| c.default_zone_config = z)
    }

    /// Apply multiple mutations in one go.
    pub fn with(self, f: impl FnOnce(&mut EngineSettings)) -> Self {
        self.map(f)
    }

    /// Validate and build the final `EngineSettings`.
    pub fn build(self) -> Result<EngineSettings, EngineSettingsError> {
        validate(&self.inner)?;
        Ok(self.inner)
    }
}

#[derive(Debug, Clone)]
pub enum EngineSettingsError {
    ZeroZones,
}

impl fmt::Display for EngineSettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineSettingsError::ZeroZones => write!(f, "max_zones must be at least 1"),
        }
    }
}
impl std::error::Error for EngineSettingsError {}

fn validate(c: &EngineSettings) -> Result<(), EngineSettingsError> {
    if c.max_zones == 0 {
        return Err(EngineSettingsError::ZeroZones);
    }
    Ok(())
}
