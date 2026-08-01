//! Zone system.

mod config;
#[allow(clippy::module_inception)]
mod zone;

pub use zone::ZoneContext;
pub use zone::ZoneId;
pub use zone::ZoneServices;
pub use zone::ZoneSink;

pub use config::ZoneConfig;

pub use zone::Zone;
