use crate::common::geo::Dimension;
use crate::painter::commands::text::Text;
use std::fmt::Error;
use vello::kurbo::Affine;
use vello::Scene;

#[allow(dead_code)]
pub fn do_paint_text(_scene: &mut Scene, _cmd: &Text, _tile_size: Dimension, _affine: Affine) -> Result<(), Error> {
    unimplemented!("Pango text rendering is not implemented for the Vello backend")
}
