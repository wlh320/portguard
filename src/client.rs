use crate::consts::{CONF_BUF_LEN, PATTERN};
use crate::proxy;
use bincode::Options;
use futures::FutureExt;
use log;
use serde::{Deserialize, Serialize};
use snowstorm::NoiseStream;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};

/// client's builtin config, will be serialized to bincode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub server_addr: SocketAddr,
    pub target_addr: String, // only for print in log
    pub server_pubkey: Vec<u8>,
    pub client_prikey: Vec<u8>,
}

#[link_section = "modify"]
#[used]
pub static CLIENT_CONF_BUF: [u8; CONF_BUF_LEN] = [0; CONF_BUF_LEN];

pub struct Client {
    /// loal port to listen
    port: u16,
}

impl Client {
    pub fn new(port: u16) -> Client {
        Client { port }
    }
    pub async fn run_client_proxy(self, server: Option<SocketAddr>) -> Result<(), Box<dyn Error>> {
        let this = Arc::new(self);
        let mut conf: ClientConfig = bincode::options()
            .with_limit(CONF_BUF_LEN as u64)
            .allow_trailing_bytes()
            .deserialize(&CLIENT_CONF_BUF)?;

        // overwrite server address
        if let Some(server_addr) = server {
            conf.server_addr = server_addr;
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
                if let Err(e) = this.handle_connection(inbound, &conf).await {
                    log::warn!("{}", e);
                }
            });
        }
        Ok(())
    }

    async fn handle_connection(
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
}
