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
    /// replace another server address in this run
    #[clap(short, long)]
    server: Option<String>
}

#[derive(Subcommand)]
enum Commands {
    /// run client
    Client(ClientArgs),
    /// run server
    Server {
        /// location of config file
        #[clap(short, long)]
        config: PathBuf,
    },
    /// generate client binary
    GenCli {
        /// location of config file
        #[clap(short, long)]
        config: PathBuf,
        /// location of output binary
        #[clap(short, long)]
        output: PathBuf,
        /// name of client
        #[clap(short, long, default_value = "user")]
        name: String,
        /// client specified remote address
        #[clap(short, long)]
        remote: Option<String>,
    },
    /// generate keypairs
    GenKey {
        /// location of config file
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
        Commands::Client(ClientArgs { port, server }) => {
            let client = Client::new(port);
            let server = server.and_then(|s| s.parse().ok());
            client.run_client_proxy(server).await?;
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
            remote
        } => {
            let content = std::fs::read_to_string(&path)?;
            let config = toml::de::from_str(&content)?;

            let mut server = portguard::server::Server::new(config, &path);
            server.gen_client(out_path, name, remote)?;
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
