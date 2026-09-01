//! Pure canonical serializer for NIP-PC Project State projections.

use std::collections::HashSet;

use nostr::{Event, EventId, Kind, Tag};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::kind::{KIND_PROJECT, KIND_PROJECT_STATE};

const PROJECT_D_MAX: usize = 1024;
const MEMBER_CAP: usize = 64;

/// Inputs needed to derive a canonical relay-signed Project State event body.
#[derive(Debug, Clone, Copy)]
pub struct ProjectStateProjectionInput<'a> {
    /// Canonical `30621:<owner-hex>:<project-d>` Project coordinate.
    pub coordinate: &'a str,
    /// Monotonic authoritative Project revision.
    pub revision: i64,
    /// Current owner-signed NIP-MP Project identity event.
    pub identity_event: &'a Event,
    /// Command or lifecycle event that produced this revision.
    pub change_event_id: &'a EventId,
    /// Whether the Project is currently deleted.
    pub deleted: bool,
    /// Authoritative related-channel set.
    pub related_channels: &'a [Uuid],
}

/// Unsigned, untimestamped fields for a relay-authored Project State event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectStateTemplate {
    /// Fixed NIP-PC Project State kind (`30623`).
    pub kind: Kind,
    /// Canonically ordered event tags.
    pub tags: Vec<Tag>,
    /// Stable compact JSON projection content.
    pub content: String,
}

/// A structural or canonicalization failure while deriving Project State.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProjectStateError {
    /// The Project coordinate is not canonical or does not match the identity.
    #[error("invalid Project coordinate: {0}")]
    Coordinate(String),
    /// The owner identity is not a valid NIP-MP Project event.
    #[error("invalid Project identity: {0}")]
    Identity(String),
    /// The relational revision or related-channel set is invalid.
    #[error("invalid Project state: {0}")]
    State(String),
    /// An output tag or JSON body could not be encoded.
    #[error("could not encode Project State: {0}")]
    Encoding(String),
}

/// Derive the canonical unsigned NIP-PC Project State event template.
///
/// Signing and strictly monotonic `created_at` allocation remain relay concerns.
pub fn project_state_template(
    input: ProjectStateProjectionInput<'_>,
) -> Result<ProjectStateTemplate, ProjectStateError> {
    let (owner, project_d) = parse_project_coordinate(input.coordinate)?;
    if input.revision < 1 {
        return Err(ProjectStateError::State(
            "revision must be a positive signed 64-bit integer".into(),
        ));
    }
    validate_identity(input.identity_event, owner, project_d)?;

    let mut related: Vec<String> = input.related_channels.iter().map(Uuid::to_string).collect();
    related.sort_unstable();
    related.dedup();
    if related.len() != input.related_channels.len() {
        return Err(ProjectStateError::State(
            "related channels must not contain duplicates".into(),
        ));
    }
    if related.len() > MEMBER_CAP {
        return Err(ProjectStateError::State(format!(
            "related channels exceed the {MEMBER_CAP}-channel cap"
        )));
    }

    let project_tags = if input.deleted {
        Vec::new()
    } else {
        canonical_live_tags(input.identity_event, project_d, &related)?
    };
    let body = ProjectionBody {
        v: 1,
        deleted: input.deleted,
        project_tags,
    };
    let content = serde_json::to_string(&body)
        .map_err(|error| ProjectStateError::Encoding(error.to_string()))?;

    let projection_d = hex::encode(Sha256::digest(input.coordinate.as_bytes()));
    let tags = vec![
        make_tag(vec!["d".into(), projection_d])?,
        make_tag(vec!["a".into(), input.coordinate.into()])?,
        make_tag(vec!["rev".into(), input.revision.to_string()])?,
        make_tag(vec![
            "e".into(),
            input.identity_event.id.to_hex(),
            String::new(),
            "identity".into(),
        ])?,
        make_tag(vec![
            "e".into(),
            input.change_event_id.to_hex(),
            String::new(),
            "change".into(),
        ])?,
    ];

    Ok(ProjectStateTemplate {
        kind: Kind::from(KIND_PROJECT_STATE as u16),
        tags,
        content,
    })
}

#[derive(Serialize)]
struct ProjectionBody {
    v: u8,
    deleted: bool,
    project_tags: Vec<Vec<String>>,
}

fn parse_project_coordinate(coordinate: &str) -> Result<(&str, &str), ProjectStateError> {
    let mut parts = coordinate.splitn(3, ':');
    let kind = parts.next();
    let owner = parts.next();
    let project_d = parts.next();
    if kind != Some("30621") {
        return Err(ProjectStateError::Coordinate("kind must be 30621".into()));
    }
    let owner = owner.ok_or_else(|| ProjectStateError::Coordinate("owner is missing".into()))?;
    if owner.len() != 64
        || !owner
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ProjectStateError::Coordinate(
            "owner must be 64 lowercase hexadecimal characters".into(),
        ));
    }
    let project_d = project_d
        .filter(|value| !value.is_empty() && value.len() <= PROJECT_D_MAX)
        .ok_or_else(|| {
            ProjectStateError::Coordinate(format!(
                "Project d must contain 1..={PROJECT_D_MAX} bytes"
            ))
        })?;
    Ok((owner, project_d))
}

fn validate_identity(event: &Event, owner: &str, project_d: &str) -> Result<(), ProjectStateError> {
    if event.kind.as_u16() as u32 != KIND_PROJECT {
        return Err(ProjectStateError::Identity(
            "event kind must be 30621".into(),
        ));
    }
    if event.pubkey.to_hex() != owner {
        return Err(ProjectStateError::Identity(
            "event signer does not match the coordinate owner".into(),
        ));
    }

    let d_tags: Vec<&[String]> = event
        .tags
        .iter()
        .map(Tag::as_slice)
        .filter(|parts| parts.first().is_some_and(|name| name == "d"))
        .collect();
    if d_tags.len() != 1 || d_tags[0].get(1).map(String::as_str) != Some(project_d) {
        return Err(ProjectStateError::Identity(
            "event must have exactly one d tag matching the coordinate".into(),
        ));
    }

    validate_nip_mp_tags(event)
}

fn validate_nip_mp_tags(event: &Event) -> Result<(), ProjectStateError> {
    let mut members = Vec::new();
    let mut singleton_counts = [0usize; 4];
    let singleton_names = ["name", "description", "buzz-channel", "buzz-visibility"];
    let singleton_limits = [256usize, 2048, 256, 256];

    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        let Some(name) = parts.first().map(String::as_str) else {
            continue;
        };
        if name == "a" {
            if !(2..=3).contains(&parts.len()) {
                return Err(ProjectStateError::Identity(
                    "member a tags must have two or three elements".into(),
                ));
            }
            members.push(parts[1].as_str());
        } else if let Some(index) = singleton_names.iter().position(|known| known == &name) {
            singleton_counts[index] += 1;
            if parts.get(1).map_or(0, String::len) > singleton_limits[index] {
                return Err(ProjectStateError::Identity(format!(
                    "{name} exceeds its NIP-MP byte limit"
                )));
            }
        }
    }
    if members.len() > MEMBER_CAP {
        return Err(ProjectStateError::Identity(
            "member a tags exceed the 64-member cap".into(),
        ));
    }
    if singleton_counts.iter().any(|count| *count > 1) {
        return Err(ProjectStateError::Identity(
            "NIP-MP singleton metadata tag is duplicated".into(),
        ));
    }
    let mut seen = HashSet::with_capacity(members.len());
    for coordinate in members {
        validate_member_coordinate(coordinate)?;
        if !seen.insert(coordinate) {
            return Err(ProjectStateError::Identity(
                "member repository coordinate is duplicated".into(),
            ));
        }
    }
    Ok(())
}

fn validate_member_coordinate(coordinate: &str) -> Result<(), ProjectStateError> {
    let mut parts = coordinate.splitn(3, ':');
    let valid = parts.next() == Some("30617")
        && parts.next().is_some_and(|owner| {
            owner.len() == 64
                && owner
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        && parts.next().is_some_and(|repo_d| !repo_d.is_empty());
    if !valid {
        return Err(ProjectStateError::Identity(
            "member repository coordinate is malformed".into(),
        ));
    }
    Ok(())
}

fn canonical_live_tags(
    identity: &Event,
    project_d: &str,
    related: &[String],
) -> Result<Vec<Vec<String>>, ProjectStateError> {
    let mut name = None;
    let mut description = None;
    let mut members = Vec::new();
    let mut home_channel = None;
    let mut visibility = None;
    let mut extensions = Vec::new();

    for tag in identity.tags.iter() {
        let parts = tag.as_slice().to_vec();
        match parts.first().map(String::as_str) {
            Some("d") => {}
            Some("name") => name = Some(parts),
            Some("description") => description = Some(parts),
            Some("a") => members.push(parts),
            Some("buzz-channel") => home_channel = Some(parts),
            Some("buzz-visibility") => visibility = Some(parts),
            Some("auth" | "buzz-related-channel") => {}
            _ => extensions.push(parts),
        }
    }
    members.sort_unstable();
    if let Some(home) = home_channel
        .as_ref()
        .and_then(|tag| tag.get(1))
        .and_then(|value| Uuid::parse_str(value).ok())
    {
        if related.iter().any(|channel| channel == &home.to_string()) {
            return Err(ProjectStateError::State(
                "the home channel cannot also be a related channel".into(),
            ));
        }
    }

    let mut tags = vec![vec!["d".into(), project_d.into()]];
    tags.extend(name);
    tags.extend(description);
    tags.extend(members);
    tags.extend(home_channel);
    tags.extend(
        related
            .iter()
            .map(|channel| vec!["buzz-related-channel".into(), channel.clone()]),
    );
    tags.extend(visibility);
    tags.extend(extensions);
    Ok(tags)
}

fn make_tag(parts: Vec<String>) -> Result<Tag, ProjectStateError> {
    Tag::parse(parts).map_err(|error| ProjectStateError::Encoding(error.to_string()))
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Tag};

    use super::*;

    fn tag(parts: &[&str]) -> Tag {
        Tag::parse(parts.iter().copied()).expect("valid test tag")
    }

    fn fixture(tags: Vec<Tag>) -> (Keys, Event) {
        let keys = Keys::generate();
        let event = EventBuilder::new(Kind::from(KIND_PROJECT as u16), "ignored")
            .tags(tags)
            .sign_with_keys(&keys)
            .expect("sign test identity");
        (keys, event)
    }

    #[test]
    fn emits_exact_canonical_live_projection() {
        let repo_b = format!("30617:{}:b", "b".repeat(64));
        let repo_a = format!("30617:{}:a", "a".repeat(64));
        let home = "11111111-1111-4111-8111-111111111111";
        let related_a = Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap();
        let related_b = Uuid::parse_str("33333333-3333-4333-8333-333333333333").unwrap();
        let (keys, identity) = fixture(vec![
            tag(&["x-ext", "one", "unchanged"]),
            tag(&["a", &repo_b, "wss://b.example"]),
            tag(&["auth", "secret"]),
            tag(&["description", "Desc"]),
            tag(&["d", "project:one"]),
            tag(&[
                "buzz-related-channel",
                "44444444-4444-4444-8444-444444444444",
            ]),
            tag(&["name", "Name"]),
            tag(&["a", &repo_a]),
            tag(&["buzz-visibility", "unlisted"]),
            tag(&["buzz-channel", home]),
            tag(&["z-ext", "two"]),
        ]);
        let change = EventBuilder::new(Kind::TextNote, "change")
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let coordinate = format!("30621:{}:project:one", keys.public_key().to_hex());

        let template = project_state_template(ProjectStateProjectionInput {
            coordinate: &coordinate,
            revision: 8,
            identity_event: &identity,
            change_event_id: &change.id,
            deleted: false,
            related_channels: &[related_b, related_a],
        })
        .unwrap();

        let expected_d = hex::encode(Sha256::digest(coordinate.as_bytes()));
        let raw_tags: Vec<Vec<String>> = template
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect();
        assert_eq!(template.kind.as_u16() as u32, KIND_PROJECT_STATE);
        assert_eq!(
            raw_tags,
            vec![
                vec!["d".into(), expected_d],
                vec!["a".into(), coordinate],
                vec!["rev".into(), "8".into()],
                vec![
                    "e".into(),
                    identity.id.to_hex(),
                    "".into(),
                    "identity".into()
                ],
                vec!["e".into(), change.id.to_hex(), "".into(), "change".into()],
            ]
        );
        assert_eq!(
            template.content,
            format!(
                "{{\"v\":1,\"deleted\":false,\"project_tags\":[[\"d\",\"project:one\"],[\"name\",\"Name\"],[\"description\",\"Desc\"],[\"a\",\"{repo_a}\"],[\"a\",\"{repo_b}\",\"wss://b.example\"],[\"buzz-channel\",\"{home}\"],[\"buzz-related-channel\",\"{related_a}\"],[\"buzz-related-channel\",\"{related_b}\"],[\"buzz-visibility\",\"unlisted\"],[\"x-ext\",\"one\",\"unchanged\"],[\"z-ext\",\"two\"]]}}"
            )
        );
    }

    #[test]
    fn emits_exact_tombstone_content() {
        let (keys, identity) = fixture(vec![tag(&["d", "gone"])]);
        let coordinate = format!("30621:{}:gone", keys.public_key().to_hex());
        let template = project_state_template(ProjectStateProjectionInput {
            coordinate: &coordinate,
            revision: 2,
            identity_event: &identity,
            change_event_id: &identity.id,
            deleted: true,
            related_channels: &[],
        })
        .unwrap();
        assert_eq!(
            template.content,
            "{\"v\":1,\"deleted\":true,\"project_tags\":[]}"
        );
    }

    #[test]
    fn hashes_max_length_colon_bearing_project_d() {
        let project_d = format!("prefix:{}", "x".repeat(PROJECT_D_MAX - 7));
        assert_eq!(project_d.len(), PROJECT_D_MAX);
        let (keys, identity) = fixture(vec![tag(&["d", &project_d])]);
        let coordinate = format!("30621:{}:{project_d}", keys.public_key().to_hex());

        let template = project_state_template(ProjectStateProjectionInput {
            coordinate: &coordinate,
            revision: 1,
            identity_event: &identity,
            change_event_id: &identity.id,
            deleted: false,
            related_channels: &[],
        })
        .unwrap();

        assert_eq!(
            template.tags[0].as_slice(),
            [
                "d",
                hex::encode(Sha256::digest(coordinate.as_bytes())).as_str()
            ]
        );
    }

    #[test]
    fn rejects_home_channel_as_related() {
        let home = "11111111-1111-4111-8111-111111111111";
        let (keys, identity) = fixture(vec![tag(&["d", "project"]), tag(&["buzz-channel", home])]);
        let coordinate = format!("30621:{}:project", keys.public_key().to_hex());
        let home = Uuid::parse_str(home).unwrap();

        assert!(matches!(
            project_state_template(ProjectStateProjectionInput {
                coordinate: &coordinate,
                revision: 1,
                identity_event: &identity,
                change_event_id: &identity.id,
                deleted: false,
                related_channels: &[home],
            }),
            Err(ProjectStateError::State(_))
        ));
    }

    #[test]
    fn rejects_mismatched_identity_and_noncanonical_state() {
        let (keys, identity) = fixture(vec![tag(&["d", "project"])]);
        let wrong_coordinate = format!("30621:{}:other", keys.public_key().to_hex());
        let duplicate = Uuid::parse_str("22222222-2222-4222-8222-222222222222").unwrap();

        assert!(matches!(
            project_state_template(ProjectStateProjectionInput {
                coordinate: &wrong_coordinate,
                revision: 1,
                identity_event: &identity,
                change_event_id: &identity.id,
                deleted: false,
                related_channels: &[],
            }),
            Err(ProjectStateError::Identity(_))
        ));
        let coordinate = format!("30621:{}:project", keys.public_key().to_hex());
        assert!(matches!(
            project_state_template(ProjectStateProjectionInput {
                coordinate: &coordinate,
                revision: 1,
                identity_event: &identity,
                change_event_id: &identity.id,
                deleted: false,
                related_channels: &[duplicate, duplicate],
            }),
            Err(ProjectStateError::State(_))
        ));
    }
}
