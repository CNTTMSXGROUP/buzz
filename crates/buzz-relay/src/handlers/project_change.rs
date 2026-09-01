//! NIP-PC collaborative Project change command handling.

use std::collections::BTreeSet;
use std::sync::Arc;

use buzz_core::event::StoredEvent;
use buzz_core::tenant::TenantContext;
use buzz_db::project_state::{ProjectChangeApplyResult, ProjectRelatedChannelChange};
use nostr::Event;
use serde::Deserialize;
use uuid::Uuid;

use crate::state::AppState;

use super::event::dispatch_persistent_event;
use super::ingest::{IngestError, IngestResult};

const PROJECT_KIND: &str = "30621";
const MAX_PATCH_CHANNELS: usize = 64;

#[derive(Debug)]
struct ParsedChange {
    owner: Vec<u8>,
    delegated_owner: Option<Vec<u8>>,
    d_tag: String,
    expected_revision: i64,
    add: Vec<Uuid>,
    remove: Vec<Uuid>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangeContent {
    v: u8,
    patch: ChangePatch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangePatch {
    related_channels: RelatedChannelsPatch,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelatedChannelsPatch {
    add: Vec<String>,
    remove: Vec<String>,
}

fn invalid(message: impl Into<String>) -> IngestError {
    IngestError::Rejected(format!("invalid: {}", message.into()))
}

fn parse_revision(value: &str) -> Result<i64, IngestError> {
    if value.is_empty()
        || value.starts_with('0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(invalid(
            "expected-revision must be a canonical positive integer",
        ));
    }
    value
        .parse::<i64>()
        .ok()
        .filter(|revision| *revision > 0)
        .ok_or_else(|| invalid("expected-revision is out of range"))
}

fn parse_coordinate(value: &str) -> Result<(Vec<u8>, String), IngestError> {
    let mut parts = value.splitn(3, ':');
    let kind = parts.next();
    let owner_hex = parts.next();
    let d_tag = parts.next();
    let (Some(PROJECT_KIND), Some(owner_hex), Some(d_tag)) = (kind, owner_hex, d_tag) else {
        return Err(invalid("a tag must contain a canonical Project coordinate"));
    };
    if owner_hex.len() != 64
        || !owner_hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid(
            "Project owner must be 64 lowercase hexadecimal characters",
        ));
    }
    if d_tag.is_empty() || d_tag.len() > buzz_db::event::D_TAG_MAX_LEN {
        return Err(invalid("Project d tag is empty or too long"));
    }
    let owner = hex::decode(owner_hex).map_err(|_| invalid("invalid Project owner"))?;
    Ok((owner, d_tag.to_owned()))
}

fn parse_channels(values: Vec<String>) -> Result<Vec<Uuid>, IngestError> {
    if values.len() > MAX_PATCH_CHANNELS {
        return Err(invalid("related-channel patch exceeds 64 entries"));
    }
    values
        .into_iter()
        .map(|value| {
            Uuid::parse_str(&value)
                .ok()
                .filter(|channel| channel.to_string() == value)
                .ok_or_else(|| invalid("related channels must be canonical UUIDs"))
        })
        .collect()
}

fn verify_delegated_owner(event: &Event, auth_tag: &[String]) -> Result<Vec<u8>, IngestError> {
    let auth_tag_json = serde_json::to_string(auth_tag)
        .map_err(|error| invalid(format!("could not encode Project auth tag: {error}")))?;
    let owner = buzz_sdk::nip_oa::verify_auth_tag(&auth_tag_json, &event.pubkey)
        .map_err(|error| invalid(format!("invalid Project auth tag: {error}")))?;
    let conditions = &auth_tag[2];
    for clause in conditions.split('&').filter(|clause| !clause.is_empty()) {
        let satisfied = if let Some(value) = clause.strip_prefix("kind=") {
            value.parse::<u16>().ok() == Some(event.kind.as_u16())
        } else if let Some(value) = clause.strip_prefix("created_at<") {
            value
                .parse::<u64>()
                .is_ok_and(|bound| event.created_at.as_secs() < bound)
        } else if let Some(value) = clause.strip_prefix("created_at>") {
            value
                .parse::<u64>()
                .is_ok_and(|bound| event.created_at.as_secs() > bound)
        } else {
            false
        };
        if !satisfied {
            return Err(invalid(format!(
                "Project command does not satisfy auth condition {clause}"
            )));
        }
    }
    Ok(owner.to_bytes().to_vec())
}

fn parse(event: &Event) -> Result<ParsedChange, IngestError> {
    if !(2..=3).contains(&event.tags.len()) {
        return Err(invalid(
            "Project change must contain two required tags and at most one auth tag",
        ));
    }
    let mut coordinate = None;
    let mut revision = None;
    let mut auth_tag = None;
    for tag in event.tags.iter() {
        match tag.as_slice() {
            [name, value] if name == "a" && coordinate.is_none() => {
                coordinate = Some(parse_coordinate(value)?);
            }
            [name, value] if name == "expected-revision" && revision.is_none() => {
                revision = Some(parse_revision(value)?);
            }
            [name, _, _, _] if name == "auth" && auth_tag.is_none() => {
                auth_tag = Some(tag.as_slice().to_vec());
            }
            _ => return Err(invalid("Project change tags are malformed")),
        }
    }
    let (owner, d_tag) = coordinate.ok_or_else(|| invalid("missing a tag"))?;
    let expected_revision = revision.ok_or_else(|| invalid("missing expected-revision tag"))?;
    let content: ChangeContent =
        serde_json::from_str(&event.content).map_err(|error| invalid(error.to_string()))?;
    if content.v != 1 {
        return Err(invalid("unsupported Project change version"));
    }
    let add = parse_channels(content.patch.related_channels.add)?;
    let remove = parse_channels(content.patch.related_channels.remove)?;
    if add.is_empty() && remove.is_empty() {
        return Err(invalid("Project change must not be empty"));
    }
    let add_set = add.iter().copied().collect::<BTreeSet<_>>();
    let remove_set = remove.iter().copied().collect::<BTreeSet<_>>();
    if add_set.len() != add.len() || remove_set.len() != remove.len() {
        return Err(invalid("Project change contains a duplicate channel"));
    }
    if !add_set.is_disjoint(&remove_set) {
        return Err(invalid("Project change adds and removes the same channel"));
    }
    let delegated_owner = auth_tag
        .as_deref()
        .map(|tag| verify_delegated_owner(event, tag))
        .transpose()?;
    Ok(ParsedChange {
        owner,
        delegated_owner,
        d_tag,
        expected_revision,
        add,
        remove,
    })
}

pub(crate) async fn handle(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
) -> Result<IngestResult, IngestError> {
    let parsed = parse(event)?;
    let change = ProjectRelatedChannelChange {
        project_owner: &parsed.owner,
        delegated_owner: parsed.delegated_owner.as_deref(),
        project_d_tag: &parsed.d_tag,
        expected_revision: parsed.expected_revision,
        add: &parsed.add,
        remove: &parsed.remove,
    };
    let outcome = state
        .db
        .apply_project_related_channel_change(tenant.community(), event, change)
        .await
        .map_err(|error| IngestError::Internal(format!("error: apply Project change: {error}")))?;

    let message = match outcome {
        ProjectChangeApplyResult::Applied(snapshot) => {
            let stored = StoredEvent::new(event.clone(), None);
            dispatch_persistent_event(
                tenant,
                state,
                &stored,
                buzz_core::kind::KIND_PROJECT_CHANGE,
                &event.pubkey.to_hex(),
                None,
            )
            .await;
            format!("revision: {}", snapshot.revision)
        }
        ProjectChangeApplyResult::Duplicate { applied_revision } => {
            format!("duplicate: already applied at revision {applied_revision}")
        }
        ProjectChangeApplyResult::ProjectNotFound => {
            return Err(invalid("Project not found"));
        }
        ProjectChangeApplyResult::ProjectDeleted => {
            return Err(IngestError::Rejected("conflict: Project is deleted".into()));
        }
        ProjectChangeApplyResult::Forbidden => {
            return Err(IngestError::Rejected(
                "restricted: actor cannot manage this Project".into(),
            ));
        }
        ProjectChangeApplyResult::Conflict { current_revision } => {
            return Err(IngestError::Rejected(format!(
                "conflict: Project revision is {current_revision}"
            )));
        }
        ProjectChangeApplyResult::InvalidMutation(message)
        | ProjectChangeApplyResult::InvalidBase(message) => return Err(invalid(message)),
    };
    if let Err(error) = super::project_state_projection::publish_project_state_for_coordinate(
        tenant,
        state,
        &parsed.owner,
        &parsed.d_tag,
    )
    .await
    {
        tracing::warn!(
            project_owner = %hex::encode(&parsed.owner),
            project_d_tag = %parsed.d_tag,
            %error,
            "accepted Project change awaits projection repair"
        );
    }
    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message,
    })
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    use super::*;

    fn command(tags: Vec<Tag>, content: &str) -> Event {
        command_with(&Keys::generate(), tags, content, None)
    }

    fn command_with(actor: &Keys, tags: Vec<Tag>, content: &str, created_at: Option<u64>) -> Event {
        let builder = EventBuilder::new(Kind::Custom(47_010), content).tags(tags);
        let builder = match created_at {
            Some(timestamp) => builder.custom_created_at(Timestamp::from(timestamp)),
            None => builder,
        };
        builder.sign_with_keys(actor).expect("sign test command")
    }

    fn auth_tag(owner: &Keys, actor: &Keys, conditions: &str) -> Tag {
        let json = buzz_sdk::nip_oa::compute_auth_tag(owner, &actor.public_key(), conditions)
            .expect("compute auth tag");
        buzz_sdk::nip_oa::parse_auth_tag(&json).expect("parse auth tag")
    }

    fn valid_content() -> &'static str {
        r#"{"v":1,"patch":{"related_channels":{"add":["11111111-1111-4111-8111-111111111111"],"remove":[]}}}"#
    }

    fn legacy_command(tags: Vec<Tag>, content: &str) -> Event {
        EventBuilder::new(Kind::Custom(47_010), content)
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign test command")
    }

    fn valid_tags() -> Vec<Tag> {
        vec![
            Tag::parse(["a", &format!("30621:{}:project:x", "a".repeat(64))])
                .expect("coordinate tag"),
            Tag::parse(["expected-revision", "1"]).expect("revision tag"),
        ]
    }

    #[test]
    fn parses_exact_v1_envelope() {
        let event = command(valid_tags(), valid_content());
        let parsed = parse(&event).expect("valid command");
        assert_eq!(parsed.expected_revision, 1);
        assert_eq!(parsed.d_tag, "project:x");
        assert_eq!(parsed.add.len(), 1);
    }

    #[test]
    fn verifies_current_auth_tag_without_rewriting_actor() {
        let actor = Keys::generate();
        let owner = Keys::generate();
        let mut tags = valid_tags();
        tags.push(auth_tag(&owner, &actor, "kind=47010&created_at>99"));
        let event = command_with(&actor, tags, valid_content(), Some(100));

        let parsed = parse(&event).expect("valid delegated command");
        assert_eq!(event.pubkey, actor.public_key());
        assert_eq!(
            parsed.delegated_owner,
            Some(owner.public_key().to_bytes().to_vec())
        );
    }

    #[test]
    fn rejects_forged_and_stale_auth_tags() {
        let actor = Keys::generate();
        let owner = Keys::generate();
        let other_agent = Keys::generate();

        let mut forged = valid_tags();
        forged.push(auth_tag(&owner, &other_agent, "kind=47010"));
        assert!(parse(&command_with(&actor, forged, valid_content(), None)).is_err());

        for conditions in ["kind=1", "created_at<100"] {
            let mut stale = valid_tags();
            stale.push(auth_tag(&owner, &actor, conditions));
            assert!(parse(&command_with(&actor, stale, valid_content(), Some(100))).is_err());
        }
    }

    #[test]
    fn rejects_noncanonical_and_ambiguous_inputs() {
        let cases = [
            (
                valid_tags(),
                r#"{"v":2,"patch":{"related_channels":{"add":[],"remove":[]}}}"#,
            ),
            (
                valid_tags(),
                r#"{"v":1,"patch":{"related_channels":{"add":["11111111-1111-4111-8111-111111111111"],"remove":["11111111-1111-4111-8111-111111111111"]}}}"#,
            ),
            (
                valid_tags(),
                r#"{"v":1,"patch":{"related_channels":{"add":[],"remove":[],"future":true}}}"#,
            ),
        ];
        for (tags, content) in cases {
            assert!(parse(&legacy_command(tags, content)).is_err());
        }

        let mut bad_revision = valid_tags();
        bad_revision[1] = Tag::parse(["expected-revision", "01"]).expect("tag");
        assert!(parse(&legacy_command(bad_revision, "{}")).is_err());

        let mut extra_tag = valid_tags();
        extra_tag.push(Tag::parse(["h", &Uuid::new_v4().to_string()]).expect("tag"));
        assert!(parse(&legacy_command(extra_tag, "{}")).is_err());

        let actor = Keys::generate();
        let owner = Keys::generate();
        let mut duplicate_auth = valid_tags();
        duplicate_auth.push(auth_tag(&owner, &actor, ""));
        duplicate_auth.push(auth_tag(&owner, &actor, ""));
        assert!(parse(&command_with(&actor, duplicate_auth, valid_content(), None)).is_err());
    }
}
