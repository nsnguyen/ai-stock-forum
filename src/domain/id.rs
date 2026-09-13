use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(
            Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid)
            }

            pub fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                value.parse().map(Self)
            }
        }
    };
}

uuid_id!(InstallationId);
uuid_id!(SessionId);
uuid_id!(CommandId);
uuid_id!(EventId);
uuid_id!(CorrelationId);
uuid_id!(CausationId);
uuid_id!(ApprovalId);
uuid_id!(SetupDraftId);
uuid_id!(ConfigurationVersionId);
uuid_id!(AgentProfileId);
uuid_id!(AgentProfileVersionId);
uuid_id!(MemoryNamespaceId);
uuid_id!(ProfileReviewToken);
uuid_id!(SkillId);
uuid_id!(SkillVersionId);
uuid_id!(SkillReviewToken);
uuid_id!(MemoryEntryId);
uuid_id!(MemoryEntryVersionId);
uuid_id!(MemoryProposalId);
uuid_id!(MemoryReviewToken);
uuid_id!(EpisodicSummaryId);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Actor {
    Human,
    System,
    Agent(AgentProfileId),
}
