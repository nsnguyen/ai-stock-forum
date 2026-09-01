use serde::{Deserialize, Serialize};

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    HelpRead,
    StatusRead,
    SetupStatusRead,
    AuditRead,
    Shutdown,
    DiscussionRun,
    McpUse,
    EngineeringJobRun,
    GitMerge,
    GitPush,
    FinanceRecommendation,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Grant,
    Deny,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRule {
    pub effect: Effect,
    pub capability: Capability,
}

impl PolicyRule {
    pub fn new(effect: Effect, capability: Capability) -> Self {
        Self { effect, capability }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyDecision {
    Granted,
    Denied,
    DeniedByDefault,
}

pub fn evaluate(capability: Capability, rules: &[PolicyRule]) -> PolicyDecision {
    let mut granted = false;

    for rule in rules {
        if rule.capability != capability {
            continue;
        }

        match rule.effect {
            Effect::Deny => return PolicyDecision::Denied,
            Effect::Grant => granted = true,
        }
    }

    if granted {
        PolicyDecision::Granted
    } else {
        PolicyDecision::DeniedByDefault
    }
}
