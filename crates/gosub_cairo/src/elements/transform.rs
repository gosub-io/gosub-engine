use std::ops::{Mul, MulAssign};

use gosub_interface::render_backend::Transform as TTransform;
use gosub_shared::geo::{Point, FP};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GsTransform {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl GsTransform {
    const fn new(elements: [f64; 6]) -> Self {
        Self {
            a: elements[0],
            b: elements[1],
            c: elements[2],
            d: elements[3],
            e: elements[4],
            f: elements[5],
        }
    }

    pub const fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    pub const fn translate(tx: f64, ty: f64) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    pub const fn scale(sx: f64, sy: f64) -> Self {
        Self {
            a: sx,
            b: 0.0,
            c: 0.0,
            d: sy,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn rotate(angle: f64) -> Self {
        let (sin, cos) = angle.sin_cos();
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn multiply(&self, other: &Self) -> Self {
        Self {
            a: self.a.mul_add(other.a, self.b * other.c),
            b: self.a.mul_add(other.b, self.b * other.d),
            c: self.c.mul_add(other.a, self.d * other.c),
            d: self.c.mul_add(other.b, self.d * other.d),
            e: self.e.mul_add(other.a, self.f * other.c) + other.e,
            f: self.e.mul_add(other.b, self.f * other.d) + other.f,
        }
    }

    pub const fn flip_x() -> Self {
        Self {
            a: -1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    pub const fn flip_y() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: -1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn skew(angle_x: f64, angle_y: f64) -> Self {
        let (sin_x, cos_x) = angle_x.sin_cos();
        let (sin_y, cos_y) = angle_y.sin_cos();

        Self {
            a: cos_y,
            b: sin_x,
            c: sin_y,
            d: cos_x,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn determinant(&self) -> f64 {
        self.a.mul_add(self.d, -(self.b * self.c))
    }

    pub fn inverse(&self) -> Self {
        let det = self.determinant();

        Self {
            a: self.d / det,
            b: -self.b / det,
            c: -self.c / det,
            d: self.a / det,
            e: self.c.mul_add(self.f, -(self.d * self.e)) / det,
            f: self.b.mul_add(self.e, -(self.a * self.f)) / det,
        }
    }

    pub const fn to_cairo_matrix(self) -> [f64; 6] {
        [self.a, self.b, self.c, self.d, self.e, self.f]
    }

    #[allow(clippy::many_single_char_names)]
    pub fn rotate_about(angle: f64, center: (f64, f64)) -> Self {
        let (sin, cos) = angle.sin_cos();
        let (cx, cy) = center;
        let a = cos;
        let b = sin;
        let c = -sin;
        let d = cos;
        let e = cy.mul_add(sin, cx.mul_add(-cos, cx));
        let f = cy.mul_add(-cos, cx.mul_add(-sin, cy));

        Self { a, b, c, d, e, f }
    }

    pub const fn as_coeffs(&self) -> [f64; 6] {
        [self.a, self.b, self.c, self.d, self.e, self.f]
    }
}

impl Mul<Self> for GsTransform {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        self.multiply(&rhs)
    }
}

impl MulAssign for GsTransform {
    fn mul_assign(&mut self, rhs: Self) {
        let result = self.multiply(&rhs);

        self.a = result.a;
        self.b = result.b;
        self.c = result.c;
        self.d = result.d;
        self.e = result.e;
        self.f = result.f;
    }
}

impl TTransform for GsTransform {
    const IDENTITY: Self = Self::scale(1.0, 1.0);
    const FLIP_X: Self = Self::new([1.0, 0., 0., -1.0, 0., 0.]);
    const FLIP_Y: Self = Self::new([-1.0, 0., 0., 1.0, 0., 0.]);

    fn scale(s: FP) -> Self {
        Self::scale(f64::from(s), f64::from(s))
    }

    fn scale_xy(sx: FP, sy: FP) -> Self {
        Self::scale(f64::from(sx), f64::from(sy))
    }

    fn translate(x: FP, y: FP) -> Self {
        Self::translate(f64::from(x), f64::from(y))
    }

    fn rotate(angle: FP) -> Self {
        Self::rotate(f64::from(angle))
    }

    fn rotate_around(angle: FP, center: Point) -> Self {
        Self::rotate_about(f64::from(angle), (f64::from(center.x), f64::from(center.y)))
    }

    fn skew_x(angle: FP) -> Self {
        Self::skew(f64::from(angle), 0.0)
    }

    fn skew_y(angle: FP) -> Self {
        Self::skew(0.0, f64::from(angle))
    }

    fn skew_xy(angle_x: FP, angle_y: FP) -> Self {
        Self::skew(f64::from(angle_x), f64::from(angle_y))
    }

    fn pre_scale(self, s: FP) -> Self {
        Self::scale(f64::from(s), f64::from(s)) * self
    }

    fn pre_scale_xy(self, sx: FP, sy: FP) -> Self {
        Self::scale_xy(sx, sy) * self
    }

    fn pre_translate(self, x: FP, y: FP) -> Self {
        Self::translate(f64::from(x), f64::from(y)) * self
    }

    fn pre_rotate(self, angle: FP) -> Self {
        Self::rotate(f64::from(angle)) * self
    }

    fn pre_rotate_around(self, angle: FP, center: Point) -> Self {
        Self::rotate_about(f64::from(angle), (f64::from(center.x), f64::from(center.y))) * self
    }

    fn then_scale(self, s: FP) -> Self {
        self * Self::scale(f64::from(s), f64::from(s))
    }

    fn then_scale_xy(self, sx: FP, sy: FP) -> Self {
        self * Self::scale(f64::from(sx), f64::from(sy))
    }

    fn then_translate(self, x: FP, y: FP) -> Self {
        self * Self::translate(f64::from(x), f64::from(y))
    }

    fn then_rotate(self, angle: FP) -> Self {
        self * Self::rotate(f64::from(angle))
    }

    fn then_rotate_around(self, angle: FP, center: Point) -> Self {
        self * Self::rotate_about(f64::from(angle), (f64::from(center.x), f64::from(center.y)))
    }

    fn as_matrix(&self) -> [FP; 6] {
        let matrix = self.as_coeffs();
        [
            matrix[0] as FP,
            matrix[1] as FP,
            matrix[2] as FP,
            matrix[3] as FP,
            matrix[4] as FP,
            matrix[5] as FP,
        ]
    }

    fn from_matrix(matrix: [FP; 6]) -> Self {
        Self::new([
            f64::from(matrix[0]),
            f64::from(matrix[1]),
            f64::from(matrix[2]),
            f64::from(matrix[3]),
            f64::from(matrix[4]),
            f64::from(matrix[5]),
        ])
    }

    fn determinant(&self) -> FP {
        self.determinant() as FP
    }

    fn inverse(self) -> Self {
        Self::inverse(&self)
    }

    fn with_translation(&self, translation: Point) -> Self {
        let mut this = *self;

        this.e = f64::from(translation.x);
        this.f = f64::from(translation.y);

        this
    }
}
