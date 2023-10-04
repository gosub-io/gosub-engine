use crate::console::{LogLevel, Printer};
use std::fmt;
use std::io::Write;

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
    fn print(&mut self, log_level: LogLevel, args: &[&dyn fmt::Display], _options: &[&str]) {
        if args.is_empty() {
            return;
        }

        match log_level {
            LogLevel::Group => {
                self.groups.push(Group { collapsed: false });
            }
            LogLevel::GroupCollapsed => {
                self.groups.push(Group { collapsed: true });
            }
            LogLevel::GroupEnd => {
                self.groups.pop();
            }
            _ => {}
        }

        let group_prefix = " > ".repeat(self.groups.len());

        let mut data = String::from("");
        for arg in args {
            data.push_str(format!("{} ", arg).as_str());
        }
        data = data.trim_end().to_string();

        let _ = match log_level {
            LogLevel::Info
            | LogLevel::Warn
            | LogLevel::Error
            | LogLevel::Log
            | LogLevel::Assert => {
                writeln!(self.writer, "{}[{}] {}", group_prefix, log_level, data)
            }
            LogLevel::Group => writeln!(self.writer, "{}Expanded group: {}", group_prefix, data),
            LogLevel::GroupCollapsed => {
                writeln!(self.writer, "{}Collapsed group: {}", group_prefix, data)
            }
            LogLevel::TimeEnd => writeln!(self.writer, "{}{} - timer ended", group_prefix, data),
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
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_text_printer() {
        let mut buffer = Vec::new();
        let mut printer = TextPrinter::new(Cursor::new(&mut buffer));

        printer.print(LogLevel::Log, &[&"Hello", &"World"], &vec![]);
        assert_eq!(
            String::from_utf8(buffer).expect("failed to convert"),
            "[log] Hello World\n"
        );

        let mut buffer = Vec::new();
        let mut printer = TextPrinter::new(Cursor::new(&mut buffer));
        printer.print(LogLevel::Info, &[&"Foo", &2, &false], &vec![]);
        printer.print(LogLevel::Warn, &[&"a", &"b"], &vec![]);
        printer.print(LogLevel::Error, &[], &vec![]);
        assert_eq!(
            String::from_utf8(buffer).expect("failed to convert"),
            "[info] Foo 2 false\n[warn] a b\n"
        );
    }
}
