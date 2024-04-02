/// functions for generating keypair and client binary
use std::fs::{self, OpenOptions};
use std::path::Path;

use anyhow::{anyhow, Result};
use chacha20poly1305::aead::{Aead, NewAead};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce}; // Or `XChaCha20Poly1305`
use memmap2::MmapOptions;
use object::{BinaryFormat, File, Object, ObjectSection};
use snowstorm::Keypair;

use crate::client::ClientConfig;
use crate::consts::{CONF_BUF_LEN, KEYPASS_LEN, PATTERN};

fn serialize_conf_to_buf(conf: &ClientConfig) -> Result<[u8; CONF_BUF_LEN], bincode::Error> {
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

pub fn gen_keypair(has_keypass: bool) -> Result<Keypair> {
    let mut keypair = snowstorm::Builder::new(PATTERN.parse()?).generate_keypair()?;
    if has_keypass {
        let mut password = rpassword::prompt_password("Input Key Passphrase: ")?.into_bytes();
        password.resize(KEYPASS_LEN, 0);
        let keypass = Key::from_slice(&password);
        let cipher = ChaCha20Poly1305::new(keypass);
        let enc_prikey = cipher.encrypt(&Nonce::default(), &keypair.private[..])?;
        keypair.private = enc_prikey;
    }
    Ok(keypair)
}

/// generate a new client binary using a callback function that modifies config
pub fn gen_client_binary<F>(in_path: &Path, out_path: &Path, mod_conf: F) -> Result<()>
where
    F: FnOnce(ClientConfig) -> ClientConfig,
{
    // 1. crate new binary
    let new_exe = in_path.with_extension("tmp");
    fs::copy(in_path, &new_exe)?;
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

/// copy existing client with a new keypair
pub fn modify_client_keypair<P: AsRef<Path>>(
    in_path: P,
    out_path: P,
    has_keypass: bool,
) -> Result<()> {
    let keypair = crate::gen::gen_keypair(has_keypass)?;
    let mod_conf = move |old_conf: ClientConfig| ClientConfig {
        client_prikey: keypair.private,
        has_keypass,
        ..old_conf
    };
    crate::gen::gen_client_binary(in_path.as_ref(), out_path.as_ref(), mod_conf)?;
    Ok(())
}

/// read config from a existing client
fn read_client_conf<P: AsRef<Path>>(path: P) -> Result<ClientConfig> {
    let file = OpenOptions::new().read(true).write(true).open(&path)?;
    let buf = unsafe { MmapOptions::new().map(&file) }?;
    let file = File::parse(&*buf)?;
    if let Some(range) = get_client_config_section(&file) {
        assert_eq!(range.1, CONF_BUF_LEN as u64);
        let base = range.0 as usize;
        let conf = ClientConfig::from_slice(&buf[base..(base + CONF_BUF_LEN)])?;
        Ok(conf)
    } else {
        Err(anyhow!("config not found"))
    }
}

/// clone a client from existing one (analogy to Dolly the sheep)
pub fn clone_client<P: AsRef<Path>>(dna_path: P, egg_path: P, out_path: P) -> Result<()> {
    let dna = crate::gen::read_client_conf(&dna_path)?;
    crate::gen::gen_client_binary(egg_path.as_ref(), out_path.as_ref(), |_| dna)?;
    Ok(())
}
