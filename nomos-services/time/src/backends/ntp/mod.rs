mod async_client;

use std::{
    num::NonZero,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use cryptarchia_engine::{time::SlotConfig, EpochConfig, Slot};
use futures::{Stream, StreamExt};
#[cfg(feature = "serde")]
use nomos_utils::bounded_duration::{MinimalBoundedDuration, NANO};
use sntpc::{fraction_to_nanoseconds, NtpResult};
use time::OffsetDateTime;
use tokio::time::{interval, MissedTickBehavior};
use tokio_stream::wrappers::IntervalStream;

use crate::{
    backends::{
        common::slot_timer,
        ntp::async_client::{AsyncNTPClient, NTPClientSettings},
        TimeBackend,
    },
    EpochSlotTickStream, SlotTick,
};

#[cfg_attr(feature = "serde", cfg_eval::cfg_eval, serde_with::serde_as)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct NtpSettings {
    /// Ntp server address
    pub ntp_server: String,
    /// Ntp server settings
    pub ntpclient_settings: NTPClientSettings,
    /// Interval for the backend to contact the ntp server and update its time
    #[cfg_attr(feature = "serde", serde_as(as = "MinimalBoundedDuration<1, NANO>"))]
    pub update_interval: Duration,
    /// Slot settings in order to compute proper slot times
    pub slot_config: SlotConfig,
    /// Epoch settings in order to compute proper epoch times
    pub epoch_config: EpochConfig,
    /// Base period length related to epochs, used to compute epochs as well
    pub base_period_length: NonZero<u64>,
}
pub struct Ntp {
    settings: NtpSettings,
    client: AsyncNTPClient,
}

impl TimeBackend for Ntp {
    type Settings = NtpSettings;

    fn init(settings: Self::Settings) -> Self {
        let client = AsyncNTPClient::new(settings.ntpclient_settings);
        Self { settings, client }
    }

    fn tick_stream(self) -> EpochSlotTickStream {
        let Self { settings, client } = self;
        let mut update_interval = interval(settings.update_interval);
        // if we miss a tick just try next one
        update_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        // contact the ntp server for first time sync right now
        let ntp_server = settings.ntp_server.clone();
        let interval: NtpResultStream = Pin::new(Box::new(
            IntervalStream::new(update_interval)
                .zip(futures::stream::repeat((client, ntp_server)))
                .filter_map(move |(_, (client, ntp_server))| {
                    Box::pin(async move { client.request_timestamp(ntp_server).await.ok() })
                }),
        ));
        // compute the initial slot ticking stream
        let local_date = OffsetDateTime::now_utc();
        let slot_timer = slot_timer(
            settings.slot_config,
            local_date,
            Slot::from_offset_and_config(local_date, settings.slot_config),
            settings.epoch_config,
            settings.base_period_length,
        );
        Pin::new(Box::new(NtpStream {
            interval,
            slot_config: settings.slot_config,
            epoch_config: settings.epoch_config,
            base_period_length: settings.base_period_length,
            slot_timer,
        }))
    }
}

type NtpResultStream = Pin<Box<dyn Stream<Item = NtpResult> + Send + Sync + Unpin>>;

/// Stream that updates itself every `interval` from an NTP server.
pub struct NtpStream {
    /// Update interval stream
    interval: NtpResultStream,
    /// Slot settings in order to compute proper slot times
    slot_config: SlotConfig,
    /// Epoch settings in order to compute proper epoch times
    epoch_config: EpochConfig,
    /// Base period length related to epochs, used to compute epochs as well
    base_period_length: NonZero<u64>,
    /// `SlotTick` interval stream. This stream is replaced when an internal
    /// clock update happens.
    slot_timer: EpochSlotTickStream,
}
impl Stream for NtpStream {
    type Item = SlotTick;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // try update time
        if let Poll::Ready(Some(timestamp)) = self.interval.poll_next_unpin(cx) {
            let seconds = Duration::from_secs(timestamp.sec().into());
            let nanos_fraction =
                Duration::from_nanos(fraction_to_nanoseconds(timestamp.sec_fraction()).into());
            let roundtrip = Duration::from_micros(timestamp.roundtrip());

            let date = OffsetDateTime::from_unix_timestamp_nanos(
                (seconds + nanos_fraction + roundtrip).as_nanos() as i128,
            )
            .expect("Datetime synchronization failed");
            let current_slot = Slot::from_offset_and_config(date, self.slot_config);
            let epoch_config = self.epoch_config;
            let base_period_length = self.base_period_length;
            self.slot_timer = slot_timer(
                self.slot_config,
                date,
                current_slot,
                epoch_config,
                base_period_length,
            );
        }
        // poll from internal last updated `SlotTick` stream
        self.slot_timer.poll_next_unpin(cx)
    }
}
