use std::{error::Error, sync::Arc};

use fast_socks5::server::Socks5Socket;
use futures::FutureExt;
use tokio::io::{self, AsyncRead, AsyncWrite, AsyncWriteExt};

pub(crate) async fn transfer<S1, S2>(inbound: S1, outbound: S2) -> Result<(), Box<dyn Error>>
where
    S1: AsyncRead + AsyncWrite + Unpin,
    S2: AsyncRead + AsyncWrite + Unpin,
{
    let (mut ri, mut wi) = io::split(inbound);
    let (mut ro, mut wo) = io::split(outbound);

    let client_to_server = async {
        io::copy(&mut ri, &mut wo).await?;
        wo.shutdown().await
    };
    let server_to_client = async {
        io::copy(&mut ro, &mut wi).await?;
        wi.shutdown().await
    };

    tokio::try_join!(client_to_server, server_to_client)?;

    Ok(())
}

pub(crate) async fn transfer_and_log_error<S1, S2>(inbound: S1, outbound: S2)
where
    S1: AsyncRead + AsyncWrite + Unpin,
    S2: AsyncRead + AsyncWrite + Unpin,
{
    let transfer = crate::proxy::transfer(inbound, outbound).map(|r| {
        if let Err(e) = r {
            log::warn!("Transfer error occured. error={}", e);
        }
    });
    transfer.await;
}

pub(crate) async fn transfer_to_socks5_and_log_error<S>(inbound: S)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let config = fast_socks5::server::Config::default();
    let socket = Socks5Socket::new(inbound, Arc::new(config));
    let transfer = socket.upgrade_to_socks5().map(|r| {
        if let Err(e) = r {
            log::warn!("Transfer error occured. error={}", e);
        }
    });
    transfer.await;
}