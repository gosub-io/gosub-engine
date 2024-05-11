//! Console api as described by <https://console.spec.whatwg.org/>
mod buffer;
mod formatter;
mod writable_printer;

use crate::console::formatter::Formatter;
use std::collections::HashMap;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// `LogLevel` is the type of log level.
#[derive(Debug)]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Log,
    Debug,
    Assert,
    Group,
    GroupCollapsed,
    GroupEnd,
    TimeLog,
    TimeEnd,
    Count,
    CountReset,
    Dir,
    Dirxml,
    Trace,
}

impl fmt::Display for LogLevel {
    // When displaying the enum, make sure it is in lowercase
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", format!("{self:?}").to_lowercase())
    }
}

/// Timer holds the start and end time of a timer.
#[allow(dead_code)]
struct Timer {
    label: String,
    start: u128,
    end: Option<u128>,
}

pub struct Group {
    label: String,
}

/// Console is the main struct that holds all the console methods.
pub struct Console {
    /// Timers that are currently running or have run
    timer_map: HashMap<String, Timer>,
    /// Counts that are currently running or have run
    count_map: HashMap<String, usize>,
    /// Stack of groups. The last group is the current group
    group_stacks: Vec<Group>,
    /// Printer that will output any data from the console
    printer: Box<dyn Printer>,
    /// Formatter that will format any data that is passed to the console
    formatter: Formatter,
}

impl Console {
    /// Creates a new Console struct.
    ///
    /// # Arguments
    ///
    /// * `printer`   the printer that will be used to print any data that is passed to the console.
    ///
    /// # Returns
    ///
    /// A new Console struct
    #[must_use]
    pub fn new(printer: Box<dyn Printer>) -> Self {
        Self {
            timer_map: HashMap::new(),
            count_map: HashMap::new(),
            group_stacks: vec![],
            printer,
            formatter: Formatter::new(),
        }
    }

    /// Returns the printer that is used by the console
    #[must_use]
    pub fn get_printer(self) -> Box<dyn Printer> {
        self.printer
    }

    /// Emit an assert message if the condition is true
    pub fn assert(&mut self, condition: bool, data: &[&dyn fmt::Display]) {
        if condition {
            return;
        }

        let message = "Assertion failed";

        let concat = if data.is_empty() {
            message.to_string()
        } else {
            format!("{}: {}", message, data[0])
        };

        self.logger(LogLevel::Assert, &[&concat]);
    }

    /// Clears the console output (if possible)
    pub fn clear(&mut self) {
        self.printer.clear();
    }

    /// Emit debug message
    pub fn debug(&mut self, data: &[&dyn fmt::Display]) {
        self.logger(LogLevel::Debug, data);
    }

    /// Emit error message
    pub fn error(&mut self, data: &[&dyn fmt::Display]) {
        self.logger(LogLevel::Error, data);
    }

    /// Emit info message
    pub fn info(&mut self, data: &[&dyn fmt::Display]) {
        self.logger(LogLevel::Info, data);
    }

    /// Emit log message
    pub fn log(&mut self, data: &[&dyn fmt::Display]) {
        self.logger(LogLevel::Log, data);
    }

    /// Emit table if tabular data is supported
    pub fn table(&mut self, tabular_data: String, _properties: &[&str]) {
        // TODO: needs implementation
        self.printer.print(LogLevel::Log, &[&tabular_data], &[]);
    }

    /// Emit a trace message
    pub fn trace(&mut self, data: &[&dyn fmt::Display]) {
        let formatted_data = self.formatter.format(data);

        self.printer.print(LogLevel::Trace, &[&formatted_data], &[]);
    }

    /// Emit a warning
    pub fn warn(&mut self, data: &[&dyn fmt::Display]) {
        self.logger(LogLevel::Warn, data);
    }

    /// Emit a list of properties of the given item
    pub fn dir(&mut self, item: &dyn fmt::Display, options: &[&str]) {
        self.printer.print(LogLevel::Dir, &[&item], options);
    }

    pub fn dirxml(&self, _data: &[&dyn fmt::Display]) {
        todo!()
    }

    /// Create a counter named "label"
    pub fn count(&mut self, label: &str) {
        let mut cnt = 1;
        if self.count_map.contains_key(label) {
            cnt = self.count_map.get(label).unwrap() + 1;
        }

        self.count_map.insert(label.to_owned(), cnt);

        let concat = format!("{}: {}", label.to_owned(), cnt);
        self.logger(LogLevel::Count, &[&concat]);
    }

    /// Reset count of the given label to 0
    pub fn count_reset(&mut self, label: &str) {
        if !self.count_map.contains_key(label) {
            self.logger(LogLevel::CountReset, &[&"label does not exist"]);
            return;
        }

        self.count_map.insert(label.to_owned(), 0);
    }

    /// Create an group that will be displayed as expanded
    pub fn group(&mut self, data: &[&dyn fmt::Display]) {
        let group_label = if data.is_empty() {
            format!("console.group.{}", Uuid::new_v4())
        } else {
            self.formatter.format(data)
        };

        let group = Group {
            label: group_label,
        };

        // Group should be expanded
        self.printer.print(LogLevel::Group, &[&group.label], &[]);

        self.group_stacks.push(group);
    }

    /// Create a group that will be displayed as collapsed
    pub fn group_collapsed(&mut self, data: &[&dyn fmt::Display]) {
        let group_label = if data.is_empty() {
            format!("console.group.{}", Uuid::new_v4())
        } else {
            self.formatter.format(data)
        };

        let group = Group {
            label: group_label,
        };

        self.printer
            .print(LogLevel::GroupCollapsed, &[&group.label], &[]);

        self.group_stacks.push(group);
    }

    /// End the current group (if any)
    pub fn group_end(&mut self) -> Option<Group> {
        self.printer.end_group();
        self.group_stacks.pop()
    }

    /// Create a timer with given label
    pub fn time(&mut self, label: &str) {
        if self.timer_map.contains_key(label) {
            let warning = format!("Timer '{label}' already started");
            self.logger(LogLevel::Warn, &[&warning]);
            return;
        }

        let start = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(n) => n.as_millis(),
            Err(_) => 0,
        };

        self.timer_map.insert(
            label.to_owned(),
            Timer {
                label: label.to_owned(),
                start,
                end: None,
            },
        );
    }

    /// Log time
    pub fn time_log(&mut self, label: &str, data: &[&dyn fmt::Display]) {
        let mut message = String::from(" ");
        for arg in data {
            message.push_str(&format!("{arg} "));
        }
        let message = message.trim_end();

        let cur = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(n) => n.as_millis(),
            Err(_) => 0,
        };

        let concat = format!(
            "{}: {}ms{}",
            label.to_owned(),
            cur - self.timer_map.get(label).unwrap().start,
            message
        );
        self.printer.print(LogLevel::TimeLog, &[&concat], &[]);
    }

    /// End the timer with the given label
    pub fn time_end(&mut self, label: &str) {
        let end = match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(n) => n.as_millis(),
            Err(_) => 0,
        };

        let concat = format!(
            "{}: {}ms",
            label.to_owned(),
            end - self.timer_map.get(label).unwrap().start
        );
        self.printer.print(LogLevel::TimeEnd, &[&concat], &[]);
    }

    fn logger(&mut self, log_level: LogLevel, args: &[&dyn fmt::Display]) {
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

pub trait Printer {
    /// Prints the given data
    fn print(&mut self, log_level: LogLevel, args: &[&dyn fmt::Display], _options: &[&str]);
    /// Clears the console output (if possible)
    fn clear(&mut self);
    /// Notify the printer that the current group has ended
    fn end_group(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::{buffer::Buffer, writable_printer::WritablePrinter};
    use regex::Regex;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::thread::sleep;

    #[test]
    fn console() {
        let buffer = Rc::new(RefCell::new(Buffer::new()));
        let printer = WritablePrinter::new(Rc::clone(&buffer));
        let mut c = Console::new(Box::new(printer));

        c.log(&[&"some", &"data", &12i32]);
        c.warn(&[&"Hello", &"World"]);

        let out = buffer.borrow().try_to_string().unwrap();
        assert_eq!(
            out,
            "\
[log] some data 12\n\
[warn] Hello World\n\
"
        );
    }

    #[test]
    fn groups_and_timers() {
        let buffer = Rc::new(RefCell::new(Buffer::new()));
        let printer = WritablePrinter::new(Rc::clone(&buffer));
        let mut c = Console::new(Box::new(printer));

        c.clear();
        c.time("foo");
        c.group(&[&"foo"]);
        c.warn(&[&"Hello", &"World"]);
        c.group(&[&"bar"]);
        c.warn(&[&"Hello", &"World"]);
        c.group_end();
        c.warn(&[&"Hello", &"World"]);
        c.clear();
        c.group_end();
        sleep(std::time::Duration::from_millis(123));
        c.time_end("foo");
        c.group_end();
        c.group_end();
        c.warn(&[&"Back", &"To root"]);

        let out = buffer.borrow().try_to_string().unwrap();
        let re = Regex::new(
            r"^--- Clear ---
 > Expanded group: foo
 > \[warn\] Hello World
 >  > Expanded group: bar
 >  > \[warn\] Hello World
 > \[warn\] Hello World
--- Clear ---
foo: 1\d\dms - timer ended
\[warn\] Back To root
$",
        )
        .unwrap();
        assert!(re.is_match(&out));
    }

    #[test]
    fn assertions() {
        let buffer = Rc::new(RefCell::new(Buffer::new()));
        let printer = WritablePrinter::new(Rc::clone(&buffer));
        let mut c = Console::new(Box::new(printer));

        c.assert(true, &[&"This assertion asserts"]);
        c.assert(false, &[&"This assertion does not assert"]);
        c.assert(true, &[]);
        c.assert(false, &[]);

        let out = buffer.borrow().try_to_string().unwrap();
        assert_eq!(
            out,
            "\
[assert] Assertion failed: This assertion does not assert\n\
[assert] Assertion failed\n\
"
        );
    }
}
