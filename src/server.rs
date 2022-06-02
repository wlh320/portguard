use crate::client::ClientConfig;
use crate::consts::{PATTERN, RPROXY_CHAN_LEN, FILEHASH_LEN};
use crate::gen;
use crate::proxy;
use crate::remote::{Remote, Target};

use blake2::{Blake2s256, Digest};
use fast_socks5::server::Socks5Socket;
use futures::lock::Mutex;
use futures::{FutureExt, StreamExt};
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
use tokio_util::compat::{FuturesAsyncReadCompatExt, TokioAsyncReadCompatExt};

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
enum RproxyConn {
    /// one who creates reverse proxy
    Client(usize, yamux::Control),
    /// one who visits reverse proxy
    Visitor(usize, Box<NoiseStream<TcpStream>>),
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
            filehash
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
        let (tx, rx) = mpsc::channel::<RproxyConn>(RPROXY_CHAN_LEN);
        let rproxy_handle = tokio::spawn(async move {
            if let Err(e) = Self::handle_reverse_proxy(rx).await {
                log::warn!("{}", e);
            }
        });
        // handle inbound connection
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

    async fn handle_connection(
        &self,
        inbound: TcpStream,
        tx: Sender<RproxyConn>,
    ) -> Result<(), Box<dyn Error>> {
        log::info!("New incoming peer_addr {:?}", inbound.peer_addr());
        // create noise stream & client auth
        let responder = snowstorm::Builder::new(PATTERN.parse()?)
            .local_private_key(&self.config.prikey)
            .build_responder()?;
        let mut enc_inbound = NoiseStream::handshake_with_verifier(inbound, responder, |key| {
            self.config.clients.contains(key)
        })
        .await?;
        // at this point, client already passed verification
        // can use `.unwrap()` here because client must have a static key
        let token = enc_inbound.get_state().get_remote_static().unwrap();
        let client_remote = self.config.clients.get(token).unwrap().remote;
        // if it specifies a remote address, use it
        let remote = client_remote.unwrap_or(self.config.remote);
        match remote {
            Remote::Proxy(target) => Self::proxy_to_target(enc_inbound, target).await?,
            Remote::Service(id) => {
                tx.send(RproxyConn::Visitor(id, Box::new(enc_inbound)))
                    .await?
            }
            Remote::RProxy(addr, id) => {
                // verify hash of client
                let mut buf: [u8; FILEHASH_LEN] = [0; FILEHASH_LEN];
                let real_hash = self.config.clients.get(token).unwrap().filehash.clone();
                enc_inbound.read_exact(&mut buf).await?;
                if real_hash.map_or(false, |f| f.hash == buf) {
                    log::debug!("filehash verify passed, received: {:?}", &buf);
                    enc_inbound.write_u8(66).await?;
                } else {
                    log::debug!("filehash verify failed, received: {:?}", &buf);
                    enc_inbound.write_u8(0).await?;
                    Err("This client has an invalid hash")?
                }
                Self::create_rproxy_conn(enc_inbound, id, addr, tx).await?
            }
        };
        Ok(())
    }
    /// handle proxy
    async fn proxy_to_target(
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
                let config = fast_socks5::server::Config::default();
                let socket = Socks5Socket::new(inbound, Arc::new(config));
                let transfer = socket.upgrade_to_socks5().map(|r| {
                    if let Err(e) = r {
                        log::warn!("Transfer error occured. error={}", e);
                    }
                });
                transfer.await;
            }
        }
        Ok(())
    }
    /// create a new rproxy connection
    async fn create_rproxy_conn(
        inbound: NoiseStream<TcpStream>,
        id: usize,
        target: Target,
        tx: Sender<RproxyConn>,
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
        tx.send(RproxyConn::Client(id, control)).await?;
        Ok(())
    }
    /// handle rproxy
    async fn handle_reverse_proxy(mut rx: Receiver<RproxyConn>) -> Result<(), Box<dyn Error>> {
        // TODO: need improvement
        type Control = Mutex<yamux::Control>;
        let mut conns: HashMap<usize, Control> = HashMap::new();
        while let Some(conn) = rx.recv().await {
            match conn {
                RproxyConn::Client(id, ctrl) => {
                    conns.insert(id, Mutex::new(ctrl));
                }
                RproxyConn::Visitor(id, inbound) => {
                    log::info!(
                        "Start proxying {:?} to rproxy service (id: {})",
                        inbound.get_inner().peer_addr(),
                        id
                    );
                    if !conns.contains_key(&id) {
                        log::warn!("Rproxy error occured. No such service (id: {})", id);
                        continue;
                    }
                    let ctrl = conns.get(&id).unwrap();
                    if let Ok(outbound) = ctrl.lock().await.open_stream().await {
                        tokio::spawn(async move {
                            proxy::transfer_and_log_error(inbound, outbound.compat()).await;
                        });
                    } else {
                        log::warn!("Rproxy error occured. Cannot connect service (id: {})", id);
                    }
                }
            }
        }
        Ok(())
    }
}
