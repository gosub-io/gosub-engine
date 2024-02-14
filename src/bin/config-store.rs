use anyhow::anyhow;
use clap::{Parser, Subcommand};
use derive_more::Display;
use gosub_config::settings::Setting;
use gosub_config::storage::*;
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
    #[clap(
        short = 'p',
        long = "path",
        global = true,
        default_value = "settings.db"
    )]
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
            let value = config_store().get(&key).unwrap();

            println!("Key            : {key}");
            println!("Current Value  : {value}");
            println!("Default Value  : {}", info.default);
            println!("Description    : {}", info.description);
        }
        Commands::List => {
            for key in config_store().find("*") {
                let value = config_store().get(&key).unwrap();
                println!("{key:40}: {value}");
            }
        }
        Commands::Set { key, value } => {
            config_store().set(&key, Setting::from_str(&value).expect("incorrect value"));
        }
        Commands::Search { key } => {
            for key in config_store().find(&key) {
                let value = config_store().get(&key).unwrap();
                println!("{key:40}: {value}");
            }
        }
    }

    Ok(())
}
