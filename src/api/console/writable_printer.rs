use crate::api::console::{LogLevel, Printer};
use std::cell::RefCell;
use std::fmt;
use std::io::Write;
use std::rc::Rc;

pub struct Group {
    collapsed: bool,
}

type Writer<W> = Rc<RefCell<W>>;

pub(crate) struct WritablePrinter<W: Write> {
    writer: Writer<W>,
    groups: Vec<Group>,
}

impl<W: Write> WritablePrinter<W> {
    pub fn new(writer: Rc<RefCell<W>>) -> WritablePrinter<W> {
        WritablePrinter {
            writer,
            groups: vec![],
        }
    }

    pub fn get_writer(&self) -> &Writer<W> {
        &self.writer
    }

    pub fn into_writer(self) -> Writer<W> {
        self.writer
    }
}

impl<W: Write> Printer for WritablePrinter<W> {
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
        let mut writer = self.writer.borrow_mut();

        let _ = match log_level {
            LogLevel::Info
            | LogLevel::Warn
            | LogLevel::Error
            | LogLevel::Log
            | LogLevel::Assert => {
                writeln!(writer, "{}[{}] {}", group_prefix, log_level, data)
            }
            LogLevel::Group => writeln!(writer, "{}Expanded group: {}", group_prefix, data),
            LogLevel::GroupCollapsed => {
                writeln!(writer, "{}Collapsed group: {}", group_prefix, data)
            }
            LogLevel::TimeEnd => writeln!(writer, "{}{} - timer ended", group_prefix, data),
            _ => Ok(()),
        };
    }

    fn clear(&mut self) {
        // nothing to clear
        _ = writeln!(self.writer.borrow_mut(), "--- Clear ---");
    }

    fn end_group(&mut self) {
        self.groups.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::console::buffer::Buffer;

    #[test]
    fn printer() {
        let buffer = Rc::new(RefCell::new(Buffer::new()));
        let mut printer = WritablePrinter::new(Rc::clone(&buffer));

        printer.print(LogLevel::Log, &[&"Hello", &"World"], &vec![]);
        assert_eq!(
            buffer.borrow_mut().to_string().expect("failed to convert"),
            "[log] Hello World\n"
        );

        let buffer = Rc::new(RefCell::new(Buffer::new()));
        let mut printer = WritablePrinter::new(Rc::clone(&buffer));
        printer.print(LogLevel::Info, &[&"Foo", &2i32, &false], &vec![]);
        printer.print(LogLevel::Warn, &[&"a", &"b"], &vec![]);
        printer.print(LogLevel::Error, &[], &vec![]);
        assert_eq!(
            buffer.borrow_mut().to_string().expect("failed to convert"),
            "[info] Foo 2 false\n[warn] a b\n"
        );
    }
}
