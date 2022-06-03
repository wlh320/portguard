use std::error::Error;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bincode::Options;
use blake2::{Blake2s256, Digest};
use curve25519_dalek::{constants::ED25519_BASEPOINT_TABLE, scalar::Scalar};
use log;
use serde::{Deserialize, Serialize};
use snowstorm::NoiseStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

use crate::consts::{CONF_BUF_LEN, PATTERN};
use crate::{gen, proxy};

/// client's builtin config, will be serialized to bincode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub server_addr: SocketAddr,
    pub target_addr: String, // TODO: should be Remote::Target, but it is untagged, cannot be decoded by bincode
    pub reverse: bool,
    pub server_pubkey: Vec<u8>,
    pub client_prikey: Vec<u8>,
}

impl ClientConfig {
    pub fn from_slice(bytes: &[u8]) -> Result<ClientConfig, bincode::Error> {
        bincode::options()
            .with_limit(CONF_BUF_LEN as u64)
            .allow_trailing_bytes()
            .deserialize(bytes)
    }

    pub fn to_vec(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::options()
            .with_limit(CONF_BUF_LEN as u64)
            .allow_trailing_bytes()
            .serialize(self)
    }
}

#[cfg_attr(target_os = "linux", link_section = ".portguard")]
#[cfg_attr(target_os = "windows", link_section = "pgmodify")]
#[cfg_attr(target_os = "macos", link_section = "__DATA,__portguard")]
#[used]
pub static CLIENT_CONF_BUF: [u8; CONF_BUF_LEN] = [0; CONF_BUF_LEN];

pub struct Client;

impl Client {
    /// entrance of client program
    pub async fn run_client(
        port: u16,
        server_addr: Option<SocketAddr>,
    ) -> Result<(), Box<dyn Error>> {
        let mut conf = ClientConfig::from_slice(&CLIENT_CONF_BUF)?;
        if let Some(addr) = server_addr {
            conf.server_addr = addr;
        }
        let conf = Arc::new(conf);
        match conf.reverse {
            true => Self::run_client_reverse_proxy(conf).await,
            false => Self::run_client_proxy(port, conf).await,
        }
    }

    /// client type: visitor (addr, socks5, rproxy)
    /// in config: remote = "127.0.0.1:xxxx"
    ///     or     remote = "socks5"
    ///     or     remote = 66
    async fn run_client_proxy(port: u16, conf: Arc<ClientConfig>) -> Result<(), Box<dyn Error>> {
        // read client config, overwrite server address
        // log information
        let listen_addr: SocketAddr = format!("127.0.0.1:{}", port).parse()?;
        log::info!("Client listening on: {:?}", listen_addr);
        log::info!("Portguard server on: {:?}", conf.server_addr);
        log::info!("Target address: {:?}", conf.target_addr);
        // start proxy
        let listener = TcpListener::bind(listen_addr).await?;
        while let Ok((inbound, _)) = listener.accept().await {
            let conf = conf.clone();
            tokio::spawn(async move {
                if let Err(e) = Client::handle_client_connection(inbound, &conf).await {
                    log::warn!("{}", e);
                }
            });
        }
        Ok(())
    }

    async fn handle_client_connection(
        inbound: TcpStream,
        conf: &ClientConfig,
    ) -> Result<(), Box<dyn Error>> {
        log::info!("New incoming peer_addr {:?}", inbound.peer_addr());
        // make noise stream
        let initiator = snowstorm::Builder::new(PATTERN.parse()?)
            .remote_public_key(&conf.server_pubkey)
            .local_private_key(&conf.client_prikey)
            .build_initiator()?;
        let outbound = TcpStream::connect(conf.server_addr).await?;
        let enc_outbound = NoiseStream::handshake(outbound, initiator).await?;
        // transfer data
        proxy::transfer_and_log_error(inbound, enc_outbound).await;
        Ok(())
    }

    /// client type: rclient (rproxy client)
    /// in config: remote = ["127.0.0.1:xxxx", 66]
    async fn run_client_reverse_proxy(conf: Arc<ClientConfig>) -> Result<(), Box<dyn Error>> {
        // must be valid address: socket addr or "socks5"
        assert!(
            conf.target_addr.to_lowercase() == "socks5"
                || conf.target_addr.parse::<SocketAddr>().is_ok()
        );
        // log information
        log::info!("Client exposing service on: {}", conf.target_addr);
        log::info!("Portguard server on: {}", conf.server_addr);
        // start reverse proxy
        loop {
            let conf = conf.clone();
            if let Err(e) = Self::make_reverse_proxy_conn(&conf).await {
                log::warn!("Failed to make reverse proxy connection. Error: {}", e);
            }
            // failed, wait and retry
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    async fn try_handshake(conf: &ClientConfig) -> Result<NoiseStream<TcpStream>, Box<dyn Error>> {
        let initiator = snowstorm::Builder::new(PATTERN.parse()?)
            .remote_public_key(&conf.server_pubkey)
            .local_private_key(&conf.client_prikey)
            .build_initiator()?;
        let conn = TcpStream::connect(&conf.server_addr).await?;
        let mut enc_conn = NoiseStream::handshake(conn, initiator).await?;
        // verify hash
        // if verification failed, we hope to abort program, so unwrap() is ok
        let mut hasher = Blake2s256::new();
        hasher.update(std::fs::read(std::env::current_exe().unwrap()).unwrap());
        let res = hasher.finalize();
        enc_conn.write_all(&res).await?;
        let ret = enc_conn.read_u8().await?;
        if ret != 66 {
            Err("Client hash is denied by server")?
        }
        Ok(enc_conn)
    }

    async fn make_reverse_proxy_conn(conf: &ClientConfig) -> Result<(), Box<dyn Error>> {
        // make connection with server
        log::info!("Trying to connect to server...");
        let enc_conn = match Self::try_handshake(conf).await {
            Ok(conn) => conn,
            Err(e) => {
                log::error!("Handshake failed. Error: {e}");
                panic!("Handshake failed");
            }
        };
        log::info!("Handshake succeeded.");
        // make yamux outbound stream and wait for incomming stream
        let yamux_config = yamux::Config::default();
        let mut yamux_conn =
            yamux::Connection::new(enc_conn.compat(), yamux_config, yamux::Mode::Server);
        while let Ok(Some(inbound)) = yamux_conn.next_stream().await {
            let conf = conf.clone();
            tokio::spawn(async move {
                if let Err(e) = Client::handle_reverse_client_connection(inbound, &conf).await {
                    log::warn!("{}", e);
                }
            });
        }
        Ok(())
    }
    /// handle yamux connection requests
    async fn handle_reverse_client_connection(
        inbound: yamux::Stream,
        conf: &ClientConfig,
    ) -> Result<(), Box<dyn Error>> {
        log::info!("New incoming request, stream id {:?}", inbound.id());
        if &conf.target_addr.to_lowercase() == "socks5" {
            // target is socks5
            proxy::transfer_to_socks5_and_log_error(inbound.compat()).await;
        } else {
            // target is socket addr
            let expose_addr = &conf
                .target_addr
                .parse::<SocketAddr>()
                .expect("Invalid target address");
            let outbound = TcpStream::connect(expose_addr).await?;
            proxy::transfer_and_log_error(inbound.compat(), outbound).await;
        }
        Ok(())
    }
    /// list current client public key
    pub fn list_pubkey(server: bool) -> Result<(), Box<dyn Error>> {
        let conf = ClientConfig::from_slice(&CLIENT_CONF_BUF)?;
        // derive pubkey
        let privkey = Scalar::from_bits(
            conf.client_prikey
                .try_into()
                .map_err(|_| "Got invalid privkey when deriving pubkey")?,
        );
        let point = (&ED25519_BASEPOINT_TABLE * &privkey).to_montgomery();
        let pubkey = base64::encode(point.to_bytes());
        println!("Client pubkey: {:?}", pubkey);
        if server {
            let key = base64::encode(conf.server_pubkey);
            println!("Server pubkey: {:?}", key);
        }
        Ok(())
    }
    /// generate client binary with a new keypair
    pub fn modify_client_keypair<P: AsRef<Path>>(
        in_path: P,
        out_path: P,
    ) -> Result<(), Box<dyn Error>> {
        let keypair = gen::gen_keypair()?;
        let mod_conf = move |old_conf: ClientConfig| ClientConfig {
            client_prikey: keypair.private,
            ..old_conf
        };
        // 2. gen new client binary
        gen::gen_client_binary(in_path.as_ref(), out_path.as_ref(), mod_conf)?;
        Ok(())
    }
}
