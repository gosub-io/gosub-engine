//! Zone system.
//!
//! A *zone* acts like a browser profile/container inside the Gosub engine.
//! It encapsulates persistent state (cookies, passwords, local/session
//! storage), identity (user agent, languages), and runtime services
//! (tabs, networking, timers). The [`Zone`] type manages the full state
//! and lifecycle.

mod config;
#[allow(clippy::module_inception)]
mod zone;

pub use zone::ZoneContext;
pub use zone::ZoneId;
pub use zone::ZoneServices;
pub use zone::ZoneSink;

pub use config::ZoneConfig;

pub use zone::Zone;
