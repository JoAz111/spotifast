//! Read-only, candidate playback quality from Spotify's existing cluster feed.
//!
//! This observer never registers a Connect receiver or requests playback. A
//! metadata-only session may receive no cluster updates at all. Missing evidence
//! means unknown quality; a target bitrate is never evidence of actual quality.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use librespot_core::dealer::protocol::{Message, PayloadValue};
use librespot_core::session::Session;
use librespot_protocol::connect::ClusterUpdate;
use librespot_protocol::player::{BitrateLevel, HiFiStatus};
use protobuf::Message as _;
use tokio::sync::watch;

/// A silent dealer reconnect cannot leave a quality claim indefinitely.
pub const QUALITY_MAX_AGE: Duration = Duration::from_secs(15);
const MAX_CLUSTER_BYTES: usize = 128 * 1024;
const MAX_IDENTIFIER_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActualBitrateLevel {
    HiFi,
    HiFi24,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActualHiFiStatus {
    On,
}

/// An in-memory observation, not a source bit-depth or hardware-output claim.
/// Debug deliberately excludes device, track, and occurrence identifiers.
#[derive(Clone, PartialEq, Eq)]
pub struct QualitySnapshot {
    pub active_device_id: String,
    pub track_uri: String,
    pub playback_id: String,
    pub bitrate_level: ActualBitrateLevel,
    pub hifi_status: ActualHiFiStatus,
    pub ordering_timestamp_ms: i64,
    pub received_at: Instant,
}

impl fmt::Debug for QualitySnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QualitySnapshot")
            .field("bitrate_level", &self.bitrate_level)
            .field("hifi_status", &self.hifi_status)
            .finish_non_exhaustive()
    }
}

impl QualitySnapshot {
    pub fn is_fresh(&self, now: Instant) -> bool {
        now.checked_duration_since(self.received_at)
            .is_some_and(|age| age < QUALITY_MAX_AGE)
    }

    /// Every identity must come from the currently authoritative playback.
    /// A URI alone cannot distinguish repeated plays of the same recording.
    pub fn matches_playback(
        &self,
        active_device_id: &str,
        track_uri: &str,
        playback_id: &str,
        now: Instant,
    ) -> bool {
        self.is_fresh(now)
            && self.active_device_id == active_device_id
            && self.track_uri == track_uri
            && self.playback_id == playback_id
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct UpdateOrder {
    changed_ms: i64,
    player_ms: i64,
    server_ms: i64,
}

struct ClusterFrame {
    order: Option<UpdateOrder>,
    quality: Option<QualitySnapshot>,
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value.bytes().all(|byte| byte.is_ascii_graphic())
}

impl ClusterFrame {
    fn from_message(message: Message, now: Instant) -> Option<Self> {
        let PayloadValue::Raw(bytes) = message.payload else {
            return None;
        };
        Self::from_bytes(&bytes, now)
    }

    fn from_bytes(bytes: &[u8], now: Instant) -> Option<Self> {
        if bytes.len() > MAX_CLUSTER_BYTES {
            return None;
        }
        let update = ClusterUpdate::parse_from_bytes(bytes).ok()?;
        Some(Self::from_update(&update, now))
    }

    fn from_update(update: &ClusterUpdate, now: Instant) -> Self {
        let Some(cluster) = update.cluster.as_ref() else {
            return Self {
                order: None,
                quality: None,
            };
        };
        let state = cluster.player_state.as_ref();
        let order = (cluster.changed_timestamp_ms > 0).then_some(UpdateOrder {
            changed_ms: cluster.changed_timestamp_ms,
            player_ms: state.map_or(0, |state| state.timestamp),
            server_ms: cluster.server_timestamp_ms,
        });
        let quality = (|| {
            let order = order?;
            let state = state?;
            if state.timestamp <= 0 || !(state.is_playing || state.is_paused) || state.is_buffering
            {
                return None;
            }
            // Only the current track is relevant; next_tracks can be prefetched.
            let track = state.track.as_ref()?;
            let track_id = track.uri.strip_prefix("spotify:track:")?;
            if track_id.len() != 22
                || !track_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
                || !identifier(&cluster.active_device_id)
                || !identifier(&state.playback_id)
            {
                return None;
            }
            let quality = state.playback_quality.as_ref()?;
            let bitrate_level = match quality.bitrate_level.enum_value().ok()? {
                BitrateLevel::hifi => ActualBitrateLevel::HiFi,
                BitrateLevel::hifi24 => ActualBitrateLevel::HiFi24,
                _ => return None,
            };
            if quality.hifi_status.enum_value().ok()? != HiFiStatus::on {
                return None;
            }
            Some(QualitySnapshot {
                active_device_id: cluster.active_device_id.clone(),
                track_uri: track.uri.clone(),
                playback_id: state.playback_id.clone(),
                bitrate_level,
                hifi_status: ActualHiFiStatus::On,
                ordering_timestamp_ms: order.changed_ms,
                received_at: now,
            })
        })();
        Self { order, quality }
    }
}

#[derive(Default)]
struct QualityTracker {
    order: Option<UpdateOrder>,
    quality: Option<QualitySnapshot>,
}

impl QualityTracker {
    /// Outer None means no event; inner None explicitly clears the observation.
    fn apply(&mut self, frame: Option<ClusterFrame>) -> Option<Option<QualitySnapshot>> {
        let Some(frame) = frame else {
            return self.clear();
        };
        let Some(order) = frame.order else {
            return self.clear();
        };
        if self.order.is_some_and(|previous| order < previous) {
            return None;
        }
        if self.order == Some(order) {
            // Exact replay must not refresh age. A conflict at the same order
            // fails closed and cannot be restored by another equal-order frame.
            let agrees = match (&self.quality, &frame.quality) {
                (Some(previous), Some(next)) => {
                    previous.active_device_id == next.active_device_id
                        && previous.track_uri == next.track_uri
                        && previous.playback_id == next.playback_id
                        && previous.bitrate_level == next.bitrate_level
                }
                (None, None) => true,
                _ => false,
            };
            return if agrees { None } else { self.clear() };
        }
        self.order = Some(order);
        let already_unknown = self.quality.is_none() && frame.quality.is_none();
        self.quality = frame.quality;
        if already_unknown {
            None
        } else {
            Some(self.quality.clone())
        }
    }

    fn clear(&mut self) -> Option<Option<QualitySnapshot>> {
        self.quality.take().map(|_| None)
    }

    fn expire(&mut self, now: Instant) -> Option<Option<QualitySnapshot>> {
        if self
            .quality
            .as_ref()
            .is_some_and(|quality| !quality.is_fresh(now))
        {
            self.clear()
        } else {
            None
        }
    }
}

type QualityNotify = Arc<dyn Fn(Option<QualitySnapshot>) + Send + Sync>;

struct ObserverControl {
    stopped: Mutex<bool>,
    notify: QualityNotify,
}

impl ObserverControl {
    fn publish(&self, quality: Option<QualitySnapshot>) {
        let stopped = self
            .stopped
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if !*stopped {
            (self.notify)(quality);
        }
    }

    fn stop(&self) {
        let mut stopped = self
            .stopped
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if !*stopped {
            *stopped = true;
            (self.notify)(None);
        }
    }
}

/// Cancellation belongs to the metadata engine, including Engine::drop.
pub(crate) struct QualityObserver {
    cancel: watch::Sender<bool>,
    control: Arc<ObserverControl>,
}

impl QualityObserver {
    pub(crate) fn start(session: Session, notify: QualityNotify) -> Self {
        let (cancel, mut cancelled) = watch::channel(false);
        let control = Arc::new(ObserverControl {
            stopped: Mutex::new(false),
            notify,
        });
        let task_control = Arc::clone(&control);
        tokio::spawn(async move {
            let run = async {
                let dealer = session.dealer();
                let Ok(mut connections) = dealer.add_listen_for("hm://pusher/v1/connections/")
                else {
                    log::warn!("quality observer unavailable: connection subscription failed");
                    return;
                };
                let Ok(mut clusters) = dealer.add_listen_for("hm://connect-state/v1/cluster")
                else {
                    log::warn!("quality observer unavailable: quality subscription failed");
                    return;
                };
                // Merely open the existing metadata dealer. No connection ID is
                // retained or sent to Connect's device-registration endpoint.
                let started = tokio::select! {
                    biased;
                    _ = cancelled.changed() => false,
                    result = tokio::time::timeout(Duration::from_secs(30), dealer.start()) => {
                        match result {
                            Ok(Ok(())) => true,
                            Ok(Err(_)) => {
                                log::warn!("quality observer unavailable: Spotify connection failed");
                                false
                            }
                            Err(_) => {
                                log::warn!("quality observer unavailable: Spotify connection timed out");
                                false
                            }
                        }
                    }
                };
                if !started {
                    return;
                }
                log::info!("quality observer connected");
                let mut observed_cluster = false;
                let mut tracker = QualityTracker::default();
                let mut tick = tokio::time::interval(Duration::from_secs(1));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        biased;
                        _ = cancelled.changed() => break,
                        _ = tick.tick() => {
                            if session.is_invalid() {
                                break;
                            }
                            if let Some(quality) = tracker.expire(Instant::now()) {
                                task_control.publish(quality);
                            }
                        }
                        connection = connections.next() => {
                            if connection.is_none() {
                                break;
                            }
                            // The public dealer API hides transient disconnects.
                            // Clear on reconnect; silent outages expire above.
                            if let Some(quality) = tracker.clear() {
                                task_control.publish(quality);
                            }
                        }
                        cluster = clusters.next() => {
                            let Some(cluster) = cluster else {
                                break;
                            };
                            let frame = ClusterFrame::from_message(cluster, Instant::now());
                            if !observed_cluster {
                                log::info!("quality update received; parsed={}", frame.is_some());
                                observed_cluster = true;
                            }
                            if let Some(quality) = tracker.apply(frame) {
                                task_control.publish(quality);
                            }
                        }
                    }
                }
            };
            run.await;
            task_control.stop();
            let _ = tokio::time::timeout(Duration::from_secs(3), session.dealer().close()).await;
        });
        Self { cancel, control }
    }

    pub(crate) fn stop(&self) {
        self.control.stop();
        self.cancel.send_replace(true);
    }
}

impl Drop for QualityObserver {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librespot_protocol::connect::Cluster;
    use librespot_protocol::player::{PlaybackQuality, PlayerState, ProvidedTrack};
    use protobuf::{EnumOrUnknown, MessageField};

    const TRACK: &str = "spotify:track:0000000000000000000001";
    const NEXT: &str = "spotify:track:0000000000000000000002";

    fn fixture(timestamp: i64, occurrence: &str, level: BitrateLevel) -> ClusterUpdate {
        ClusterUpdate {
            cluster: MessageField::some(Cluster {
                changed_timestamp_ms: timestamp,
                server_timestamp_ms: timestamp,
                active_device_id: "receiver".into(),
                player_state: MessageField::some(PlayerState {
                    timestamp,
                    playback_id: occurrence.into(),
                    is_playing: true,
                    track: MessageField::some(ProvidedTrack {
                        uri: TRACK.into(),
                        ..Default::default()
                    }),
                    playback_quality: MessageField::some(PlaybackQuality {
                        bitrate_level: EnumOrUnknown::new(level),
                        hifi_status: EnumOrUnknown::new(HiFiStatus::on),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn apply(tracker: &mut QualityTracker, update: &ClusterUpdate, now: Instant) {
        let bytes = update.write_to_bytes().unwrap();
        tracker.apply(ClusterFrame::from_bytes(&bytes, now));
    }

    #[test]
    fn actual_lossless_transitions_and_targets() {
        let now = Instant::now();
        let mut tracker = QualityTracker::default();
        apply(&mut tracker, &fixture(1, "first", BitrateLevel::hifi), now);
        assert_eq!(
            tracker.quality.as_ref().unwrap().bitrate_level,
            ActualBitrateLevel::HiFi
        );
        let mut lossy = fixture(2, "second", BitrateLevel::very_high);
        let quality = lossy
            .cluster
            .as_mut()
            .unwrap()
            .player_state
            .as_mut()
            .unwrap()
            .playback_quality
            .as_mut()
            .unwrap();
        quality.target_bitrate_level = EnumOrUnknown::new(BitrateLevel::hifi24);
        quality.target_bitrate_available = true;
        apply(&mut tracker, &lossy, now);
        assert!(tracker.quality.is_none());
        apply(
            &mut tracker,
            &fixture(3, "third", BitrateLevel::hifi24),
            now,
        );
        assert_eq!(
            tracker.quality.as_ref().unwrap().bitrate_level,
            ActualBitrateLevel::HiFi24
        );
    }

    #[test]
    fn current_track_and_occurrence_must_match() {
        let now = Instant::now();
        let mut update = fixture(1, "first", BitrateLevel::hifi);
        update
            .cluster
            .as_mut()
            .unwrap()
            .player_state
            .as_mut()
            .unwrap()
            .next_tracks
            .push(ProvidedTrack {
                uri: NEXT.into(),
                ..Default::default()
            });
        let quality = ClusterFrame::from_update(&update, now).quality.unwrap();
        assert!(quality.matches_playback("receiver", TRACK, "first", now));
        assert!(!quality.matches_playback("other", TRACK, "first", now));
        assert!(!quality.matches_playback("receiver", NEXT, "first", now));
        assert!(!quality.matches_playback("receiver", TRACK, "repeat", now));
        assert!(!quality.matches_playback("receiver", TRACK, "", now));
    }

    #[test]
    fn stale_replay_and_equal_order_conflict_fail_closed() {
        let now = Instant::now();
        let update = fixture(2, "current", BitrateLevel::hifi);
        let mut tracker = QualityTracker::default();
        apply(&mut tracker, &update, now);
        apply(
            &mut tracker,
            &fixture(1, "old", BitrateLevel::very_high),
            now,
        );
        assert_eq!(tracker.quality.as_ref().unwrap().playback_id, "current");
        apply(&mut tracker, &update, now + Duration::from_secs(10));
        assert_eq!(tracker.quality.as_ref().unwrap().received_at, now);
        apply(
            &mut tracker,
            &fixture(2, "current", BitrateLevel::very_high),
            now,
        );
        assert!(tracker.quality.is_none());
        apply(&mut tracker, &update, now);
        assert!(tracker.quality.is_none());
        apply(&mut tracker, &fixture(3, "new", BitrateLevel::hifi), now);
        assert!(tracker.quality.is_some());
    }

    #[test]
    fn unknown_quality_does_not_repeatedly_wake_the_ui() {
        let now = Instant::now();
        let mut tracker = QualityTracker::default();
        let lossy = fixture(1, "first", BitrateLevel::very_high);
        assert!(
            tracker
                .apply(Some(ClusterFrame::from_update(&lossy, now)))
                .is_none()
        );
        let later = fixture(2, "first", BitrateLevel::very_high);
        assert!(
            tracker
                .apply(Some(ClusterFrame::from_update(&later, now)))
                .is_none()
        );
        assert_eq!(tracker.order.unwrap().changed_ms, 2);
        assert!(
            tracker
                .apply(Some(ClusterFrame::from_update(
                    &fixture(3, "first", BitrateLevel::hifi),
                    now
                )))
                .unwrap()
                .is_some()
        );
        assert_eq!(
            tracker.apply(Some(ClusterFrame::from_update(
                &fixture(4, "first", BitrateLevel::very_high),
                now
            ))),
            Some(None)
        );
        assert!(
            tracker
                .apply(Some(ClusterFrame::from_update(
                    &fixture(5, "first", BitrateLevel::very_high),
                    now
                )))
                .is_none()
        );
    }

    #[test]
    fn player_order_precedes_server_delivery_order() {
        let now = Instant::now();
        let current = fixture(10, "current", BitrateLevel::hifi);
        let mut older = fixture(10, "old", BitrateLevel::hifi);
        older.cluster.as_mut().unwrap().server_timestamp_ms = 100;
        older
            .cluster
            .as_mut()
            .unwrap()
            .player_state
            .as_mut()
            .unwrap()
            .timestamp = 9;
        let mut tracker = QualityTracker::default();
        apply(&mut tracker, &current, now);
        apply(&mut tracker, &older, now);
        assert_eq!(tracker.quality.as_ref().unwrap().playback_id, "current");
    }

    #[test]
    fn malformed_missing_unknown_and_buffering_clear() {
        let now = Instant::now();
        let baseline = fixture(1, "current", BitrateLevel::hifi);
        let mut variants = Vec::new();
        let mut update = fixture(2, "current", BitrateLevel::hifi);
        update
            .cluster
            .as_mut()
            .unwrap()
            .player_state
            .as_mut()
            .unwrap()
            .playback_quality
            .as_mut()
            .unwrap()
            .bitrate_level = EnumOrUnknown::from_i32(99);
        variants.push(update);
        let mut update = fixture(2, "current", BitrateLevel::hifi);
        update
            .cluster
            .as_mut()
            .unwrap()
            .player_state
            .as_mut()
            .unwrap()
            .playback_quality
            .as_mut()
            .unwrap()
            .hifi_status = EnumOrUnknown::from_i32(99);
        variants.push(update);
        let mut update = fixture(2, "current", BitrateLevel::hifi);
        update
            .cluster
            .as_mut()
            .unwrap()
            .player_state
            .as_mut()
            .unwrap()
            .playback_quality
            .clear();
        variants.push(update);
        let mut update = fixture(2, "current", BitrateLevel::hifi);
        update
            .cluster
            .as_mut()
            .unwrap()
            .player_state
            .as_mut()
            .unwrap()
            .is_buffering = true;
        variants.push(update);
        let mut update = fixture(2, "", BitrateLevel::hifi);
        update.cluster.as_mut().unwrap().active_device_id.clear();
        variants.push(update);
        variants.push(ClusterUpdate::default());
        for update in variants {
            let mut tracker = QualityTracker::default();
            apply(&mut tracker, &baseline, now);
            apply(&mut tracker, &update, now);
            assert!(tracker.quality.is_none());
        }
        let mut tracker = QualityTracker::default();
        apply(&mut tracker, &baseline, now);
        tracker.apply(ClusterFrame::from_bytes(&[0xff], now));
        assert!(tracker.quality.is_none());
        assert!(ClusterFrame::from_bytes(&vec![0; MAX_CLUSTER_BYTES + 1], now).is_none());
    }

    #[test]
    fn freshness_disconnect_and_reconnect_require_new_evidence() {
        let now = Instant::now();
        let update = fixture(1, "first", BitrateLevel::hifi);
        let mut tracker = QualityTracker::default();
        apply(&mut tracker, &update, now);
        assert!(
            tracker
                .expire(now + QUALITY_MAX_AGE - Duration::from_millis(1))
                .is_none()
        );
        assert_eq!(tracker.expire(now + QUALITY_MAX_AGE), Some(None));
        apply(&mut tracker, &update, now + QUALITY_MAX_AGE);
        assert!(tracker.quality.is_none());
        apply(&mut tracker, &fixture(2, "first", BitrateLevel::hifi), now);
        assert_eq!(tracker.clear(), Some(None));
        apply(&mut tracker, &fixture(2, "first", BitrateLevel::hifi), now);
        assert!(tracker.quality.is_none());
        apply(&mut tracker, &fixture(3, "second", BitrateLevel::hifi), now);
        assert!(tracker.quality.is_some());
    }

    #[test]
    fn cancellation_prevents_later_publication_and_debug_redacts_identifiers() {
        let values = Arc::new(Mutex::new(Vec::new()));
        let output = Arc::clone(&values);
        let control = ObserverControl {
            stopped: Mutex::new(false),
            notify: Arc::new(move |quality| output.lock().unwrap().push(quality)),
        };
        let now = Instant::now();
        let quality =
            ClusterFrame::from_update(&fixture(1, "secret-occurrence", BitrateLevel::hifi), now)
                .quality
                .unwrap();
        let debug = format!("{quality:?}");
        assert!(!debug.contains(TRACK));
        assert!(!debug.contains("secret-occurrence"));
        assert!(!debug.contains("receiver"));
        control.publish(Some(quality.clone()));
        control.stop();
        control.publish(Some(quality));
        control.stop();
        let values = values.lock().unwrap();
        assert_eq!(values.len(), 2);
        assert!(values[0].is_some());
        assert!(values[1].is_none());
    }
}
