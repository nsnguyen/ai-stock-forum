use serde::{Deserialize, Serialize};

use crate::domain::{sha256, Actor, CommandId, CorrelationId, Sha256Digest};
use crate::policy::Capability;

pub const MAX_INPUT_BYTES: usize = 4096;
pub const DEFAULT_AUDIT_LIMIT: u16 = 20;
pub const MAX_AUDIT_LIMIT: u16 = 100;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuditLimit(u16);

impl AuditLimit {
    pub fn new(value: u16) -> Result<Self, AuditLimitError> {
        if (1..=MAX_AUDIT_LIMIT).contains(&value) {
            Ok(Self(value))
        } else {
            Err(AuditLimitError::OutOfRange)
        }
    }

    pub fn get(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AuditLimitError {
    OutOfRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplicationCommand {
    ShowHelp,
    ShowStatus,
    ShowSetupStatus,
    ShowAuditTail { limit: AuditLimit },
    RejectInput(InputRejection),
    RequestShutdown,
}

impl ApplicationCommand {
    pub fn audit_tail(limit: u16) -> Result<Self, AuditLimitError> {
        Ok(Self::ShowAuditTail {
            limit: AuditLimit::new(limit)?,
        })
    }

    pub fn required_capability(&self) -> Capability {
        match self {
            Self::ShowHelp | Self::RejectInput(_) => Capability::HelpRead,
            Self::ShowStatus => Capability::StatusRead,
            Self::ShowSetupStatus => Capability::SetupStatusRead,
            Self::ShowAuditTail { .. } => Capability::AuditRead,
            Self::RequestShutdown => Capability::Shutdown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandEnvelope {
    pub command_id: CommandId,
    pub correlation_id: CorrelationId,
    pub actor: Actor,
    pub command: ApplicationCommand,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputRejectionCategory {
    InvalidEncoding,
    Oversized,
    Malformed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputRejection {
    pub category: InputRejectionCategory,
    pub safe_token: Option<String>,
    pub byte_length: usize,
    pub input_digest: Sha256Digest,
}

impl InputRejection {
    pub fn from_input(
        category: InputRejectionCategory,
        safe_token: Option<String>,
        input: &[u8],
    ) -> Self {
        Self {
            category,
            safe_token,
            byte_length: input.len(),
            input_digest: sha256(input),
        }
    }
}
