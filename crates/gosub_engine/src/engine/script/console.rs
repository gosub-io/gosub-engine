//! `console` for page scripts: `gosub_jsapi`'s console on the global object.
//!
//! The glue is written by hand against the `gosub_webexecutor` traits, so it names no
//! script engine. The `gosub_webinterop` macros can bind these methods, but their variadic
//! glue throws on a call without arguments (`console.log()`, `console.count()`).

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use gosub_interface::script::ConsoleLevel;
use gosub_jsapi::console::{Console, LogLevel, Printer};
use gosub_shared::types::Result;
use gosub_webexecutor::js::{
    VariadicArgsInternal, WebContext, WebFunctionCallBackVariadic, WebFunctionVariadic, WebObject, WebRuntime, WebValue,
};

use super::ConsoleSink;

/// One argument of a console call.
struct Arg {
    /// As `String(value)` gives it.
    text: String,
    truthy: bool,
}

type Method = fn(&mut Console, &[Arg]);

/// The most text one console call passes on, in bytes, each argument counting for one
/// more (the space after it, and an empty argument is not free either). The argument that
/// goes over it is cut and marked, and the ones after it are not looked at: a call cannot
/// take memory by the number of its arguments.
pub(super) const MAX_CALL_TEXT: usize = 64 * 1024;

/// Ends the text of a call that was cut.
pub(super) const CUT: &str = "... (cut)";

/// The arguments of a call as text, up to [`MAX_CALL_TEXT`] in all.
fn bounded<V: WebValue>(values: &[V]) -> Vec<Arg> {
    let mut args: Vec<Arg> = Vec::new();
    let mut left = MAX_CALL_TEXT;

    for value in values {
        if left == 0 {
            if let Some(last) = args.last_mut().filter(|last| !last.text.ends_with(CUT)) {
                last.text.push_str(CUT);
            }
            break;
        }

        let mut text = value.as_string().unwrap_or_default();
        if text.len() > left {
            let mut end = left;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.shrink_to_fit();
            text.push_str(CUT);
            left = 0;
        } else {
            left = (left - text.len()).saturating_sub(1);
        }
        args.push(Arg {
            text,
            truthy: value.as_bool().unwrap_or(false),
        });
    }

    args
}

const METHODS: &[(&str, Method)] = &[
    ("assert", |console, args| {
        let condition = args.first().is_some_and(|arg| arg.truthy);
        console.assert(condition, &displays(args.get(1..).unwrap_or_default()));
    }),
    ("clear", |console, _| console.clear()),
    ("debug", |console, args| console.debug(&displays(args))),
    ("error", |console, args| console.error(&displays(args))),
    ("info", |console, args| console.info(&displays(args))),
    ("log", |console, args| console.log(&displays(args))),
    ("table", |console, args| {
        if let Some(data) = args.first() {
            console.table(data.text.clone(), &[]);
        }
    }),
    ("trace", |console, args| console.trace(&displays(args))),
    ("warn", |console, args| console.warn(&displays(args))),
    ("dir", |console, args| {
        if let Some(item) = args.first() {
            console.dir(&item.text, &[]);
        }
    }),
    ("dirxml", |console, args| console.dirxml(&displays(args))),
    ("count", |console, args| console.count(label(args))),
    ("countReset", |console, args| console.count_reset(label(args))),
    ("group", |console, args| console.group(&displays(args))),
    ("groupCollapsed", |console, args| {
        console.group_collapsed(&displays(args))
    }),
    ("groupEnd", |console, _| {
        console.group_end();
    }),
    ("time", |console, args| console.time(label(args))),
    ("timeLog", |console, args| {
        console.time_log(label(args), &displays(args.get(1..).unwrap_or_default()));
    }),
    ("timeEnd", |console, args| console.time_end(label(args))),
];

fn displays(args: &[Arg]) -> Vec<&dyn fmt::Display> {
    args.iter().map(|arg| &arg.text as &dyn fmt::Display).collect()
}

/// The first argument as a label; `default` when there is none, as the console spec has it.
fn label(args: &[Arg]) -> &str {
    args.first().map_or("default", |arg| &arg.text)
}

/// Puts `console` on the global object of `context`. Its output goes to `sink`.
pub(super) fn install<RT: WebRuntime>(mut context: RT::Context, sink: ConsoleSink) -> Result<()> {
    let console = Rc::new(RefCell::new(Console::new(Box::new(SinkPrinter { sink, depth: 0 }))));

    let object = RT::Object::new(&context)?;
    context.set_on_global_object("console", object.clone().into())?;

    for &(name, method) in METHODS {
        let console = Rc::clone(&console);
        let function = RT::FunctionVariadic::new(context.clone(), move |cb| {
            let ctx = cb.context();
            // read before the console is borrowed: a toString() may itself log
            let args = bounded(&cb.args().as_vec(ctx.clone()));

            method(&mut console.borrow_mut(), &args);

            match RT::Value::new_undefined(ctx) {
                Ok(undefined) => cb.ret(undefined),
                Err(e) => cb.error(e),
            }
        })?;
        object.set_method_variadic(name, &function)?;
    }

    Ok(())
}

/// Hands what the console prints to the sink, one message per call, indented by group.
struct SinkPrinter {
    sink: ConsoleSink,
    depth: usize,
}

impl Printer for SinkPrinter {
    fn print(&mut self, log_level: LogLevel, args: &[&dyn fmt::Display], _options: &[&str]) {
        let text = args.iter().map(ToString::to_string).collect::<Vec<_>>().join(" ");
        let text = format!("{}{text}", "  ".repeat(self.depth));

        let level = match log_level {
            LogLevel::Debug => ConsoleLevel::Debug,
            LogLevel::Info => ConsoleLevel::Info,
            LogLevel::Warn | LogLevel::CountReset => ConsoleLevel::Warn,
            LogLevel::Error | LogLevel::Assert => ConsoleLevel::Error,
            LogLevel::Group | LogLevel::GroupCollapsed => {
                // the label itself is outside the group it opens
                self.depth += 1;
                ConsoleLevel::Log
            }
            _ => ConsoleLevel::Log,
        };

        (self.sink)(level, text);
    }

    fn clear(&mut self) {
        self.depth = 0;
    }

    fn end_group(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }
}
