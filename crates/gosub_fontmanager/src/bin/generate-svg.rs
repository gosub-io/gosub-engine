use freetype::Library;
use gosub_fontmanager::FontManager;
use gosub_interface::font::FontStyle;

const TEST_STRING: &str = r"A B C D E F G H I J K L M N O P Q R S T U V W X Y Z
a b c d e f g h i j k l m n o p q r s t u v w x y z
0 1 2 3 4 5 6 7 8 9 ( ) $ % @ & ¢ € [ \ ] ^ _ ` { | } ~ < > # = + - * / : ; , . ! ?
¡ ¿ ˆ ˜ ¨ ´ ` ˘ ˙ ˚ ˝ ˛ ˇ ˆ ˇ ˘ ˙ ˚ ˛ ˜ ˝ ˇ ˘ ˙ ˚ ˛ ˜ ˝ ˇ ˘ ˙ ˚ ˛ ˜ ˝
À Á Â Ã Ä Å Æ Ç È É Ê Ë Ì Í Î Ï Ð Ñ Ò Ó Ô Õ Ö Ø Ù Ú Û Ü Ý Þ ß
à á â ã ä å æ ç è é ê ë ì í î ï ð ñ ò ó ô õ ö ø ù ú û ü ý þ ÿ
Ā ā Ă ă Ą ą Ć ć Ĉ ĉ Ċ ċ Č č Ď ď Đ đ Ē ē Ĕ ĕ Ė ė Ę ę Ě ě Ĝ ĝ Ğ ğ
Ġ ġ Ģ ģ Ĥ ĥ Ħ ħ Ĩ ĩ Ī ī Ĭ ĭ Į į İ ı Ĳ ĳ Ĵ ĵ Ķ ķ ĸ Ĺ ĺ Ļ ļ Ľ ľ
Ŀ ŀ Ł ł Ń ń Ņ ņ Ň ň ŉ Ŋ ŋ Ō ō Ŏ ŏ Ő ő Œ œ Ŕ ŕ Ŗ ŗ Ř ř Ś ś Ŝ ŝ
Ş ş Š š Ţ ţ Ť ť Ŧ ŧ Ũ ũ Ū ū Ŭ ŭ Ů ů Ű ű Ų ų Ŵ ŵ Ŷ ŷ Ÿ Ź ź Ż ż Ž ž
ſ ƀ Ɓ Ƃ ƃ Ƅ ƅ Ɔ Ƈ ƈ Ɖ Ɗ Ƌ ƌ ƍ Ǝ Ə Ɛ Ƒ ƒ Ɠ Ɣ ƕ Ɩ Ɨ Ƙ ƙ ƚ ƛ Ɯ Ɲ ƞ Ɵ
Ơ ơ Ƣ ƣ Ƥ ƥ Ʀ Ƨ ƨ Ʃ ƪ ƫ Ƭ ƭ Ʈ Ư ư Ʊ Ʋ Ƴ ƴ Ƶ ƶ Ʒ Ƹ ƹ ƺ ƻ Ƽ ƽ ƾ ƿ
ǀ ǁ ǂ ǃ Ǆ ǅ ǆ Ǉ ǈ ǉ Ǌ ǋ ǌ Ǎ ǎ Ǐ ǐ Ǒ ǒ Ǔ ǔ Ǖ ǖ Ǘ ǘ Ǚ ǚ Ǜ ǜ ǝ
Ǟ ǟ Ǡ ǡ Ǣ ǣ Ǥ ǥ Ǧ ǧ Ǩ ǩ Ǫ ǫ Ǭ ǭ Ǯ ǯ ǰ Ǳ ǲ ǳ Ǵ ǵ Ƕ Ƿ Ǹ ǹ Ǻ ǻ
Ǽ ǽ Ǿ ǿ Ȁ ȁ Ȃ ȃ Ȅ ȅ Ȇ ȇ Ȉ ȉ Ȋ ȋ Ȍ ȍ Ȏ ȏ Ȑ ȑ Ȓ ȓ Ȕ ȕ Ȗ ȗ Ș ș Ț ț
Ȝ ȝ Ȟ ȟ Ƞ ȡ Ȣ ȣ Ȥ ȥ Ȧ ȧ Ȩ ȩ Ȫ ȫ Ȭ ȭ Ȯ ȯ Ȱ ȱ Ȳ ȳ ȴ ȵ ȶ ȷ ȸ ȹ Ⱥ
Ȼ ȼ Ƚ Ⱦ ȿ ɀ Ɂ ɂ Ƀ Ʉ Ʌ Ɇ ɇ Ɉ ɉ Ɋ ɋ Ɍ ɍ Ɏ ɏ ɐ ɑ ɒ ɓ ɔ ɕ ɖ ɗ ɘ ə ɚ
ɜ ɝ ɞ ɟ ɠ ɡ ɢ ɣ ɤ ɥ ɦ ɧ ɨ ɩ ɪ ɫ ɬ ɭ ɮ ɯ ɰ ɱ ɲ ɳ ɴ ɵ ɶ ɷ ɸ ɹ
\u{EA84} \u{EA84} \u{EA84} \u{EA84} \u{EA84}
Hello world from the Gosub FontManager system!
";

fn main() {
    colog::init();

    let manager = FontManager::new();

    let arg = std::env::args().nth(1);
    let binding = arg.unwrap_or("arial".into());
    let font = binding.as_str();

    let Some(font_info) = manager.find(&[font], FontStyle::Normal) else {
        eprintln!("Font not found: {font}");
        return;
    };

    let library = Library::init().unwrap();
    let path = font_info.path.unwrap();
    let face = library.new_face(path, font_info.index.unwrap_or(0) as isize).unwrap();

    char_to_svg(&face, TEST_STRING);
}

fn char_to_svg(face: &freetype::Face, content: &str) {
    face.set_char_size(10 * 64, 0, 10, 0).unwrap();

    println!("<?xml version=\"1.0\" standalone=\"no\"?>");
    println!("<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\"");
    println!("\"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">");
    println!("<svg viewBox=\"0 0 100 200\" xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\">");

    let mut x_pos = -155.0;
    let mut y_pos = 10.0;

    for c in content.chars() {
        if c == '\n' {
            x_pos = -155.0;
            y_pos += 10.0;
            continue;
        }

        if c == ' ' {
            x_pos += 2.5;
            continue;
        }

        let result = face.load_char(c as usize, freetype::face::LoadFlag::NO_SCALE);
        if result.is_err() {
            continue;
        }

        let glyph = face.glyph();
        let metrics = glyph.metrics();
        // let xmin = metrics.horiBearingX - 5;
        let width = metrics.width + 10;
        // let ymin = -metrics.horiBearingY - 5;
        // let height = metrics.height + 10;
        // let scale_factor = 10.0 / width as f32;
        let scale_factor = 0.0056;

        let outline = glyph.outline().unwrap();

        for contour in outline.contours_iter() {
            let start = contour.start();

            // dbg!(x_pos, y_pos);
            println!(
                "<g transform=\"translate({}, {}) scale({})\">",
                x_pos - 1.0,
                y_pos - 1.0,
                scale_factor
            );

            println!(
                "<path fill=\"none\" stroke=\"black\" stroke-width=\"16\" d=\"M {} {}",
                start.x, -start.y
            );
            for curve in contour {
                draw_curve(curve);
            }
            println!("Z \" />");
            println!("</g>");
        }

        x_pos += width as f32 * scale_factor;
        x_pos += 1.0;
    }
    println!("</svg>");
}

fn draw_curve(curve: freetype::outline::Curve) {
    match curve {
        freetype::outline::Curve::Line(pt) => println!("L {} {}", pt.x, -pt.y),
        freetype::outline::Curve::Bezier2(pt1, pt2) => {
            println!("Q {} {} {} {}", pt1.x, -pt1.y, pt2.x, -pt2.y);
        }
        freetype::outline::Curve::Bezier3(pt1, pt2, pt3) => {
            println!("C {} {} {} {} {} {}", pt1.x, -pt1.y, pt2.x, -pt2.y, pt3.x, -pt3.y);
        }
    }
}
