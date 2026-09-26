//! Conversion between the colour spaces css-color-4 defines.
//!
//! Every space converts to and from CIE XYZ with a D65 white, as in the spec's sample code
//! (css-color-4 §18, "Sample code for color conversions"). The matrices are the ones printed
//! there, written as rationals where the spec gives them, so they hold at `f64` precision. The
//! WPT relative-colour tests expect a colour taken through three spaces and back to stay within
//! 1e-4.
//!
//! Nothing here clamps. A display-p3 green is outside sRGB, so converting it gives sRGB channels
//! below 0 and above 1, which relative colour syntax has to pass on (css-color-5 §4). Gamut
//! mapping happens when painting.
//!
//! Components are in the units each notation writes them in: 0-1 for the RGB spaces and XYZ,
//! 0-100 for Lab/LCH lightness and for the HSL/HWB percentages, 0-1 for Oklab lightness, and
//! degrees for every hue. A polar space reports the hue of an achromatic colour as NaN, which
//! the caller turns into a missing component (css-color-4 §4.4.1).

/// A colour space a component triple can be in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Space {
    Srgb,
    SrgbLinear,
    DisplayP3,
    DisplayP3Linear,
    A98Rgb,
    ProphotoRgb,
    Rec2020,
    XyzD50,
    XyzD65,
    Lab,
    Lch,
    Oklab,
    Oklch,
    Hsl,
    Hwb,
}

impl Space {
    /// Index of the hue component, for the spaces that have one.
    #[must_use]
    pub fn hue_index(self) -> Option<usize> {
        match self {
            Space::Lch | Space::Oklch => Some(2),
            Space::Hsl | Space::Hwb => Some(0),
            _ => None,
        }
    }
}

/// The kind of each component, used to decide which components of two spaces are analogous
/// (css-color-4 §12.2) when carrying a missing component across a conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Analogy {
    Red,
    Green,
    Blue,
    Lightness,
    Colorfulness,
    Hue,
    OpponentA,
    OpponentB,
    /// Analogous to nothing: HWB's whiteness and blackness.
    None,
}

impl Space {
    /// What each of this space's components is analogous to.
    #[must_use]
    pub fn component_kinds(self) -> [Analogy; 3] {
        use Analogy::{Blue, Colorfulness, Green, Hue, Lightness, OpponentA, OpponentB, Red};
        match self {
            Space::Srgb
            | Space::SrgbLinear
            | Space::DisplayP3
            | Space::DisplayP3Linear
            | Space::A98Rgb
            | Space::ProphotoRgb
            | Space::Rec2020
            | Space::XyzD50
            | Space::XyzD65 => [Red, Green, Blue],
            Space::Lab | Space::Oklab => [Lightness, OpponentA, OpponentB],
            Space::Lch | Space::Oklch => [Lightness, Colorfulness, Hue],
            Space::Hsl => [Hue, Colorfulness, Lightness],
            Space::Hwb => [Hue, Analogy::None, Analogy::None],
        }
    }
}

type Matrix = [[f64; 3]; 3];

fn multiply(m: &Matrix, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

/// The D50 white point, which Lab, LCH, ProPhoto and `xyz-d50` are relative to.
const D50: [f64; 3] = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];

// --- the RGB spaces ---------------------------------------------------------------------------

const SRGB_TO_XYZ: Matrix = [
    [506_752.0 / 1_228_815.0, 87_881.0 / 245_763.0, 12_673.0 / 70_218.0],
    [87_098.0 / 409_605.0, 175_762.0 / 245_763.0, 12_673.0 / 175_545.0],
    [7_918.0 / 409_605.0, 87_881.0 / 737_289.0, 1_001_167.0 / 1_053_270.0],
];
const XYZ_TO_SRGB: Matrix = [
    [12_831.0 / 3_959.0, -329.0 / 214.0, -1_974.0 / 3_959.0],
    [-851_781.0 / 878_810.0, 1_648_619.0 / 878_810.0, 36_519.0 / 878_810.0],
    [705.0 / 12_673.0, -2_585.0 / 12_673.0, 705.0 / 667.0],
];

const P3_TO_XYZ: Matrix = [
    [608_311.0 / 1_250_200.0, 189_793.0 / 714_400.0, 198_249.0 / 1_000_160.0],
    [35_783.0 / 156_275.0, 247_089.0 / 357_200.0, 198_249.0 / 2_500_400.0],
    [0.0, 32_229.0 / 714_400.0, 5_220_557.0 / 5_000_800.0],
];
const XYZ_TO_P3: Matrix = [
    [446_124.0 / 178_915.0, -333_277.0 / 357_830.0, -72_051.0 / 178_915.0],
    [-14_852.0 / 17_905.0, 63_121.0 / 35_810.0, 423.0 / 17_905.0],
    [11_844.0 / 330_415.0, -50_337.0 / 660_830.0, 316_169.0 / 330_415.0],
];

const A98_TO_XYZ: Matrix = [
    [573_536.0 / 994_567.0, 263_643.0 / 1_420_810.0, 187_206.0 / 994_567.0],
    [
        591_459.0 / 1_989_134.0,
        6_239_551.0 / 9_945_670.0,
        374_412.0 / 4_972_835.0,
    ],
    [
        53_769.0 / 1_989_134.0,
        351_524.0 / 4_972_835.0,
        4_929_758.0 / 4_972_835.0,
    ],
];
const XYZ_TO_A98: Matrix = [
    [1_829_569.0 / 896_150.0, -506_331.0 / 896_150.0, -308_931.0 / 896_150.0],
    [-851_781.0 / 878_810.0, 1_648_619.0 / 878_810.0, 36_519.0 / 878_810.0],
    [
        16_779.0 / 1_248_040.0,
        -147_721.0 / 1_248_040.0,
        1_266_979.0 / 1_248_040.0,
    ],
];

const REC2020_TO_XYZ: Matrix = [
    [
        63_426_534.0 / 99_577_255.0,
        20_160_776.0 / 139_408_157.0,
        47_086_771.0 / 278_816_314.0,
    ],
    [
        26_158_966.0 / 99_577_255.0,
        472_592_308.0 / 697_040_785.0,
        8_267_143.0 / 139_408_157.0,
    ],
    [0.0, 19_567_812.0 / 697_040_785.0, 295_819_943.0 / 278_816_314.0],
];
const XYZ_TO_REC2020: Matrix = [
    [
        30_757_411.0 / 17_917_100.0,
        -6_372_589.0 / 17_917_100.0,
        -4_539_589.0 / 17_917_100.0,
    ],
    [
        -19_765_991.0 / 29_648_200.0,
        47_925_759.0 / 29_648_200.0,
        467_509.0 / 29_648_200.0,
    ],
    [
        792_561.0 / 44_930_125.0,
        -1_921_689.0 / 44_930_125.0,
        42_328_811.0 / 44_930_125.0,
    ],
];

/// ProPhoto is defined against D50, so these go to and from `xyz-d50`.
const PROPHOTO_TO_XYZ_D50: Matrix = [
    [
        0.797_766_644_900_642_3,
        0.135_181_297_400_533_08,
        0.031_347_734_128_392_2,
    ],
    [0.288_074_828_819_401_3, 0.711_835_234_241_873, 0.000_089_936_938_725_64],
    [0.0, 0.0, 0.825_104_602_510_460_2],
];
const XYZ_D50_TO_PROPHOTO: Matrix = [
    [
        1.345_786_881_647_158_3,
        -0.255_572_087_379_794_64,
        -0.051_101_864_975_545_26,
    ],
    [
        -0.544_630_705_124_901_9,
        1.508_247_742_845_146_8,
        0.020_527_447_436_421_39,
    ],
    [0.0, 0.0, 1.211_967_545_638_945_2],
];

/// Bradford chromatic adaptation between the two white points.
const D65_TO_D50: Matrix = [
    [
        1.047_929_792_544_997,
        0.022_946_870_601_609_652,
        -0.050_192_266_289_205_24,
    ],
    [
        0.029_627_808_770_055_99,
        0.990_434_426_753_879_9,
        -0.017_073_799_063_418_826,
    ],
    [
        -0.009_243_040_646_204_504,
        0.015_055_191_490_298_152,
        0.751_874_281_428_137_1,
    ],
];
const D50_TO_D65: Matrix = [
    [
        0.955_473_421_488_075,
        -0.023_098_454_948_764_71,
        0.063_259_243_200_570_72,
    ],
    [
        -0.028_369_709_333_863_7,
        1.009_995_398_081_304_1,
        0.021_041_441_191_917_323,
    ],
    [
        0.012_314_014_864_481_998,
        -0.020_507_649_298_898_964,
        1.330_365_926_242_124,
    ],
];

/// The sRGB transfer function, which display-p3 shares. Extended to negative values by
/// mirroring, so an out-of-gamut channel survives the round trip.
fn srgb_to_linear(c: f64) -> f64 {
    let abs = c.abs();
    if abs <= 0.040_45 {
        c / 12.92
    } else {
        c.signum() * ((abs + 0.055) / 1.055).powf(2.4)
    }
}

fn srgb_from_linear(c: f64) -> f64 {
    let abs = c.abs();
    if abs > 0.003_130_8 {
        c.signum() * (1.055 * abs.powf(1.0 / 2.4) - 0.055)
    } else {
        12.92 * c
    }
}

fn a98_to_linear(c: f64) -> f64 {
    c.signum() * c.abs().powf(563.0 / 256.0)
}

fn a98_from_linear(c: f64) -> f64 {
    c.signum() * c.abs().powf(256.0 / 563.0)
}

fn prophoto_to_linear(c: f64) -> f64 {
    const ET2: f64 = 16.0 / 512.0;
    let abs = c.abs();
    if abs <= ET2 {
        c / 16.0
    } else {
        c.signum() * abs.powf(1.8)
    }
}

fn prophoto_from_linear(c: f64) -> f64 {
    const ET: f64 = 1.0 / 512.0;
    let abs = c.abs();
    if abs >= ET {
        c.signum() * abs.powf(1.0 / 1.8)
    } else {
        16.0 * c
    }
}

const REC2020_ALPHA: f64 = 1.099_296_826_809_44;
const REC2020_BETA: f64 = 0.018_053_968_510_807;

fn rec2020_to_linear(c: f64) -> f64 {
    let abs = c.abs();
    if abs < REC2020_BETA * 4.5 {
        c / 4.5
    } else {
        c.signum() * ((abs + REC2020_ALPHA - 1.0) / REC2020_ALPHA).powf(1.0 / 0.45)
    }
}

fn rec2020_from_linear(c: f64) -> f64 {
    let abs = c.abs();
    if abs > REC2020_BETA {
        c.signum() * (REC2020_ALPHA * abs.powf(0.45) - (REC2020_ALPHA - 1.0))
    } else {
        4.5 * c
    }
}

fn each(v: [f64; 3], f: fn(f64) -> f64) -> [f64; 3] {
    [f(v[0]), f(v[1]), f(v[2])]
}

// --- Lab and Oklab ----------------------------------------------------------------------------

const KAPPA: f64 = 24_389.0 / 27.0;
const EPSILON: f64 = 216.0 / 24_389.0;

fn xyz_d50_to_lab(xyz: [f64; 3]) -> [f64; 3] {
    let f = |v: f64| {
        if v > EPSILON {
            v.cbrt()
        } else {
            (KAPPA * v + 16.0) / 116.0
        }
    };
    let [fx, fy, fz] = [f(xyz[0] / D50[0]), f(xyz[1] / D50[1]), f(xyz[2] / D50[2])];
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn lab_to_xyz_d50(lab: [f64; 3]) -> [f64; 3] {
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = lab[1] / 500.0 + fy;
    let fz = fy - lab[2] / 200.0;
    let cube = |f: f64| {
        let cubed = f * f * f;
        if cubed > EPSILON {
            cubed
        } else {
            (116.0 * f - 16.0) / KAPPA
        }
    };
    let y = if lab[0] > KAPPA * EPSILON {
        fy * fy * fy
    } else {
        lab[0] / KAPPA
    };
    [cube(fx) * D50[0], y * D50[1], cube(fz) * D50[2]]
}

const XYZ_TO_LMS: Matrix = [
    [0.819_022_437_996_703, 0.361_906_260_052_890_4, -0.128_873_781_520_987_9],
    [
        0.032_983_653_932_388_5,
        0.929_286_861_586_343_4,
        0.036_144_666_350_642_4,
    ],
    [
        0.048_177_189_359_624_2,
        0.264_239_531_752_730_8,
        0.633_547_828_469_430_9,
    ],
];
const LMS_TO_OKLAB: Matrix = [
    [0.210_454_268_309_314, 0.793_617_774_702_305_4, -0.004_072_043_011_619_3],
    [1.977_998_532_431_168_4, -2.428_592_242_048_58, 0.450_593_709_617_411],
    [
        0.025_904_042_465_547_8,
        0.782_771_712_457_529_6,
        -0.808_675_754_923_077_4,
    ],
];
const LMS_TO_XYZ: Matrix = [
    [
        1.226_879_875_845_924_3,
        -0.557_814_994_460_217_1,
        0.281_391_045_665_964_7,
    ],
    [
        -0.040_575_745_214_800_8,
        1.112_286_803_280_317,
        -0.071_711_058_065_516_4,
    ],
    [
        -0.076_372_936_674_660_1,
        -0.421_493_332_402_243_2,
        1.586_924_019_836_781_6,
    ],
];
const OKLAB_TO_LMS: Matrix = [
    [1.0, 0.396_337_777_376_174_9, 0.215_803_757_309_913_6],
    [1.0, -0.105_561_345_815_658_6, -0.063_854_172_825_813_3],
    [1.0, -0.089_484_177_529_811_9, -1.291_485_548_019_409_2],
];

fn xyz_d65_to_oklab(xyz: [f64; 3]) -> [f64; 3] {
    let lms = multiply(&XYZ_TO_LMS, xyz);
    multiply(&LMS_TO_OKLAB, each(lms, f64::cbrt))
}

fn oklab_to_xyz_d65(oklab: [f64; 3]) -> [f64; 3] {
    let lms = multiply(&OKLAB_TO_LMS, oklab);
    multiply(&LMS_TO_XYZ, each(lms, |v| v * v * v))
}

/// Rectangular to polar. A chroma at or under `epsilon` has no hue, which comes back as NaN.
/// The thresholds are the sample code's, and absorb rounding error from a round trip.
fn to_polar(v: [f64; 3], epsilon: f64) -> [f64; 3] {
    let chroma = v[1].hypot(v[2]);
    let hue = if chroma <= epsilon {
        f64::NAN
    } else {
        normalize_hue(v[2].atan2(v[1]).to_degrees())
    };
    [v[0], chroma, hue]
}

fn from_polar(v: [f64; 3]) -> [f64; 3] {
    let hue = if v[2].is_nan() { 0.0 } else { v[2].to_radians() };
    [v[0], v[1] * hue.cos(), v[1] * hue.sin()]
}

/// A hue brought into `[0, 360)`.
#[must_use]
pub fn normalize_hue(hue: f64) -> f64 {
    let hue = hue.rem_euclid(360.0);
    // `rem_euclid` can return exactly 360 for a tiny negative input.
    if hue >= 360.0 {
        0.0
    } else {
        hue
    }
}

// --- HSL and HWB ------------------------------------------------------------------------------

/// HSL to sRGB (0-1 channels), unclamped. Saturation and lightness are 0-100.
#[must_use]
pub fn hsl_to_srgb(hsl: [f64; 3]) -> [f64; 3] {
    let hue = if hsl[0].is_nan() { 0.0 } else { hsl[0] };
    let hue = normalize_hue(hue);
    let (sat, light) = (hsl[1] / 100.0, hsl[2] / 100.0);
    let f = |n: f64| {
        let k = (n + hue / 30.0) % 12.0;
        let a = sat * light.min(1.0 - light);
        light - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}

/// sRGB (0-1 channels) to HSL, with NaN for the hue of an achromatic colour.
#[must_use]
pub fn srgb_to_hsl(rgb: [f64; 3]) -> [f64; 3] {
    let [red, green, blue] = rgb;
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let light = (min + max) / 2.0;
    let d = max - min;
    let mut hue = f64::NAN;
    let mut sat = 0.0;
    if d != 0.0 {
        sat = if light == 0.0 || light == 1.0 {
            0.0
        } else {
            (max - light) / light.min(1.0 - light)
        };
        hue = if max == red {
            (green - blue) / d + if green < blue { 6.0 } else { 0.0 }
        } else if max == green {
            (blue - red) / d + 2.0
        } else {
            (red - green) / d + 4.0
        } * 60.0;
    }
    // A colour far outside sRGB can come out with a negative saturation, which is the same
    // colour on the opposite side of the wheel.
    if sat < 0.0 {
        hue += 180.0;
        sat = sat.abs();
    }
    if !hue.is_nan() {
        hue = normalize_hue(hue);
    }
    [hue, sat * 100.0, light * 100.0]
}

/// HWB to sRGB (0-1 channels). Whiteness and blackness are 0-100, and are normalized when they
/// add up to more than 100, so `hwb(0 60% 60%)` is a grey.
#[must_use]
pub fn hwb_to_srgb(hwb: [f64; 3]) -> [f64; 3] {
    let (white, black) = (hwb[1] / 100.0, hwb[2] / 100.0);
    if white + black >= 1.0 {
        let grey = white / (white + black);
        return [grey, grey, grey];
    }
    let scale = 1.0 - white - black;
    hsl_to_srgb([hwb[0], 100.0, 50.0]).map(|channel| channel * scale + white)
}

/// sRGB (0-1 channels) to HWB, with NaN for the hue of an achromatic colour.
#[must_use]
pub fn srgb_to_hwb(rgb: [f64; 3]) -> [f64; 3] {
    // The sample code's tolerance for rounding error after several conversions.
    const ACHROMATIC: f64 = 1.0 / 100_000.0;
    let hsl = srgb_to_hsl(rgb);
    let white = rgb[0].min(rgb[1]).min(rgb[2]);
    let black = 1.0 - rgb[0].max(rgb[1]).max(rgb[2]);
    let hue = if white + black >= 1.0 - ACHROMATIC {
        f64::NAN
    } else {
        hsl[0]
    };
    [hue, white * 100.0, black * 100.0]
}

// --- the hub ----------------------------------------------------------------------------------

/// A triple in `space` converted to XYZ with a D65 white.
#[must_use]
pub fn to_xyz_d65(space: Space, v: [f64; 3]) -> [f64; 3] {
    match space {
        Space::Srgb => multiply(&SRGB_TO_XYZ, each(v, srgb_to_linear)),
        Space::SrgbLinear => multiply(&SRGB_TO_XYZ, v),
        Space::DisplayP3 => multiply(&P3_TO_XYZ, each(v, srgb_to_linear)),
        Space::DisplayP3Linear => multiply(&P3_TO_XYZ, v),
        Space::A98Rgb => multiply(&A98_TO_XYZ, each(v, a98_to_linear)),
        Space::ProphotoRgb => multiply(&D50_TO_D65, multiply(&PROPHOTO_TO_XYZ_D50, each(v, prophoto_to_linear))),
        Space::Rec2020 => multiply(&REC2020_TO_XYZ, each(v, rec2020_to_linear)),
        Space::XyzD50 => multiply(&D50_TO_D65, v),
        Space::XyzD65 => v,
        Space::Lab => multiply(&D50_TO_D65, lab_to_xyz_d50(v)),
        Space::Lch => multiply(&D50_TO_D65, lab_to_xyz_d50(from_polar(v))),
        Space::Oklab => oklab_to_xyz_d65(v),
        Space::Oklch => oklab_to_xyz_d65(from_polar(v)),
        Space::Hsl => to_xyz_d65(Space::Srgb, hsl_to_srgb(v)),
        Space::Hwb => to_xyz_d65(Space::Srgb, hwb_to_srgb(v)),
    }
}

/// A D65 XYZ triple converted to `space`. The hue of an achromatic colour comes back NaN.
#[must_use]
pub fn from_xyz_d65(space: Space, xyz: [f64; 3]) -> [f64; 3] {
    // The thresholds under which the sample code calls a chroma zero.
    const LCH_ACHROMATIC: f64 = 0.0015;
    const OKLCH_ACHROMATIC: f64 = 0.000_004;
    match space {
        Space::Srgb => each(multiply(&XYZ_TO_SRGB, xyz), srgb_from_linear),
        Space::SrgbLinear => multiply(&XYZ_TO_SRGB, xyz),
        Space::DisplayP3 => each(multiply(&XYZ_TO_P3, xyz), srgb_from_linear),
        Space::DisplayP3Linear => multiply(&XYZ_TO_P3, xyz),
        Space::A98Rgb => each(multiply(&XYZ_TO_A98, xyz), a98_from_linear),
        Space::ProphotoRgb => each(
            multiply(&XYZ_D50_TO_PROPHOTO, multiply(&D65_TO_D50, xyz)),
            prophoto_from_linear,
        ),
        Space::Rec2020 => each(multiply(&XYZ_TO_REC2020, xyz), rec2020_from_linear),
        Space::XyzD50 => multiply(&D65_TO_D50, xyz),
        Space::XyzD65 => xyz,
        Space::Lab => xyz_d50_to_lab(multiply(&D65_TO_D50, xyz)),
        Space::Lch => to_polar(xyz_d50_to_lab(multiply(&D65_TO_D50, xyz)), LCH_ACHROMATIC),
        Space::Oklab => xyz_d65_to_oklab(xyz),
        Space::Oklch => to_polar(xyz_d65_to_oklab(xyz), OKLCH_ACHROMATIC),
        Space::Hsl => srgb_to_hsl(from_xyz_d65(Space::Srgb, xyz)),
        Space::Hwb => srgb_to_hwb(from_xyz_d65(Space::Srgb, xyz)),
    }
}

/// A triple converted from one space to another. A missing input component must already have
/// been replaced by zero. The hue of an achromatic result comes back NaN.
#[must_use]
pub fn convert(from: Space, to: Space, v: [f64; 3]) -> [f64; 3] {
    if from == to {
        return v;
    }
    // sRGB, HSL and HWB convert among themselves directly, avoiding the rounding error of going
    // through XYZ.
    match (from, to) {
        (Space::Hsl, Space::Srgb) => return hsl_to_srgb(v),
        (Space::Hwb, Space::Srgb) => return hwb_to_srgb(v),
        (Space::Srgb, Space::Hsl) => return srgb_to_hsl(v),
        (Space::Srgb, Space::Hwb) => return srgb_to_hwb(v),
        (Space::Hsl | Space::Hwb, Space::Hsl | Space::Hwb) => {
            return convert(Space::Srgb, to, convert(from, Space::Srgb, v))
        }
        _ => {}
    }
    from_xyz_d65(to, to_xyz_d65(from, v))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Space; 15] = [
        Space::Srgb,
        Space::SrgbLinear,
        Space::DisplayP3,
        Space::DisplayP3Linear,
        Space::A98Rgb,
        Space::ProphotoRgb,
        Space::Rec2020,
        Space::XyzD50,
        Space::XyzD65,
        Space::Lab,
        Space::Lch,
        Space::Oklab,
        Space::Oklch,
        Space::Hsl,
        Space::Hwb,
    ];

    fn close(a: [f64; 3], b: [f64; 3], epsilon: f64) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() <= epsilon)
    }

    #[test]
    fn every_space_round_trips_through_every_other() {
        // A chromatic colour, so no hue goes missing on the way.
        let start = [0.4, 0.2, 0.6];
        for via in ALL {
            let there = convert(Space::Srgb, via, start);
            let back = convert(via, Space::Srgb, there);
            assert!(close(back, start, 1e-9), "{via:?}: {start:?} came back as {back:?}");
        }
    }

    #[test]
    fn white_is_white_everywhere() {
        let white = to_xyz_d65(Space::Srgb, [1.0, 1.0, 1.0]);
        assert!(close(from_xyz_d65(Space::Lab, white), [100.0, 0.0, 0.0], 1e-3));
        assert!(close(from_xyz_d65(Space::Oklab, white), [1.0, 0.0, 0.0], 1e-6));
        assert!(close(from_xyz_d65(Space::DisplayP3, white), [1.0, 1.0, 1.0], 1e-9));
        // Achromatic, so the polar spaces give no hue.
        assert!(from_xyz_d65(Space::Lch, white)[2].is_nan());
        assert!(from_xyz_d65(Space::Oklch, white)[2].is_nan());
        assert!(srgb_to_hsl([1.0, 1.0, 1.0])[0].is_nan());
        assert!(srgb_to_hwb([1.0, 1.0, 1.0])[0].is_nan());
    }

    #[test]
    fn rebeccapurple_matches_the_spec_values() {
        let rgb = [0.4, 0.2, 0.6];
        assert!(close(convert(Space::Srgb, Space::Hsl, rgb), [270.0, 50.0, 40.0], 1e-9));
        assert!(close(convert(Space::Srgb, Space::Hwb, rgb), [270.0, 20.0, 40.0], 1e-9));
        // css-color-4 gives rebeccapurple as lab(32.4 38.4 -47.7) to one decimal.
        assert!(close(
            convert(Space::Srgb, Space::Lab, rgb),
            [32.39, 38.43, -47.69],
            0.01
        ));
    }

    #[test]
    fn out_of_gamut_values_are_not_clamped() {
        // display-p3's pure green is outside sRGB; the WPT value is -0.5116 1.01827 -0.31067.
        let srgb = convert(Space::DisplayP3, Space::Srgb, [0.0, 1.0, 0.0]);
        assert!(close(srgb, [-0.5116, 1.01827, -0.31067], 1e-4), "{srgb:?}");
    }
}
