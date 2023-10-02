mod formatter;
mod text_printer;

use crate::console::formatter::Formatter;
use std::collections::HashMap;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Timer holds the start and end time of a timer.
struct Timer {
    label: String,
    start: u128,
    end: Option<u128>,
}

struct Group {
    label: String,
}

/// Console is the main struct that holds all the console methods.
pub struct Console {
    timers: HashMap<String, Timer>,
    counts: HashMap<String, usize>,
    group_stacks: Vec<Group>,
    printer: Box<dyn Printer>,
    formatter: Formatter,
    current_group_stack: Option<usize>,
}

impl Console {
    /// Creates a new Console struct.
    fn new(printer: Box<dyn Printer>) -> Console {
        Console {
            timers: HashMap::new(),
            counts: HashMap::new(),
            group_stacks: vec![],
            printer,
            formatter: Formatter::new(),
            current_group_stack: None,
        }
    }

    fn assert(&mut self, condition: bool, data: &[&dyn fmt::Display]) {
        if condition {
            return;
        }

        let data = data.clone();

        let message = "Assertion failed";

        let concat = if data.is_empty() {
            message.to_string()
        } else {
            format!("{}: {}", message, data[0])
        };

        self.logger("assert", &[&concat]);
    }

    fn clear(&mut self) {
        if !self.group_stacks.is_empty() {
            // Clear group stack (what is this?)
        }

        self.printer.clear();
    }

    fn debug(&mut self, data: &[&dyn fmt::Display]) {
        self.logger("debug", data);
    }

    fn error(&mut self, data: &[&dyn fmt::Display]) {
        self.logger("error", data);
    }

    fn info(&mut self, data: &[&dyn fmt::Display]) {
        self.logger("info", data);
    }

    fn log(&mut self, data: &[&dyn fmt::Display]) {
        self.logger("log", data);
    }

    fn table(&mut self, _tabular_data: String, _properties: &[&str]) {
        todo!()
    }

    fn trace(&mut self, _item: &dyn fmt::Display, _options: &[&str]) {
        todo!()
    }

    fn warn(&mut self, data: &[&dyn fmt::Display]) {
        self.logger("warn", data);
    }

    fn dir(&mut self, item: &dyn fmt::Display, options: &[&str]) {
        self.printer.print("dir", &[&item], options);
    }

    fn dirxml(&self, _data: &[&dyn fmt::Display]) {
        todo!()
    }

    fn count(&mut self, label: String) {
        let mut cnt = 1;
        if self.counts.contains_key(&label) {
            cnt = self.counts.get(&label).unwrap() + 1;
        }

        self.counts.insert(label.clone(), cnt + 1);

        let concat = format!("{}: {}", label, cnt);
        self.logger("count", &[&concat]);
    }

    fn count_reset(&mut self, label: String) {
        if !self.counts.contains_key(&label) {
            self.logger("countReset", &[&"label does not exist"]);
            return;
        }

        self.counts.insert(label.clone(), 1);
    }

    /// Create an expanded group
    fn group(&mut self, data: &[&dyn fmt::Display]) {
        let group_label = if data.is_empty() {
            format!("console.group.{}", Uuid::new_v4())
        } else {
            self.formatter.format(data).to_string()
        };

        let group = Group {
            label: group_label.clone(),
        };

        // Group should be expanded
        self.printer.print("group", &[&group.label], &[]);

        self.group_stacks.push(group);
    }

    /// Create a collapsed group
    fn group_collapsed(&mut self, data: &[&dyn fmt::Display]) {
        let group_label = if data.is_empty() {
            format!("console.group.{}", Uuid::new_v4())
        } else {
            self.formatter.format(data).to_string()
        };

        let group = Group {
            label: group_label.clone(),
        };

        self.printer.print("groupCollapsed", &[&group.label], &[]);

        self.group_stacks.push(group);
    }

    /// End the last group
    fn group_end(&mut self) -> Option<Group> {
        self.printer.end_group();
        self.group_stacks.pop()
    }

    /// Create a timer
    fn time(&mut self, label: String) {
        if self.timers.contains_key(&label) {
            let warning = format!("Timer '{}' already started", label);
            self.logger("warning", &[&warning]);
            return;
        }

        let start = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(n) => n.as_millis(),
            Err(_) => 0,
        };

        self.timers.insert(
            label.clone(),
            Timer {
                label: label.clone(),
                start,
                end: None,
            },
        );
    }

    /// Log time
    fn time_log(&self, _label: String, _data: &[&dyn fmt::Display]) {
        todo!()
    }

    /// End the given timer
    fn time_end(&mut self, label: String) {
        let end = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(n) => n.as_millis(),
            Err(_) => 0,
        };

        let concat = format!(
            "{}: {}ms",
            label,
            end - self.timers.get(&label).unwrap().start
        );
        self.printer.print("timeEnd", &[&concat], &[]);
    }

    fn logger(&mut self, log_level: &str, args: &[&dyn fmt::Display]) {
        if args.is_empty() {
            return;
        }

        let first = args[0];
        let rest = &args[1..];
        if rest.is_empty() {
            self.printer.print(log_level, &[&first], &[]);
            return;
        }

        self.printer
            .print(log_level, &[&self.formatter.format(args)], &[]);
    }
}

trait Printer {
    fn print(&mut self, log_level: &str, args: &[&dyn fmt::Display], _options: &[&str]);
    fn clear(&mut self);
    fn end_group(&mut self);
}

#[cfg(test)]

mod tests {
    use super::*;
    use crate::console::text_printer::TextPrinter;
    use std::thread::sleep;

    #[test]
    fn test_console() {
        let stdout = std::io::stdout();
        let handle = stdout.lock();

        let mut c = Console::new(Box::new(TextPrinter::new(Box::new(handle))));
        c.log(&[&"Log", &12]);
        c.warn(&[&"Hello", &"World"]);
        c.time("foo".into());
        c.group(&[&"foo"]);
        c.warn(&[&"Hello", &"World"]);
        c.group(&[&"bar"]);
        c.warn(&[&"Hello", &"World"]);
        c.group_end();
        c.warn(&[&"Hello", &"World"]);
        c.clear();
        c.group_end();
        sleep(std::time::Duration::from_millis(123));
        c.time_end("foo".into());
        c.group_end();
        c.group_end();
        c.warn(&[&"Back", &"To root"]);

        c.assert(true, &[&"This assertion asserts"]);
        c.assert(false, &[&"This assertion does not assert"]);
        c.assert(true, &[]);
        c.assert(false, &[]);
    }
}
