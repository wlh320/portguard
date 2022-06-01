/// functions for generating keypair and client binary
use crate::client::ClientConfig;
use crate::consts::{CONF_BUF_LEN, PATTERN};

use memmap2::MmapOptions;
use object::{BinaryFormat, File, Object, ObjectSection};
use snowstorm::Keypair;
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::path::Path;

fn serialize_conf_to_buf(conf: &ClientConfig) -> Result<[u8; CONF_BUF_LEN], Box<dyn Error>> {
    let v = conf.to_vec()?;
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

pub fn gen_keypair() -> Result<Keypair, snowstorm::snow::Error> {
    let key = snowstorm::Builder::new(PATTERN.parse()?).generate_keypair()?;
    Ok(key)
}

pub fn gen_client_binary<P: AsRef<Path>, F>(
    in_path: P,
    out_path: P,
    mod_conf: F,
) -> Result<(), Box<dyn Error>>
where
    F: FnOnce(ClientConfig) -> ClientConfig,
{
    // 1. crate new binary
    let new_exe = in_path.as_ref().with_extension("tmp");
    fs::copy(&in_path, &new_exe)?;
    let file = OpenOptions::new().read(true).write(true).open(&new_exe)?;
    let mut buf = unsafe { MmapOptions::new().map_mut(&file) }?;
    let file = File::parse(&*buf)?;

    // 2. save config to new binary
    if let Some(range) = get_client_config_section(&file) {
        log::debug!("Copying config to client");
        assert_eq!(range.1, CONF_BUF_LEN as u64);
        let base = range.0 as usize;

        let old_conf = ClientConfig::from_slice(&buf[base..(base + CONF_BUF_LEN)])?;
        let new_conf = mod_conf(old_conf);

        let conf_buf = serialize_conf_to_buf(&new_conf)?;
        buf[base..(base + CONF_BUF_LEN)].copy_from_slice(&conf_buf);

        let perms = fs::metadata(in_path)?.permissions();
        fs::set_permissions(&new_exe, perms)?;
        fs::rename(&new_exe, out_path)?;
    } else {
        fs::remove_file(&new_exe)?;
    }
    Ok(())
}