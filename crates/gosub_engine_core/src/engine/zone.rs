//! Zone system.
//!
//! A *zone* acts like a browser profile/container inside the Gosub engine.
//! It encapsulates persistent state (cookies, passwords, local/session
//! storage), identity (user agent, languages), and runtime services
//! (tabs, networking, timers).
//!
//! The `zone` module organizes this functionality into smaller components:
//!
//! - [`ZoneConfig`] — configuration for creating a new zone
//!   application to access a zone
//! - [`ZoneId`] — a unique identifier for a zone
//! - [`ZoneServices`] — collection of shared services bound to a zone
//! - [`ZoneContext`] — context for zone operations
//! - [`ZoneSink`] — a sink for zone events
//!
//! Internally, the [`Zone`] type manages the full state and lifecycle.

mod config;
mod zone;

pub use zone::ZoneContext;
pub use zone::ZoneId;
pub use zone::ZoneServices;
pub use zone::ZoneSink;

pub use config::ZoneConfig;

// Internal type, not exposed publicly.
pub(crate) use zone::Zone;
