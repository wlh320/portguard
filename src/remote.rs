use std::{
    error::Error,
    fmt,
    net::{AddrParseError, SocketAddr},
};

use serde::{Deserialize, Serialize};

/// Type for target address
/// for serialize https://github.com/serde-rs/serde/issues/1560#issuecomment-1666846833
#[derive(PartialEq, Eq, Debug, Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Target {
    /// target address is builtin socks5
    Socks5,
    /// target address is a socket address
    #[serde(untagged)]
    Addr(SocketAddr),
}

impl fmt::Display for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Target::Addr(a) => a.to_string(),
                Target::Socks5 => String::from("socks5"),
            }
        )
    }
}

/// Type for identifying remote
#[derive(PartialEq, Eq, Debug, Copy, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Remote {
    /// visitor of remote address, for `ssh -L` or
    /// visitor of builtin socks5 server, for `ssh -D`
    Proxy(Target),
    /// visitor of reverse proxy, need service id, for `ssh -R` visitor
    Service(usize),
    /// client of reverse proxy, need addr and service id, for ssh -R` client
    RProxy(Target, usize),
}

impl Remote {
    /// if input only target, client is proxy client
    fn from_target(target: &str) -> Result<Remote, AddrParseError> {
        if target.to_lowercase() == "socks5" {
            Ok(Remote::Proxy(Target::Socks5))
        } else {
            target
                .parse::<SocketAddr>()
                .map(Target::Addr)
                .map(Remote::Proxy)
        }
    }
    /// if input only id, client is service visitor
    fn from_id(id: usize) -> Remote {
        Remote::Service(id)
    }
    /// if input both target and id, client is reverse proxy client
    fn from_target_and_id(target: &str, id: usize) -> Result<Remote, AddrParseError> {
        if target.to_lowercase() == "socks5" {
            Ok(Remote::RProxy(Target::Socks5, id))
        } else {
            let addr = target.parse::<SocketAddr>()?;
            Ok(Remote::RProxy(Target::Addr(addr), id))
        }
    }
    /// parse optional input
    pub fn try_parse(target: Option<&str>, id: Option<usize>) -> Result<Remote, Box<dyn Error>> {
        match target {
            None => match id {
                Some(id) => Ok(Remote::from_id(id)),
                None => Err("Invalid remote address")?,
            },
            Some(target) => Ok(match id {
                None => Remote::from_target(target)?,
                Some(id) => Remote::from_target_and_id(target, id)?,
            }),
        }
    }
}

impl fmt::Display for Remote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Remote::Proxy(t) => t.to_string(),
                Remote::Service(id) => format!("service (id: {})", id),
                Remote::RProxy(t, _id) => t.to_string(),
            }
        )
    }
}
