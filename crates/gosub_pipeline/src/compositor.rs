#[cfg(feature = "backend_cairo")]
pub mod cairo;
#[cfg(feature = "backend_vello")]
pub mod vello;
#[cfg(feature = "backend_skia")]
pub mod skia;

pub trait Composable {
    type Config;
    type Return;

    fn compose(config: Self::Config) -> Self::Return;
}
