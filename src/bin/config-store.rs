use clap::{Parser, Subcommand};
use derive_more::Display;
use gosub_engine::config::settings::{Setting, SettingInfo};
use gosub_engine::config::storage::json_storage::JsonStorageAdapter;
use gosub_engine::config::storage::sqlite_storage::SqliteStorageAdapter;
use gosub_engine::config::ConfigStore;

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

#[derive(Debug, Clone, Copy, clap::ValueEnum, Display)]
enum Engine {
    Sqlite,
    Json,
}

impl std::str::FromStr for Engine {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "sqlite" => Ok(Engine::Sqlite),
            "json" => Ok(Engine::Json),
            _ => Err(()),
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

enum Config {
    Sqlite(ConfigStore<SqliteStorageAdapter>),
    Json(ConfigStore<JsonStorageAdapter>),
}

impl Config {
    fn find(&self, search: &str) -> Vec<String> {
        match self {
            Self::Sqlite(store) => store.find(search),
            Self::Json(store) => store.find(search),
        }
    }

    fn has(&self, key: &str) -> bool {
        match self {
            Self::Sqlite(store) => store.has(key),
            Self::Json(store) => store.has(key),
        }
    }

    fn get(&mut self, key: &str) -> Setting {
        match self {
            Self::Sqlite(store) => store.get(key),
            Self::Json(store) => store.get(key),
        }
    }

    fn get_info(&self, key: &str) -> Option<SettingInfo> {
        match self {
            Self::Sqlite(store) => store.get_info(key),
            Self::Json(store) => store.get_info(key),
        }
    }

    fn set(&mut self, key: &str, value: Setting) {
        match self {
            Self::Sqlite(store) => store.set(key, value),
            Self::Json(store) => store.set(key, value),
        }
    }
}

fn main() {
    let args = Cli::parse();

    let mut store: Config = match args.global_opts.engine {
        Engine::Sqlite => {
            let store = SqliteStorageAdapter::new(args.global_opts.path.as_str());
            Config::Sqlite(ConfigStore::new(store, true))
        }
        Engine::Json => {
            let store = JsonStorageAdapter::new(args.global_opts.path.as_str());
            Config::Json(ConfigStore::new(store, true))
        }
    };

    match args.command {
        Commands::View { key } => {
            if !store.has(key.as_str()) {
                println!("Key not found");
                return;
            }

            let info = store.get_info(key.as_str()).unwrap();
            let value = store.get(key.as_str());

            println!("Key            : {}", key);
            println!("Current Value  : {}", value);
            println!("Default Value  : {}", info.default);
            println!("Description    : {}", info.description);
        }
        Commands::List => {
            for key in store.find("*") {
                let value = store.get(key.as_str());
                println!("{:40}: {}", key, value);
            }
        }
        Commands::Set { key, value } => {
            store.set(
                key.as_str(),
                Setting::from_string(value.as_str()).expect("incorrect value"),
            );
        }
        Commands::Search { key } => {
            for key in store.find(key.as_str()) {
                let value = store.get(key.as_str());
                println!("{:40}: {}", key, value);
            }
        }
    }
}
