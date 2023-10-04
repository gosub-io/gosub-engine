use std::fmt;
use crate::console::Printer;
use colored::*;

pub struct Group {
    collapsed: bool,
}

pub(crate) struct BufferPrinter {
    groups: Vec<Group>,
    buffer: String,
}

impl BufferPrinter {
    pub fn new() -> BufferPrinter {
        BufferPrinter {
            groups: vec![],
            buffer: "".to_string(),
        }
    }
}

impl Printer for BufferPrinter {
    fn print(&mut self, log_level: LogLevel, args: &[&dyn fmt::Display], _options: &[&str])
    {
        if args.is_empty() {
            return;
        }

        match log_level {
            LogLevel::Group => {
                self.groups.push(Group { collapsed: false });
            },
            LogLevel::GroupCollapsed => {
                self.groups.push(Group { collapsed: true });
            }
            Loglevel::GroupEnd => {
                self.groups.pop();
            },
            _ => {},
        }

        let group_prefix = " > ".repeat(self.groups.len());

        let mut data = String::from("");
        for arg in args {
            data.push_str(format!("{}", arg).as_str());
        }

        match log_level {
            LogLevel::Info => println!("{}[{}] {}", group_prefix, log_level, data.on_bright_blue().color("white")),
            LogLevel::Warn => println!("{}[{}] {}", group_prefix, log_level, data.on_bright_yellow().color("black")),
            LogLevel::Error => println!("{}[{}] {}", group_prefix, log_level, data.on_bright_red().color("white")),
            LogLevel::Log => println!("{}[{}] {}", group_prefix, log_level, data),
            LogLevel::Assert => println!("{}[{}] {}", group_prefix, log_level, data),
            LogLevel::Group => println!("{}Expanded group: {}", group_prefix, data),
            LogLevel::GroupCollapsed => println!("{}Collapsed group: {}", group_prefix, data),
            LogLevel::TimeEnd => println!("{}{} - timer ended", group_prefix, data),
            _ => {},
        }
    }

    fn clear(&mut self) {
        // nothing to clear
        println!("--- Clear ---");
    }

    fn end_group(&mut self) {
        self.groups.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_printer() {
        let mut printer = BufferPrinter::new();
        printer.print("log".into(), &[&"Hello".to_string(), &"World".to_string()], &vec![]);
        printer.print("info".into(), &[&"Something when wrong".to_string(), &"This too!".to_string()], &vec![]);
        printer.print("warn".into(), &[&"Something when wrong".to_string(), &"This too!".to_string()], &vec![]);
        printer.print("error".into(), &[&"Something when wrong".to_string(), &"This too!".to_string()], &vec![]);

        assert_eq!(printer.buffer, 0);
    }
}