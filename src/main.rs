mod config;
mod index;
mod prune;
mod search;
mod store;
mod topics;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "amaranthine", about = "Persistent knowledge base for AI dev")]
struct Cli {
    /// Memory directory path (overrides auto-detection)
    #[arg(short, long, env = "AMARANTHINE_DIR")]
    dir: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Store an entry under a topic
    Store {
        /// Topic name (becomes filename, e.g. "rust-ffi")
        topic: String,
        /// Text to store
        text: String,
    },
    /// Search across all memory files
    Search {
        /// Search query (case-insensitive substring, or regex with -r)
        query: String,
        /// Treat query as regex
        #[arg(short, long)]
        regex: bool,
    },
    /// Generate topic index manifest
    Index,
    /// Show entries from the last N days
    Recent {
        /// Days to look back
        #[arg(default_value = "7")]
        days: u64,
    },
    /// List all topics with entry counts
    Topics,
    /// Flag stale topic files
    Prune {
        /// Days without updates before flagging
        #[arg(long, default_value = "30")]
        stale: u64,
    },
    /// Initialize a new memory directory
    Init {
        /// Path to initialize (default: .amaranthine/ in cwd)
        path: Option<String>,
    },
}

fn main() {
    let cli = Cli::parse();
    let dir = config::resolve_dir(cli.dir);

    let result = match cli.command {
        Command::Store { topic, text } => store::run(&dir, &topic, &text),
        Command::Search { query, regex } => search::run(&dir, &query, regex),
        Command::Index => index::run(&dir),
        Command::Recent { days } => topics::recent(&dir, days),
        Command::Topics => topics::list(&dir),
        Command::Prune { stale } => prune::run(&dir, stale),
        Command::Init { path } => config::init(path),
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
