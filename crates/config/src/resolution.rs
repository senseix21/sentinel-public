//! Layered resolution with per-setting origin tracking.
//!
//! Precedence, fixed by ADR 003 and made concrete by ADR 008:
//!
//! ```text
//! compiled defaults  <  config file  <  persisted settings  <  environment
//! ```
//!
//! An API change writes the persisted layer. It is not a fifth layer above the
//! environment, which is why a change to an environment-pinned setting is
//! reported as persisted-but-shadowed rather than applied.
//!
//! Within a layer, a camera-specific entry beats that layer's all-camera entry.
//! Across layers, precedence dominates: a later layer's all-camera entry
//! overrides an earlier layer's camera-specific entry. One rule, stated once, is
//! easier to reason about at three in the morning than a rule that depends on
//! which axis happens to be more specific.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
};

use surveillance_common::types::CameraId;

use crate::{
    classification::{Origin, Scope, Secrecy, SettingClass, SettingId},
    value::SettingValue,
};

/// Identifies one setting, optionally scoped to one camera.
///
/// A per-camera setting with no camera is a template: it supplies the value for
/// every camera that does not override it.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SettingKey {
    id: SettingId,
    camera: Option<CameraId>,
}

impl SettingKey {
    /// Creates a key with no camera scope.
    ///
    /// Always legal: a global setting has no camera, and a per-camera setting
    /// with no camera is the all-camera template.
    #[must_use]
    pub const fn unscoped(id: SettingId) -> Self {
        Self { id, camera: None }
    }

    /// Creates a camera-scoped key.
    ///
    /// # Errors
    /// Returns [`ScopeError::NotCameraScoped`] when the setting is global.
    pub fn scoped(id: SettingId, camera: CameraId) -> Result<Self, ScopeError> {
        if id.scope() == Scope::Global {
            return Err(ScopeError::NotCameraScoped(id));
        }
        Ok(Self {
            id,
            camera: Some(camera),
        })
    }

    #[must_use]
    pub const fn id(&self) -> SettingId {
        self.id
    }

    #[must_use]
    pub const fn camera(&self) -> Option<&CameraId> {
        self.camera.as_ref()
    }
}

impl fmt::Display for SettingKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.camera {
            Some(camera) => write!(formatter, "{}[{}]", self.id, camera.as_str()),
            None => write!(formatter, "{}", self.id),
        }
    }
}

/// A scope violation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeError {
    /// A camera was supplied for a setting that is not camera-scoped.
    NotCameraScoped(SettingId),
}

impl fmt::Display for ScopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotCameraScoped(id) => {
                write!(
                    formatter,
                    "setting {id} is global and takes no camera scope"
                )
            }
        }
    }
}

impl Error for ScopeError {}

/// One precedence layer.
#[derive(Clone)]
pub struct Layer {
    origin: Origin,
    values: BTreeMap<SettingKey, SettingValue>,
}

impl Layer {
    #[must_use]
    pub const fn new(origin: Origin) -> Self {
        Self {
            origin,
            values: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn origin(&self) -> Origin {
        self.origin
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Adds a value to this layer.
    ///
    /// # Errors
    /// Rejects a value whose kind does not match the setting, and rejects
    /// content the layer is not permitted to carry: the persisted layer holds
    /// neither bootstrap settings nor secrets.
    pub fn insert(&mut self, key: SettingKey, value: SettingValue) -> Result<(), LayerError> {
        if value.kind() != key.id().kind() {
            return Err(LayerError::KindMismatch {
                id: key.id(),
                expected: key.id().kind().as_str(),
                found: value.kind().as_str(),
            });
        }
        if self.origin == Origin::Persisted {
            if key.id().class() == SettingClass::Bootstrap {
                return Err(LayerError::BootstrapNotPersistable(key.id()));
            }
            if key.id().secrecy() == Secrecy::Secret {
                return Err(LayerError::SecretNotPersistable(key.id()));
            }
        }
        self.values.insert(key, value);
        Ok(())
    }

    /// Adds a value, panicking on a violation. Test-only convenience.
    #[cfg(test)]
    fn with(mut self, key: SettingKey, value: SettingValue) -> Self {
        self.insert(key, value).expect("valid test layer entry");
        self
    }

    fn get(&self, id: SettingId, camera: Option<&CameraId>) -> Option<&SettingValue> {
        // Specificity within the layer: the camera's own entry, then the
        // all-camera template.
        camera
            .and_then(|camera| {
                self.values.get(&SettingKey {
                    id,
                    camera: Some(camera.clone()),
                })
            })
            .or_else(|| self.values.get(&SettingKey { id, camera: None }))
    }

    fn keys(&self) -> impl Iterator<Item = &SettingKey> {
        self.values.keys()
    }

    /// Every entry, in key order.
    ///
    /// Deterministic order matters: the last-known-good cache checksums its
    /// contents, and an unstable iteration order would make an unchanged
    /// configuration produce a different file each time it is written.
    pub fn iter(&self) -> impl Iterator<Item = (&SettingKey, &SettingValue)> {
        self.values.iter()
    }
}

impl fmt::Debug for Layer {
    /// Renders keys and values. Secret values render redacted, because
    /// [`SettingValue`]'s own `Debug` redacts them.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Layer")
            .field("origin", &self.origin)
            .field("values", &self.values)
            .finish()
    }
}

/// A layer that was given content it may not carry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayerError {
    KindMismatch {
        id: SettingId,
        expected: &'static str,
        found: &'static str,
    },
    /// Bootstrap settings are resolved before the store exists.
    BootstrapNotPersistable(SettingId),
    /// Phase 7 does not persist credentials; see ADR 003's open secret store.
    SecretNotPersistable(SettingId),
}

impl fmt::Display for LayerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KindMismatch {
                id,
                expected,
                found,
            } => write!(formatter, "setting {id} expects {expected}, found {found}"),
            Self::BootstrapNotPersistable(id) => {
                write!(formatter, "setting {id} is bootstrap and is not persisted")
            }
            Self::SecretNotPersistable(id) => {
                write!(formatter, "setting {id} is secret and is not persisted")
            }
        }
    }
}

impl Error for LayerError {}

/// One setting's effective value and the layer that supplied it.
pub struct Resolved {
    value: SettingValue,
    origin: Origin,
}

impl Resolved {
    #[must_use]
    pub const fn value(&self) -> &SettingValue {
        &self.value
    }

    #[must_use]
    pub const fn origin(&self) -> Origin {
        self.origin
    }
}

impl fmt::Debug for Resolved {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Resolved")
            .field("value", &self.value)
            .field("origin", &self.origin)
            .finish()
    }
}

/// The effective configuration, with per-setting origin.
#[derive(Clone)]
pub struct ResolvedSettings {
    layers: Vec<Layer>,
    cameras: BTreeSet<CameraId>,
}

impl ResolvedSettings {
    /// Resolves layers into effective values.
    ///
    /// Layers are sorted by [`Origin`] precedence, so callers may supply them
    /// in any order without changing the result.
    #[must_use]
    pub fn resolve(mut layers: Vec<Layer>) -> Self {
        layers.sort_by_key(Layer::origin);
        let cameras = layers
            .iter()
            .flat_map(Layer::keys)
            .filter_map(|key| key.camera().cloned())
            .collect();
        Self { layers, cameras }
    }

    /// The effective value for a setting, for a specific camera or globally.
    #[must_use]
    pub fn effective(&self, id: SettingId, camera: Option<&CameraId>) -> Option<Resolved> {
        // Highest precedence first: the last layer that supplies the setting
        // wins outright, whether it does so specifically or as a template.
        self.layers.iter().rev().find_map(|layer| {
            layer.get(id, camera).map(|value| Resolved {
                value: value.clone(),
                origin: layer.origin(),
            })
        })
    }

    /// The origin of a setting's effective value.
    #[must_use]
    pub fn origin_of(&self, id: SettingId, camera: Option<&CameraId>) -> Option<Origin> {
        self.effective(id, camera).map(|resolved| resolved.origin)
    }

    /// Whether a setting's effective value comes from a layer that outranks the
    /// persisted store, which is what makes a persisted change ineffective.
    #[must_use]
    pub fn shadows_persisted(&self, id: SettingId, camera: Option<&CameraId>) -> Option<Origin> {
        self.origin_of(id, camera)
            .filter(|origin| *origin > Origin::Persisted)
    }

    /// Resolves again with an extra layer applied on top of its own origin.
    ///
    /// Used to evaluate a proposed batch before anything is written: the caller
    /// validates the *result* of the change rather than the change in
    /// isolation, which is the only way to catch a group invariant that a
    /// single legal-looking field would breach.
    ///
    /// The overlay is appended last, and the sort is stable, so it outranks an
    /// existing layer of the same origin.
    #[must_use]
    pub fn with_overlay(&self, overlay: Layer) -> Self {
        let mut layers = self.layers.clone();
        layers.push(overlay);
        Self::resolve(layers)
    }

    /// The value a specific layer supplies, ignoring precedence.
    ///
    /// Distinguishes "the persisted store already holds this value" from "the
    /// effective value happens to equal it", which decide whether a change is a
    /// change at all.
    #[must_use]
    pub fn layer_value(
        &self,
        origin: Origin,
        id: SettingId,
        camera: Option<&CameraId>,
    ) -> Option<&SettingValue> {
        self.layers
            .iter()
            .rev()
            .filter(|layer| layer.origin() == origin)
            .find_map(|layer| layer.get(id, camera))
    }

    /// Every camera mentioned by any layer.
    #[must_use]
    pub fn cameras(&self) -> &BTreeSet<CameraId> {
        &self.cameras
    }

    /// Settings that have no value in any layer.
    #[must_use]
    pub fn missing(&self) -> Vec<SettingId> {
        SettingId::ALL
            .iter()
            .copied()
            .filter(|id| id.scope() == Scope::Global && self.effective(*id, None).is_none())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Layer, LayerError, ResolvedSettings, SettingKey};
    use crate::{
        classification::{Origin, SettingId},
        secret::Secret,
        value::SettingValue,
    };
    use std::time::Duration;
    use surveillance_common::types::CameraId;

    fn camera(id: &str) -> CameraId {
        CameraId::new(id.to_owned()).expect("valid test camera identifier")
    }

    fn key(id: SettingId) -> SettingKey {
        SettingKey::unscoped(id)
    }

    fn scoped(id: SettingId, name: &str) -> SettingKey {
        SettingKey::scoped(id, camera(name)).expect("camera-scoped setting")
    }

    #[test]
    fn a_single_layer_supplies_its_own_origin() {
        let resolved = ResolvedSettings::resolve(vec![Layer::new(Origin::Default).with(
            key(SettingId::RetentionInterval),
            SettingValue::Duration(Duration::from_secs(60)),
        )]);
        let effective = resolved
            .effective(SettingId::RetentionInterval, None)
            .expect("value present");
        assert_eq!(effective.origin(), Origin::Default);
        assert_eq!(effective.value().render(), "1m");
    }

    #[test]
    fn each_layer_overrides_the_one_below_it() {
        let layers = vec![
            Layer::new(Origin::Default).with(
                key(SettingId::RetentionInterval),
                SettingValue::Duration(Duration::from_secs(60)),
            ),
            Layer::new(Origin::File).with(
                key(SettingId::RetentionInterval),
                SettingValue::Duration(Duration::from_secs(120)),
            ),
            Layer::new(Origin::Persisted).with(
                key(SettingId::RetentionInterval),
                SettingValue::Duration(Duration::from_secs(180)),
            ),
            Layer::new(Origin::Environment).with(
                key(SettingId::RetentionInterval),
                SettingValue::Duration(Duration::from_secs(240)),
            ),
        ];
        let resolved = ResolvedSettings::resolve(layers);
        let effective = resolved
            .effective(SettingId::RetentionInterval, None)
            .expect("value present");
        assert_eq!(effective.origin(), Origin::Environment);
        assert_eq!(effective.value().render(), "4m");
    }

    #[test]
    fn layer_order_of_submission_does_not_matter() {
        let ascending = ResolvedSettings::resolve(vec![
            Layer::new(Origin::Default).with(
                key(SettingId::ApiMaxRequestBytes),
                SettingValue::Unsigned(1),
            ),
            Layer::new(Origin::Environment).with(
                key(SettingId::ApiMaxRequestBytes),
                SettingValue::Unsigned(2),
            ),
        ]);
        let descending = ResolvedSettings::resolve(vec![
            Layer::new(Origin::Environment).with(
                key(SettingId::ApiMaxRequestBytes),
                SettingValue::Unsigned(2),
            ),
            Layer::new(Origin::Default).with(
                key(SettingId::ApiMaxRequestBytes),
                SettingValue::Unsigned(1),
            ),
        ]);
        assert_eq!(
            ascending
                .effective(SettingId::ApiMaxRequestBytes, None)
                .map(|resolved| resolved.value().render()),
            descending
                .effective(SettingId::ApiMaxRequestBytes, None)
                .map(|resolved| resolved.value().render()),
        );
    }

    #[test]
    fn a_camera_specific_entry_beats_a_template_in_the_same_layer() {
        let resolved = ResolvedSettings::resolve(vec![Layer::new(Origin::File)
            .with(
                key(SettingId::CameraDetectionSampleFps),
                SettingValue::Unsigned(5),
            )
            .with(
                scoped(SettingId::CameraDetectionSampleFps, "camera-02"),
                SettingValue::Unsigned(15),
            )]);
        assert_eq!(
            resolved
                .effective(
                    SettingId::CameraDetectionSampleFps,
                    Some(&camera("camera-02"))
                )
                .map(|value| value.value().render()),
            Some("15".to_owned())
        );
        assert_eq!(
            resolved
                .effective(
                    SettingId::CameraDetectionSampleFps,
                    Some(&camera("camera-01"))
                )
                .map(|value| value.value().render()),
            Some("5".to_owned())
        );
    }

    #[test]
    fn precedence_dominates_specificity_across_layers() {
        // The documented rule: a higher layer's template overrides a lower
        // layer's camera-specific entry.
        let resolved = ResolvedSettings::resolve(vec![
            Layer::new(Origin::File).with(
                scoped(SettingId::CameraDetectionSampleFps, "camera-02"),
                SettingValue::Unsigned(15),
            ),
            Layer::new(Origin::Environment).with(
                key(SettingId::CameraDetectionSampleFps),
                SettingValue::Unsigned(1),
            ),
        ]);
        let effective = resolved
            .effective(
                SettingId::CameraDetectionSampleFps,
                Some(&camera("camera-02")),
            )
            .expect("value present");
        assert_eq!(effective.value().render(), "1");
        assert_eq!(effective.origin(), Origin::Environment);
    }

    #[test]
    fn environment_shadows_a_persisted_value() {
        let resolved = ResolvedSettings::resolve(vec![
            Layer::new(Origin::Persisted).with(
                key(SettingId::RetentionMaxAge),
                SettingValue::Duration(Duration::from_secs(60)),
            ),
            Layer::new(Origin::Environment).with(
                key(SettingId::RetentionMaxAge),
                SettingValue::Duration(Duration::from_secs(120)),
            ),
        ]);
        assert_eq!(
            resolved.shadows_persisted(SettingId::RetentionMaxAge, None),
            Some(Origin::Environment)
        );
    }

    #[test]
    fn a_persisted_value_that_wins_is_not_reported_as_shadowed() {
        let resolved = ResolvedSettings::resolve(vec![
            Layer::new(Origin::File).with(
                key(SettingId::RetentionMaxAge),
                SettingValue::Duration(Duration::from_secs(60)),
            ),
            Layer::new(Origin::Persisted).with(
                key(SettingId::RetentionMaxAge),
                SettingValue::Duration(Duration::from_secs(120)),
            ),
        ]);
        assert_eq!(
            resolved.shadows_persisted(SettingId::RetentionMaxAge, None),
            None
        );
    }

    #[test]
    fn the_persisted_layer_refuses_bootstrap_settings() {
        let mut layer = Layer::new(Origin::Persisted);
        let error = layer
            .insert(
                key(SettingId::ServerHealthBind),
                SettingValue::Text("127.0.0.1:3000".to_owned()),
            )
            .expect_err("bootstrap settings are not persisted");
        assert_eq!(
            error,
            LayerError::BootstrapNotPersistable(SettingId::ServerHealthBind)
        );
    }

    #[test]
    fn the_persisted_layer_refuses_secrets() {
        let mut layer = Layer::new(Origin::Persisted);
        let error = layer
            .insert(
                scoped(SettingId::CameraRtspPassword, "camera-01"),
                SettingValue::Secret(Secret::new("hunter2".to_owned())),
            )
            .expect_err("secrets are not persisted in Phase 7");
        assert_eq!(
            error,
            LayerError::SecretNotPersistable(SettingId::CameraRtspPassword)
        );
    }

    #[test]
    fn the_environment_layer_accepts_secrets() {
        let mut layer = Layer::new(Origin::Environment);
        assert!(layer
            .insert(
                scoped(SettingId::CameraRtspPassword, "camera-01"),
                SettingValue::Secret(Secret::new("hunter2".to_owned())),
            )
            .is_ok());
    }

    #[test]
    fn a_layer_refuses_a_value_of_the_wrong_kind() {
        let mut layer = Layer::new(Origin::File);
        let error = layer
            .insert(
                key(SettingId::RetentionInterval),
                SettingValue::Unsigned(60),
            )
            .expect_err("an interval is a duration, not a count");
        assert!(matches!(error, LayerError::KindMismatch { .. }));
    }

    #[test]
    fn a_global_setting_cannot_be_camera_scoped() {
        assert!(SettingKey::scoped(SettingId::RetentionInterval, camera("camera-01")).is_err());
    }

    #[test]
    fn cameras_are_collected_from_every_layer() {
        let resolved = ResolvedSettings::resolve(vec![
            Layer::new(Origin::File).with(
                scoped(SettingId::CameraDetectionSampleFps, "camera-01"),
                SettingValue::Unsigned(5),
            ),
            Layer::new(Origin::Persisted).with(
                scoped(SettingId::CameraDetectionSampleFps, "camera-02"),
                SettingValue::Unsigned(7),
            ),
        ]);
        assert_eq!(resolved.cameras().len(), 2);
        assert!(resolved.cameras().contains(&camera("camera-01")));
        assert!(resolved.cameras().contains(&camera("camera-02")));
    }

    #[test]
    fn an_absent_setting_resolves_to_nothing_rather_than_a_default() {
        let resolved = ResolvedSettings::resolve(vec![Layer::new(Origin::Default)]);
        assert!(resolved
            .effective(SettingId::RetentionInterval, None)
            .is_none());
        assert!(resolved.missing().contains(&SettingId::RetentionInterval));
    }
}
