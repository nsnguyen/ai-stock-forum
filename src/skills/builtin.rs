use uuid::Uuid;

use crate::domain::{DomainError, SkillId, SkillVersionId};

use super::{
    ContentDigest, SkillDraft, SkillProvenance, SkillResource, SkillVersion, SkillsProjection,
};

struct ManifestDefinition {
    skill_id: u128,
    skill_version_id: u128,
    manifest_id: &'static str,
    expected_digest: &'static str,
    display_name: &'static str,
    description: &'static str,
    use_when: &'static str,
    tags: &'static [&'static str],
    instructions: &'static str,
    resource_name: &'static str,
    resource_body: &'static str,
}

const MANIFEST_DEFINITIONS: [ManifestDefinition; 4] = [
    ManifestDefinition {
        skill_id: 0x1001,
        skill_version_id: 0x2001,
        manifest_id: "evidence-review",
        expected_digest: "f9df82257cf896c23e65af4d4f45987d80fc2624fcade850f146e2f960e7d27c",
        display_name: "Evidence Review",
        description: "Evaluate claims using traceable evidence before drawing conclusions.",
        use_when: "Use when reviewing research, market commentary, or financial claims.",
        tags: &["evidence", "research"],
        instructions: "Decompose each conclusion into checkable claims.\nPrefer primary sources; label secondary summaries.\nRecord contradictions, confidence, and citations.\nDo not make unsupported financial conclusions.",
        resource_name: "Evidence checklist",
        resource_body: "Source date, issuer, scope, and limitations.",
    },
    ManifestDefinition {
        skill_id: 0x1002,
        skill_version_id: 0x2002,
        manifest_id: "filing-analysis",
        expected_digest: "00b1aeae3bd3be4706e88ec47cfbbe68296a60700e659ca277bc5fa5fa391099",
        display_name: "Filing Analysis",
        description: "Read company filings for material changes and reporting uncertainty.",
        use_when: "Use when comparing periodic filings or assessing reported financial results.",
        tags: &["filings", "financials"],
        instructions: "Compare periods and identify material changes.\nReview footnotes, cash flow, and accounting changes.\nState uncertainty and do not make unsupported financial conclusions.",
        resource_name: "Filing checklist",
        resource_body: "Compare periods, notes, cash flow, and accounting policy.",
    },
    ManifestDefinition {
        skill_id: 0x1003,
        skill_version_id: 0x2003,
        manifest_id: "catalyst-mapping",
        expected_digest: "78a982df8c3fc763d241f3850e6dd898f9344d0d3dbb00473658e0b4bd1e49a0",
        display_name: "Catalyst Mapping",
        description: "Map events that could change a market narrative or company outlook.",
        use_when: "Use when assessing time-bounded events and their possible market effects.",
        tags: &["catalysts", "events"],
        instructions: "Define the event window and transmission mechanism.\nEstimate probability, dependencies, evidence, and counter-catalysts.\nDo not make unsupported financial conclusions.",
        resource_name: "Catalyst checklist",
        resource_body: "Event, window, mechanism, evidence, dependencies, and counters.",
    },
    ManifestDefinition {
        skill_id: 0x1004,
        skill_version_id: 0x2004,
        manifest_id: "risk-checklist",
        expected_digest: "eddd5889fafe3e93f9ecbd1a767197acfa3f9468cfcbc8b8c2e5f3b2d89a2f1d",
        display_name: "Risk Checklist",
        description: "Surface downside factors before acting on an investment thesis.",
        use_when: "Use when reviewing market, company, or security-specific uncertainty.",
        tags: &["risk", "review"],
        instructions: "Cover market, company, financial, regulatory, execution, and liquidity risk.\nFor each, state likelihood, severity, indicators, and mitigants.\nDo not make unsupported financial conclusions.",
        resource_name: "Risk checklist",
        resource_body: "Likelihood, severity, indicators, and mitigants by risk category.",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinSkillManifest {
    manifest_id: String,
    manifest_version: u32,
    expected_digest: ContentDigest,
    skill: SkillVersion,
}

impl BuiltinSkillManifest {
    pub fn manifest_id(&self) -> &str {
        &self.manifest_id
    }

    pub fn manifest_version(&self) -> u32 {
        self.manifest_version
    }

    pub fn expected_digest(&self) -> &ContentDigest {
        &self.expected_digest
    }

    pub fn skill(&self) -> &SkillVersion {
        &self.skill
    }
}

pub fn builtin_manifests() -> Result<Vec<BuiltinSkillManifest>, DomainError> {
    MANIFEST_DEFINITIONS
        .iter()
        .map(BuiltinSkillManifest::from_definition)
        .collect()
}

pub fn reconcile_builtin_manifests(
    projection: &mut SkillsProjection,
    manifests: &[BuiltinSkillManifest],
) -> Result<Vec<SkillVersion>, DomainError> {
    let mut canonical_manifests = manifests.to_vec();
    canonical_manifests.sort_by_key(|manifest| {
        (
            manifest.skill.skill_id(),
            manifest.skill.version(),
            manifest.skill.skill_version_id(),
        )
    });

    for manifest in &canonical_manifests {
        let history = projection.history(manifest.skill.skill_id());
        let stored_v1 = history
            .iter()
            .find(|skill| skill.version() == manifest.skill.version());
        match stored_v1 {
            Some(existing) if builtin_manifest_matches(existing, manifest) => {}
            Some(_) => return Err(DomainError::InvalidSkillVersion),
            None if history.is_empty() => projection.insert(&manifest.skill)?,
            None => return Err(DomainError::InvalidSkillVersion),
        }
    }

    Ok(canonical_manifests
        .into_iter()
        .map(|manifest| manifest.skill)
        .collect())
}

impl BuiltinSkillManifest {
    fn from_definition(definition: &ManifestDefinition) -> Result<Self, DomainError> {
        let expected_digest = ContentDigest::parse(definition.expected_digest)?;
        let draft = SkillDraft::new(
            definition.display_name.to_owned(),
            definition.description.to_owned(),
            definition.use_when.to_owned(),
            definition
                .tags
                .iter()
                .map(|tag| (*tag).to_owned())
                .collect(),
            definition.instructions.to_owned(),
            vec![SkillResource {
                name: definition.resource_name.to_owned(),
                body: definition.resource_body.to_owned(),
            }],
        )?;
        let skill = SkillVersion::create(
            SkillId::from_uuid(Uuid::from_u128(definition.skill_id)),
            SkillVersionId::from_uuid(Uuid::from_u128(definition.skill_version_id)),
            0,
            SkillProvenance::BuiltIn {
                manifest_id: definition.manifest_id.to_owned(),
                manifest_version: 1,
                manifest_digest: expected_digest.clone(),
            },
            draft,
        )?;
        if skill.content_digest() != &expected_digest {
            return Err(DomainError::InvalidSkillVersion);
        }
        Ok(Self {
            manifest_id: definition.manifest_id.to_owned(),
            manifest_version: 1,
            expected_digest,
            skill,
        })
    }
}

fn builtin_manifest_matches(existing: &SkillVersion, manifest: &BuiltinSkillManifest) -> bool {
    let provenance_matches = matches!(
        existing.provenance(),
        SkillProvenance::BuiltIn {
            manifest_id,
            manifest_version,
            manifest_digest,
        } if manifest_id == manifest.manifest_id()
            && *manifest_version == manifest.manifest_version()
            && manifest_digest == manifest.expected_digest()
    );

    existing == &manifest.skill
        && existing.content_digest() == manifest.expected_digest()
        && provenance_matches
}
