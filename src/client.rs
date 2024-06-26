use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use backoff::{future::retry, ExponentialBackoff};
use bincode::Options;
use blake2::{Blake2s256, Digest};
use chacha20poly1305::aead::{Aead, NewAead};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce}; // Or `XChaCha20Poly1305`
use curve25519_dalek::EdwardsPoint;
use log;
use serde::{Deserialize, Serialize};
use snowstorm::NoiseStream;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

use crate::consts::{CONF_BUF_LEN, KEYPASS_LEN, PATTERN};
use crate::proxy;

/// client's builtin config, will be serialized to bincode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub server_addr: SocketAddr,
    pub target_addr: String, // TODO: should be Remote::Target, but it is untagged, cannot be decoded by bincode
    pub reverse: bool,
    pub server_pubkey: Vec<u8>,
    pub client_prikey: Vec<u8>,
    pub has_keypass: bool, // client prikey passphrase
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
#[cfg_attr(target_os = "android", link_section = ".portguard")]
#[cfg_attr(target_os = "windows", link_section = "pgmodify")]
#[cfg_attr(target_os = "macos", link_section = "__DATA,__portguard")]
#[used]
pub static CLIENT_CONF_BUF: [u8; CONF_BUF_LEN] = [0; CONF_BUF_LEN];

pub struct Client;

impl Client {
    /// entrance of client program
    pub async fn run_client(port: u16, server_addr: Option<SocketAddr>) -> Result<()> {
        let mut conf = ClientConfig::from_slice(&CLIENT_CONF_BUF)?;
        if let Some(addr) = server_addr {
            conf.server_addr = addr;
        }
        // verfify client key passphrase
        if conf.has_keypass {
            conf.client_prikey = Self::decrypt_client_prikey(conf.client_prikey)?;
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
    async fn run_client_proxy(port: u16, conf: Arc<ClientConfig>) -> Result<()> {
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
    async fn handle_client_connection(inbound: TcpStream, conf: &ClientConfig) -> Result<()> {
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
    async fn run_client_reverse_proxy(conf: Arc<ClientConfig>) -> Result<()> {
        // must be valid address: socket addr or "socks5"
        assert!(
            conf.target_addr.to_lowercase() == "socks5"
                || conf.target_addr.parse::<SocketAddr>().is_ok()
        );
        // log information
        log::info!("Client exposing service on: {}", conf.target_addr);
        log::info!("Portguard server on: {}", conf.server_addr);
        // start reverse proxy
        let try_conn = || async {
            let conf = conf.clone();
            Self::make_reverse_proxy_conn(&conf).await.map_err(|e| {
                log::warn!("Failed to make reverse proxy connection. Error: {}", e);
                backoff::Error::transient(e)
            })
        };
        retry(ExponentialBackoff::default(), try_conn).await
    }
    async fn try_handshake(conf: &ClientConfig) -> Result<NoiseStream<TcpStream>> {
        let initiator = snowstorm::Builder::new(PATTERN.parse()?)
            .remote_public_key(&conf.server_pubkey)
            .local_private_key(&conf.client_prikey)
            .build_initiator()?;
        let conn = TcpStream::connect(&conf.server_addr).await?;
        let mut enc_conn = NoiseStream::handshake(conn, initiator).await?;
        // verify hash
        let mut hasher = Blake2s256::new();
        hasher.update(std::fs::read(std::env::current_exe()?)?);
        let res = hasher.finalize();
        enc_conn.write_all(&res).await?;
        let ret = enc_conn.read_u8().await?;
        match ret {
            66 => Ok(enc_conn),
            88 => panic!("Service is already online!"),
            _ => Err(anyhow!("Client hash is denied by server"))?,
        }
    }
    async fn make_reverse_proxy_conn(conf: &ClientConfig) -> Result<()> {
        // make connection with server
        log::info!("Trying to connect to server...");
        let enc_conn = Self::try_handshake(conf).await?;
        log::info!("Handshake succeeded.");
        // make yamux outbound stream and wait for incomming stream
        let yamux_config = yamux::Config::default();
        let mut yamux_conn =
            yamux::Connection::new(enc_conn.compat(), yamux_config, yamux::Mode::Server);
        while let Some(inbound) = yamux_conn.next_stream().await? {
            let conf = conf.clone();
            tokio::spawn(async move {
                if let Err(e) = Client::handle_reverse_client_connection(inbound, &conf).await {
                    log::warn!("{}", e);
                }
            });
        }
        log::info!("Connection closed.");
        Err(anyhow!("Connection lost"))
    }
    /// handle yamux connection requests
    async fn handle_reverse_client_connection(
        inbound: yamux::Stream,
        conf: &ClientConfig,
    ) -> Result<(), io::Error> {
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
    /// verify key password
    fn decrypt_client_prikey(key: Vec<u8>) -> Result<Vec<u8>> {
        let mut password = rpassword::prompt_password("Input Key Passphrase: ")?.into_bytes();
        password.resize(KEYPASS_LEN, 0);
        let keypass = Key::from_slice(&password);
        let cipher = ChaCha20Poly1305::new(keypass);
        let key = cipher.decrypt(&Nonce::default(), &key[..])?;
        Ok(key)
    }

    /// list current client public key
    pub fn list_pubkey(server: bool) -> Result<()> {
        let conf = ClientConfig::from_slice(&CLIENT_CONF_BUF)?;
        let bits = conf
            .client_prikey
            .try_into()
            .map_err(|_| anyhow!("Got invalid privkey when deriving pubkey"))?;
        let point = EdwardsPoint::mul_base_clamped(bits).to_montgomery();
        let pubkey = base64::encode(point.to_bytes());
        println!("Client pubkey: {:?}", pubkey);
        if server {
            let key = base64::encode(conf.server_pubkey);
            println!("Server pubkey: {:?}", key);
        }
        Ok(())
    }
}
