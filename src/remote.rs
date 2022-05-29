use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, str::FromStr};

// untagged for unit variant of Enum
// solution from https://github.com/serde-rs/serde/issues/1560
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

/// Type for identifying target remote address
#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Remote {
    /// visitor of remote address, for `ssh -L`
    Addr(SocketAddr),
    /// visitor of builtin socks5 server, for `ssh -D`
    #[serde(with = "strings::socks5")]
    Socks5,
    /// visitor of reverse proxy, need service id, for `ssh -R` visitor
    Rvisitor(usize),
    /// client of reverse proxy, need addr and service id, for ssh -R` client
    Rclient(SocketAddr, usize),
}
impl FromStr for Remote {
    type Err = std::net::AddrParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // TODO: parse remote address
        if s.to_lowercase() == "socks5" {
            Ok(Remote::Socks5)
        } else {
            s.parse::<SocketAddr>().map(Remote::Addr)
        }
    }
}
impl ToString for Remote {
    fn to_string(&self) -> String {
        match self {
            Remote::Addr(a) => a.to_string(),
            Remote::Socks5 => String::from("socks5"),
            Remote::Rvisitor(id) => format!("reverse service {}", id),
            Remote::Rclient(remote, _id ) => remote.to_string(),
        }
    }
}

