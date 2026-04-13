#[cfg(not(any(feature = "backend_cairo", feature = "backend_vello", feature = "backend_skia")))]
compile_error!("Either the 'backend_cairo' 'backend_skia' or 'backend_vello' feature must be enabled");

#[cfg(feature = "backend_cairo")]
pub mod cairo;
#[cfg(feature = "backend_skia")]
pub mod skia;
#[cfg(feature = "backend_vello")]
pub mod vello;

pub trait Composable {
    type Config;
    type Return;

    fn compose(config: Self::Config) -> Self::Return;
}
