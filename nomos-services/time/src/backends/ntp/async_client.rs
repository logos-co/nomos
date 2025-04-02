use std::{
    fmt::{Debug, Formatter},
    net::SocketAddr,
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
    pub local_socket: SocketAddr,
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
        let socket = &UdpSocket::bind(self.settings.local_socket)
            .await
            .map_err(Error::Io)?;
        let hosts = lookup_host(&pool).await.map_err(Error::Io)?;
        let mut checks = hosts
            .map(move |host| get_time(host, socket, self.ntp_context))
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
        net::{IpAddr, Ipv4Addr, SocketAddr},
        sync::Arc,
        time::Duration,
    };

    use sntpc::{sntp_process_response, RawNtpPacket, WrappedIoError};

    use super::*;

    fn get_version(li_vn_mode: u8) -> u8 {
        (li_vn_mode & 0b0011_1000) >> 3
    }

    #[tokio::test]
    async fn real_ntp_request() -> Result<(), Error> {
        // 0.europe.pool.ntp.org
        // Uses IP instead of domain to avoid random SNTP::ResponseAddressMismatch
        // errors.
        let ntp_server_ip = "40.119.6.228";
        // let ntp_server_ip = "185.251.115.30";
        let ntp_server_address = format!("{ntp_server_ip}:123");

        let settings = NTPClientSettings {
            timeout: Duration::from_secs(60),
            local_socket: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 12345),
        };
        let client = AsyncNTPClient::new(settings);

        let _response = client.request_timestamp(ntp_server_address).await?;

        Ok(())
    }

    async fn deconstructed_ntp_request(domain: &str) -> Result<(&str, u8, u8), ()> {
        println!("\n[{}]", domain);
        let ntp_server_address = format!("{domain}:123");
        let destination = ntp_server_address.parse::<SocketAddr>().map_err(|_| ())?;

        let ntp_context = NtpContext::new(StdTimestampGen::default());
        let packet = sntpc::NtpPacket::new(ntp_context.timestamp_gen);
        let raw_packet = sntpc::RawNtpPacket::from(&packet);

        let local_socket_address = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 12346);
        let local_socket = UdpSocket::bind(local_socket_address)
            .await
            .expect("Failed to bind local socket");

        let raw_packet_inner: &[u8; size_of::<sntpc::RawNtpPacket>()] = unsafe {
            std::mem::transmute::<&sntpc::RawNtpPacket, &[u8; size_of::<sntpc::RawNtpPacket>()]>(
                &raw_packet,
            )
        };

        let res = match local_socket.send_to(raw_packet_inner, destination).await {
            Ok(size) => {
                if size == raw_packet_inner.len() {
                    println!("Request size matched. Sent: {}", size);
                    Ok(())
                } else {
                    println!(
                        "Request size mismatch. Sent: {}, expected: {}",
                        size,
                        raw_packet_inner.len()
                    );
                    Err(sntpc::Error::ResponseSizeMismatch(
                        raw_packet_inner.len(),
                        size,
                    ))
                }
            }
            Err(e) => {
                println!("Failed to send request: {:?}", &e);
                Err(sntpc::Error::Network(e.into()))
            }
        };

        // let send_request_result = sntpc::SendRequestResult::from(packet);
        // let x = sntp_process_response(destination, &local_socket, ntp_context,
        // send_request_result)     .await;
        //
        // dbg!("sntp_process_response: {:?}", x);

        let mut response_buf = RawNtpPacket::default();

        let (response, src) = timeout(
            Duration::from_secs(10),
            local_socket.recv_from(response_buf.0.as_mut()),
        )
        .await
        .map_err(|_| ())?
        .map_err(|_| {})?;

        // let (response, src) = local_socket
        //     .recv_from(response_buf.0.as_mut())
        //     .await
        //     .unwrap();
        let response = sntpc::NtpPacket::from(response_buf);

        Ok((
            domain,
            get_version(packet.li_vn_mode),
            get_version(response.li_vn_mode),
        ))
    }

    #[tokio::test]
    async fn deconstructed() {
        let ntp_server_ip = "40.119.6.228"; // time.windows.com
                                            // let ntp_server_ip = "185.251.115.30"; // 0.europe.pool.ntp.org
        let domains = [
            "0.europe.pool.ntp.org",
            "185.251.115.30",
            "time.windows.com",
            "40.119.6.228",
            "51.137.137.111",
            "time.apple.com",
            "17.253.28.253",
            "time.cloudflare.com",
            "162.159.200.123",
            "ntppool1.time.nl",
            "94.198.159.15",
            "nts.netnod.se",
            "ptbtime1.ptb.de",
            "192.53.103.108",
        ];

        for domain in domains {
            if let Ok((domain, sent_version, received_version)) =
                deconstructed_ntp_request(domain).await
            {
                println!("-> Domain: {domain}, sent version: {sent_version}, received version: {received_version}");
            } else {
                println!("-> Failed to get response for domain: {domain}");
            }
        }
    }
}
