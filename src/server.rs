use std::borrow::Borrow;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use blake2::{Blake2s256, Digest};
use dashmap::DashMap;
use log;
use serde::{Deserialize, Serialize};
use snowstorm::NoiseStream;
use tokio::io::{self, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

use crate::client::ClientConfig;
use crate::consts::{FILEHASH_LEN, PATTERN};
use crate::gen;
use crate::proxy;
use crate::remote::{Remote, Target};

// type ConnMap = HashMap<usize, Mutex<yamux::Control>>;

/// copy from https://users.rust-lang.org/t/serialize-a-vec-u8-to-json-as-base64/57781/2
mod base64_serde {
    use serde::{Deserialize, Serialize};
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        let base64 = base64::encode(v);
        String::serialize(&base64, s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let base64 = String::deserialize(d)?;
        base64::decode(base64.as_bytes()).map_err(serde::de::Error::custom)
    }
}

#[derive(PartialEq, Eq, Clone, Debug, Serialize, Deserialize)]
struct FileHash {
    #[serde(with = "base64_serde")]
    hash: Vec<u8>,
}

#[derive(Eq, Debug, Serialize, Deserialize)]
struct ClientEntry {
    /// user name
    name: String,
    /// client public key for auth
    #[serde(with = "base64_serde")]
    pubkey: Vec<u8>,
    /// file hash, for verifying reverse proxy
    #[serde(flatten)]
    filehash: Option<FileHash>,
    /// client specified remote address
    remote: Option<Remote>,
}

impl PartialEq for ClientEntry {
    fn eq(&self, other: &ClientEntry) -> bool {
        self.pubkey == other.pubkey
    }
}
impl Hash for ClientEntry {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.pubkey.hash(state);
    }
}
impl Borrow<[u8]> for ClientEntry {
    fn borrow(&self) -> &[u8] {
        &self.pubkey
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ServerConfig {
    /// server public ip or domain
    #[serde(default = "default_host")]
    host: String,
    /// server listen port
    #[serde(default = "default_port")]
    port: u16,
    /// default remote address hope to proxy
    #[serde(default = "default_remote")]
    remote: Remote,
    /// server public key
    #[serde(with = "base64_serde", default)]
    pubkey: Vec<u8>,
    /// server private key
    #[serde(with = "base64_serde", default)]
    prikey: Vec<u8>,
    /// sequence of clients
    #[serde(skip_serializing_if = "HashSet::is_empty", default)]
    clients: HashSet<ClientEntry>,
}

fn default_port() -> u16 {
    8022
}

fn default_host() -> String {
    "192.168.1.1".to_string()
}

fn default_remote() -> Remote {
    Remote::Proxy(Target::Socks5)
}

impl ServerConfig {
    fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = toml::ser::to_string(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

/// Portguard server
pub struct Server {
    config_path: PathBuf,
    config: ServerConfig,
    conns: DashMap<usize, yamux::Control>,
}

impl Server {
    pub fn build(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(&path)?;
        let config: ServerConfig = toml::de::from_str(&content)?;
        Ok(Server {
            config,
            config_path: path.as_ref().into(),
            conns: DashMap::new(),
        })
    }
    /// code for generation
    pub fn gen_client<P: AsRef<Path>>(
        &mut self,
        in_path: P,
        out_path: P,
        username: String,
        oremote: Option<Remote>,
        has_keypass: bool,
    ) -> Result<()> {
        // 1. set client config
        let keypair = gen::gen_keypair(has_keypass)?;
        let remote = oremote.unwrap_or(self.config.remote);
        let reverse = matches!(remote, Remote::RProxy(_, _));
        let cli_conf: ClientConfig = ClientConfig {
            server_addr: format!("{}:{}", self.config.host, self.config.port).parse()?,
            target_addr: remote.to_string(),
            reverse,
            server_pubkey: self.config.pubkey.clone(),
            client_prikey: keypair.private,
            has_keypass,
        };
        // 2. gen client binary
        gen::gen_client_binary(in_path.as_ref(), out_path.as_ref(), |_| cli_conf)?;
        let filehash = if reverse {
            let mut hasher = Blake2s256::new();
            hasher.update(std::fs::read(out_path.as_ref()).unwrap());
            let res = hasher.finalize();
            Some(FileHash { hash: res.to_vec() })
        } else {
            None
        };
        // 3. add new client to server config
        let client = ClientEntry {
            name: username,
            pubkey: keypair.public,
            remote: oremote,
            filehash,
        };
        self.config.clients.insert(client);
        // 4. save server config
        self.config.save(&self.config_path)?;
        Ok(())
    }
    pub fn gen_key(&mut self) -> Result<()> {
        // gen key
        let keypair = gen::gen_keypair(false)?;
        self.config.pubkey = keypair.public;
        self.config.prikey = keypair.private;
        // save
        self.config.save(&self.config_path)?;
        Ok(())
    }

    /// server functions:
    /// handle_xxx -> handle incoming connections
    /// start_xxx  -> spawn proxy tasks
    pub async fn run_server_proxy(self) -> Result<()> {
        let this1 = Arc::new(self);
        let this2 = Arc::clone(&this1);
        let listen_addr: SocketAddr = format!("0.0.0.0:{}", this1.config.port).parse().unwrap();
        log::info!("Listening on port: {:?}", listen_addr);

        // TODO: spawn to handle config hot-reloading

        // spwan to handle inbound connection
        let listener = TcpListener::bind(listen_addr).await?;
        while let Ok((inbound, _)) = listener.accept().await {
            let this = Arc::clone(&this2);
            tokio::spawn(async move {
                if let Err(e) = this.handle_connection(inbound).await {
                    log::warn!("{}", e);
                }
            });
        }
        Ok(())
    }
    /// handle inbound connection
    async fn handle_connection(&self, inbound: TcpStream) -> Result<()> {
        let enc_inbound = self.accept_noise_stream(inbound).await?;
        // at this point, client already passed verification
        // can use `.unwrap()` here because client must have a static key
        let token = enc_inbound.get_state().get_remote_static().unwrap();
        let client_remote = self.config.clients.get(token).unwrap().remote;
        let remote = client_remote.unwrap_or(self.config.remote);
        match remote {
            Remote::Proxy(target) => Self::start_proxy_to_target(enc_inbound, target).await?,
            Remote::Service(id) => self.start_proxy_to_rproxy_conn(id, enc_inbound).await?,
            Remote::RProxy(target, id) => {
                let enc_inbound = self.try_handshake(id, enc_inbound).await?;
                self.start_new_rproxy_conn(enc_inbound, id, target).await?;
            }
        };
        Ok(())
    }
    /// start to handle proxy
    async fn start_proxy_to_target(
        inbound: NoiseStream<TcpStream>,
        target: Target,
    ) -> Result<(), io::Error> {
        let peer_addr = inbound.get_inner().peer_addr()?;
        match target {
            Target::Addr(addr) => {
                log::info!("Start proxying {peer_addr} to {addr}");
                let outbound = TcpStream::connect(addr).await?;
                proxy::transfer_and_log_error(inbound, outbound).await;
            }
            Target::Socks5 => {
                log::info!("Start proxying {peer_addr} to built-in socks5 server");
                proxy::transfer_to_socks5_and_log_error(inbound).await;
            }
        }
        Ok(())
    }
    /// start to handle rproxy conn for visitor
    async fn start_proxy_to_rproxy_conn(
        &self,
        id: usize,
        inbound: NoiseStream<TcpStream>,
    ) -> Result<()> {
        let peer_addr = inbound.get_inner().peer_addr();
        log::info!("Start proxying {peer_addr:?} to rproxy service (id: {id})");
        let mut ctrl = self
            .conns
            .get_mut(&id)
            .ok_or_else(|| anyhow!("Service offline"))?;
        let outbound = ctrl.open_stream().await?;
        tokio::spawn(async move {
            proxy::transfer_and_log_error(inbound, outbound.compat()).await;
        });
        Ok(())
    }
    /// start a new rproxy connection
    async fn start_new_rproxy_conn(
        &self,
        inbound: NoiseStream<TcpStream>,
        id: usize,
        target: Target,
    ) -> Result<()> {
        // 1. make conneciton
        let peer_addr = inbound.get_inner().peer_addr()?;
        let target = target.to_string();
        log::info!("Start reverse proxy ({peer_addr}:{target}) as service (id {id})");
        let yamux_config = yamux::Config::default();
        let mut yamux_conn =
            yamux::Connection::new(inbound.compat(), yamux_config, yamux::Mode::Client);
        let control = yamux_conn.control();
        // 2. update connection map
        self.conns.insert(id, control);
        tokio::spawn(async move {
            while let Ok(Some(_)) = yamux_conn.next_stream().await {}
            yamux_conn.control().close().await
        })
        .await
        .ok();
        self.conns.remove(&id);
        log::info!("Service {id} disconnect.");
        Ok(())
    }

    /// helper function
    async fn accept_noise_stream(
        &self,
        inbound: TcpStream,
    ) -> Result<NoiseStream<TcpStream>, snowstorm::SnowstormError> {
        log::info!("New incoming stream (peer_addr {:?})", inbound.peer_addr());
        // create noise stream & client auth
        let responder = snowstorm::Builder::new(PATTERN.parse()?)
            .local_private_key(&self.config.prikey)
            .build_responder()?;
        let enc_inbound = NoiseStream::handshake_with_verifier(inbound, responder, |key| {
            self.config.clients.contains(key)
        })
        .await?;
        Ok(enc_inbound)
    }
    async fn try_handshake(
        &self,
        id: usize,
        mut enc_inbound: NoiseStream<TcpStream>,
    ) -> Result<NoiseStream<TcpStream>> {
        if self.conns.contains_key(&id) {
            enc_inbound.write_u8(88).await?;
            Err(anyhow!("Service already online"))?
        }
        // verify hash of client
        let token = enc_inbound.get_state().get_remote_static().unwrap();
        let mut buf: [u8; FILEHASH_LEN] = [0; FILEHASH_LEN];
        let real_hash = &self.config.clients.get(token).unwrap().filehash;
        enc_inbound.read_exact(&mut buf).await?;
        if real_hash.as_ref().map_or(false, |f| f.hash == buf) {
            log::debug!("filehash verify passed, received: {:?}", &buf);
            enc_inbound.write_u8(66).await?;
        } else {
            log::debug!("filehash verify failed, received: {:?}", &buf);
            enc_inbound.write_u8(0).await?;
            Err(anyhow!("This client has an invalid hash"))?
        }
        Ok(enc_inbound)
    }
}
