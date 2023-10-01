use std::fmt;
use std::io::Write;
use crate::console::Printer;
use colored::*;

pub struct Group {
    collapsed: bool,
}

pub(crate) struct TextPrinter<W: Write> {
    writer: W,
    groups: Vec<Group>,
}

impl<W: Write> TextPrinter<W> {
    pub fn new(writer: W) -> TextPrinter<W> {
        TextPrinter {
            writer,
            groups: vec![],
        }
    }
}

impl<W: Write> Printer for TextPrinter<W> {
    fn print(&mut self, log_level: &str, args: &[&dyn fmt::Display], _options: &[&str])
    {
        if args.len() == 0 {
            return;
        }

        match log_level {
            "group" => {
                self.groups.push(Group { collapsed: false });
            },
            "groupCollapse" => {
                self.groups.push(Group { collapsed: true });
            }
            "groupEnd" => {
                self.groups.pop();
            },
            _ => {},
        }

        let group_prefix = " > ".repeat(self.groups.len());

        let mut data = String::from("");
        for arg in args {
            data.push_str(format!("{} ", arg).as_str());
        }
        data = data.trim_end().to_string();

        let _ = match log_level {
            "info" => writeln!(self.writer, "{}[{}] {}", group_prefix, log_level, data.on_bright_blue().color("white")),
            "warn" => writeln!(self.writer, "{}[{}] {}", group_prefix, log_level, data.on_bright_yellow().color("black")),
            "error" => writeln!(self.writer, "{}[{}] {}", group_prefix, log_level, data.on_bright_red().color("white")),
            "log" => writeln!(self.writer, "{}[{}] {}", group_prefix, log_level, data),
            "assert" => writeln!(self.writer, "{}[{}] {}", group_prefix, log_level, data),
            "group" => writeln!(self.writer, "{}Expanded group: {}", group_prefix, data),
            "groupCollapsed" => writeln!(self.writer, "{}Collapsed group: {}", group_prefix, data),
            "timeEnd" => writeln!(self.writer, "{}{} - timer ended", group_prefix, data),
            _ => Ok(()),
        };
    }

    fn clear(&mut self) {
        // nothing to clear
        _ = writeln!(self.writer, "--- Clear ---");
    }

    fn end_group(&mut self) {
        self.groups.pop();
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use super::*;

    #[test]
    fn test_text_printer() {
        let mut buffer = Vec::new();
        let mut printer = TextPrinter::new(Cursor::new(&mut buffer));

        printer.print("log", &[&"Hello", &"World"], &vec![]);
        assert_eq!(String::from_utf8(buffer).expect("failed to convert"), "[log] Hello World");

        let mut buffer = Vec::new();
        let mut printer = TextPrinter::new(Cursor::new(&mut buffer));
        printer.print("info", &[&"Foo", &2, &false], &vec![]);
        printer.print("warn", &[&"a", &"b"], &vec![]);
        printer.print("error", &[], &vec![]);
        assert_eq!(String::from_utf8(buffer).expect("failed to convert"), "[info] Foo 2 false\n[warn] a b");
    }
}