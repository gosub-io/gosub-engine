#[cfg(any(feature = "pango", feature = "skia"))]
pub(crate) mod fontconfig_lock {
    use parking_lot::{Mutex, MutexGuard};

    /// Serialises this crate's use of the process-global fontconfig configuration.
    ///
    /// fontconfig's documented thread safety does not cover mutating the current config while
    /// another thread reads it, and "reads it" includes everything that goes through it
    /// indirectly: Pango's ft2 font map walks the config's font set, and Skia's
    /// `FontMgr::new()` calls `FcInitLoadConfigAndFonts()`, which builds a config and mmaps its
    /// caches. Registering a web font (`FcConfigBuildFonts`) underneath either one segfaults.
    ///
    /// The lock lives at the crate root rather than in one font system because the two ends of
    /// the race are in *different* ones - a Pango `list_families()` crashing while Skia builds a
    /// config on another thread - and neither module can see the other's lock.
    static FONTCONFIG: Mutex<()> = Mutex::new(());

    /// Hold the fontconfig lock for the current scope.
    ///
    /// Not reentrant: no path that takes it may call another that does. Today the direct FFI
    /// (`fontconfig_match`, font registration) and the Pango font-map walks are disjoint.
    pub(crate) fn hold() -> MutexGuard<'static, ()> {
        FONTCONFIG.lock()
    }
}

pub mod cosmic_system;
pub mod parley_system;

#[cfg(feature = "pango")]
pub mod pango_system;
#[cfg(feature = "skia")]
pub mod skia_system;

pub use cosmic_system::CosmicFontSystem;
pub use parley_system::ParleyFontSystem;

#[cfg(feature = "pango")]
pub use pango_system::PangoFontSystem;
#[cfg(feature = "skia")]
pub use skia_system::SkiaFontSystem;
