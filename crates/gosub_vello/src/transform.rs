use std::ops::{Mul, MulAssign};

use vello::kurbo::Affine;

use gosub_interface::render_backend::Transform as TTransform;
use gosub_shared::geo::{Point, FP};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform(pub(crate) Affine);

impl From<Affine> for Transform {
    fn from(transform: Affine) -> Self {
        Transform(transform)
    }
}

impl Mul<Self> for Transform {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Transform(self.0 * rhs.0)
    }
}

impl MulAssign for Transform {
    fn mul_assign(&mut self, rhs: Self) {
        self.0 *= rhs.0;
    }
}

impl TTransform for Transform {
    const IDENTITY: Self = Transform(Affine::IDENTITY);
    const FLIP_X: Self = Transform(Affine::FLIP_X);
    const FLIP_Y: Self = Transform(Affine::FLIP_Y);

    fn scale(s: FP) -> Self {
        Affine::scale(f64::from(s)).into()
    }

    fn scale_xy(sx: FP, sy: FP) -> Self {
        Affine::scale_non_uniform(f64::from(sx), f64::from(sy)).into()
    }

    fn translate(x: FP, y: FP) -> Self {
        Affine::translate((f64::from(x), f64::from(y))).into()
    }

    fn rotate(angle: FP) -> Self {
        Affine::rotate(f64::from(angle)).into()
    }

    fn rotate_around(angle: FP, center: Point) -> Self {
        Affine::rotate_about(f64::from(angle), (f64::from(center.x), f64::from(center.y)).into()).into()
    }

    fn skew_x(angle: FP) -> Self {
        Affine::skew(f64::from(angle), 0.0).into()
    }

    fn skew_y(angle: FP) -> Self {
        Affine::skew(0.0, f64::from(angle)).into()
    }

    fn skew_xy(angle_x: FP, angle_y: FP) -> Self {
        Affine::skew(f64::from(angle_x), f64::from(angle_y)).into()
    }

    fn pre_scale(self, s: FP) -> Self {
        self.0.pre_scale(f64::from(s)).into()
    }

    fn pre_scale_xy(self, sx: FP, sy: FP) -> Self {
        self.0.pre_scale_non_uniform(f64::from(sx), f64::from(sy)).into()
    }

    fn pre_translate(self, x: FP, y: FP) -> Self {
        self.0.pre_translate((f64::from(x), f64::from(y)).into()).into()
    }

    fn pre_rotate(self, angle: FP) -> Self {
        self.0.pre_rotate(f64::from(angle)).into()
    }

    fn pre_rotate_around(self, angle: FP, center: Point) -> Self {
        self.0
            .pre_rotate_about(f64::from(angle), (f64::from(center.x), f64::from(center.y)).into())
            .into()
    }

    fn then_scale(self, s: FP) -> Self {
        self.0.then_scale(f64::from(s)).into()
    }

    fn then_scale_xy(self, sx: FP, sy: FP) -> Self {
        self.0.then_scale_non_uniform(f64::from(sx), f64::from(sy)).into()
    }

    fn then_translate(self, x: FP, y: FP) -> Self {
        self.0.then_translate((f64::from(x), f64::from(y)).into()).into()
    }

    fn then_rotate(self, angle: FP) -> Self {
        self.0.then_rotate(f64::from(angle)).into()
    }

    fn then_rotate_around(self, angle: FP, center: Point) -> Self {
        self.0
            .then_rotate_about(f64::from(angle), (f64::from(center.x), f64::from(center.y)).into())
            .into()
    }

    fn as_matrix(&self) -> [FP; 6] {
        let matrix = self.0.as_coeffs();
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
        Affine::new([
            f64::from(matrix[0]),
            f64::from(matrix[1]),
            f64::from(matrix[2]),
            f64::from(matrix[3]),
            f64::from(matrix[4]),
            f64::from(matrix[5]),
        ])
        .into()
    }

    fn determinant(&self) -> FP {
        self.0.determinant() as FP
    }

    fn inverse(self) -> Self {
        self.0.inverse().into()
    }

    fn with_translation(&self, translation: Point) -> Self {
        self.0
            .with_translation((translation.x64(), translation.y64()).into())
            .into()
    }
}
