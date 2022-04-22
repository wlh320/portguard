use crate::client::{ClientConfig, CLIENT_CONF_BUF};
use crate::consts::{CONF_BUF_LEN, PATTERN};
use crate::proxy::transfer;

use futures::FutureExt;
use log;
use memmap2::MmapOptions;
use object::{File, Object, ObjectSection};
use serde::{Deserialize, Serialize};
use snowstorm::NoiseStream;
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};

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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    /// server public ip or domain
    #[serde(default = "default_host")]
    host: String,
    /// server listen port
    #[serde(default = "default_port")]
    pub port: u16,
    /// remote address hope to proxy
    #[serde(default = "default_remote")]
    remote: String,
    /// server public key
    #[serde(with = "base64_serde", default)]
    pubkey: Vec<u8>,
    /// server private key
    #[serde(with = "base64_serde", default)]
    prikey: Vec<u8>,
    #[serde(default)]
    /// infomation of clients
    clients: HashMap<String, ClientEntry>,
}

fn default_port() -> u16 {
    6000
}

fn default_host() -> String {
    "192.168.1.1".to_string()
}

fn default_remote() -> String {
    "127.0.0.1:8080".to_string()
}

impl ServerConfig {
    fn save<P: AsRef<Path>>(&self, path: P) -> Result<(), Box<dyn Error>> {
        let content = toml::ser::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

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
    // fn from_config()
    pub fn gen_client<P: AsRef<Path>>(&mut self, exe_path: P, username: String) -> Result<(), Box<dyn Error>> {
        let mut cli_conf: ClientConfig = bincode::deserialize(&CLIENT_CONF_BUF)?;
        log::debug!("Previous static variable: {:?}", cli_conf);
        // 1. set client config
        let key = snowstorm::Builder::new(PATTERN.parse()?).generate_keypair()?;
        cli_conf.client_prikey = key.private;
        log::debug!("Client private key: {:?}", base64::encode(&cli_conf.client_prikey));
        cli_conf.server_pubkey = self.config.pubkey.clone();
        cli_conf.server_addr = format!("{}:{}", self.config.host, self.config.port).parse()?;
        cli_conf.target_addr = self.config.remote.parse()?;
        log::debug!("New client static variable: {:?}", cli_conf);
        // 2. crate new binary
        let exe = env::current_exe()?;
        let new_exe = exe.with_extension("tmp");
        fs::copy(&exe, &new_exe)?;
        let file = OpenOptions::new().read(true).write(true).open(&new_exe)?;
        let mut buf = unsafe { MmapOptions::new().map_mut(&file) }?;
        let file = File::parse(&*buf)?;
        // 3. save config to new binary
        if let Some(range) = get_section(&file, "modify") {
            assert_eq!(range.1, CONF_BUF_LEN as u64);

            let conf_buf = serialize_conf_to_buf(&cli_conf)?;
            let base = range.0 as usize;
            buf[base..(base + CONF_BUF_LEN)].copy_from_slice(&conf_buf);

            let perms = fs::metadata(&exe)?.permissions();
            fs::set_permissions(&new_exe, perms)?;
            fs::rename(&new_exe, exe_path)?;
        } else {
            fs::remove_file(&new_exe)?;
        }
        // 4. add new client to server config
        let client = ClientEntry { name: username, pubkey: key.public };
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
        let out_addr: SocketAddr = this.config.remote.parse().unwrap();
        log::info!("Listening on port: {:?}", listen_addr);

        let listener = TcpListener::bind(listen_addr).await?;

        while let Ok((inbound, _)) = listener.accept().await {
            let this = Arc::clone(&this);
            tokio::spawn(async move {
                if let Err(e) = this.handle_connection(inbound, out_addr).await {
                    log::warn!("{}", e);
                }
            });
        }
        Ok(())
    }

    async fn handle_connection(
        &self,
        inbound: TcpStream,
        out_addr: SocketAddr,
    ) -> Result<(), Box<dyn Error>> {
        log::info!("New incoming peer_addr {:?}", inbound.peer_addr());
        let responder = snowstorm::Builder::new(PATTERN.parse()?)
            .local_private_key(&self.config.prikey)
            .build_responder()?;
        let enc_inbound = NoiseStream::handshake_with_verifier(inbound, responder, |key| {
            let token = base64::encode(key);
            self.config.clients.contains_key(&token)
        })
        .await?;
        let outbound = TcpStream::connect(out_addr).await?;
        let transfer = transfer(enc_inbound, outbound).map(|r| {
            if let Err(e) = r {
                log::error!("Transfer error occured. error={}", e);
            }
        });
        transfer.await;
        Ok(())
    }
}

fn serialize_conf_to_buf(conf: &ClientConfig) -> Result<[u8; CONF_BUF_LEN], Box<dyn Error>> {
    let v = &bincode::serialize(&conf)?;
    let mut bytes: [u8; CONF_BUF_LEN] = [0; CONF_BUF_LEN];
    // for i in 0..v.len() {
        // bytes[i] = v[i];
    // }
    bytes[..v.len()].clone_from_slice(&v[..]);
    Ok(bytes)
}

fn get_section(file: &File, name: &str) -> Option<(u64, u64)> {
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
