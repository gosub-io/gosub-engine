//! Engine configuration.
//!
//! [`EngineConfig`] holds the engine-wide, set-once values; it is passed to
//! `GosubEngine::new()` and frozen from then on. If changing a value at runtime
//! would require reconstructing an engine component, it belongs here; otherwise
//! it belongs in the dynamic settings store ([`crate::engine::settings_store`]).
//!
//! ```rust
//! use gosub_engine::EngineConfig;
//!
//! let engine_cfg = EngineConfig::default();
//! assert_eq!(engine_cfg.max_zones, 8);
//! ```
//!
//! ```rust
//! use gosub_engine::EngineConfig;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let cfg = EngineConfig::builder()
//!     .max_zones(12)
//!     .build()?;
//! # Ok(()) }
//! ```

use std::fmt;

use crate::zone::ZoneConfig;

/// Overall engine configuration (engine-wide, set-once knobs).
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Maximum number of zones; enforced by `GosubEngine::create_zone`.
    pub max_zones: usize,
    /// Used when creating zones without an explicit config.
    pub default_zone_config: ZoneConfig,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_zones: 8,
            default_zone_config: ZoneConfig::default(),
        }
    }
}

impl EngineConfig {
    /// Start building an `EngineConfig` from defaults using a fluent builder.
    pub fn builder() -> EngineConfigBuilder {
        EngineConfigBuilder::default()
    }
}

/// Fluent builder for [`EngineConfig`] with validation.
#[derive(Debug, Clone, Default)]
pub struct EngineConfigBuilder {
    inner: EngineConfig,
}

impl EngineConfigBuilder {
    #[inline]
    fn map(mut self, f: impl FnOnce(&mut EngineConfig)) -> Self {
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
    pub fn with(self, f: impl FnOnce(&mut EngineConfig)) -> Self {
        self.map(f)
    }

    /// Validate and build the final `EngineConfig`.
    pub fn build(self) -> Result<EngineConfig, EngineConfigError> {
        validate(&self.inner)?;
        Ok(self.inner)
    }
}

#[derive(Debug, Clone)]
pub enum EngineConfigError {
    ZeroZones,
}

impl fmt::Display for EngineConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineConfigError::ZeroZones => write!(f, "max_zones must be at least 1"),
        }
    }
}
impl std::error::Error for EngineConfigError {}

fn validate(c: &EngineConfig) -> Result<(), EngineConfigError> {
    if c.max_zones == 0 {
        return Err(EngineConfigError::ZeroZones);
    }
    Ok(())
}
