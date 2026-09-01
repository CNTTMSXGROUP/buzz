//! Pure canonical serializer for NIP-PC Project State projections.

use std::collections::HashSet;

use nostr::{Event, EventId, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
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

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProjectionBody {
    v: u8,
    deleted: bool,
    project_tags: Vec<Vec<String>>,
}

/// Validate a relay-authored Project State event and return its CAS revision.
///
/// This checks the event signature and relay author, the exact canonical
/// envelope and compact JSON encoding, and the canonical ordering and shape of
/// all known effective Project tags. Unknown Project extension tags remain
/// valid, but may appear only after the known tag groups.
pub fn validate_project_state_projection(
    event: &Event,
    relay_pubkey: &PublicKey,
    coordinate: &str,
) -> Result<u64, ProjectStateError> {
    let (_, project_d) = parse_project_coordinate(coordinate)?;
    event
        .verify()
        .map_err(|error| ProjectStateError::State(format!("invalid event signature: {error}")))?;
    if event.kind.as_u16() as u32 != KIND_PROJECT_STATE || &event.pubkey != relay_pubkey {
        return Err(ProjectStateError::State(
            "event is not a Project State signed by the relay".into(),
        ));
    }

    let raw_tags: Vec<&[String]> = event.tags.iter().map(Tag::as_slice).collect();
    if raw_tags.len() != 5 {
        return Err(ProjectStateError::State(
            "projection must have exactly five tags".into(),
        ));
    }
    let projection_d = hex::encode(Sha256::digest(coordinate.as_bytes()));
    if raw_tags[0] != ["d", projection_d.as_str()]
        || raw_tags[1] != ["a", coordinate]
        || raw_tags[2].len() != 2
        || raw_tags[2][0] != "rev"
        || !canonical_positive_i64(&raw_tags[2][1])
        || !canonical_event_reference(raw_tags[3], "identity")
        || !canonical_event_reference(raw_tags[4], "change")
    {
        return Err(ProjectStateError::State(
            "projection envelope is not canonical".into(),
        ));
    }
    let revision = raw_tags[2][1]
        .parse::<u64>()
        .map_err(|error| ProjectStateError::State(format!("invalid revision: {error}")))?;

    let body: ProjectionBody = serde_json::from_str(&event.content)
        .map_err(|error| ProjectStateError::State(format!("invalid projection JSON: {error}")))?;
    let canonical = serde_json::to_string(&body)
        .map_err(|error| ProjectStateError::Encoding(error.to_string()))?;
    if body.v != 1 || canonical != event.content {
        return Err(ProjectStateError::State(
            "projection JSON is not supported canonical version 1".into(),
        ));
    }
    if body.deleted {
        if !body.project_tags.is_empty() {
            return Err(ProjectStateError::State(
                "deleted projection must have no Project tags".into(),
            ));
        }
    } else {
        validate_canonical_live_projection_tags(&body.project_tags, project_d)?;
    }
    Ok(revision)
}

fn canonical_positive_i64(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<i64>().is_ok_and(|value| value > 0)
}

fn canonical_event_reference(tag: &[String], marker: &str) -> bool {
    matches!(tag, [name, id, relay, tag_marker]
        if name == "e"
            && relay.is_empty()
            && tag_marker == marker
            && is_lower_hex64(id)
            && EventId::parse(id).is_ok())
}

fn is_lower_hex64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_canonical_live_projection_tags(
    tags: &[Vec<String>],
    project_d: &str,
) -> Result<(), ProjectStateError> {
    if !matches!(tags.first().map(Vec::as_slice), Some([name, value]) if name == "d" && value == project_d)
    {
        return Err(ProjectStateError::State(
            "live projection must begin with its matching Project d tag".into(),
        ));
    }

    let mut name = None;
    let mut description = None;
    let mut members = Vec::new();
    let mut home_channel = None;
    let mut related = Vec::new();
    let mut visibility = None;
    let mut extensions = Vec::new();
    let mut seen_member_coordinates = HashSet::new();

    for tag in &tags[1..] {
        let tag_name = tag.first().map(String::as_str);
        match tag_name {
            Some("d" | "auth") => {
                return Err(ProjectStateError::State(format!(
                    "{tag_name:?} is not valid in this projection position"
                )))
            }
            Some("name") => {
                validate_projection_singleton(tag, &name, 256)?;
                name = Some(tag.clone());
            }
            Some("description") => {
                validate_projection_singleton(tag, &description, 2048)?;
                description = Some(tag.clone());
            }
            Some("a") => {
                if !(2..=3).contains(&tag.len()) {
                    return Err(ProjectStateError::State(
                        "member a tag has invalid shape".into(),
                    ));
                }
                validate_member_coordinate(&tag[1])?;
                if !seen_member_coordinates.insert(tag[1].as_str()) {
                    return Err(ProjectStateError::State(
                        "member repository coordinate is duplicated".into(),
                    ));
                }
                members.push(tag.clone());
            }
            Some("buzz-channel") => {
                validate_projection_singleton(tag, &home_channel, 256)?;
                home_channel = Some(tag.clone());
            }
            Some("buzz-related-channel") => {
                let canonical_uuid = tag
                    .get(1)
                    .and_then(|value| Uuid::parse_str(value).ok())
                    .is_some_and(|uuid| uuid.to_string() == tag[1]);
                if tag.len() != 2 || !canonical_uuid {
                    return Err(ProjectStateError::State(
                        "related-channel tag is not a canonical UUID".into(),
                    ));
                }
                related.push(tag.clone());
            }
            Some("buzz-visibility") => {
                validate_projection_singleton(tag, &visibility, 256)?;
                visibility = Some(tag.clone());
            }
            _ => extensions.push(tag.clone()),
        }
    }
    if members.len() > MEMBER_CAP || related.len() > MEMBER_CAP {
        return Err(ProjectStateError::State(
            "projection collection exceeds the 64-member cap".into(),
        ));
    }
    let mut sorted_members = members.clone();
    sorted_members.sort_unstable();
    let mut sorted_related = related.clone();
    sorted_related.sort_unstable();
    sorted_related.dedup();
    if members != sorted_members || related != sorted_related {
        return Err(ProjectStateError::State(
            "projection collections are not in canonical order".into(),
        ));
    }
    if let Some(home) = home_channel.as_ref().and_then(|tag| tag.get(1)) {
        let canonical_home = Uuid::parse_str(home).ok().map(|uuid| uuid.to_string());
        if canonical_home
            .as_ref()
            .is_some_and(|home| related.iter().any(|tag| tag.get(1) == Some(home)))
        {
            return Err(ProjectStateError::State(
                "home channel cannot also be a related channel".into(),
            ));
        }
    }

    let mut canonical = vec![vec!["d".into(), project_d.into()]];
    canonical.extend(name);
    canonical.extend(description);
    canonical.extend(members);
    canonical.extend(home_channel);
    canonical.extend(related);
    canonical.extend(visibility);
    canonical.extend(extensions);
    if canonical != tags {
        return Err(ProjectStateError::State(
            "Project tags are not in canonical group order".into(),
        ));
    }
    Ok(())
}

fn validate_projection_singleton(
    tag: &[String],
    prior: &Option<Vec<String>>,
    max_len: usize,
) -> Result<(), ProjectStateError> {
    if prior.is_some() || tag.len() != 2 || tag[1].len() > max_len {
        return Err(ProjectStateError::State(format!(
            "{} tag has invalid shape, length, or cardinality",
            tag.first().map_or("metadata", String::as_str)
        )));
    }
    Ok(())
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

    fn signed_projection(relay: &Keys, template: &ProjectStateTemplate) -> Event {
        EventBuilder::new(template.kind, &template.content)
            .tags(template.tags.clone())
            .sign_with_keys(relay)
            .expect("sign test projection")
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

    #[test]
    fn validates_canonical_projection_and_rejects_noncanonical_envelope_or_json() {
        let (owner, identity) = fixture(vec![tag(&["d", "project"])]);
        let relay = Keys::generate();
        let coordinate = format!("30621:{}:project", owner.public_key().to_hex());
        let template = project_state_template(ProjectStateProjectionInput {
            coordinate: &coordinate,
            revision: 7,
            identity_event: &identity,
            change_event_id: &identity.id,
            deleted: false,
            related_channels: &[],
        })
        .unwrap();
        let event = signed_projection(&relay, &template);
        assert_eq!(
            validate_project_state_projection(&event, &relay.public_key(), &coordinate),
            Ok(7)
        );

        let mut reordered = template.clone();
        reordered.tags.swap(0, 1);
        assert!(validate_project_state_projection(
            &signed_projection(&relay, &reordered),
            &relay.public_key(),
            &coordinate,
        )
        .is_err());

        let mut spaced_json = template;
        spaced_json.content.insert(0, ' ');
        assert!(validate_project_state_projection(
            &signed_projection(&relay, &spaced_json),
            &relay.public_key(),
            &coordinate,
        )
        .is_err());
    }

    #[test]
    fn rejects_noncanonical_live_project_tags() {
        let (owner, identity) = fixture(vec![tag(&["d", "project"])]);
        let relay = Keys::generate();
        let coordinate = format!("30621:{}:project", owner.public_key().to_hex());
        let template = project_state_template(ProjectStateProjectionInput {
            coordinate: &coordinate,
            revision: 1,
            identity_event: &identity,
            change_event_id: &identity.id,
            deleted: false,
            related_channels: &[],
        })
        .unwrap();
        let invalid_tag_sets = [
            vec![vec!["d".into(), "other".into()]],
            vec![
                vec!["d".into(), "project".into()],
                vec!["d".into(), "project".into()],
            ],
            vec![
                vec!["d".into(), "project".into()],
                vec!["buzz-related-channel".into(), "not-a-uuid".into()],
            ],
            vec![
                vec!["d".into(), "project".into()],
                vec!["name".into(), "Name".into(), "extra".into()],
            ],
            vec![
                vec!["d".into(), "project".into()],
                vec!["description".into(), "Desc".into()],
                vec!["name".into(), "Name".into()],
            ],
        ];

        for project_tags in invalid_tag_sets {
            let mut invalid = template.clone();
            invalid.content = serde_json::to_string(&ProjectionBody {
                v: 1,
                deleted: false,
                project_tags,
            })
            .unwrap();
            assert!(validate_project_state_projection(
                &signed_projection(&relay, &invalid),
                &relay.public_key(),
                &coordinate,
            )
            .is_err());
        }
    }
}
