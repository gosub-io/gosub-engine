#[cfg(feature = "backend_cairo")]
pub mod cairo;
#[cfg(feature = "backend_vello")]
pub mod vello;

pub trait Composable {
    type Config;
    type Return;

    fn compose(config: Self::Config) -> Self::Return;
}
