use std::fmt::{Debug, Display, Write};
use std::ops::{Div, Mul, MulAssign};

use crate::layout::TextLayout;
use crate::svg::SvgRenderer;
pub use geo::*;
use gosub_shared::types::Result;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use smallvec::SmallVec;

pub mod geo;
pub mod layout;
pub mod svg;

pub trait WindowHandle: HasDisplayHandle + HasWindowHandle + Send + Sync + Clone {}

impl<T> WindowHandle for T where T: HasDisplayHandle + HasWindowHandle + Send + Sync + Clone {}

pub trait RenderBackend: Sized + Debug + 'static {
    type Rect: Rect;
    type Border: Border<Self>;
    type BorderSide: BorderSide<Self>;
    type BorderRadius: BorderRadius;
    type Transform: Transform;
    type Text: Text;
    type Gradient: Gradient<Self>;
    type Color: Color;
    type Image: Image;
    type Brush: Brush<Self>;
    type Scene: Scene<Self> + Send;
    type SVGRenderer: SvgRenderer<Self>;

    type ActiveWindowData<'a>;
    type WindowData<'a>;

    fn draw_rect(&mut self, data: &mut Self::WindowData<'_>, rect: &RenderRect<Self>);
    fn draw_text(&mut self, data: &mut Self::WindowData<'_>, text: &RenderText<Self>);
    fn apply_scene(&mut self, data: &mut Self::WindowData<'_>, scene: &Self::Scene, transform: Option<Self::Transform>);
    fn reset(&mut self, data: &mut Self::WindowData<'_>);
    // fn layer_push(&mut self, data: &mut Self::WindowData<'_>);
    // fn layer_pop(&mut self, data: &mut Self::WindowData<'_>);

    fn activate_window<'a>(
        &mut self,
        handle: impl WindowHandle + 'a,
        data: &mut Self::WindowData<'_>,
        size: SizeU32,
    ) -> Result<Self::ActiveWindowData<'a>>;
    fn suspend_window(
        &mut self,
        handle: impl WindowHandle,
        data: &mut Self::ActiveWindowData<'_>,
        window_data: &mut Self::WindowData<'_>,
    ) -> Result<()>;

    fn create_window_data<'a>(&mut self, handle: impl WindowHandle) -> Result<Self::WindowData<'a>>;

    fn resize_window(
        &mut self,
        window_data: &mut Self::WindowData<'_>,
        active_window_data: &mut Self::ActiveWindowData<'_>,
        size: SizeU32,
    ) -> Result<()>;
    fn render(
        &mut self,
        window_data: &mut Self::WindowData<'_>,
        active_data: &mut Self::ActiveWindowData<'_>,
    ) -> Result<()>;
}

pub trait Scene<B: RenderBackend>: Clone + Debug {
    fn draw_rect(&mut self, rect: &RenderRect<B>);
    fn draw_text(&mut self, text: &RenderText<B>);

    fn debug_draw_simple_text(&mut self, text: &str, pos: Point, size: FP);
    fn apply_scene(&mut self, scene: &B::Scene, transform: Option<B::Transform>);
    fn reset(&mut self);

    fn new() -> Self;
}

pub struct RenderRect<B: RenderBackend> {
    pub rect: B::Rect,
    pub transform: Option<B::Transform>,
    pub radius: Option<B::BorderRadius>,
    pub brush: B::Brush,
    pub brush_transform: Option<B::Transform>,
    pub border: Option<RenderBorder<B>>,
}

impl<B: RenderBackend> RenderRect<B> {
    pub fn new(rect: B::Rect, brush: B::Brush) -> Self {
        Self {
            rect,
            transform: None,
            radius: None,
            brush,
            brush_transform: None,
            border: None,
        }
    }

    pub fn with_border(rect: B::Rect, brush: B::Brush, border: RenderBorder<B>) -> Self {
        Self {
            rect,
            transform: None,
            radius: None,
            brush,
            brush_transform: None,
            border: Some(border),
        }
    }

    pub fn border(&mut self, border: RenderBorder<B>) {
        self.border = Some(border);
    }

    pub fn transform(&mut self, transform: B::Transform) {
        self.transform = Some(transform);
    }

    pub fn radius(&mut self, radius: B::BorderRadius) {
        self.radius = Some(radius);
    }

    pub fn brush_transform(&mut self, brush_transform: B::Transform) {
        self.brush_transform = Some(brush_transform);
    }
}

pub struct RenderText<B: RenderBackend> {
    pub text: B::Text,
    pub rect: B::Rect,
    pub transform: Option<B::Transform>,
    pub brush: B::Brush,
    pub brush_transform: Option<B::Transform>,
}

impl<B: RenderBackend> RenderText<B> {
    pub fn new(text: B::Text, rect: B::Rect, brush: B::Brush) -> Self {
        Self {
            text,
            rect,
            transform: None,
            brush,
            brush_transform: None,
        }
    }

    pub fn transform(&mut self, transform: B::Transform) {
        self.transform = Some(transform);
    }

    pub fn brush_transform(&mut self, brush_transform: B::Transform) {
        self.brush_transform = Some(brush_transform);
    }
}

pub struct RenderBorder<B: RenderBackend> {
    pub border: B::Border,
    pub transform: Option<B::Transform>,
}

impl<B: RenderBackend> RenderBorder<B> {
    pub fn new(border: B::Border) -> Self {
        Self {
            border,
            transform: None,
        }
    }

    pub fn transform(&mut self, transform: B::Transform) {
        self.transform = Some(transform);
    }
}

pub trait Rect {
    fn new(x: FP, y: FP, width: FP, height: FP) -> Self;

    fn from_point(point: Point, size: Size) -> Self;
}

pub trait Border<B: RenderBackend> {
    fn new(all: B::BorderSide) -> Self;

    fn empty() -> Self;

    fn all(left: B::BorderSide, right: B::BorderSide, top: B::BorderSide, bottom: B::BorderSide) -> Self;

    fn left(&mut self, side: B::BorderSide);

    fn right(&mut self, side: B::BorderSide);

    fn top(&mut self, side: B::BorderSide);

    fn bottom(&mut self, side: B::BorderSide);
}

pub trait BorderSide<B: RenderBackend> {
    fn new(width: FP, style: BorderStyle, brush: B::Brush) -> Self;
}

#[derive(Clone, Copy)]
pub enum BorderStyle {
    Solid,
    Dashed,
    Dotted,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
    None,
    Hidden,
}

impl BorderStyle {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(style: &str) -> Self {
        match style {
            "none" => Self::None,
            "hidden" => Self::Hidden,
            "dotted" => Self::Dotted,
            "dashed" => Self::Dashed,
            "solid" => Self::Solid,
            "double" => Self::Double,
            "groove" => Self::Groove,
            "ridge" => Self::Ridge,
            "inset" => Self::Inset,
            "outset" => Self::Outset,
            _ => Self::None,
        }
    }
}

#[derive(Clone, Copy)]
pub enum Radius {
    Uniform(FP),
    Elliptical(FP, FP),
}

impl Radius {
    pub fn offset(&self) -> Size {
        match self {
            Radius::Uniform(value) => Size::uniform(value.powi(2).div(2.0).sqrt() - *value),
            Radius::Elliptical(x, y) => {
                //TODO: is this correct?

                let theta = (std::f64::consts::PI / 4.0) as FP;
                let ox = x * theta.cos();
                let oy = y * theta.sin();

                Size::new(ox - *x, oy - *y)
            }
        }
    }

    pub fn radi_x(&self) -> FP {
        match self {
            Radius::Uniform(value) => *value,
            Radius::Elliptical(x, _) => *x,
        }
    }

    pub fn radi_y(&self) -> FP {
        match self {
            Radius::Uniform(value) => *value,
            Radius::Elliptical(_, y) => *y,
        }
    }

    pub fn radii(&self) -> [FP; 2] {
        match self {
            Radius::Uniform(value) => [*value, *value],
            Radius::Elliptical(x, y) => [*x, *y],
        }
    }

    pub fn radii_f64(&self) -> (f64, f64) {
        match self {
            Radius::Uniform(value) => (*value as f64, *value as f64),
            Radius::Elliptical(x, y) => (*x as f64, *y as f64),
        }
    }
}

impl From<FP> for Radius {
    fn from(value: FP) -> Self {
        Radius::Uniform(value)
    }
}

impl From<[FP; 2]> for Radius {
    fn from(value: [FP; 2]) -> Self {
        Radius::Elliptical(value[0], value[1])
    }
}

impl From<(FP, FP)> for Radius {
    fn from(value: (FP, FP)) -> Self {
        Radius::Elliptical(value.0, value.1)
    }
}

impl From<Radius> for (f64, f64) {
    fn from(value: Radius) -> Self {
        match value {
            Radius::Uniform(value) => (value as f64, value as f64),
            Radius::Elliptical(x, y) => (x as f64, y as f64),
        }
    }
}

impl From<Radius> for f64 {
    fn from(value: Radius) -> Self {
        match value {
            Radius::Uniform(value) => value as f64,
            Radius::Elliptical(x, y) => (x * y).sqrt() as f64,
        }
    }
}

impl From<Radius> for FP {
    fn from(value: Radius) -> Self {
        match value {
            Radius::Uniform(value) => value,
            Radius::Elliptical(x, y) => (x * y).sqrt(),
        }
    }
}

impl From<Radius> for [FP; 2] {
    fn from(value: Radius) -> Self {
        match value {
            Radius::Uniform(value) => [value, value],
            Radius::Elliptical(x, y) => [x, y],
        }
    }
}

impl From<Radius> for (FP, FP) {
    fn from(value: Radius) -> Self {
        match value {
            Radius::Uniform(value) => (value, value),
            Radius::Elliptical(x, y) => (x, y),
        }
    }
}

pub trait BorderRadius:
    Sized
    + From<FP>
    + From<Radius>
    + From<[FP; 4]>
    + From<[Radius; 4]>
    + From<[FP; 8]>
    + From<(FP, FP, FP, FP)>
    + From<(Radius, Radius, Radius, Radius)>
    + From<(FP, FP, FP, FP, FP, FP, FP, FP)>
{
    fn empty() -> Self {
        Self::uniform(0.0)
    }
    fn uniform(radius: FP) -> Self {
        Self::from(radius)
    }
    fn uniform_radius(radius: Radius) -> Self;
    fn uniform_elliptical(radius_x: FP, radius_y: FP) -> Self {
        Self::from([radius_x, radius_y, radius_x, radius_y])
    }

    fn all(radius: FP) -> Self {
        let radius = radius.into();
        Self::all_radius(radius, radius, radius, radius)
    }
    fn all_elliptical(&self, radius_x: FP, radius_y: FP) -> Self {
        let radius = Radius::Elliptical(radius_x, radius_y);

        Self::all_radius(radius, radius, radius, radius)
    }
    fn all_radius(tl: Radius, tr: Radius, dl: Radius, dr: Radius) -> Self;

    fn top_left(&mut self, radius: FP) {
        self.top_left_radius(radius.into());
    }
    fn top_left_elliptical(&mut self, radius_x: FP, radius_y: FP) {
        self.top_left_radius(Radius::Elliptical(radius_x, radius_y));
    }
    fn top_left_radius(&mut self, radius: Radius);

    fn top_right(&mut self, radius: FP) {
        self.top_right_radius(radius.into());
    }
    fn top_right_elliptical(&mut self, radius_x: FP, radius_y: FP) {
        self.top_right_radius(Radius::Elliptical(radius_x, radius_y));
    }
    fn top_right_radius(&mut self, radius: Radius);

    fn bottom_left(&mut self, radius: FP) {
        self.bottom_left_radius(radius.into());
    }
    fn bottom_left_elliptical(&mut self, radius_x: FP, radius_y: FP) {
        self.bottom_left_radius(Radius::Elliptical(radius_x, radius_y));
    }
    fn bottom_left_radius(&mut self, radius: Radius);

    fn bottom_right(&mut self, radius: FP) {
        self.bottom_right_radius(radius.into());
    }
    fn bottom_right_elliptical(&mut self, radius_x: FP, radius_y: FP) {
        self.bottom_right_radius(Radius::Elliptical(radius_x, radius_y));
    }
    fn bottom_right_radius(&mut self, radius: Radius);
}

pub trait Transform: Sized + Mul<Self> + MulAssign + Clone + Send {
    const IDENTITY: Self;
    const FLIP_X: Self;
    const FLIP_Y: Self;

    fn scale(s: FP) -> Self;
    fn scale_xy(sx: FP, sy: FP) -> Self;

    fn translate(x: FP, y: FP) -> Self;

    fn rotate(angle: FP) -> Self;

    fn rotate_around(angle: FP, center: Point) -> Self;

    fn skew_x(angle: FP) -> Self;

    fn skew_y(angle: FP) -> Self;

    fn skew_xy(angle_x: FP, angle_y: FP) -> Self;

    fn pre_scale(self, s: FP) -> Self;

    fn pre_scale_xy(self, sx: FP, sy: FP) -> Self;

    fn pre_translate(self, x: FP, y: FP) -> Self;

    fn pre_rotate(self, angle: FP) -> Self;

    fn pre_rotate_around(self, angle: FP, center: Point) -> Self;

    fn then_scale(self, s: FP) -> Self;

    fn then_scale_xy(self, sx: FP, sy: FP) -> Self;

    fn then_translate(self, x: FP, y: FP) -> Self;

    fn then_rotate(self, angle: FP) -> Self;

    fn then_rotate_around(self, angle: FP, center: Point) -> Self;

    fn as_matrix(&self) -> [FP; 6];

    fn from_matrix(matrix: [FP; 6]) -> Self;

    fn determinant(&self) -> FP;

    fn inverse(self) -> Self;

    fn with_translation(&self, translation: Point) -> Self;

    fn tx(&self) -> FP {
        self.as_matrix()[4]
    }

    fn ty(&self) -> FP {
        self.as_matrix()[5]
    }

    fn set_xy(&mut self, x: FP, y: FP) {
        let mut matrix = self.as_matrix();
        matrix[4] = x;
        matrix[5] = y;
        *self = Self::from_matrix(matrix);
    }
}

pub trait Text {
    type Font;

    fn new<TL: TextLayout>(node: &TL) -> Self
    where
        TL::Font: Into<Self::Font>;
}

pub struct ColorStop<B: RenderBackend> {
    pub offset: FP,
    pub color: B::Color,
}

pub type ColorStops<B> = SmallVec<[ColorStop<B>; 4]>;

pub trait Gradient<B: RenderBackend> {
    fn new_linear(start: Point, end: Point, stops: ColorStops<B>) -> Self;

    fn new_radial_two_point(
        start_center: Point,
        start_radius: FP,
        end_center: Point,
        end_radius: FP,
        stops: ColorStops<B>,
    ) -> Self;

    fn new_radial(center: Point, radius: FP, stops: ColorStops<B>) -> Self
    where
        Self: Sized,
    {
        Self::new_radial_two_point(center, radius, center, radius, stops)
    }

    fn new_sweep(center: Point, start_angle: FP, end_angle: FP, stops: ColorStops<B>) -> Self;
}

pub trait Color {
    fn new(r: u8, g: u8, b: u8) -> Self
    where
        Self: Sized,
    {
        Self::with_alpha(r, g, b, 255)
    }

    fn with_alpha(r: u8, g: u8, b: u8, a: u8) -> Self;

    fn rgb(r: u8, g: u8, b: u8) -> Self
    where
        Self: Sized,
    {
        Self::new(r, g, b)
    }

    fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self
    where
        Self: Sized,
    {
        Self::with_alpha(r, g, b, a)
    }

    fn tuple3(tup: (u8, u8, u8)) -> Self
    where
        Self: Sized,
    {
        Self::new(tup.0, tup.1, tup.2)
    }

    fn tuple4(tup: (u8, u8, u8, u8)) -> Self
    where
        Self: Sized,
    {
        Self::with_alpha(tup.0, tup.1, tup.2, tup.3)
    }

    fn alpha(self, a: u8) -> Self
    where
        Self: Sized,
    {
        Self::with_alpha(self.r(), self.g(), self.b(), a)
    }

    fn r(&self) -> u8;
    fn g(&self) -> u8;
    fn b(&self) -> u8;
    fn a(&self) -> u8;

    const WHITE: Self;
    const BLACK: Self;
    const RED: Self;
    const GREEN: Self;
    const BLUE: Self;
    const YELLOW: Self;
    const CYAN: Self;
    const MAGENTA: Self;
    const TRANSPARENT: Self;
}

pub trait Image: Clone + Send {
    fn new(size: (FP, FP), data: Vec<u8>) -> Self;

    fn from_img(img: image::DynamicImage) -> Self;

    fn width(&self) -> u32;
    fn height(&self) -> u32;
}

pub trait Brush<B: RenderBackend>: Clone {
    fn gradient(gradient: B::Gradient) -> Self;

    fn color(color: B::Color) -> Self;

    fn image(image: B::Image) -> Self;
}

pub enum ImageBuffer<B: RenderBackend> {
    Image(B::Image),
    Scene(B::Scene, SizeU32),
}

impl<B: RenderBackend> Debug for ImageBuffer<B> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageBuffer::Image(_) => write!(f, "ImageBuffer::Image"),
            ImageBuffer::Scene(_, size) => write!(f, "ImageBuffer::Scene({:?})", size),
        }
    }
}

impl<B: RenderBackend> Clone for ImageBuffer<B> {
    fn clone(&self) -> Self {
        match self {
            ImageBuffer::Image(img) => ImageBuffer::Image(img.clone()),
            ImageBuffer::Scene(scene, size) => ImageBuffer::Scene(scene.clone(), *size),
        }
    }
}

impl<B: RenderBackend> ImageBuffer<B> {
    pub fn width(&self) -> u32 {
        match self {
            ImageBuffer::Image(img) => img.width(),
            ImageBuffer::Scene(_, size) => size.width,
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            ImageBuffer::Image(img) => img.height(),
            ImageBuffer::Scene(_, size) => size.height,
        }
    }

    pub fn size(&self) -> SizeU32 {
        match self {
            ImageBuffer::Image(img) => SizeU32::new(img.width(), img.height()),
            ImageBuffer::Scene(_, size) => *size,
        }
    }

    pub fn size_tuple(&self) -> (FP, FP) {
        match self {
            ImageBuffer::Image(img) => (img.width() as FP, img.height() as FP),
            ImageBuffer::Scene(_, size) => (size.width as FP, size.height as FP),
        }
    }
}

pub enum ImageCacheEntry<'a, B: RenderBackend> {
    Image(&'a ImageBuffer<B>),
    Pending,
    None,
}

pub trait ImgCache<B: RenderBackend>: Sized + Send {
    fn new() -> Self {
        Self::with_capacity(0)
    }

    fn with_capacity(capacity: usize) -> Self;

    fn add(&mut self, url: String, img: ImageBuffer<B>, size: Option<SizeU32>);

    fn add_pending(&mut self, url: String);

    fn get(&self, url: &str) -> ImageCacheEntry<B>;
}

pub struct NodeDesc {
    pub id: u64,
    pub name: String,
    pub children: Vec<NodeDesc>,
    pub attributes: Vec<(String, String)>,
    pub properties: Vec<(String, String)>,
    pub pos: Point,
    pub size: Size,
    pub text: Option<String>,
}

impl Display for NodeDesc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.write(f, 0)
    }
}

impl NodeDesc {
    fn write(&self, f: &mut impl Write, indent: usize) -> std::fmt::Result {
        if self.name == "<unknown>" || self.name.is_empty() {
            for child in &self.children {
                child.write(f, indent)?;
            }
            return Ok(());
        }

        for _ in 0..indent {
            write!(f, "  ")?;
        }

        let is_special = self.name.starts_with('#') || self.name.starts_with('!');

        if is_special {
            write!(f, "{}: {}", self.id, self.name)?;
        } else {
            write!(f, "{}: <{}", self.id, self.name)?;
        }

        if let Some(text) = &self.text {
            write!(f, " =[{}]", text)?;
        }

        for (key, value) in &self.attributes {
            write!(f, " {}=\"{}\"", key, value)?;
        }

        if !is_special {
            write!(f, ">")?;
        }

        writeln!(
            f,
            " @ ({}x{}) [{}x{}]",
            self.pos.x, self.pos.y, self.size.width, self.size.height
        )?;

        for child in &self.children {
            child.write(f, indent + 1)?;
        }

        if !is_special {
            for _ in 0..indent {
                write!(f, "  ")?;
            }
            writeln!(f, "{}: </{}>", self.id, self.name)?;
        }

        Ok(())
    }
}
