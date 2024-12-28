use crate::chrome::ChromeHandle;
use crate::config::HasRenderBackend;

pub trait HasChrome: HasRenderBackend + Sized {
    type ChromeHandle: ChromeHandle<Self>;
}
