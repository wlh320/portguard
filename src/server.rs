use crate::client::{ClientConfig};
use crate::consts::{CONF_BUF_LEN, PATTERN};
use crate::proxy;
use crate::remote::Remote;

use bincode::Options;
use fast_socks5::server::Socks5Socket;
use futures::lock::Mutex;
use futures::FutureExt;
use log;
use memmap2::MmapOptions;
use object::{BinaryFormat, File, Object, ObjectSection};
use serde::{Deserialize, Serialize};
use snowstorm::NoiseStream;
use std::collections::HashMap;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ClientEntry {
    /// user name
    name: String,
    /// client public key for auth
    #[serde(with = "base64_serde")]
    pubkey: Vec<u8>,
    /// client specified remote address
    remote: Option<Remote>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    /// server public ip or domain
    #[serde(default = "default_host")]
    host: String,
    /// server listen port
    #[serde(default = "default_port")]
    pub port: u16,
    /// default remote address hope to proxy
    #[serde(default = "default_remote")]
    remote: Remote,
    /// server public key
    #[serde(with = "base64_serde", default)]
    pubkey: Vec<u8>,
    /// server private key
    #[serde(with = "base64_serde", default)]
    prikey: Vec<u8>,
    #[serde(serialize_with = "toml::ser::tables_last", default)]
    /// infomation of clients
    clients: HashMap<String, ClientEntry>,
}

fn default_port() -> u16 {
    8022
}

fn default_host() -> String {
    "192.168.1.1".to_string()
}

fn default_remote() -> Remote {
    Remote::Socks5
}

impl ServerConfig {
    fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn Error>> {
        let content = toml::ser::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

#[derive(Debug)]
enum RproxyConn {
    Client(usize, yamux::Control),
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
        remote: Option<Remote>,
    ) -> Result<(), Box<dyn Error>> {
        // 1. set client config
        let key = snowstorm::Builder::new(PATTERN.parse()?).generate_keypair()?;
        let cli_conf: ClientConfig = ClientConfig { 
            server_addr: format!("{}:{}", self.config.host, self.config.port).parse()?, 
            target_addr: remote.unwrap_or(self.config.remote).to_string(), 
            server_pubkey: self.config.pubkey.clone(), 
            client_prikey: key.private
        };

        // 2. crate new binary
        let new_exe = in_path.as_ref().with_extension("tmp");
        fs::copy(&in_path, &new_exe)?;
        let file = OpenOptions::new().read(true).write(true).open(&new_exe)?;
        let mut buf = unsafe { MmapOptions::new().map_mut(&file) }?;
        let file = File::parse(&*buf)?;

        // 3. save config to new binary
        if let Some(range) = get_client_config_section(&file) {
            log::debug!("Copying config to client");
            assert_eq!(range.1, CONF_BUF_LEN as u64);

            let conf_buf = serialize_conf_to_buf(&cli_conf)?;
            let base = range.0 as usize;
            buf[base..(base + CONF_BUF_LEN)].copy_from_slice(&conf_buf);

            let perms = fs::metadata(in_path)?.permissions();
            fs::set_permissions(&new_exe, perms)?;
            fs::rename(&new_exe, out_path)?;
        } else {
            fs::remove_file(&new_exe)?;
        }

        // 4. add new client to server config
        let client = ClientEntry {
            name: username,
            pubkey: key.public,
            remote,
        };
        let ent = self.config.clients.entry(base64::encode(&client.pubkey));
        ent.or_insert(client);
        // 5. save server config
        self.config.save(&self.config_path)?;
        Ok(())
    }

    pub fn gen_key(&mut self) -> Result<(), Box<dyn Error>> {
        // gen key
        let key = snowstorm::Builder::new(PATTERN.parse()?).generate_keypair()?;
        self.config.pubkey = key.public;
        self.config.prikey = key.private;
        // save
        self.config.save(&self.config_path)?;
        Ok(())
    }

    pub async fn run_server_proxy(self) -> Result<(), Box<dyn Error>> {
        let this = Arc::new(self);
        let listen_addr: SocketAddr = format!("0.0.0.0:{}", this.config.port).parse().unwrap();
        log::info!("Listening on port: {:?}", listen_addr);

        // handle reverse proxy
        let (tx, rx) = mpsc::channel::<RproxyConn>(100);
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
        let enc_inbound = NoiseStream::handshake_with_verifier(inbound, responder, |key| {
            let token = base64::encode(key);
            self.config.clients.contains_key(&token)
        })
        .await?;
        let token = base64::encode(enc_inbound.get_state().get_remote_static().unwrap());
        // at this point, client already passed verification
        // can use `.unwrap()` here because client must have a static key
        let client_remote = self.config.clients.get(&token).unwrap().remote;
        // if it specifies a remote address, use it
        let remote = client_remote.unwrap_or(self.config.remote);

        match remote {
            Remote::Addr(out_addr) => Self::proxy_to_remote(enc_inbound, out_addr).await?,
            Remote::Socks5 => Self::proxy_to_socks5(enc_inbound).await?,
            Remote::Rvisitor(id) => {
                tx.send(RproxyConn::Visitor(id, Box::new(enc_inbound)))
                    .await?
            }
            Remote::Rclient(addr, id) => {
                Self::create_rproxy_conn(enc_inbound, id, addr, tx).await?
            }
        };
        Ok(())
    }

    async fn handle_reverse_proxy(mut rx: Receiver<RproxyConn>) -> Result<(), Box<dyn Error>> {
        type Control = Arc<Mutex<yamux::Control>>;
        let mut conns: HashMap<usize, Control> = HashMap::new();
        while let Some(conn) = rx.recv().await {
            match conn {
                RproxyConn::Client(id, ctrl) => {
                    conns.insert(id, Arc::new(Mutex::new(ctrl)));
                }
                RproxyConn::Visitor(id, inbound) => {
                    log::info!(
                        "Start proxying {:?} to rproxy service (id: {})",
                        inbound.get_inner().peer_addr(),
                        id
                    );
                    if let Some(ctrl) = conns.get(&id) {
                        if let Ok(outbound) = ctrl.lock().await.open_stream().await {
                            tokio::spawn(async move {
                                let transfer =
                                    proxy::transfer(inbound, outbound.compat()).map(|r| {
                                        if let Err(e) = r {
                                            log::warn!("Transfer error occured. error={}", e);
                                        }
                                    });
                                transfer.await;
                            });
                        } else {
                            log::warn!("Rproxy error occured. Cannot connect service (id: {})", id);
                        }
                    } else {
                        log::warn!("Rproxy error occured. No such service (id: {})", id);
                    }
                }
            }
        }
        Ok(())
    }

    // server proxy function
    async fn proxy_to_remote(
        inbound: NoiseStream<TcpStream>,
        out_addr: SocketAddr,
    ) -> Result<(), Box<dyn Error>> {
        log::info!(
            "Start proxying {:?} to {:?}",
            inbound.get_inner().peer_addr(),
            out_addr
        );
        let outbound = TcpStream::connect(out_addr).await?;
        let transfer = proxy::transfer(inbound, outbound).map(|r| {
            if let Err(e) = r {
                log::warn!("Transfer error occured. error={}", e);
            }
        });
        transfer.await;
        Ok(())
    }
    async fn proxy_to_socks5(inbound: NoiseStream<TcpStream>) -> Result<(), Box<dyn Error>> {
        log::info!(
            "Start proxying {:?} to built-in socks5 server",
            inbound.get_inner().peer_addr(),
        );
        let socks5_config = fast_socks5::server::Config::default();
        let config = Arc::new(socks5_config);
        let socket = Socks5Socket::new(inbound, config);
        let transfer = socket.upgrade_to_socks5().map(|r| {
            if let Err(e) = r {
                log::warn!("Transfer error occured. error={}", e);
            }
        });
        transfer.await;
        Ok(())
    }
    async fn create_rproxy_conn(
        inbound: NoiseStream<TcpStream>,
        id: usize,
        expose_addr: SocketAddr,
        tx: Sender<RproxyConn>,
    ) -> Result<(), Box<dyn Error>> {
        // TODO: incoming reverse client
        log::info!(
            "Start reverse proxy to {:?}:{:?} as a service (id {:?})",
            inbound.get_inner().peer_addr(),
            expose_addr,
            id
        );
        let yamux_config = yamux::Config::default();
        let mut yamux_conn =
            yamux::Connection::new(inbound.compat(), yamux_config, yamux::Mode::Client);
        let control = yamux_conn.control();
        tokio::spawn(async move {
            loop {
                match yamux_conn.next_stream().await {
                    Ok(Some(_)) => (),
                    Err(e) => {
                        log::info!("{}", e);
                        break;
                    }
                    Ok(None) => {
                        log::info!("closed");
                        break;
                    }
                }
            }
        });
        tx.send(RproxyConn::Client(id, control)).await?;
        Ok(())
    }
}

fn serialize_conf_to_buf(conf: &ClientConfig) -> Result<[u8; CONF_BUF_LEN], Box<dyn Error>> {
    let v = bincode::options()
        .with_limit(CONF_BUF_LEN as u64)
        .allow_trailing_bytes()
        .serialize(&conf)?;
    // let v = &bincode::serialize(&conf)?;
    let mut bytes: [u8; CONF_BUF_LEN] = [0; CONF_BUF_LEN];
    bytes[..v.len()].clone_from_slice(&v[..]);
    Ok(bytes)
}

fn get_client_config_section(file: &File) -> Option<(u64, u64)> {
    let name = match file.format() {
        BinaryFormat::Elf => ".portguard",
        BinaryFormat::Pe => "pgmodify",
        BinaryFormat::MachO => "__portguard",
        _ => todo!(),
    };
    for section in file.sections() {
        match section.name() {
            Ok(n) if n == name => {
                return section.file_range();
            }
            _ => {}
        }
    }
    None
}

// #[cfg(test)]
// mod tests {
//     #[test]
//     fn test_serde_remote() {
//         use super::super::server::ClientEntry;
//         use super::Remote;

//         // let r_rc = Remote::Rclient("127.0.0.1:1080".parse().unwrap(), 123);
//         let r_r = Remote::Addr("127.0.0.1:1080".parse().unwrap());
//         // let r_s = Remote::Socks5;
//         let r_rv = Remote::Rvisitor(666);
//         let ct = ClientEntry {
//             name: String::from("aaa"),
//             pubkey: Vec::new(),
//             remote: Some(r_rv),
//         };
//         let s = toml::to_string(&r_r).unwrap();
//         assert_eq!(s, String::from("1"));
//     }
// }
