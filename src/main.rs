use clap::{Args, Parser, Subcommand};
use portguard::client::Client;
use portguard::server::{Server, ServerConfig};
use portguard::Remote;
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
    /// Run client, default command
    client: ClientArgs,
}

#[derive(Debug, Args)]
struct ClientArgs {
    /// local port to listen
    #[clap(short, long, default_value_t = 6000)]
    port: u16,
    /// use another server address in this run
    #[clap(short, long)]
    server: Option<String>,
    /// whether a reverse proxy client
    #[clap(short, long)]
    reverse: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Run client
    Client(ClientArgs),
    /// Run server
    Server {
        /// location of config file
        #[clap(short, long)]
        config: PathBuf,
    },
    /// Generate client binary
    GenCli {
        /// location of config file
        #[clap(short, long)]
        config: PathBuf,
        /// location of input binary (current binary by default)
        #[clap(short, long)]
        input: Option<PathBuf>,
        /// location of output binary
        #[clap(short, long)]
        output: PathBuf,
        /// name of client
        #[clap(short, long, default_value = "user")]
        name: String,
        /// client's target address, can be socket address or "socks5"
        #[clap(short, long)]
        target: Option<String>,
        /// service id of a reverse proxy
        #[clap(short, long)]
        sid: Option<usize>,
    },
    /// Generate keypairs
    GenKey {
        /// location of config file
        #[clap(short, long)]
        config: PathBuf,
    },
    /// List client pubkey in client config
    ListKey {
        /// if set this flag, then also list server pubkey
        #[clap(short, long)]
        server: bool,
    },
}

async fn run() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let client_cmd = cli.command.unwrap_or(Commands::Client(cli.client));
    match client_cmd {
        Commands::Client(ClientArgs {
            port,
            server,
            reverse,
        }) => {
            let client = Client::new(port);
            let server = server.and_then(|s| s.parse().ok());
            if reverse {
                client.run_client_reverse_proxy(server).await?;
            } else {
                client.run_client_proxy(server).await?;
            }
        }
        Commands::Server { config: path } => {
            let content = std::fs::read_to_string(&path)?;
            let config: ServerConfig = toml::de::from_str(&content)?;
            let server = Server::new(config, &path);
            server.run_server_proxy().await?;
        }
        Commands::GenCli {
            config: path,
            input: in_path,
            output: out_path,
            name,
            target,
            sid,
        } => {
            let in_path = in_path.unwrap_or(env::current_exe()?);
            let content = std::fs::read_to_string(&path)?;
            let config = toml::de::from_str(&content)?;
            let remote = Remote::try_parse(target, sid)
                .map_err(|e| {
                    log::warn!("Invalid remote input, use default, error {}", e);
                })
                .ok();

            let mut server = portguard::server::Server::new(config, &path);
            server.gen_client(in_path, out_path, name, remote)?;
        }
        Commands::GenKey { config: path } => {
            let content = std::fs::read_to_string(&path)?;
            let config = toml::de::from_str(&content)?;

            let mut server = portguard::server::Server::new(config, &path);
            server.gen_key()?;
        }
        Commands::ListKey { server } => {
            Client::list_pubkey(server)?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info")
    }
    env_logger::init();
    run().await.map_err(|e| {
        log::error!("{:?}", e);
        e
    })
}
