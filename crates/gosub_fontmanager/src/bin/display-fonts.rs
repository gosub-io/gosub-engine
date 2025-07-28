use cow_utils::CowUtils;
use gosub_fontmanager::FontManager;
use prettytable::{Attr, Cell, Row, Table};

fn main() {
    colog::init();

    let arg = std::env::args().nth(1);
    let binding = arg.unwrap_or_default();
    let pattern = binding.as_str();

    let manager = FontManager::new();
    render_table(&manager, pattern);
}

fn render_table(manager: &FontManager, family: &str) {
    let mut table = Table::new();
    table.set_format(*prettytable::format::consts::FORMAT_NO_BORDER_LINE_SEPARATOR);
    table.set_titles(Row::new(vec![
        Cell::new("Family").with_style(Attr::Bold),
        Cell::new("Style").with_style(Attr::Bold),
        Cell::new("Weight").with_style(Attr::Bold),
        Cell::new("Stretch").with_style(Attr::Bold),
        Cell::new("Monospaced").with_style(Attr::Bold),
        Cell::new("Path").with_style(Attr::Bold),
        Cell::new("Index").with_style(Attr::Bold),
    ]));

    for info in manager.available_fonts() {
        if !family.is_empty() {
            let fam = info.family.cow_to_ascii_lowercase();
            let family_lower = family.cow_to_ascii_lowercase();
            if !fam.contains(&*family_lower) {
                continue;
            }
        }

        table.add_row(Row::new(vec![
            Cell::new(&info.family),
            Cell::new(&format!("{}", &info.style)),
            Cell::new(&info.weight.to_string()),
            Cell::new(&info.stretch.to_string()),
            Cell::new(&info.monospaced.to_string()),
            if info.path.is_some() {
                Cell::new(info.path.clone().unwrap().to_str().unwrap())
            } else {
                Cell::new("N/A")
            },
            Cell::new(&info.index.unwrap_or(0).to_string()),
        ]));
    }

    table.printstd();
    println!("\n\n\n");
}
