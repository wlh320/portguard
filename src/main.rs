use clap::{Args, Parser, Subcommand};
use portguard::client::Client;
use portguard::server::{Server, ServerConfig};
use std::env;
use std::error::Error;
use std::path::PathBuf;

#[derive(Parser)]
#[clap(author, version, about)]
#[clap(args_conflicts_with_subcommands = true)]

struct Cli {
    #[clap(subcommand)]
    command: Option<Commands>,

    #[clap(flatten)]
    /// run client, default command
    client: ClientArgs,
}

#[derive(Debug, Args)]
struct ClientArgs {
    /// local port to listen
    #[clap(short, long, default_value_t = 6000)]
    port: u16,
}

#[derive(Subcommand)]
enum Commands {
    /// run client
    Client(ClientArgs),
    /// run server
    Server {
        /// config file
        #[clap(short, long)]
        config: PathBuf,
    },
    /// generate client binary
    GenCli {
        #[clap(short, long)]
        config: PathBuf,
        #[clap(short, long)]
        output: PathBuf,
        #[clap(short, long, default_value = "user")]
        name: String,
    },
    /// generate keypairs
    GenKey {
        #[clap(short, long)]
        config: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info")
    }
    env_logger::init();

    let cli = Cli::parse();
    let client_cmd = cli.command.unwrap_or(Commands::Client(cli.client));
    match client_cmd {
        Commands::Client(ClientArgs { port }) => {
            let client = Client::new(port);
            client.run_client_proxy().await?;
        }
        Commands::Server { config: path } => {
            let content = std::fs::read_to_string(&path)?;
            let config: ServerConfig = toml::de::from_str(&content)?;
            let server = Server::new(config, &path);
            server.run_server_proxy().await?;
        }
        Commands::GenCli {
            config: path,
            output: out_path,
            name,
        } => {
            let content = std::fs::read_to_string(&path)?;
            let config = toml::de::from_str(&content)?;

            let mut server = portguard::server::Server::new(config, &path);
            server.gen_client(out_path, name)?;
        }
        Commands::GenKey { config: path } => {
            let content = std::fs::read_to_string(&path)?;
            let config = toml::de::from_str(&content)?;

            let mut server = portguard::server::Server::new(config, &path);
            server.gen_key()?;
        }
    }
    Ok(())
}
