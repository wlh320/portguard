use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    net::{AddrParseError, SocketAddr},
};

// untagged for unit variant of Enum
// solution from <https://github.com/serde-rs/serde/issues/1560>
// TODO: any better solutions ???
macro_rules! named_unit_variant {
    ($variant:ident) => {
        pub mod $variant {
            pub fn serialize<S>(serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(stringify!($variant))
            }

            pub fn deserialize<'de, D>(deserializer: D) -> Result<(), D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                struct V;
                impl<'de> serde::de::Visitor<'de> for V {
                    type Value = ();
                    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                        f.write_str(concat!("\"", stringify!($variant), "\""))
                    }
                    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                        if value == stringify!($variant) {
                            Ok(())
                        } else {
                            Err(E::invalid_value(serde::de::Unexpected::Str(value), &self))
                        }
                    }
                }
                deserializer.deserialize_str(V)
            }
        }
    };
}
mod strings {
    named_unit_variant!(socks5);
}

/// Type for target address
#[derive(PartialEq, Eq, Debug, Copy, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Target {
    /// target address is a socket address
    Addr(SocketAddr),
    /// target address is builtin socks5
    #[serde(with = "strings::socks5")]
    Socks5,
}
impl ToString for Target {
    fn to_string(&self) -> String {
        match self {
            Target::Addr(a) => a.to_string(),
            Target::Socks5 => String::from("socks5"),
        }
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
    fn from_target_and_id(target: &str, id: usize) -> Result<Remote, Box<dyn Error>> {
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
                None => Err("No target address")?,
            },
            Some(target) => Ok(match id {
                None => Remote::from_target(target)?,
                Some(id) => Remote::from_target_and_id(target, id)?,
            }),
        }
    }
}

impl ToString for Remote {
    fn to_string(&self) -> String {
        match self {
            Remote::Proxy(t) => t.to_string(),
            Remote::Service(id) => format!("(sid {})", id),
            Remote::RProxy(t, _id) => t.to_string(),
        }
    }
}
