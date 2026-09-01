use std::ops::Deref;

use serde::{Deserialize, Deserializer, Serialize};

use crate::domain::{sha256, Actor, CommandId, CorrelationId, Sha256Digest};
use crate::policy::Capability;

pub const MAX_INPUT_BYTES: usize = 4096;
pub const DEFAULT_AUDIT_LIMIT: u16 = 20;
pub const MAX_AUDIT_LIMIT: u16 = 100;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct AuditLimit(u16);

impl AuditLimit {
    pub fn new(value: u16) -> Result<Self, AuditLimitError> {
        Self::try_from(value)
    }

    pub fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for AuditLimit {
    type Error = AuditLimitError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        if (1..=MAX_AUDIT_LIMIT).contains(&value) {
            Ok(Self(value))
        } else {
            Err(AuditLimitError::OutOfRange)
        }
    }
}

impl<'de> Deserialize<'de> for AuditLimit {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u16::deserialize(deserializer)?;
        Self::try_from(value).map_err(|_| serde::de::Error::custom("invalid audit limit"))
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum AuditLimitError {
    OutOfRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case", deny_unknown_fields)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    Unknown,
}

pub const MAX_SAFE_TOKEN_CHARS: usize = 64;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SafeTokenError {
    Empty,
    TooLong,
    UnsafeCharacter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SafeToken(String);

impl SafeToken {
    pub fn new(value: impl Into<String>) -> Result<Self, SafeTokenError> {
        let value = value.into();
        if value.is_empty() {
            return Err(SafeTokenError::Empty);
        }
        if value.chars().count() > MAX_SAFE_TOKEN_CHARS {
            return Err(SafeTokenError::TooLong);
        }
        if value.chars().any(|character| character.is_control() || character.is_whitespace()) {
            return Err(SafeTokenError::UnsafeCharacter);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for SafeToken {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl<'de> Deserialize<'de> for SafeToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(|_| serde::de::Error::custom("invalid safe token"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputRejection {
    pub category: InputRejectionCategory,
    pub safe_token: Option<SafeToken>,
    pub byte_length: usize,
    pub input_digest: Sha256Digest,
}

impl InputRejection {
    pub fn from_input(
        category: InputRejectionCategory,
        safe_token: Option<SafeToken>,
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
