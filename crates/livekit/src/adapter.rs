use std::{error::Error, fmt, future::Future, pin::Pin};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TimestampAuthority {
    /// The adapter sends the caller-supplied 90 kHz media timestamp.
    #[default]
    Media,
    /// The adapter derives timestamps internally and cannot support latency qualification.
    AdapterClock,
}

/// One access unit offered to a publication adapter, described well enough
/// that an adapter publishing it as encoded media does not have to re-derive
/// anything the ingestion path already knows.
#[derive(Clone, Copy, Debug)]
pub struct EncodedAccessUnit<'a> {
    pub annex_b: &'a [u8],
    pub rtp_timestamp: u32,
    pub epoch: u64,
    /// Whether a decoder can begin here: an IDR carrying its parameter sets.
    ///
    /// This is the classification the media plane already made, not a fresh
    /// guess from the bytes. An adapter that must label frames key or delta
    /// takes the answer from here rather than re-parsing.
    pub starts_decodable_run: bool,
    /// Elapsed media time since publication began, in microseconds.
    ///
    /// Derived from the camera's media timestamps, so it advances with the
    /// stream rather than with the publisher's own clock. Adapters that need a
    /// capture timestamp use this; nothing recomputes it from wall time.
    pub capture_timestamp_us: i64,
}

/// What an adapter did with one offered access unit.
///
/// Offering is not publishing. An adapter can accept an access unit and still
/// not put it on the wire — it may have no track yet, or the transport may not
/// be ready — and that is a different fact from a failed session. Reporting it
/// here is what lets the difference show up as a count rather than as media
/// that silently never arrives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendOutcome {
    /// Handed to the transport.
    Published,
    /// Not published, and not a session failure. The next access unit may well
    /// be published; tearing the session down here would turn a momentary
    /// refusal into a reconnect loop.
    Dropped,
}

pub type AdapterFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AdapterError>> + Send + 'a>>;

/// Facts an adapter knows that the owner cannot derive from what it hands over.
///
/// The owner counts what it offers and what came back. An adapter that gates on
/// subscribers or transforms media on the way out holds state the owner never
/// sees, and without this it would only be visible as an unexplained gap
/// between `access_units` and `adapter_drops`.
///
/// Deliberately three narrow numbers rather than an adapter-specific type: a
/// pass-through adapter reports zeros, and the owner stays generic.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AdapterObservation {
    /// Subscribers the adapter believes are present, for adapters that gate on
    /// it. Zero from an adapter that publishes regardless.
    pub viewers: u64,
    /// Media items the adapter produced from what it was offered — decoded and
    /// re-encoded pictures, for a transcoding adapter. Not the same as the
    /// access units offered, and the ratio is the thing worth watching.
    pub media_produced: u64,
    /// Times the adapter restarted internal machinery without failing the
    /// session. A steadily climbing value is a degraded live view that reports
    /// itself healthy, which is the failure this exists to make visible.
    pub restarts: u64,
}

pub trait PublicationAdapter: Send + 'static {
    fn timestamp_authority(&self) -> TimestampAuthority;
    fn connect(&mut self) -> AdapterFuture<'_, ()>;
    fn send_access_unit<'a>(
        &'a mut self,
        access_unit: EncodedAccessUnit<'a>,
    ) -> AdapterFuture<'a, SendOutcome>;
    fn disconnect(&mut self) -> AdapterFuture<'_, ()>;

    /// Adapter-held counters, sampled by the owner.
    ///
    /// Defaulted so an adapter with nothing extra to report does not have to
    /// say so, and so adding this did not change any existing implementation.
    fn observe(&self) -> AdapterObservation {
        AdapterObservation::default()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterErrorKind {
    Unavailable,
    Authentication,
    Connection,
    Transport,
    Shutdown,
}

#[derive(Debug)]
pub struct AdapterError {
    kind: AdapterErrorKind,
}

impl AdapterError {
    #[must_use]
    pub const fn new(kind: AdapterErrorKind) -> Self {
        Self { kind }
    }
    #[must_use]
    pub const fn kind(&self) -> AdapterErrorKind {
        self.kind
    }
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(
            self.kind,
            AdapterErrorKind::Connection | AdapterErrorKind::Transport
        )
    }
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "LiveKit adapter failure: {:?}", self.kind)
    }
}
impl Error for AdapterError {}

pub struct MemoryAdapter {
    pub fail_connect: Option<AdapterErrorKind>,
    pub fail_send_after: Option<usize>,
    pub connected: bool,
    pub access_units: Vec<Vec<u8>>,
    pub timestamps: Vec<(u64, u32)>,
    /// Per access unit, what the media plane said about it: whether it starts
    /// a decodable run, and its media-derived capture time in microseconds.
    pub descriptions: Vec<(bool, i64)>,
}

impl MemoryAdapter {
    #[must_use]
    pub fn healthy() -> Self {
        Self {
            fail_connect: None,
            fail_send_after: None,
            connected: false,
            access_units: Vec::new(),
            timestamps: Vec::new(),
            descriptions: Vec::new(),
        }
    }
}

impl PublicationAdapter for MemoryAdapter {
    fn timestamp_authority(&self) -> TimestampAuthority {
        TimestampAuthority::Media
    }

    fn connect(&mut self) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            if let Some(kind) = self.fail_connect {
                Err(AdapterError::new(kind))
            } else {
                self.connected = true;
                Ok(())
            }
        })
    }
    fn send_access_unit<'a>(
        &'a mut self,
        access_unit: EncodedAccessUnit<'a>,
    ) -> AdapterFuture<'a, SendOutcome> {
        Box::pin(async move {
            if self
                .fail_send_after
                .is_some_and(|limit| self.access_units.len() >= limit)
            {
                Err(AdapterError::new(AdapterErrorKind::Transport))
            } else {
                self.access_units.push(access_unit.annex_b.to_vec());
                self.timestamps
                    .push((access_unit.epoch, access_unit.rtp_timestamp));
                self.descriptions.push((
                    access_unit.starts_decodable_run,
                    access_unit.capture_timestamp_us,
                ));
                Ok(SendOutcome::Published)
            }
        })
    }
    fn disconnect(&mut self) -> AdapterFuture<'_, ()> {
        Box::pin(async move {
            self.connected = false;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_adapter_has_explicit_connect_send_disconnect_lifetime() {
        let mut adapter = MemoryAdapter::healthy();
        adapter
            .connect()
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        adapter
            .send_access_unit(EncodedAccessUnit {
                annex_b: &[0, 0, 0, 1, 0x65],
                rtp_timestamp: 42,
                epoch: 7,
                starts_decodable_run: true,
                capture_timestamp_us: 466_667,
            })
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        adapter
            .disconnect()
            .await
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(adapter.access_units.len(), 1);
        assert_eq!(adapter.timestamps, vec![(7, 42)]);
        assert_eq!(adapter.descriptions, vec![(true, 466_667)]);
        assert!(!adapter.connected);
    }
}
