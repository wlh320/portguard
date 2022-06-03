use crate::client::ClientConfig;
use crate::consts::{FILEHASH_LEN, PATTERN, RPROXY_CHAN_LEN};
use crate::gen;
use crate::proxy;
use crate::remote::{Remote, Target};

use blake2::{Blake2s256, Digest};
use futures::lock::Mutex;
use futures::StreamExt;
use log;
use serde::{Deserialize, Serialize};
use snowstorm::NoiseStream;
use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::hash::{Hash, Hasher};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::RwLock;
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

type ConnMap = Arc<RwLock<HashMap<usize, Mutex<yamux::Control>>>>;

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
pub struct ServerConfig {
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
    fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn Error>> {
        let content = toml::ser::to_string(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

#[derive(Debug)]
enum RproxyEvent {
    /// one who creates reverse proxy
    Client(usize, yamux::Control),
    /// one who visits reverse proxy
    Visitor(usize, Box<NoiseStream<TcpStream>>),
    /// cancel a reverse proxy
    Cancel(usize),
}

/// Portguard server
pub struct Server {
    config_path: PathBuf,
    config: ServerConfig,
}

impl Server {
    pub fn new(config: ServerConfig, path: &Path) -> Server {
        Server {
            config,
            config_path: path.to_owned(),
        }
    }

    pub fn gen_client<P: AsRef<Path>>(
        &mut self,
        in_path: P,
        out_path: P,
        username: String,
        oremote: Option<Remote>,
    ) -> Result<(), Box<dyn Error>> {
        // 1. set client config
        let keypair = gen::gen_keypair()?;
        let remote = oremote.unwrap_or(self.config.remote);
        let reverse = matches!(remote, Remote::RProxy(_, _));
        let cli_conf: ClientConfig = ClientConfig {
            server_addr: format!("{}:{}", self.config.host, self.config.port).parse()?,
            target_addr: remote.to_string(),
            reverse,
            server_pubkey: self.config.pubkey.clone(),
            client_prikey: keypair.private,
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

    pub fn gen_key(&mut self) -> Result<(), Box<dyn Error>> {
        // gen key
        let keypair = gen::gen_keypair()?;
        self.config.pubkey = keypair.public;
        self.config.prikey = keypair.private;
        // save
        self.config.save(&self.config_path)?;
        Ok(())
    }

    pub async fn run_server_proxy(self) -> Result<(), Box<dyn Error>> {
        let this = Arc::new(self);
        let listen_addr: SocketAddr = format!("0.0.0.0:{}", this.config.port).parse().unwrap();
        log::info!("Listening on port: {:?}", listen_addr);

        // spwan to handle reverse proxy
        let (tx, rx) = mpsc::channel::<RproxyEvent>(RPROXY_CHAN_LEN);
        let rproxy_handle = tokio::spawn(async move {
            if let Err(e) = Self::handle_reverse_proxy(rx).await {
                log::warn!("{}", e);
            }
        });
        // TODO: spwan to handle config file hot-reloading

        // spwan to handle inbound connection
        let listener = TcpListener::bind(listen_addr).await?;
        while let Ok((inbound, _)) = listener.accept().await {
            let this = Arc::clone(&this);
            let tx = tx.clone();
            tokio::spawn(async move {
                if let Err(e) = this.handle_connection(inbound, tx).await {
                    log::warn!("{}", e);
                }
            });
        }
        rproxy_handle.await?;
        Ok(())
    }
    /// handle inbound connection
    async fn handle_connection(
        &self,
        inbound: TcpStream,
        tx: Sender<RproxyEvent>,
    ) -> Result<(), Box<dyn Error>> {
        log::info!("New incoming peer_addr {:?}", inbound.peer_addr());
        // create noise stream & client auth
        let responder = snowstorm::Builder::new(PATTERN.parse()?)
            .local_private_key(&self.config.prikey)
            .build_responder()?;
        let enc_inbound = NoiseStream::handshake_with_verifier(inbound, responder, |key| {
            self.config.clients.contains(key)
        })
        .await?;
        // at this point, client already passed verification
        // can use `.unwrap()` here because client must have a static key
        let token = enc_inbound.get_state().get_remote_static().unwrap();
        let client_remote = self.config.clients.get(token).unwrap().remote;
        let remote = client_remote.unwrap_or(self.config.remote);
        match remote {
            Remote::Proxy(target) => Self::start_proxy_to_target(enc_inbound, target).await?,
            Remote::Service(id) => {
                tx.send(RproxyEvent::Visitor(id, Box::new(enc_inbound)))
                    .await?
            }
            Remote::RProxy(addr, id) => {
                let enc_inbound = self.verify_file_hash(enc_inbound).await?;
                Self::start_new_rproxy_conn(enc_inbound, id, addr, tx).await?;
            }
        };
        Ok(())
    }
    /// handle reverse proxy
    async fn handle_reverse_proxy(mut rx: Receiver<RproxyEvent>) -> Result<(), Box<dyn Error>> {
        // TODO: need improvement
        let conns: ConnMap = Arc::new(RwLock::new(HashMap::new()));
        while let Some(event) = rx.recv().await {
            let conns = conns.clone();
            tokio::spawn(async move {
                if let Err(e) = Self::handle_rproxy_event(event, conns).await {
                    log::warn!("Rproxy Error occured: {}", e);
                }
            });
        }
        Ok(())
    }
    async fn handle_rproxy_event(
        event: RproxyEvent,
        conns: ConnMap,
    ) -> Result<(), Box<dyn Error>> {
        match event {
            RproxyEvent::Client(id, ctrl) => {
                conns.write().await.insert(id, Mutex::new(ctrl));
            }
            RproxyEvent::Cancel(id) => {
                if let Some(ctrl) = conns.write().await.remove(&id) {
                    ctrl.lock().await.close().await?;
                }
            }
            RproxyEvent::Visitor(id, inbound) => {
                let conns = conns.read().await;
                let ctrl = conns.get(&id).ok_or("No such service")?;
                Self::start_proxy_to_rproxy_conn(inbound, id, ctrl).await;
            }
        }
        Ok(())
    }
    /// following are helper functions:
    /// start
    async fn start_proxy_to_rproxy_conn(
        inbound: Box<NoiseStream<TcpStream>>,
        id: usize,
        ctrl: &Mutex<yamux::Control>,
    ) {
        let peer_addr = inbound.get_inner().peer_addr();
        log::info!("Start proxying {peer_addr:?} to rproxy service (id: {id})");
        if let Ok(outbound) = ctrl.lock().await.open_stream().await {
            proxy::transfer_and_log_error(inbound, outbound.compat()).await;
        } else {
            log::warn!("Cannot connect service (id: {id})");
        }
    }
    /// verify file hash of rproxy client
    async fn verify_file_hash(
        &self,
        mut enc_inbound: NoiseStream<TcpStream>,
    ) -> Result<NoiseStream<TcpStream>, Box<dyn Error>> {
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
            Err("This client has an invalid hash")?
        }
        Ok(enc_inbound)
    }
    /// start to handle proxy
    async fn start_proxy_to_target(
        inbound: NoiseStream<TcpStream>,
        target: Target,
    ) -> Result<(), Box<dyn Error>> {
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
    /// start a new rproxy connection
    async fn start_new_rproxy_conn(
        inbound: NoiseStream<TcpStream>,
        id: usize,
        target: Target,
        tx: Sender<RproxyEvent>,
    ) -> Result<(), Box<dyn Error>> {
        let peer_addr = inbound.get_inner().peer_addr()?;
        let target = target.to_string();
        log::info!("Start reverse proxy to {peer_addr}:{target} as a service (id {id})",);
        let yamux_config = yamux::Config::default();
        let yamux_conn =
            yamux::Connection::new(inbound.compat(), yamux_config, yamux::Mode::Client);
        let control = yamux_conn.control();
        // TODO: dont know why yamux client needs to do this.
        tokio::task::spawn(yamux::into_stream(yamux_conn).for_each(|_| async {}));
        tx.send(RproxyEvent::Client(id, control)).await?;
        Ok(())
    }
}
