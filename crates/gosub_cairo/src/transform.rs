use std::ops::{Mul, MulAssign};

use gosub_shared::render_backend::geo::{Point, FP};
use gosub_shared::render_backend::Transform as TTransform;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Transform {
    const fn new(elements: [f64; 6]) -> Transform {
        Transform {
            a: elements[0],
            b: elements[1],
            c: elements[2],
            d: elements[3],
            e: elements[4],
            f: elements[5],
        }
    }

    pub fn identity() -> Self {
        Transform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn translate(tx: f64, ty: f64) -> Self {
        Transform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: tx,
            f: ty,
        }
    }

    pub const fn scale(sx: f64, sy: f64) -> Self {
        Transform {
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
        Transform {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn multiply(&self, other: &Self) -> Self {
        Transform {
            a: self.a * other.a + self.b * other.c,
            b: self.a * other.b + self.b * other.d,
            c: self.c * other.a + self.d * other.c,
            d: self.c * other.b + self.d * other.d,
            e: self.e * other.a + self.f * other.c + other.e,
            f: self.e * other.b + self.f * other.d + other.f,
        }
    }

    pub fn flip_x() -> Self {
        Transform {
            a: -1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn flip_y() -> Self {
        Transform {
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
        Transform {
            a: cos_y,
            b: sin_x,
            c: sin_y,
            d: cos_x,
            e: 0.0,
            f: 0.0,
        }
    }

    pub fn determinant(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    pub fn inverse(&self) -> Self {
        let det = self.determinant();
        Transform {
            a: self.d / det,
            b: -self.b / det,
            c: -self.c / det,
            d: self.a / det,
            e: (self.c * self.f - self.d * self.e) / det,
            f: (self.b * self.e - self.a * self.f) / det,
        }
    }

    pub fn to_cairo_matrix(&self) -> [f64; 6] {
        [self.a, self.b, self.c, self.d, self.e, self.f]
    }

    pub fn rotate_about(angle: f64, center: (f64, f64)) -> Self {
        let (sin, cos) = angle.sin_cos();
        let (cx, cy) = center;
        let a = cos;
        let b = sin;
        let c = -sin;
        let d = cos;
        let e = cx - cx * cos + cy * sin;
        let f = cy - cx * sin - cy * cos;
        Transform { a, b, c, d, e, f }
    }

    pub fn as_coeffs(&self) -> [f64; 6] {
        [self.a, self.b, self.c, self.d, self.e, self.f]
    }
}

impl Mul<Self> for Transform {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        self.multiply(&rhs)
    }
}

impl MulAssign for Transform {
    fn mul_assign(&mut self, rhs: Self) {
        let res = self.multiply(&rhs);

        self.a = res.a;
        self.b = res.b;
        self.c = res.c;
        self.d = res.d;
        self.e = res.e;
        self.f = res.f;
    }
}


// pub const IDENTITY: Transform = Transform::scale(1.0, 1.0);
// pub const FLIP_Y: Transform = Transform::new([1.0, 0., 0., -1.0, 0., 0.]);
// pub const FLIP_X: Transform = Transform::new([-1.0, 0., 0., 1.0, 0., 0.]);

impl TTransform for Transform {
    const IDENTITY: Self = Transform::scale(1.0, 1.0);
    const FLIP_X: Self = Transform::new([1.0, 0., 0., -1.0, 0., 0.]);
    const FLIP_Y: Self = Transform::new([-1.0, 0., 0., 1.0, 0., 0.]);

    fn scale(s: FP) -> Self {
        Transform::scale(s as f64, s as f64)
    }

    fn scale_xy(sx: FP, sy: FP) -> Self {
        Transform::scale(sx as f64, sy as f64)
    }

    fn translate(x: FP, y: FP) -> Self {
        Transform::translate(x as f64, y as f64)
    }

    fn rotate(angle: FP) -> Self {
        Transform::rotate(angle as f64)
    }

    fn rotate_around(angle: FP, center: Point) -> Self {
        Transform::rotate_about(angle as f64, (center.x as f64, center.y as f64))
    }

    fn skew_x(angle: FP) -> Self {
        Transform::skew(angle as f64, 0.0)
    }

    fn skew_y(angle: FP) -> Self {
        Transform::skew(0.0, angle as f64)
    }

    fn skew_xy(angle_x: FP, angle_y: FP) -> Self {
        Transform::skew(angle_x as f64, angle_y as f64)
    }

    fn pre_scale(self, s: FP) -> Self {
        Transform::scale(s as f64, s as f64) * self
    }

    fn pre_scale_xy(self, sx: FP, sy: FP) -> Self {
        Transform::scale_xy(sx, sy) * self
    }

    fn pre_translate(self, x: FP, y: FP) -> Self {
        Transform::translate(x as f64, y as f64) * self
    }

    fn pre_rotate(self, angle: FP) -> Self {
        Transform::rotate(angle as f64) * self
    }

    fn pre_rotate_around(self, angle: FP, center: Point) -> Self {
        Transform::rotate_about(angle as f64, (center.x as f64, center.y as f64)) * self
    }

    fn then_scale(self, s: FP) -> Self {
        self * Transform::scale(s as f64, s as f64)
    }

    fn then_scale_xy(self, sx: FP, sy: FP) -> Self {
        self * Transform::scale(sx as f64, sy as f64)
    }

    fn then_translate(self, x: FP, y: FP) -> Self {
        self * Transform::translate(x as f64, y as f64)
    }

    fn then_rotate(self, angle: FP) -> Self {
        self * Transform::rotate(angle as f64)
    }

    fn then_rotate_around(self, angle: FP, center: Point) -> Self {
        self * Transform::rotate_about(angle as f64, (center.x as f64, center.y as f64))
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
        Transform::new([
            matrix[0] as f64,
            matrix[1] as f64,
            matrix[2] as f64,
            matrix[3] as f64,
            matrix[4] as f64,
            matrix[5] as f64,
        ])
    }

    fn determinant(&self) -> FP {
        self.determinant() as FP
    }

    fn inverse(self) -> Self {
        Transform::inverse(&self)
    }

    fn with_translation(&self, _translation: Point) -> Self {
        todo!("with_translation")
        // self
        //     .with_translation((translation.x64(), translation.y64()).into())
        //     .into()
    }
}
