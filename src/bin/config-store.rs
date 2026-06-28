use anyhow::anyhow;
use clap::{Parser, Subcommand};
use derive_more::Display;
use gosub_config::settings::Setting;
use gosub_config::storage::{JsonStorageAdapter, SqliteStorageAdapter};
use gosub_config::{config_store, config_store_write, StorageAdapter};
use std::str::FromStr;

#[derive(Debug, Parser)]
#[clap(name = "Config-Store", version = "0.1.0", author = "Gosub")]
struct Cli {
    #[clap(flatten)]
    global_opts: GlobalOpts,

    #[clap(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    #[clap(arg_required_else_help = true, about = "View a setting")]
    View {
        #[clap(required = true, short = 'k', long = "key")]
        key: String,
    },
    #[clap(about = "List all settings")]
    List,
    #[clap(arg_required_else_help = true, about = "Set a setting")]
    Set {
        #[clap(required = true, short = 'k', long = "key")]
        key: String,
        #[clap(required = true, short = 'v', long = "value")]
        value: String,
    },
    #[clap(arg_required_else_help = true, about = "Search for a setting")]
    Search {
        #[clap(required = true, short = 'k', long = "key")]
        key: String,
    },
}

#[derive(Clone, Copy, Debug, Display, clap::ValueEnum)]
enum Engine {
    Sqlite,
    Json,
}

impl std::str::FromStr for Engine {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sqlite" => Ok(Engine::Sqlite),
            "json" => Ok(Engine::Json),
            _ => Err(anyhow!("problem reading config")),
        }
    }
}

#[derive(Debug, Parser)]
struct GlobalOpts {
    #[clap(short = 'e', long = "engine", global = true, default_value = "sqlite")]
    engine: Engine,
    #[clap(short = 'p', long = "path", global = true, default_value = "settings.db")]
    path: String,
}

fn main() -> anyhow::Result<()> {
    let args = Cli::parse();

    let storage: Box<dyn StorageAdapter> = match args.global_opts.engine {
        Engine::Sqlite => Box::new(SqliteStorageAdapter::try_from(&args.global_opts.path)?),
        Engine::Json => Box::new(JsonStorageAdapter::try_from(&args.global_opts.path)?),
    };

    config_store_write().set_storage(storage);

    match args.command {
        Commands::View { key } => {
            if !config_store().has(&key) {
                println!("Key not found");
                return Ok(());
            }

            let info = config_store().get_info(&key).unwrap();
            let value = config_store().get(&key)?.unwrap_or(info.default.clone());

            println!("Key            : {key}");
            println!("Type           : {}", value.type_name());
            println!("Current Value  : {}", value.value_string());
            println!("Default Value  : {}", info.default.value_string());
            if let Some(constraint) = &info.constraint {
                println!("Allowed Values : {constraint}");
            }
            println!("Description    : {}", info.description);
        }
        Commands::List => {
            print_table(&config_store().find("*"))?;
        }
        Commands::Set { key, value } => {
            config_store().set(&key, Setting::from_str(&value)?)?;
        }
        Commands::Search { key } => {
            print_table(&config_store().find(&key))?;
        }
    }

    Ok(())
}

/// A single rendered row of the settings table.
struct Row {
    key: String,
    type_name: String,
    value: String,
    allowed: String,
}

/// Prints the given settings as an aligned table with a header row. The `ALLOWED` column is only
/// shown when at least one of the settings has a constraint.
fn print_table(keys: &[String]) -> anyhow::Result<()> {
    let store = config_store();

    let mut rows = Vec::new();
    for key in keys {
        let Some(info) = store.get_info(key) else {
            continue;
        };
        let value = store.get(key)?.unwrap_or_else(|| info.default.clone());
        let rendered = value.value_string();

        rows.push(Row {
            key: key.clone(),
            type_name: value.type_name().to_string(),
            // Em dash for empty values (e.g. an empty map) so the column never looks blank.
            value: if rendered.is_empty() { "—".to_string() } else { rendered },
            allowed: info.constraint.as_ref().map(|c| c.compact()).unwrap_or_default(),
        });
    }

    if rows.is_empty() {
        println!("No settings found");
        return Ok(());
    }

    let show_allowed = rows.iter().any(|r| !r.allowed.is_empty());

    // Column widths account for both the header label and the widest cell (measured in chars).
    let width = |header: &str, cells: &dyn Fn(&Row) -> &str| {
        rows.iter().map(|r| cells(r).chars().count()).chain([header.chars().count()]).max().unwrap_or(0)
    };
    let kw = width("KEY", &|r| &r.key);
    let tw = width("TYPE", &|r| &r.type_name);
    let vw = width("VALUE", &|r| &r.value);

    let mut header = format!("{:<kw$}  {:<tw$}  {:<vw$}", "KEY", "TYPE", "VALUE");
    if show_allowed {
        header.push_str("  ALLOWED");
    }
    println!("{header}");
    println!("{}", "─".repeat(header.chars().count()));

    for r in &rows {
        let mut line = format!("{:<kw$}  {:<tw$}  {:<vw$}", r.key, r.type_name, r.value);
        if show_allowed && !r.allowed.is_empty() {
            line.push_str("  ");
            line.push_str(&r.allowed);
        }
        // Trim trailing spaces from rows without an allowed value.
        println!("{}", line.trim_end());
    }

    Ok(())
}
