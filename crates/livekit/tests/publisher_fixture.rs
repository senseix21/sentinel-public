use std::{num::NonZeroUsize, path::PathBuf, sync::Arc, time::Duration};

use surveillance_common::types::CameraId;
use surveillance_livekit::{
    adapter::CliPublicationAdapter,
    config::{LiveKitConfig, LiveKitCredentials, PublicationBackoff},
    domain::PublicationIdentity,
    owner::LivePublicationOwner,
};
use surveillance_media::{
    distribution::{MediaDistributor, QueueCapacity},
    packet::{H264AccessUnit, H264AccessUnitAssembler, H264NalUnit, MediaTimestamp},
};
use tokio::sync::watch;
use url::Url;

fn access_unit(ticks: i64, nals: &[&[u8]]) -> Arc<H264AccessUnit> {
    let mut assembler = H264AccessUnitAssembler::new();
    let timestamp = MediaTimestamp::new(ticks, 90_000)
        .unwrap_or_else(|error| panic!("fixture timestamp: {error}"));
    let mut completed = None;
    for (index, bytes) in nals.iter().enumerate() {
        completed = assembler
            .push(
                timestamp,
                H264NalUnit::from_ebsp(Arc::from(*bytes))
                    .unwrap_or_else(|error| panic!("fixture NAL: {error}")),
                index + 1 == nals.len(),
            )
            .unwrap_or_else(|error| panic!("fixture access unit: {error}"));
    }
    Arc::new(completed.unwrap_or_else(|| panic!("fixture access unit must complete")))
}

fn fixture_config() -> LiveKitConfig {
    let endpoint =
        std::env::var("PHASE5_LIVEKIT_URL").unwrap_or_else(|_| "ws://127.0.0.1:7880".to_owned());
    let camera = CameraId::new("fixture-camera").unwrap_or_else(|error| panic!("{error}"));
    LiveKitConfig {
        endpoint: Url::parse(&endpoint).unwrap_or_else(|error| panic!("fixture URL: {error}")),
        credentials: LiveKitCredentials::new("devkey".to_owned(), "development-secret".to_owned())
            .unwrap_or_else(|error| panic!("{error}")),
        identity: PublicationIdentity::for_camera("fixture", &camera)
            .unwrap_or_else(|error| panic!("{error}")),
        deployment_namespace: "fixture".to_owned(),
        cli_path: PathBuf::from("/usr/local/bin/lk"),
        cli_version: "2.18.2".to_owned(),
        queue_items: NonZeroUsize::new(4).unwrap_or(NonZeroUsize::MIN),
        max_lag: Duration::from_millis(500),
        max_access_unit_bytes: NonZeroUsize::new(1024).unwrap_or(NonZeroUsize::MIN),
        ipc_write_timeout: Duration::from_millis(250),
        shutdown_timeout: Duration::from_secs(2),
        backoff: PublicationBackoff::new(
            Duration::from_millis(50),
            Duration::from_millis(100),
            Duration::from_secs(1),
        )
        .unwrap_or_else(|error| panic!("{error}")),
    }
}

#[tokio::test]
#[ignore = "orchestrated by scripts/phase5/outage-fixture.sh"]
async fn livekit_outage_retries_without_blocking_recorder_queue() {
    let capacity = QueueCapacity::new(4).unwrap_or_else(|error| panic!("{error}"));
    let (distributor, subscriptions) = MediaDistributor::new(capacity, capacity);
    let (owner, health) = LivePublicationOwner::new(
        fixture_config(),
        CliPublicationAdapter::new(fixture_config()),
        subscriptions.live,
    );
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(owner.run(shutdown_rx));
    let keyframe = access_unit(
        0,
        &[
            &[0x67, 100, 0, 31],
            &[0x68, 0],
            &[0x65, 1, 2],
            &[0x65, 3, 4],
        ],
    );
    let delta = access_unit(6_000, &[&[0x41, 5, 6]]);
    distributor.publish(keyframe);
    distributor.publish(delta);
    tokio::time::sleep(Duration::from_secs(2)).await;

    assert_eq!(subscriptions.recorder.metrics().accepted, 2);
    assert_eq!(subscriptions.recorder.metrics().rejected_full, 0);
    assert!(health.snapshot().counters.connection_attempts >= 1);

    shutdown_tx.send_replace(true);
    let exit = task.await.unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(exit.snapshot.counters.access_units, 0);
}
