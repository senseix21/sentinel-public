//! Change classification, reporting, and transactional planning.
//!
//! A batch is planned before anything is written. Planning validates the
//! *result* of applying the whole batch, not each change in isolation, because
//! a setting can be individually legal and jointly invalid — raising a queue
//! capacity is a legal integer that can breach the pipeline memory budget.
//!
//! The plan is all-or-nothing. There is no partial application and no coercion:
//! an out-of-range value is rejected with the setting named, never clamped to a
//! bound. Clamping would leave the operator believing they set something they
//! did not.

use std::{collections::BTreeSet, error::Error, fmt};

use surveillance_common::types::CameraId;

use crate::{
    assemble,
    classification::{
        ApplicationBoundary, Origin, Scope, SettingClass, SettingId, ValidationGroup,
    },
    resolution::{Layer, ResolvedSettings, SettingKey},
    secret::Secret,
    settings::OneCameraConfig,
    value::SettingValue,
};

/// One requested change.
#[derive(Clone)]
pub struct ProposedChange {
    pub key: SettingKey,
    pub value: SettingValue,
}

/// What happened to one setting in a batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeOutcome {
    /// Effective now.
    Applied,
    /// Stored; effective at the named boundary.
    PendingBoundary { boundary: ApplicationBoundary },
    /// Stored; effective after a process restart.
    PendingRestart,
    /// Stored and audited, but a higher-precedence layer supplies the effective
    /// value, so the change has no effect. Reported rather than hidden.
    PersistedButShadowed { winning_origin: Origin },
    /// Stored, but the running system refused it. The previous effective value
    /// stays in force; see ADR 008 Decision 4.
    ApplyFailed { category: ApplyFailureCategory },
    /// The store already holds this value. No audit row is written.
    Unchanged,
}

impl ChangeOutcome {
    /// Stable label for metrics and API responses.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::PendingBoundary { .. } => "pending_boundary",
            Self::PendingRestart => "pending_restart",
            Self::PersistedButShadowed { .. } => "persisted_but_shadowed",
            Self::ApplyFailed { .. } => "apply_failed",
            Self::Unchanged => "unchanged",
        }
    }

    /// Whether the change is in force.
    #[must_use]
    pub const fn effective(self) -> bool {
        matches!(self, Self::Applied | Self::Unchanged)
    }
}

impl fmt::Display for ChangeOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why the running system refused an already-stored change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplyFailureCategory {
    /// A resource could not be acquired.
    Resource,
    /// An executable or path no longer exists.
    Missing,
    /// The subsystem that owns the setting was unavailable.
    Unavailable,
}

impl ApplyFailureCategory {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Resource => "resource",
            Self::Missing => "missing",
            Self::Unavailable => "unavailable",
        }
    }
}

/// The outcome of a batch.
#[derive(Clone, Debug)]
pub struct ChangeReport {
    pub generation: u64,
    pub outcomes: Vec<(SettingKey, ChangeOutcome)>,
}

impl ChangeReport {
    /// Settings awaiting a boundary, counted per boundary, for metrics.
    #[must_use]
    pub fn pending_by_boundary(&self) -> Vec<(ApplicationBoundary, usize)> {
        ApplicationBoundary::ALL
            .iter()
            .map(|boundary| {
                let count = self
                    .outcomes
                    .iter()
                    .filter(|(_, outcome)| match outcome {
                        ChangeOutcome::PendingBoundary { boundary: pending } => pending == boundary,
                        ChangeOutcome::PendingRestart => {
                            *boundary == ApplicationBoundary::ProcessRestart
                        }
                        _ => false,
                    })
                    .count();
                (*boundary, count)
            })
            .filter(|(_, count)| *count > 0)
            .collect()
    }

    /// Whether anything in the batch actually changed stored state.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.outcomes
            .iter()
            .all(|(_, outcome)| *outcome == ChangeOutcome::Unchanged)
    }
}

/// A validated batch, ready to persist.
pub struct UpdatePlan {
    entries: Vec<(SettingKey, SettingValue, Option<SettingValue>)>,
    outcomes: Vec<(SettingKey, ChangeOutcome)>,
}

impl fmt::Debug for UpdatePlan {
    /// Shows outcomes and the keys to be written, never values.
    ///
    /// A plan cannot hold a secret — the planner refuses them — but rendering
    /// only keys keeps that true regardless of what the table later declares.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdatePlan")
            .field(
                "writes",
                &self
                    .entries
                    .iter()
                    .map(|(key, _, _)| key.to_string())
                    .collect::<Vec<_>>(),
            )
            .field("outcomes", &self.outcomes)
            .finish()
    }
}

impl UpdatePlan {
    /// The rows to write, each with the value it replaces, if any.
    ///
    /// Excludes unchanged settings: they neither write nor audit.
    #[must_use]
    pub fn entries(&self) -> &[(SettingKey, SettingValue, Option<SettingValue>)] {
        &self.entries
    }

    /// The per-setting outcomes, including unchanged ones.
    #[must_use]
    pub fn outcomes(&self) -> &[(SettingKey, ChangeOutcome)] {
        &self.outcomes
    }

    /// Whether the plan writes anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Builds the report for a committed generation.
    #[must_use]
    pub fn into_report(self, generation: u64) -> ChangeReport {
        ChangeReport {
            generation,
            outcomes: self.outcomes,
        }
    }
}

/// Why a batch was refused in full.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateRejection {
    /// The key does not name a setting.
    UnknownKey { key: String },
    /// A bootstrap setting, or a secret, submitted through the API.
    NotOperational { id: SettingId },
    /// The same setting appears twice in one batch.
    Duplicate { key: SettingKey },
    /// A value whose shape does not match the setting.
    Malformed { id: SettingId, reason: &'static str },
    /// A camera scope on a global setting, or a missing one.
    Scope { id: SettingId, reason: &'static str },
    /// The batch is legal field by field but breaks a group invariant.
    GroupInvariant {
        group: ValidationGroup,
        detail: String,
    },
    /// The caller may not perform this action.
    Unauthorized,
    /// The mutating surface is disabled for this deployment.
    SurfaceDisabled,
    /// The server is shutting down and will not accept changes.
    ShuttingDown,
}

impl fmt::Display for UpdateRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownKey { key } => write!(formatter, "unknown setting {key}"),
            Self::NotOperational { id } => {
                write!(formatter, "setting {id} cannot be changed through the API")
            }
            Self::Duplicate { key } => write!(formatter, "setting {key} appears twice"),
            Self::Malformed { id, reason } => {
                write!(formatter, "setting {id} is malformed: {reason}")
            }
            Self::Scope { id, reason } => write!(formatter, "setting {id} {reason}"),
            Self::GroupInvariant { group, detail } => write!(formatter, "{group}: {detail}"),
            Self::Unauthorized => formatter.write_str("not authorized"),
            Self::SurfaceDisabled => formatter.write_str("configuration API is disabled"),
            Self::ShuttingDown => formatter.write_str("server is shutting down"),
        }
    }
}

impl Error for UpdateRejection {}

/// Plans a batch against the current configuration.
///
/// # Errors
/// Returns the first rejection. The batch is refused whole: no entry in the
/// returned plan is ever partially applied, and a rejection means nothing was
/// written.
pub fn plan(
    current: &ResolvedSettings,
    changes: Vec<ProposedChange>,
    salt: &Secret<String>,
) -> Result<UpdatePlan, UpdateRejection> {
    let mut seen = BTreeSet::new();
    let mut overlay = Layer::new(Origin::Persisted);

    for change in &changes {
        let id = change.key.id();
        if !seen.insert(change.key.clone()) {
            return Err(UpdateRejection::Duplicate {
                key: change.key.clone(),
            });
        }
        if id.class() == SettingClass::Bootstrap || !id.api_editable() {
            return Err(UpdateRejection::NotOperational { id });
        }
        if change.value.kind() != id.kind() {
            return Err(UpdateRejection::Malformed {
                id,
                reason: "value does not match the setting's declared kind",
            });
        }
        if id.scope() == Scope::Global && change.key.camera().is_some() {
            return Err(UpdateRejection::Scope {
                id,
                reason: "is global and takes no camera scope",
            });
        }
        overlay
            .insert(change.key.clone(), change.value.clone())
            .map_err(|_| UpdateRejection::NotOperational { id })?;
    }

    let candidate = current.with_overlay(overlay);
    validate_groups(&changes, &candidate)?;

    let mut entries = Vec::new();
    let mut outcomes = Vec::new();
    for change in changes {
        let id = change.key.id();
        let stored = current
            .layer_value(Origin::Persisted, id, change.key.camera())
            .cloned();
        let unchanged = stored
            .as_ref()
            .is_some_and(|existing| existing.same_as(&change.value, salt));

        let outcome = if unchanged {
            ChangeOutcome::Unchanged
        } else {
            entries.push((change.key.clone(), change.value.clone(), stored));
            classify(&candidate, &change.key)
        };
        outcomes.push((change.key, outcome));
    }

    Ok(UpdatePlan { entries, outcomes })
}

/// Classifies a change that will be written.
fn classify(candidate: &ResolvedSettings, key: &SettingKey) -> ChangeOutcome {
    let id = key.id();
    if let Some(winning_origin) = candidate.shadows_persisted(id, key.camera()) {
        return ChangeOutcome::PersistedButShadowed { winning_origin };
    }
    match id.boundary() {
        ApplicationBoundary::Immediate => ChangeOutcome::Applied,
        ApplicationBoundary::ProcessRestart => ChangeOutcome::PendingRestart,
        boundary => ChangeOutcome::PendingBoundary { boundary },
    }
}

/// Re-validates every group the batch touches, against the post-change state.
fn validate_groups(
    changes: &[ProposedChange],
    candidate: &ResolvedSettings,
) -> Result<(), UpdateRejection> {
    let mut groups = BTreeSet::new();
    let mut cameras: BTreeSet<CameraId> = BTreeSet::new();
    let mut touches_all_cameras = false;

    for change in changes {
        for group in change.key.id().groups() {
            groups.insert(group.as_str());
        }
        if change.key.id().scope() == Scope::PerCamera {
            match change.key.camera() {
                Some(camera) => {
                    cameras.insert(camera.clone());
                }
                // A template change affects every configured camera.
                None => touches_all_cameras = true,
            }
        }
    }

    if touches_all_cameras {
        cameras.extend(candidate.cameras().iter().cloned());
    }

    for group in ValidationGroup::ALL {
        if !groups.contains(group.as_str()) {
            continue;
        }
        match group {
            ValidationGroup::StorageWatermarks => {
                assemble::storage_watermarks(candidate).map_err(|error| {
                    UpdateRejection::GroupInvariant {
                        group: *group,
                        detail: error.to_string(),
                    }
                })?;
            }
            ValidationGroup::Retention => {
                assemble::retention_policy(candidate).map_err(|error| {
                    UpdateRejection::GroupInvariant {
                        group: *group,
                        detail: error.to_string(),
                    }
                })?;
            }
            ValidationGroup::CameraMemoryBudget
            | ValidationGroup::DecodedFrameBudget
            | ValidationGroup::ReconnectPolicy => {
                // All three are enforced by the one transaction validator the
                // pipeline itself uses, so a batch cannot be accepted here and
                // rejected at pipeline start.
                for camera in &cameras {
                    let settings =
                        assemble::one_camera_settings(candidate, camera).map_err(|error| {
                            UpdateRejection::GroupInvariant {
                                group: *group,
                                detail: format!("{camera}: {error}", camera = camera.as_str()),
                            }
                        })?;
                    OneCameraConfig::validate(settings).map_err(|error| {
                        UpdateRejection::GroupInvariant {
                            group: *group,
                            detail: format!("{camera}: {error}", camera = camera.as_str()),
                        }
                    })?;
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{plan, ChangeOutcome, ProposedChange, UpdateRejection};
    use crate::{
        classification::{ApplicationBoundary, Origin, SettingId, ValidationGroup},
        defaults,
        resolution::{Layer, ResolvedSettings, SettingKey},
        secret::Secret,
        value::SettingValue,
    };
    use std::time::Duration;
    use surveillance_common::types::CameraId;

    fn salt() -> Secret<String> {
        Secret::new("unit".to_owned())
    }

    fn camera() -> CameraId {
        CameraId::new("camera-01".to_owned()).expect("valid test camera identifier")
    }

    fn deployment() -> Layer {
        let mut layer = Layer::new(Origin::File);
        let mut set = |id, value| {
            layer
                .insert(SettingKey::unscoped(id), value)
                .expect("valid test entry");
        };
        set(
            SettingId::CameraRtspEndpoint,
            SettingValue::Text("rtsp://127.0.0.1:8554/camera-01".to_owned()),
        );
        set(SettingId::CameraVideoWidth, SettingValue::Unsigned(1920));
        set(SettingId::CameraVideoHeight, SettingValue::Unsigned(1080));
        set(
            SettingId::CameraDetectionDecodedFrameMaxBytes,
            SettingValue::Unsigned(1920 * 1080 * 3),
        );
        layer
    }

    fn current() -> ResolvedSettings {
        ResolvedSettings::resolve(vec![defaults::layer(), deployment()])
    }

    fn change(id: SettingId, value: SettingValue) -> ProposedChange {
        ProposedChange {
            key: SettingKey::unscoped(id),
            value,
        }
    }

    #[test]
    fn an_immediate_setting_is_applied() {
        // `api.max_request_bytes` is one of the two settings the HTTP adapter
        // genuinely re-reads per request, which is what earns it `Immediate`.
        let planned = plan(
            &current(),
            vec![change(
                SettingId::ApiMaxRequestBytes,
                SettingValue::Unsigned(8_192),
            )],
            &salt(),
        )
        .expect("valid change");
        assert_eq!(planned.outcomes()[0].1, ChangeOutcome::Applied);
        assert_eq!(planned.entries().len(), 1);
    }

    #[test]
    fn a_setting_its_owner_captures_at_startup_is_pending_restart() {
        // Retention is read once when the owner is constructed, so reporting it
        // as applied would tell an operator their change is in force when it is
        // not.
        let planned = plan(
            &current(),
            vec![change(
                SettingId::RetentionInterval,
                SettingValue::Duration(Duration::from_secs(30)),
            )],
            &salt(),
        )
        .expect("valid change");
        assert_eq!(planned.outcomes()[0].1, ChangeOutcome::PendingRestart);
    }

    #[test]
    fn a_segment_scoped_setting_is_pending_its_boundary() {
        let planned = plan(
            &current(),
            vec![change(
                SettingId::CameraRecordingSegmentDuration,
                SettingValue::Duration(Duration::from_secs(45)),
            )],
            &salt(),
        )
        .expect("valid change");
        assert_eq!(
            planned.outcomes()[0].1,
            ChangeOutcome::PendingBoundary {
                boundary: ApplicationBoundary::NextSegment
            }
        );
    }

    #[test]
    fn a_restart_scoped_setting_is_pending_restart_and_does_not_apply_early() {
        let planned = plan(
            &current(),
            vec![change(
                SettingId::MetadataQueueItems,
                SettingValue::Unsigned(512),
            )],
            &salt(),
        )
        .expect("valid change");
        assert_eq!(planned.outcomes()[0].1, ChangeOutcome::PendingRestart);
        assert!(
            !planned.outcomes()[0].1.effective(),
            "a restart-scoped change must not report itself as in force"
        );
    }

    #[test]
    fn a_change_shadowed_by_the_environment_is_reported_not_hidden() {
        let mut environment = Layer::new(Origin::Environment);
        environment
            .insert(
                SettingKey::unscoped(SettingId::RetentionInterval),
                SettingValue::Duration(Duration::from_secs(600)),
            )
            .expect("valid entry");
        let current = ResolvedSettings::resolve(vec![defaults::layer(), deployment(), environment]);

        let planned = plan(
            &current,
            vec![change(
                SettingId::RetentionInterval,
                SettingValue::Duration(Duration::from_secs(30)),
            )],
            &salt(),
        )
        .expect("valid change");
        assert_eq!(
            planned.outcomes()[0].1,
            ChangeOutcome::PersistedButShadowed {
                winning_origin: Origin::Environment
            }
        );
        assert_eq!(
            planned.entries().len(),
            1,
            "a shadowed change is still stored and audited"
        );
    }

    #[test]
    fn resetting_a_stored_value_to_itself_is_unchanged_and_writes_nothing() {
        let mut persisted = Layer::new(Origin::Persisted);
        persisted
            .insert(
                SettingKey::unscoped(SettingId::RetentionInterval),
                SettingValue::Duration(Duration::from_secs(30)),
            )
            .expect("valid entry");
        let current = ResolvedSettings::resolve(vec![defaults::layer(), deployment(), persisted]);

        let planned = plan(
            &current,
            vec![change(
                SettingId::RetentionInterval,
                SettingValue::Duration(Duration::from_secs(30)),
            )],
            &salt(),
        )
        .expect("valid change");
        assert_eq!(planned.outcomes()[0].1, ChangeOutcome::Unchanged);
        assert!(
            planned.is_empty(),
            "an unchanged setting writes no audit row"
        );
    }

    #[test]
    fn setting_a_value_equal_to_a_lower_layer_is_still_a_change() {
        // The number is the same, but the origin becomes Persisted, which
        // changes what happens when the file layer is next edited.
        let planned = plan(
            &current(),
            vec![change(
                SettingId::ApiMaxRequestBytes,
                SettingValue::Unsigned(16 * 1024),
            )],
            &salt(),
        )
        .expect("valid change");
        assert_eq!(planned.outcomes()[0].1, ChangeOutcome::Applied);
        assert_eq!(planned.entries().len(), 1);
    }

    #[test]
    fn a_bootstrap_setting_is_refused() {
        let error = plan(
            &current(),
            vec![change(
                SettingId::ServerHealthBind,
                SettingValue::Text("0.0.0.0:80".to_owned()),
            )],
            &salt(),
        )
        .expect_err("bootstrap settings are not API-editable");
        assert_eq!(
            error,
            UpdateRejection::NotOperational {
                id: SettingId::ServerHealthBind
            }
        );
    }

    #[test]
    fn a_secret_is_refused_through_the_api() {
        let error = plan(
            &current(),
            vec![ProposedChange {
                key: SettingKey::scoped(SettingId::CameraRtspPassword, camera())
                    .expect("camera scoped"),
                value: SettingValue::Secret(Secret::new("hunter2".to_owned())),
            }],
            &salt(),
        )
        .expect_err("credentials are not accepted through the API in Phase 7");
        assert_eq!(
            error,
            UpdateRejection::NotOperational {
                id: SettingId::CameraRtspPassword
            }
        );
    }

    #[test]
    fn a_duplicate_setting_in_one_batch_is_refused() {
        let error = plan(
            &current(),
            vec![
                change(
                    SettingId::RetentionInterval,
                    SettingValue::Duration(Duration::from_secs(30)),
                ),
                change(
                    SettingId::RetentionInterval,
                    SettingValue::Duration(Duration::from_secs(45)),
                ),
            ],
            &salt(),
        )
        .expect_err("one batch must not set the same setting twice");
        assert!(matches!(error, UpdateRejection::Duplicate { .. }));
    }

    #[test]
    fn a_value_of_the_wrong_kind_is_refused() {
        let error = plan(
            &current(),
            vec![change(
                SettingId::RetentionInterval,
                SettingValue::Unsigned(30),
            )],
            &salt(),
        )
        .expect_err("an interval is a duration");
        assert!(matches!(error, UpdateRejection::Malformed { .. }));
    }

    #[test]
    fn one_invalid_field_rejects_the_whole_batch() {
        // The first change is legal on its own; the batch must still write
        // nothing.
        let error = plan(
            &current(),
            vec![
                change(
                    SettingId::RetentionInterval,
                    SettingValue::Duration(Duration::from_secs(30)),
                ),
                change(
                    SettingId::StorageWatermarkWarning,
                    SettingValue::Unsigned(99),
                ),
            ],
            &salt(),
        )
        .expect_err("99 breaks the watermark ordering");
        assert!(matches!(
            error,
            UpdateRejection::GroupInvariant {
                group: ValidationGroup::StorageWatermarks,
                ..
            }
        ));
    }

    #[test]
    fn a_group_invariant_broken_by_a_single_legal_field_is_caught() {
        let error = plan(
            &current(),
            vec![change(
                SettingId::StorageWatermarkWarning,
                SettingValue::Unsigned(95),
            )],
            &salt(),
        )
        .expect_err("warning must stay below critical");
        assert!(matches!(
            error,
            UpdateRejection::GroupInvariant {
                group: ValidationGroup::StorageWatermarks,
                ..
            }
        ));
    }

    #[test]
    fn a_watermark_batch_that_moves_all_four_together_is_accepted() {
        // The same field that fails alone succeeds as part of a consistent set.
        let planned = plan(
            &current(),
            vec![
                change(
                    SettingId::StorageWatermarkWarning,
                    SettingValue::Unsigned(85),
                ),
                change(
                    SettingId::StorageWatermarkCritical,
                    SettingValue::Unsigned(92),
                ),
                change(
                    SettingId::StorageWatermarkEmergency,
                    SettingValue::Unsigned(98),
                ),
                change(
                    SettingId::StorageWatermarkHysteresis,
                    SettingValue::Unsigned(3),
                ),
            ],
            &salt(),
        )
        .expect("a consistent watermark set is legal");
        assert_eq!(planned.entries().len(), 4);
    }

    #[test]
    fn a_protected_window_longer_than_retention_is_refused() {
        let error = plan(
            &current(),
            vec![change(
                SettingId::RetentionProtectedRecent,
                SettingValue::Duration(Duration::from_secs(90 * 24 * 60 * 60)),
            )],
            &salt(),
        )
        .expect_err("the protected window must be shorter than the retention age");
        assert!(matches!(
            error,
            UpdateRejection::GroupInvariant {
                group: ValidationGroup::Retention,
                ..
            }
        ));
    }

    #[test]
    fn a_queue_capacity_that_breaches_the_memory_budget_is_refused() {
        let mut with_camera = Layer::new(Origin::Persisted);
        with_camera
            .insert(
                SettingKey::scoped(SettingId::CameraQueueRecorderItems, camera())
                    .expect("camera scoped"),
                SettingValue::Unsigned(32),
            )
            .expect("valid entry");
        let current = ResolvedSettings::resolve(vec![defaults::layer(), deployment(), with_camera]);

        let error = plan(
            &current,
            vec![ProposedChange {
                key: SettingKey::scoped(SettingId::CameraQueueRecorderItems, camera())
                    .expect("camera scoped"),
                value: SettingValue::Unsigned(8_192),
            }],
            &salt(),
        )
        .expect_err("a queue capacity that breaches the memory budget is not legal");
        assert!(matches!(
            error,
            UpdateRejection::GroupInvariant {
                group: ValidationGroup::CameraMemoryBudget,
                ..
            }
        ));
    }

    #[test]
    fn nothing_is_written_when_a_batch_is_rejected() {
        let before = current();
        let error = plan(
            &before,
            vec![change(
                SettingId::StorageWatermarkWarning,
                SettingValue::Unsigned(99),
            )],
            &salt(),
        );
        assert!(error.is_err());
        // The planner never mutates its input; the caller holds the only path
        // to the store, and it is handed a plan or an error, never both.
        assert_eq!(
            before
                .effective(SettingId::StorageWatermarkWarning, None)
                .map(|value| value.value().render()),
            Some("80".to_owned())
        );
    }

    #[test]
    fn an_empty_batch_plans_to_nothing() {
        let planned = plan(&current(), Vec::new(), &salt()).expect("an empty batch is legal");
        assert!(planned.is_empty());
        assert!(planned.outcomes().is_empty());
    }

    #[test]
    fn pending_counts_are_reported_per_boundary() {
        let planned = plan(
            &current(),
            vec![
                change(
                    SettingId::CameraRecordingSegmentDuration,
                    SettingValue::Duration(Duration::from_secs(45)),
                ),
                change(SettingId::MetadataQueueItems, SettingValue::Unsigned(512)),
            ],
            &salt(),
        )
        .expect("valid batch");
        let report = planned.into_report(7);
        assert_eq!(report.generation, 7);
        let pending = report.pending_by_boundary();
        assert!(pending.contains(&(ApplicationBoundary::NextSegment, 1)));
        assert!(pending.contains(&(ApplicationBoundary::ProcessRestart, 1)));
    }

    #[test]
    fn an_unknown_key_is_rejected_before_anything_else() {
        assert_eq!(
            SettingId::from_key("retention.max_agee"),
            None,
            "the API resolves keys through the same table"
        );
    }
}
