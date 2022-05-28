use crate::consts::{CONF_BUF_LEN, PATTERN};
use crate::proxy;
use bincode::Options;
use curve25519_dalek::{constants::ED25519_BASEPOINT_TABLE, scalar::Scalar};
use futures::FutureExt;
use log;
use serde::{Deserialize, Serialize};
use snowstorm::NoiseStream;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::compat::{
    FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt,
};

/// client's builtin config, will be serialized to bincode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub server_addr: SocketAddr,
    pub target_addr: String, // only for print in log
    pub server_pubkey: Vec<u8>,
    pub client_prikey: Vec<u8>,
}

#[cfg_attr(target_os = "linux", link_section = ".portguard")]
#[cfg_attr(target_os = "windows", link_section = "pgmodify")]
#[cfg_attr(target_os = "macos", link_section = "__DATA,__portguard")]
#[used]
pub static CLIENT_CONF_BUF: [u8; CONF_BUF_LEN] = [0; CONF_BUF_LEN];

pub struct Client {
    /// local port to listen
    port: u16,
}

impl Client {
    pub fn new(port: u16) -> Client {
        Client { port }
    }

    /// normal client
    pub async fn run_client_proxy(
        self,
        server_addr: Option<SocketAddr>,
    ) -> Result<(), Box<dyn Error>> {
        let this = Arc::new(self);
        let mut conf: ClientConfig = bincode::options()
            .with_limit(CONF_BUF_LEN as u64)
            .allow_trailing_bytes()
            .deserialize(&CLIENT_CONF_BUF)?;

        // overwrite server address
        if let Some(addr) = server_addr {
            conf.server_addr = addr;
        }
        let shared_conf = Arc::new(conf);
        let listen_addr: SocketAddr = format!("127.0.0.1:{}", this.port).parse()?;
        log::info!("Client listening on: {:?}", listen_addr);
        log::info!("Portguard server on: {:?}", shared_conf.server_addr);
        log::info!("Target address: {:?}", shared_conf.target_addr);
        log::debug!(
            "Portguard server public key: {:?}",
            base64::encode(&shared_conf.server_pubkey)
        );

        let listener = TcpListener::bind(listen_addr).await?;

        while let Ok((inbound, _)) = listener.accept().await {
            let this = this.clone();
            let conf = shared_conf.clone();
            tokio::spawn(async move {
                if let Err(e) = this.handle_client_connection(inbound, &conf).await {
                    log::warn!("{}", e);
                }
            });
        }
        Ok(())
    }

    async fn handle_client_connection(
        &self,
        inbound: TcpStream,
        conf: &ClientConfig,
    ) -> Result<(), Box<dyn Error>> {
        log::info!("New incoming peer_addr {:?}", inbound.peer_addr());
        let initiator = snowstorm::Builder::new(PATTERN.parse()?)
            .remote_public_key(&conf.server_pubkey)
            .local_private_key(&conf.client_prikey)
            .build_initiator()?;
        let outbound = TcpStream::connect(conf.server_addr).await?;
        let enc_outbound = NoiseStream::handshake(outbound, initiator).await?;

        let transfer = proxy::transfer(inbound, enc_outbound).map(|r| {
            if let Err(e) = r {
                log::error!("Transfer error occured. error={}", e);
            }
        });
        transfer.await;
        Ok(())
    }

    /// TODO: impl reverse client
    pub async fn run_reverse_client_proxy(
        self,
        server_addr: Option<SocketAddr>,
    ) -> Result<(), Box<dyn Error>> {
        let this = Arc::new(self);
        let mut conf: ClientConfig = bincode::options()
            .with_limit(CONF_BUF_LEN as u64)
            .allow_trailing_bytes()
            .deserialize(&CLIENT_CONF_BUF)?;

        // overwrite server address
        if let Some(addr) = server_addr {
            conf.server_addr = addr;
        }
        let shared_conf = Arc::new(conf);
        let expose_addr: SocketAddr = format!("127.0.0.1:{}", this.port).parse()?;
        log::info!("Client exposing service on: {:?}", expose_addr);
        log::info!("Portguard server on: {:?}", shared_conf.server_addr);
        log::debug!(
            "Portguard server public key: {:?}",
            base64::encode(&shared_conf.server_pubkey)
        );
        let conf = shared_conf.clone();
        // make connection with server
        let initiator = snowstorm::Builder::new(PATTERN.parse()?)
            .remote_public_key(&conf.server_pubkey)
            .local_private_key(&conf.client_prikey)
            .build_initiator()?;
        let conn = TcpStream::connect(&conf.server_addr).await?;
        let enc_conn = NoiseStream::handshake(conn, initiator).await?;

        // yamux outbound stream and wait for incomming stream
        let yamux_config = yamux::Config::default();
        let mut yamux_conn =
            yamux::Connection::new(enc_conn.compat(), yamux_config, yamux::Mode::Server);

        while let Ok(Some(inbound)) = yamux_conn.next_stream().await {
            let this = this.clone();
            tokio::spawn(async move {
                if let Err(e) = this
                    .handle_reverse_client_connection(inbound, &expose_addr)
                    .await
                {
                    log::warn!("{}", e);
                }
            });
        }
        Ok(())
    }

    async fn handle_reverse_client_connection(
        &self,
        inbound: yamux::Stream,
        expose_addr: &SocketAddr,
    ) -> Result<(), Box<dyn Error>> {
        log::info!("New incoming request, stream id {:?}", inbound.id());
        let outbound = TcpStream::connect(expose_addr).await?;
        let transfer = proxy::transfer(inbound.compat(), outbound).map(|r| {
            if let Err(e) = r {
                log::error!("Transfer error occured. error={}", e);
            }
        });
        transfer.await;
        Ok(())
    }

    pub fn list_pubkey(server: bool) -> Result<(), Box<dyn Error>> {
        let conf: ClientConfig = bincode::options()
            .with_limit(CONF_BUF_LEN as u64)
            .allow_trailing_bytes()
            .deserialize(&CLIENT_CONF_BUF)?;

        // derive pubkey
        let privkey = Scalar::from_bits(conf.client_prikey.try_into().unwrap());
        let point = (&ED25519_BASEPOINT_TABLE * &privkey).to_montgomery();
        let pubkey = base64::encode(point.to_bytes());
        println!("Client pubkey: {:?}", pubkey);
        if server {
            let key = base64::encode(conf.server_pubkey);
            println!("Server pubkey: {:?}", key);
        }
        Ok(())
    }
}
