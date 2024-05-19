use std::fmt::Debug;
use std::ops::{Div, Mul, MulAssign};

use smallvec::SmallVec;

pub trait RenderBackend: Sized + Debug {
    type Rect: Rect;
    type Border: Border<Self>;
    type BorderSide: BorderSide<Self>;
    type BorderRadius: BorderRadius;
    type Transform: Transform;
    type PreRenderText: PreRenderText<Self>;
    type Text: Text<Self>;
    type Gradient: Gradient<Self>;
    type Color: Color;
    type Image: Image;
    type Brush: Brush<Self>;

    fn draw_rect(&mut self, rect: &RenderRect<Self>);
    fn draw_text(&mut self, text: &RenderText<Self>);
    fn reset(&mut self);
}

pub type FP = f32;

#[derive(Clone, Copy)]
pub struct Point {
    pub x: FP,
    pub y: FP,
}

#[derive(Debug)]
pub struct Size {
    pub width: FP,
    pub height: FP,
}

impl Size {
    pub fn new(width: FP, height: FP) -> Self {
        Self { width, height }
    }

    pub fn uniform(size: FP) -> Self {
        Self {
            width: size,
            height: size,
        }
    }
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

    fn all(
        left: B::BorderSide,
        right: B::BorderSide,
        top: B::BorderSide,
        bottom: B::BorderSide,
    ) -> Self;

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

pub trait Transform: Sized + Mul<Self> + MulAssign {
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

    fn with_translation(&self, translation: (FP, FP)) -> Self;
}

pub trait PreRenderText<B: RenderBackend> {
    fn new(text: String, font: Option<Vec<String>>, size: FP) -> Self;

    fn with_lh(text: String, font: Option<Vec<String>>, size: FP, line_height: FP) -> Self;

    fn prerender(&mut self, backend: &B) -> Size;
    fn value(&self) -> &str;
    fn fs(&self) -> FP;

    //TODO: Who should be responsible for line breaking if the text is too long?
}

pub trait Text<B: RenderBackend> {
    fn new(pre: &mut B::PreRenderText, backend: &B) -> Self;
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

pub trait Image {
    fn new(size: (FP, FP), data: Vec<u8>) -> Self;

    fn from_img(img: &image::DynamicImage) -> Self;
}

pub trait Brush<B: RenderBackend> {
    fn gradient(gradient: B::Gradient) -> Self;

    fn color(color: B::Color) -> Self;

    fn image(image: B::Image) -> Self;
}
