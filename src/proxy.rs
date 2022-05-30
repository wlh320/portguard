use futures::FutureExt;
use std::error::Error;
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
