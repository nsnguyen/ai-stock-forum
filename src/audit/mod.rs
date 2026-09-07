use serde::{Deserialize, Serialize};

use crate::{
    app::{ApplicationEvent, EventEnvelope, InputRejectionCategory, SafeToken},
    domain::{Actor, CorrelationId},
};

pub const MODULE_NAME: &str = "audit";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEntry {
    pub sequence: u64,
    pub occurred_at_ms: i64,
    pub actor: Actor,
    pub kind: String,
    pub correlation_id: CorrelationId,
    pub summary: String,
}

impl AuditEntry {
    pub fn from_event(envelope: &EventEnvelope) -> Self {
        Self {
            sequence: envelope.sequence,
            occurred_at_ms: envelope.occurred_at_ms,
            actor: envelope.actor.clone(),
            kind: envelope.event.kind().to_owned(),
            correlation_id: envelope.correlation_id,
            summary: summary(&envelope.event),
        }
    }
}

fn summary(event: &ApplicationEvent) -> String {
    match event {
        ApplicationEvent::InstallationInitialized { installation_id } => {
            format!("installation initialized: {installation_id}")
        }
        ApplicationEvent::ProcessSessionStarted { session_id } => {
            format!("process session started: {session_id}")
        }
        ApplicationEvent::PreviousSessionInterrupted { session_id } => {
            format!("previous session interrupted: {session_id}")
        }
        ApplicationEvent::HelpViewed => "help viewed".to_owned(),
        ApplicationEvent::StatusViewed => "status viewed".to_owned(),
        ApplicationEvent::SetupStatusViewed => "setup status viewed".to_owned(),
        ApplicationEvent::AuditTailViewed { limit } => {
            format!("audit tail viewed: {}", limit.get())
        }
        ApplicationEvent::CommandRejected { rejection } => format!(
            "command rejected: category={}, token={}, bytes={}",
            rejection_category(rejection.category),
            rejection
                .safe_token
                .as_ref()
                .map(defensive_escape)
                .as_deref()
                .unwrap_or("none"),
            rejection.byte_length,
        ),
        ApplicationEvent::ShutdownRequested => "shutdown requested".to_owned(),
        ApplicationEvent::ProcessSessionEnded { session_id, reason } => {
            format!("process session ended: {session_id}, reason={reason:?}")
        }
        ApplicationEvent::ProjectionRebuilt { through_sequence } => {
            format!("projection rebuilt through sequence {through_sequence}")
        }
        ApplicationEvent::AgentProfileCreated { profile } => {
            format!(
                "agent profile created: profile={}, version={}",
                profile.profile_id(),
                profile.version().get(),
            )
        }
        ApplicationEvent::AgentProfileVersionActivated {
            profile,
            previous_version_id,
        } => format!(
            "agent profile version activated: profile={}, version={}, previous_version={previous_version_id}",
            profile.profile_id(),
            profile.version().get(),
        ),
        ApplicationEvent::AgentProfilesListed {
            total_count,
            returned_count,
            truncated,
        } => format!(
            "agent profiles listed: total_count={total_count}, returned_count={returned_count}, truncated={truncated}"
        ),
        ApplicationEvent::AgentProfileViewed {
            profile_id,
            active_version_id,
        } => format!(
            "agent profile viewed: profile={profile_id}, active_version={active_version_id}"
        ),
        ApplicationEvent::AgentProfileHistoryViewed {
            profile_id,
            total_count,
            returned_count,
            truncated,
            active_version_id,
        } => format!(
            "agent profile history viewed: profile={profile_id}, active_version={active_version_id}, total_count={total_count}, returned_count={returned_count}, truncated={truncated}"
        ),
        ApplicationEvent::AgentProfileVersionViewed {
            profile_id,
            profile_version_id,
            version,
            predecessor_version_id,
        } => format!(
            "agent profile version viewed: profile={profile_id}, profile_version={profile_version_id}, version={}, predecessor={}",
            version.get(),
            predecessor_version_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "none".to_owned())
        ),
        ApplicationEvent::SkillCreated {
            skill,
            display_name,
            provenance,
        } => format!(
            "skill created: skill={}, version={}, name={}, provenance={provenance:?}",
            skill.skill_id(),
            skill.version().get(),
            defensive_text(display_name)
        ),
        ApplicationEvent::SkillVersionActivated {
            skill,
            previous_version_id,
            display_name,
            provenance,
        } => format!(
            "skill version activated: skill={}, version={}, previous_version={}, name={}, provenance={provenance:?}",
            skill.skill_id(),
            skill.version().get(),
            previous_version_id,
            defensive_text(display_name)
        ),
        ApplicationEvent::SkillsListed {
            total_count,
            returned_count,
            truncated,
            ..
        } => format!(
            "skills listed: total_count={total_count}, returned_count={returned_count}, truncated={truncated}"
        ),
        ApplicationEvent::SkillViewed {
            skill,
            display_name,
            provenance,
        } => format!(
            "skill viewed: skill={}, version={}, name={}, provenance={provenance:?}",
            skill.skill_id(),
            skill.version().get(),
            defensive_text(display_name)
        ),
        ApplicationEvent::SkillHistoryViewed {
            skill_id,
            active,
            total_count,
            returned_count,
            truncated,
            ..
        } => format!(
            "skill history viewed: skill={skill_id}, active_version={}, total_count={total_count}, returned_count={returned_count}, truncated={truncated}",
            active.skill_version_id()
        ),
        ApplicationEvent::SkillVersionViewed {
            skill,
            display_name,
            provenance,
            predecessor_version_id,
        } => format!(
            "skill version viewed: skill={}, version={}, predecessor={}, name={}, provenance={provenance:?}",
            skill.skill_id(),
            skill.version().get(),
            predecessor_version_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "none".to_owned()),
            defensive_text(display_name)
        ),
        ApplicationEvent::AgentSkillAssigned { profile, skill, .. } => format!(
            "agent skill assigned: profile={}, profile_version={}, skill={}, skill_version={}",
            profile.profile_id(),
            profile.profile_version_id(),
            skill.skill_id(),
            skill.skill_version_id()
        ),
        ApplicationEvent::AgentSkillUpgraded {
            profile,
            expected,
            replacement,
            ..
        } => format!(
            "agent skill upgraded: profile={}, profile_version={}, skill={}, from={}, to={}",
            profile.profile_id(),
            profile.profile_version_id(),
            replacement.skill_id(),
            expected.skill_version_id(),
            replacement.skill_version_id()
        ),
        ApplicationEvent::AgentSkillUnassigned {
            profile, expected, ..
        } => format!(
            "agent skill unassigned: profile={}, profile_version={}, skill={}, skill_version={}",
            profile.profile_id(),
            profile.profile_version_id(),
            expected.skill_id(),
            expected.skill_version_id()
        ),
    }
}

fn rejection_category(category: InputRejectionCategory) -> &'static str {
    match category {
        InputRejectionCategory::InvalidEncoding => "invalid_encoding",
        InputRejectionCategory::Oversized => "oversized",
        InputRejectionCategory::Malformed => "malformed",
        InputRejectionCategory::Unknown => "unknown",
    }
}

fn defensive_escape(token: &SafeToken) -> String {
    token.chars().flat_map(char::escape_default).collect()
}

fn defensive_text(value: &str) -> String {
    value.chars().flat_map(char::escape_default).collect()
}
