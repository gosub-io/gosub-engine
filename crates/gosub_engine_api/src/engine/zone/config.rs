//! Zone configuration.
//!
//! `ZoneConfig` controls properties and limits for a single
//! [`Zone`](crate::zone::Zone) in the Gosub engine. A *zone* acts like a
//! browser profile/container: it defines behavior (e.g. JS/images enabled),
//! identity (e.g. user agent, languages), and limits (e.g. max tabs).
//!
//! `ZoneConfig` provides sensible defaults via [`Default`] and a fluent
//! [`ZoneConfig::builder()`] for customization with validation.
//!
//! # Examples
//!
//! ## Use defaults
//! ```rust
//! use gosub_engine_api::zone::ZoneConfig;
//! let cfg = ZoneConfig::default();
//! assert_eq!(cfg.max_tabs, 16);
//! ```
//!
//! ## Customize with the builder
//! ```rust
//! use gosub_engine_api::zone::ZoneConfig;
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let cfg = ZoneConfig::builder()
//!     .max_tabs(10)
//!     .user_agent("Gosub/0.1")
//!     .accept_languages("en-US,en;q=0.9,nl;q=0.8")
//!     .javascript_enabled(true)
//!     .images_enabled(true)
//!     .font_scale(1.25)
//!     .minimum_font_size(12)
//!     .build()?; // returns Result<ZoneConfig, ZoneConfigError>
//! # Ok(()) }
//! ```
//!
//! # Fields (summary)
//! - `max_tabs`: Maximum number of tabs allowed in the zone (default: 16).
//! - `user_agent`: Optional UA string to send with requests.
//! - `accept_languages`: Optional `Accept-Language` header value.
//! - `do_not_track`: Send `DNT: 1` header if `true`.
//! - `javascript_enabled`: Execute JavaScript if `true`.
//! - `images_enabled`: Load images if `true`.
//! - `plugins_enabled`: Enable plugins if `true`.
//! - `font_scale`: UI/content scale factor (validated range `0.25..=10.0`).
//! - `default_font_family`: Optional default font family name.
//! - `default_font_size`: Default font size in CSS px (default: 16).
//! - `minimum_font_size`: Minimum allowed font size in CSS px (must be ≤ `default_font_size`).
//! - `enable_local_file_access`: Allow `file://` (sandboxing concerns).
//!
//! # Notes
//!
//! Note that most of these fields are not implemented but are here to show
//! the intended design. The actual implementation may change without notice.
//!
//! # Errors
//!
//! Builder validation can return [`ZoneConfigError`] if values are invalid
//! (e.g. `font_scale` outside `0.25..=10.0`, `minimum_font_size > default_font_size`,
//! or `max_tabs == 0`).

use crate::storage::PartitionPolicy;
use std::fmt;

#[derive(Debug, Clone)]
pub struct ZoneConfig {
    /// Maximum number of tabs allowed in this zone.
    pub max_tabs: usize,
    /// Optional User-Agent string to send with requests.
    pub user_agent: Option<String>,
    /// Optional Accept-Language header value.
    pub accept_languages: Option<String>,
    /// Send DNT: 1 header if true.
    pub do_not_track: bool,
    /// Enable or disable JavaScript execution.
    pub javascript_enabled: bool,
    /// Enable or disable image loading.
    pub images_enabled: bool,
    /// Enable or disable plugins (e.g. Flash).
    pub plugins_enabled: bool,
    /// UI/content scale factor (1.0 = normal size).
    pub font_scale: f32,
    /// Optional default font family name.
    pub default_font_family: Option<String>,
    /// Default font size in CSS px (e.g. 16).
    pub default_font_size: u32,
    /// Minimum allowed font size in CSS px (must be ≤ default_font_size).
    pub minimum_font_size: u32,
    /// Allow access to local file:// URLs (may have sandboxing concerns).
    pub enable_local_file_access: bool,
    /// Policy for storage partitioning (cookies, localStorage, etc.).
    pub partition_policy: PartitionPolicy,
}

impl Default for ZoneConfig {
    fn default() -> Self {
        Self {
            max_tabs: 16,
            user_agent: None,
            accept_languages: None,
            do_not_track: false,
            javascript_enabled: true,
            images_enabled: true,
            plugins_enabled: false,
            font_scale: 1.0,
            default_font_family: None,
            default_font_size: 16,
            minimum_font_size: 0,
            enable_local_file_access: false,
            partition_policy: PartitionPolicy::TopLevelOrigin,
        }
    }
}

impl ZoneConfig {
    pub fn builder() -> ZoneConfigBuilder {
        ZoneConfigBuilder::default()
    }
}

/// Builder for [`ZoneConfig`], mirroring `EngineConfigBuilder`.
#[derive(Debug, Clone)]
pub struct ZoneConfigBuilder {
    inner: ZoneConfig,
}

impl Default for ZoneConfigBuilder {
    fn default() -> Self {
        Self {
            inner: ZoneConfig::default(),
        }
    }
}

impl ZoneConfigBuilder {
    #[inline]
    fn map(mut self, f: impl FnOnce(&mut ZoneConfig)) -> Self {
        f(&mut self.inner);
        self
    }

    #[must_use]
    pub fn max_tabs(self, n: usize) -> Self {
        self.map(|c| c.max_tabs = n)
    }
    #[must_use]
    pub fn user_agent<S: Into<String>>(self, ua: S) -> Self {
        self.map(|c| c.user_agent = Some(ua.into()))
    }
    #[must_use]
    pub fn accept_languages<S: Into<String>>(self, langs: S) -> Self {
        self.map(|c| c.accept_languages = Some(langs.into()))
    }
    #[must_use]
    pub fn do_not_track(self, dnt: bool) -> Self {
        self.map(|c| c.do_not_track = dnt)
    }
    #[must_use]
    pub fn javascript_enabled(self, on: bool) -> Self {
        self.map(|c| c.javascript_enabled = on)
    }
    #[must_use]
    pub fn images_enabled(self, on: bool) -> Self {
        self.map(|c| c.images_enabled = on)
    }
    #[must_use]
    pub fn plugins_enabled(self, on: bool) -> Self {
        self.map(|c| c.plugins_enabled = on)
    }
    #[must_use]
    pub fn font_scale(self, scale: f32) -> Self {
        self.map(|c| c.font_scale = scale)
    }
    #[must_use]
    pub fn default_font_family<S: Into<String>>(self, fam: S) -> Self {
        self.map(|c| c.default_font_family = Some(fam.into()))
    }
    #[must_use]
    pub fn default_font_size(self, px: u32) -> Self {
        self.map(|c| c.default_font_size = px)
    }
    #[must_use]
    pub fn minimum_font_size(self, px: u32) -> Self {
        self.map(|c| c.minimum_font_size = px)
    }
    #[must_use]
    pub fn enable_local_file_access(self, on: bool) -> Self {
        self.map(|c| c.enable_local_file_access = on)
    }
    #[must_use]
    pub fn partition_policy(self, policy: PartitionPolicy) -> Self {
        self.map(|c| c.partition_policy = policy)
    }

    /// Apply multiple changes in one go.
    pub fn with(self, f: impl FnOnce(&mut ZoneConfig)) -> Self {
        self.map(f)
    }

    /// Validate and build the final config.
    pub fn build(self) -> Result<ZoneConfig, ZoneConfigError> {
        validate(&self.inner)?;
        Ok(self.inner)
    }
}

/// These checks help prevent common configuration errors and ensure a valid zone setup.
#[derive(Debug, Clone)]
pub enum ZoneConfigError {
    /// Invalid font scale (must be 0.25..=10.0).
    InvalidFontScale(f32),
    /// Minimum font size cannot exceed default font size.
    MinFontLarger { min: u32, default: u32 },
    /// max_tabs must be at least 1.
    ZeroTabs,
}

impl fmt::Display for ZoneConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ZoneConfigError::InvalidFontScale(s) => {
                write!(f, "font_scale {s} is out of range (expected 0.25..=10.0)")
            }
            ZoneConfigError::MinFontLarger { min, default } => write!(
                f,
                "minimum_font_size ({min}) > default_font_size ({default})"
            ),
            ZoneConfigError::ZeroTabs => write!(f, "max_tabs must be at least 1"),
        }
    }
}
impl std::error::Error for ZoneConfigError {}

fn validate(c: &ZoneConfig) -> Result<(), ZoneConfigError> {
    if !(0.25..=10.0).contains(&c.font_scale) {
        return Err(ZoneConfigError::InvalidFontScale(c.font_scale));
    }
    if c.minimum_font_size > c.default_font_size {
        return Err(ZoneConfigError::MinFontLarger {
            min: c.minimum_font_size,
            default: c.default_font_size,
        });
    }
    if c.max_tabs == 0 {
        return Err(ZoneConfigError::ZeroTabs);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::storage::types::PartitionPolicy;

    #[test]
    fn defaults_are_sensible() {
        let c = ZoneConfig::default();
        assert_eq!(c.max_tabs, 16);
        assert_eq!(c.user_agent, None);
        assert_eq!(c.accept_languages, None);
        assert!(!c.do_not_track);
        assert!(c.javascript_enabled);
        assert!(c.images_enabled);
        assert!(!c.plugins_enabled);
        assert_eq!(c.font_scale, 1.0);
        assert_eq!(c.default_font_family, None);
        assert_eq!(c.default_font_size, 16);
        assert_eq!(c.minimum_font_size, 0);
        assert!(!c.enable_local_file_access);
        assert_eq!(c.partition_policy, PartitionPolicy::TopLevelOrigin);
    }

    #[test]
    fn builder_happy_path() {
        let cfg = ZoneConfig::builder()
            .max_tabs(10)
            .user_agent("Gosub/0.1")
            .accept_languages("en-US,en;q=0.9,nl;q=0.8")
            .do_not_track(true)
            .javascript_enabled(true)
            .images_enabled(false)
            .plugins_enabled(true)
            .font_scale(1.25)
            .default_font_family("Inter")
            .default_font_size(18)
            .minimum_font_size(12)
            .enable_local_file_access(true)
            .partition_policy(PartitionPolicy::TopLevelOrigin) // or whatever variants you have
            .build()
            .expect("valid config");

        assert_eq!(cfg.max_tabs, 10);
        assert_eq!(cfg.user_agent.as_deref(), Some("Gosub/0.1"));
        assert_eq!(
            cfg.accept_languages.as_deref(),
            Some("en-US,en;q=0.9,nl;q=0.8")
        );
        assert!(cfg.do_not_track);
        assert!(cfg.javascript_enabled);
        assert!(!cfg.images_enabled);
        assert!(cfg.plugins_enabled);
        assert!((cfg.font_scale - 1.25).abs() < f32::EPSILON);
        assert_eq!(cfg.default_font_family.as_deref(), Some("Inter"));
        assert_eq!(cfg.default_font_size, 18);
        assert_eq!(cfg.minimum_font_size, 12);
        assert!(cfg.enable_local_file_access);
        assert_eq!(cfg.partition_policy, PartitionPolicy::TopLevelOrigin);
    }

    #[test]
    fn builder_with_helper() {
        let cfg = ZoneConfig::builder()
            .with(|c| {
                c.max_tabs = 7;
                c.javascript_enabled = false;
            })
            .build()
            .unwrap();

        assert_eq!(cfg.max_tabs, 7);
        assert!(!cfg.javascript_enabled);
    }

    #[test]
    fn invalid_font_scale_rejected() {
        // too small
        let err = ZoneConfig::builder().font_scale(0.1).build().unwrap_err();
        match err {
            ZoneConfigError::InvalidFontScale(s) => assert!((s - 0.1).abs() < f32::EPSILON),
            _ => panic!("expected InvalidFontScale"),
        }

        // too large
        let err = ZoneConfig::builder().font_scale(10.5).build().unwrap_err();
        match err {
            ZoneConfigError::InvalidFontScale(s) => assert!((s - 10.5).abs() < f32::EPSILON),
            _ => panic!("expected InvalidFontScale"),
        }
    }

    #[test]
    fn min_font_cannot_exceed_default() {
        let err = ZoneConfig::builder()
            .default_font_size(16)
            .minimum_font_size(17)
            .build()
            .unwrap_err();

        match err {
            ZoneConfigError::MinFontLarger { min, default } => {
                assert_eq!(min, 17);
                assert_eq!(default, 16);
            }
            _ => panic!("expected MinFontLarger"),
        }
    }

    #[test]
    fn zero_tabs_is_invalid() {
        let err = ZoneConfig::builder().max_tabs(0).build().unwrap_err();
        matches!(err, ZoneConfigError::ZeroTabs);
    }

    #[test]
    fn partition_policy_can_be_set() {
        let cfg = ZoneConfig::builder()
            .partition_policy(PartitionPolicy::TopLevelOrigin)
            .build()
            .unwrap();
        assert_eq!(cfg.partition_policy, PartitionPolicy::TopLevelOrigin);
    }
}
