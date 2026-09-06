use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::domain::DomainError;

use super::{AgentBindings, AgentReadiness, AgentRole, ProfileField, canonicalize_visible_text};

const BINDING_REFERENCE_MAX_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct BindingReferenceId(String);

impl BindingReferenceId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let canonical = canonicalize_visible_text(
            ProfileField::ModelProvider,
            &value,
            BINDING_REFERENCE_MAX_BYTES,
            false,
        )?;
        if canonical != value || value.chars().any(char::is_whitespace) {
            return Err(DomainError::InvalidProfileField {
                field: "binding_reference_id",
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for BindingReferenceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceBindingRef {
    connection_id: BindingReferenceId,
    model_id: BindingReferenceId,
}

impl InferenceBindingRef {
    pub fn new(connection_id: BindingReferenceId, model_id: BindingReferenceId) -> Self {
        Self {
            connection_id,
            model_id,
        }
    }

    pub fn connection_id(&self) -> &BindingReferenceId {
        &self.connection_id
    }

    pub fn model_id(&self) -> &BindingReferenceId {
        &self.model_id
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineeringBindingRef {
    runtime_id: BindingReferenceId,
}

impl EngineeringBindingRef {
    pub fn new(runtime_id: BindingReferenceId) -> Self {
        Self { runtime_id }
    }

    pub fn runtime_id(&self) -> &BindingReferenceId {
        &self.runtime_id
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AgentBindingCatalogSnapshot {
    inference: BTreeSet<InferenceBindingRef>,
    engineering: BTreeSet<EngineeringBindingRef>,
}

impl AgentBindingCatalogSnapshot {
    pub fn new(
        inference: Vec<InferenceBindingRef>,
        engineering: Vec<EngineeringBindingRef>,
    ) -> Self {
        Self {
            inference: inference.into_iter().collect(),
            engineering: engineering.into_iter().collect(),
        }
    }

    pub fn inference(&self) -> impl ExactSizeIterator<Item = &InferenceBindingRef> {
        self.inference.iter()
    }

    pub fn engineering(&self) -> impl ExactSizeIterator<Item = &EngineeringBindingRef> {
        self.engineering.iter()
    }

    pub fn readiness(&self, role: AgentRole, bindings: &AgentBindings) -> AgentReadiness {
        if bindings.inference.is_none() && bindings.engineering.is_none() {
            return AgentReadiness::Unbound;
        }

        let inference_ready = bindings
            .inference
            .as_ref()
            .is_some_and(|binding| self.inference.contains(binding));
        let selected_engineering_available = bindings
            .engineering
            .as_ref()
            .is_none_or(|binding| self.engineering.contains(binding));
        let required_engineering_ready = role != AgentRole::Engineering
            || bindings
                .engineering
                .as_ref()
                .is_some_and(|binding| self.engineering.contains(binding));

        if inference_ready && selected_engineering_available && required_engineering_ready {
            AgentReadiness::Ready
        } else {
            AgentReadiness::BindingUnavailable
        }
    }
}
