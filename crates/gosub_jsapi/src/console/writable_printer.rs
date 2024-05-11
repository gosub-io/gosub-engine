use crate::console::{LogLevel, Printer};
use std::cell::RefCell;
use std::fmt;
use std::io::Write;
use std::rc::Rc;

#[allow(dead_code)]
pub struct Group {
    collapsed: bool,
}

type Writer<W> = Rc<RefCell<W>>;

/// A writable printer that can be used to write to a buffer
pub struct WritablePrinter<W: Write> {
    writer: Writer<W>,
    groups: Vec<Group>,
}

impl<W: Write> WritablePrinter<W> {
    /// Creates a new writable printer
    #[allow(dead_code)]
    pub fn new(writer: Rc<RefCell<W>>) -> Self {
        Self {
            writer,
            groups: vec![],
        }
    }

    /// Returns a reference to the writer
    #[allow(dead_code)]
    pub fn get_writer(&self) -> &Writer<W> {
        &self.writer
    }

    /// Returns the writer
    #[allow(dead_code)]
    pub fn into_writer(self) -> Writer<W> {
        self.writer
    }
}

impl<W: Write> Printer for WritablePrinter<W> {
    /// Print the given string (as defined in args) to the console at the given loglevel. Additional options can be provided
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

        let mut data = String::new();
        for arg in args {
            data.push_str(format!("{arg} ").as_str());
        }
        data = data.trim_end().to_string();
        let mut writer = self.writer.borrow_mut();

        let _ = match log_level {
            LogLevel::Info
            | LogLevel::Warn
            | LogLevel::Error
            | LogLevel::Log
            | LogLevel::Assert => {
                writeln!(writer, "{group_prefix}[{log_level}] {data}")
            }
            LogLevel::Group => writeln!(writer, "{group_prefix}Expanded group: {data}"),
            LogLevel::GroupCollapsed => {
                writeln!(writer, "{group_prefix}Collapsed group: {data}")
            }
            LogLevel::TimeEnd => writeln!(writer, "{group_prefix}{data} - timer ended"),
            _ => Ok(()),
        };
    }

    /// Clears the current group
    fn clear(&mut self) {
        // nothing to clear
        _ = writeln!(self.writer.borrow_mut(), "--- Clear ---");
    }

    /// Ends the current group
    fn end_group(&mut self) {
        self.groups.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::buffer::Buffer;

    #[test]
    fn printer() {
        let buffer = Rc::new(RefCell::new(Buffer::new()));
        let mut printer = WritablePrinter::new(Rc::clone(&buffer));

        printer.print(LogLevel::Log, &[&"Hello", &"World"], &[]);
        assert_eq!(
            buffer
                .borrow_mut()
                .try_to_string()
                .expect("failed to convert"),
            "[log] Hello World\n"
        );

        let buffer = Rc::new(RefCell::new(Buffer::new()));
        let mut printer = WritablePrinter::new(Rc::clone(&buffer));
        printer.print(LogLevel::Info, &[&"Foo", &2i32, &false], &[]);
        printer.print(LogLevel::Warn, &[&"a", &"b"], &[]);
        printer.print(LogLevel::Error, &[], &[]);
        assert_eq!(
            buffer
                .borrow_mut()
                .try_to_string()
                .expect("failed to convert"),
            "[info] Foo 2 false\n[warn] a b\n"
        );
    }
}
