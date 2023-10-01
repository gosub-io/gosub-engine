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
    fn print(&mut self, log_level: String, args: &[&dyn fmt::Display], _options: &[&str])
    {
        if args.len() == 0 {
            return;
        }

        match log_level.as_str() {
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
            data.push_str(format!("{}", arg).as_str());
        }

        match log_level.as_str() {
            "info" => println!("{}[{}] {}", group_prefix, log_level, data.on_bright_blue().color("white")),
            "warn" => println!("{}[{}] {}", group_prefix, log_level, data.on_bright_yellow().color("black")),
            "error" => println!("{}[{}] {}", group_prefix, log_level, data.on_bright_red().color("white")),
            "log" => println!("{}[{}] {}", group_prefix, log_level, data),
            "assert" => println!("{}[{}] {}", group_prefix, log_level, data),
            "group" => println!("{}Expanded group: {}", group_prefix, data),
            "groupCollapsed" => println!("{}Collapsed group: {}", group_prefix, data),
            "timeEnd" => println!("{}{} - timer ended", group_prefix, data),
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
    fn test_buffer_printer() {
        let mut printer = BufferPrinter::new();
        printer.print("log".into(), &[&"Hello".to_string(), &"World".to_string()], &vec![]);
        printer.print("info".into(), &[&"Something when wrong".to_string(), &"This too!".to_string()], &vec![]);
        printer.print("warn".into(), &[&"Something when wrong".to_string(), &"This too!".to_string()], &vec![]);
        printer.print("error".into(), &[&"Something when wrong".to_string(), &"This too!".to_string()], &vec![]);

        assert_eq!(printer.buffer, 0);
    }
}