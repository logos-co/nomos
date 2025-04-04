use std::{
    fmt::{Debug, Formatter},
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use futures::{stream::FuturesUnordered, StreamExt, TryStreamExt};
#[cfg(feature = "serde")]
use nomos_utils::bounded_duration::{MinimalBoundedDuration, NANO};
use sntpc::{get_time, Error as SntpError, NtpContext, NtpResult, StdTimestampGen};
use tokio::{
    net::{lookup_host, ToSocketAddrs, UdpSocket},
    time::{error::Elapsed, timeout},
};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Io(std::io::Error),
    #[error("NTP internal error: {0:?}")]
    Sntp(SntpError),
    #[error("NTP request timeout, elapsed: {0:?}")]
    Timeout(Elapsed),
}

#[cfg_attr(feature = "serde", cfg_eval::cfg_eval, serde_with::serde_as)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Copy, Clone)]
pub struct NTPClientSettings {
    /// NTP server requests timeout duration
    #[cfg_attr(feature = "serde", serde_as(as = "MinimalBoundedDuration<1, NANO>"))]
    pub timeout: Duration,
    /// Local socket address to listen for responses
    /// The bound IP behaves like a server listening IP. This means for WAN
    /// requests you probably want to bind to `0.0.0.0`. For LAN requests
    /// you can simply bind to the local IP.
    pub interface: IpAddr,
}

#[derive(Clone)]
pub struct AsyncNTPClient {
    settings: NTPClientSettings,
    ntp_context: NtpContext<StdTimestampGen>,
}

impl Debug for AsyncNTPClient {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncNTPClient")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl AsyncNTPClient {
    #[must_use]
    pub fn new(settings: NTPClientSettings) -> Self {
        let ntp_context = NtpContext::new(StdTimestampGen::default());
        Self {
            settings,
            ntp_context,
        }
    }

    async fn get_time(&self, host: SocketAddr) -> sntpc::Result<NtpResult> {
        let socket = UdpSocket::bind(SocketAddr::new(self.settings.interface, 0))
            .await
            .map_err(|_| sntpc::Error::Network)?; // same error that get_time returns for io
        get_time(host, &socket, self.ntp_context).await
    }

    /// Request a timestamp from an NTP server
    ///
    /// # Errors
    ///
    /// Making a request to a domain that resolves to multiple IPs (randomly)
    /// will result in a [`Error::Sntp`] containing a
    /// [`ResponseAddressMismatch`](sntpc::Error::ResponseAddressMismatch)
    /// error.
    pub async fn request_timestamp<Addresses: ToSocketAddrs + Sync>(
        &self,
        pool: Addresses,
    ) -> Result<NtpResult, Error> {
        let hosts = lookup_host(&pool).await.map_err(Error::Io)?;
        let mut checks = hosts
            .map(|host| self.get_time(host))
            .collect::<FuturesUnordered<_>>()
            .into_stream();
        timeout(self.settings.timeout, checks.select_next_some())
            .await
            .map_err(Error::Timeout)?
            .map_err(Error::Sntp)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::{IpAddr, Ipv4Addr},
        time::Duration,
    };

    use super::*;

    // This test is disabled macOS because NTP v4 requests fail on Github's macOS
    // runners. Everywhere else it works fine, including other macOS machines.
    // The request seems to be sent successfully, but the test timeouts when
    // receiving the response. Responses only come through when querying
    // `time.windows.com`, which runs NTP v3. The library we're using,
    // [`sntpc`], requires NTP v4.
    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn real_ntp_request() -> Result<(), Error> {
        // 0.europe.pool.ntp.org
        // Uses IP instead of domain to avoid random SNTP::ResponseAddressMismatch
        // errors.
        let ntp_server_ip = "pool.ntp.org";
        let ntp_server_address = format!("{ntp_server_ip}:123");

        let settings = NTPClientSettings {
            timeout: Duration::from_secs(3),
            interface: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
        };
        let client = AsyncNTPClient::new(settings);

        let _response = client.request_timestamp(ntp_server_address).await?;

        Ok(())
    }
}
