//! Transactional persistence for collaborative Project state.

use std::collections::BTreeSet;

use buzz_core::kind::{
    event_kind_u32, KIND_DELETION, KIND_PROJECT, KIND_PROJECT_CHANGE, KIND_PROJECT_STATE,
};
use buzz_core::project_state::{
    project_state_template, ProjectStateProjectionInput, ProjectStateTemplate,
};
use buzz_core::{CommunityId, StoredEvent};
use chrono::{DateTime, Utc};
use nostr::{Event, EventId};
use sqlx::Row;
use uuid::Uuid;

use crate::event::insert_event_in_transaction;
use crate::replaceable::{
    event_replacement_lock_key, ParameterizedReplacePrecondition, ParameterizedReplaceStatus,
};
use crate::{Db, DbError, Result};

const RELATED_CHANNEL_CAP: usize = 64;

/// A validated Project related-channel mutation supplied by the relay parser.
#[derive(Clone, Copy, Debug)]
pub struct ProjectRelatedChannelChange<'a> {
    /// Owner pubkey from the canonical kind:30621 coordinate.
    pub project_owner: &'a [u8],
    /// Verbatim `d` value from the canonical kind:30621 coordinate.
    pub project_d_tag: &'a str,
    /// Relational revision the actor observed.
    pub expected_revision: i64,
    /// Channels to add to the effective related-channel set.
    pub add: &'a [Uuid],
    /// Channels to remove from the effective related-channel set.
    pub remove: &'a [Uuid],
}

/// Canonical effective state returned for relay projection signing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectStateSnapshot {
    /// Monotonic relational revision produced by the command.
    pub revision: i64,
    /// Current owner-signed kind:30621 identity event id.
    pub identity_event_id: Vec<u8>,
    /// Command event id that produced this revision.
    pub change_event_id: Vec<u8>,
    /// Sorted effective related-channel set.
    pub related_channels: Vec<Uuid>,
}

/// Outcome of applying one Project related-channel command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectChangeApplyResult {
    /// The command and its effective state committed atomically.
    Applied(ProjectStateSnapshot),
    /// This exact command already committed, possibly before later revisions.
    Duplicate {
        /// Revision originally produced by the command.
        applied_revision: i64,
    },
    /// No live owner-signed Project identity exists at the coordinate.
    ProjectNotFound,
    /// The materialized Project is deleted and awaits owner recreation.
    ProjectDeleted,
    /// The actor is neither the Project owner nor an active home-channel owner/admin.
    Forbidden,
    /// The expected revision is stale, or lifecycle state changed first.
    Conflict {
        /// Current materialized revision.
        current_revision: i64,
    },
    /// The patch is invalid against current effective state.
    InvalidMutation(String),
    /// The current owner identity cannot be materialized as canonical v1 state.
    InvalidBase(String),
}

/// Coherent Project state awaiting a relay-signed kind:30623 projection.
#[derive(Clone, Debug)]
pub struct ProjectStateProjectionCandidate {
    community_id: CommunityId,
    template: ProjectStateTemplate,
    previous_created_at: Option<u64>,
    project_owner: Vec<u8>,
    project_d_tag: String,
    revision: i64,
    identity_event_id: Vec<u8>,
    change_event_id: Vec<u8>,
    observed_projected_revision: i64,
    observed_projection_pubkey: Option<Vec<u8>>,
    projection_pubkey: Vec<u8>,
}

impl ProjectStateProjectionCandidate {
    /// Community containing the Project.
    #[must_use]
    pub const fn community_id(&self) -> CommunityId {
        self.community_id
    }

    /// Unsigned canonical fields the relay must timestamp and sign.
    #[must_use]
    pub const fn template(&self) -> &ProjectStateTemplate {
        &self.template
    }

    /// Timestamp of the current live projection for this relay key, if any.
    #[must_use]
    pub const fn previous_created_at(&self) -> Option<u64> {
        self.previous_created_at
    }
}

/// Outcome of committing a relay-signed Project State projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectStateProjectionCommitResult {
    /// The projection and durable retry marker committed atomically.
    Committed,
    /// Project state or projection ownership changed after the candidate loaded.
    Stale,
}

/// Result category for an owner identity or deletion lifecycle event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectLifecycleStatus {
    /// The event changed authoritative Project state.
    Applied,
    /// The event was newly stored but had no effect on the current Project head.
    NoEffect,
    /// The exact event was already stored.
    Duplicate,
    /// A newer owner identity already dominates the submitted identity.
    Superseded,
}

/// Atomic persistence result for a Project lifecycle event.
#[derive(Clone, Debug)]
pub struct ProjectLifecycleApplyResult {
    /// Stored representation used by relay dispatch when the event was inserted.
    pub event: StoredEvent,
    /// Whether and how authoritative Project state changed.
    pub status: ProjectLifecycleStatus,
    /// Canonical state after an applied lifecycle mutation.
    pub snapshot: Option<ProjectStateSnapshot>,
}

impl ProjectLifecycleApplyResult {
    /// Whether this call newly persisted the submitted event.
    #[must_use]
    pub fn was_inserted(&self) -> bool {
        matches!(
            self.status,
            ProjectLifecycleStatus::Applied | ProjectLifecycleStatus::NoEffect
        )
    }
}
fn tag_parts(tag: &serde_json::Value) -> Option<Vec<&str>> {
    tag.as_array()?
        .iter()
        .map(serde_json::Value::as_str)
        .collect()
}

fn parse_base_state(
    tags: &serde_json::Value,
) -> std::result::Result<(Option<Uuid>, BTreeSet<Uuid>), String> {
    let mut home = None;
    let mut related = BTreeSet::new();
    for tag in tags.as_array().ok_or("Project tags are not an array")? {
        let Some(parts) = tag_parts(tag) else {
            return Err("Project contains a non-string tag".into());
        };
        match parts.as_slice() {
            ["buzz-channel", value] => {
                let channel = Uuid::parse_str(value)
                    .ok()
                    .filter(|id| id.to_string() == *value);
                home = channel;
            }
            ["buzz-related-channel", value] => {
                let channel = Uuid::parse_str(value)
                    .ok()
                    .filter(|id| id.to_string() == *value)
                    .ok_or("Project contains a non-canonical related channel")?;
                if !related.insert(channel) {
                    return Err("Project contains a duplicate related channel".into());
                }
            }
            ["buzz-related-channel", ..] => {
                return Err("Project contains a malformed related-channel tag".into());
            }
            _ => {}
        }
    }
    if related.len() > RELATED_CHANNEL_CAP {
        return Err("Project contains more than 64 related channels".into());
    }
    if home.is_some_and(|channel| related.contains(&channel)) {
        return Err("Project home channel cannot also be related".into());
    }
    Ok((home, related))
}

fn validate_patch(change: ProjectRelatedChannelChange<'_>) -> Option<String> {
    if change.project_owner.len() != 32 {
        return Some("Project owner must be 32 bytes".into());
    }
    if change.project_d_tag.is_empty() || change.project_d_tag.len() > crate::event::D_TAG_MAX_LEN {
        return Some("Project d tag is empty or too long".into());
    }
    if change.expected_revision < 1 {
        return Some("expected revision must be positive".into());
    }
    if change.add.is_empty() && change.remove.is_empty() {
        return Some("Project change must not be empty".into());
    }
    if change.add.len() > RELATED_CHANNEL_CAP || change.remove.len() > RELATED_CHANNEL_CAP {
        return Some("Project change exceeds the 64-channel patch bound".into());
    }
    let add = change.add.iter().copied().collect::<BTreeSet<_>>();
    let remove = change.remove.iter().copied().collect::<BTreeSet<_>>();
    if add.len() != change.add.len() || remove.len() != change.remove.len() {
        return Some("Project change contains a duplicate channel".into());
    }
    if !add.is_disjoint(&remove) {
        return Some("Project change adds and removes the same channel".into());
    }
    None
}

async fn replace_related_channels(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    community_id: CommunityId,
    owner: &[u8],
    d_tag: &str,
    related: &BTreeSet<Uuid>,
) -> Result<()> {
    sqlx::query(
        "DELETE FROM project_related_channels WHERE community_id=$1 \
         AND project_owner=$2 AND project_d_tag=$3",
    )
    .bind(community_id.as_uuid())
    .bind(owner)
    .bind(d_tag)
    .execute(&mut **tx)
    .await?;
    for channel in related {
        sqlx::query(
            "INSERT INTO project_related_channels \
             (community_id, project_owner, project_d_tag, channel_id) VALUES ($1,$2,$3,$4)",
        )
        .bind(community_id.as_uuid())
        .bind(owner)
        .bind(d_tag)
        .bind(channel)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

impl Db {
    /// Atomically accept an owner-signed Project identity and materialize it.
    ///
    /// A newer identity is a full recovery snapshot. Existing relational
    /// revisions advance rather than resetting, including recreation after a
    /// deletion. Duplicate and superseded identities leave state unchanged.
    pub async fn apply_project_identity_event(
        &self,
        community_id: CommunityId,
        event: &Event,
    ) -> Result<ProjectLifecycleApplyResult> {
        if event_kind_u32(event) != KIND_PROJECT {
            return Err(DbError::InvalidData(
                "Project identity persistence requires kind 30621".into(),
            ));
        }
        let d_tag = crate::event::extract_d_tag(event).unwrap_or_default();
        if d_tag.is_empty() || d_tag.len() > crate::event::D_TAG_MAX_LEN {
            return Err(DbError::InvalidData("invalid Project d tag".into()));
        }
        let tags = serde_json::to_value(&event.tags)?;
        let (_, related) = parse_base_state(&tags).map_err(DbError::InvalidData)?;
        let owner = event.pubkey.to_bytes();

        let mut tx = self.begin_transaction().await?;
        self.deletion_store()
            .guard_transaction(&mut tx, community_id)
            .await?;
        let coordinate_lock = event_replacement_lock_key(
            community_id,
            KIND_PROJECT as i32,
            owner.as_slice(),
            Some(d_tag.as_bytes()),
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(coordinate_lock)
            .execute(&mut *tx)
            .await?;
        let current_head = sqlx::query(
            "SELECT revision, deleted, last_event_id FROM project_state_heads \
             WHERE community_id=$1 AND project_owner=$2 AND project_d_tag=$3 FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(owner.as_slice())
        .bind(&d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(head) = current_head
            .as_ref()
            .filter(|head| head.get::<bool, _>("deleted"))
        {
            let tombstone_event_id: Vec<u8> = head.try_get("last_event_id")?;
            let tombstone_created_at: DateTime<Utc> =
                sqlx::query_scalar("SELECT created_at FROM events WHERE community_id=$1 AND id=$2")
                    .bind(community_id.as_uuid())
                    .bind(tombstone_event_id)
                    .fetch_optional(&mut *tx)
                    .await?
                    .ok_or_else(|| {
                        DbError::InvalidData(
                            "Project tombstone event is missing from history".into(),
                        )
                    })?;
            let identity_created_at =
                DateTime::<Utc>::from_timestamp(event.created_at.as_secs() as i64, 0)
                    .ok_or_else(|| DbError::InvalidTimestamp(event.created_at.as_secs() as i64))?;
            // A tombstone dominates every identity in its second. Recreating a
            // Project requires an unambiguously later owner event.
            if identity_created_at <= tombstone_created_at {
                tx.rollback().await?;
                return Ok(ProjectLifecycleApplyResult {
                    event: StoredEvent::new(event.clone(), None),
                    status: ProjectLifecycleStatus::Superseded,
                    snapshot: None,
                });
            }
        }
        let had_unmaterialized_identity: bool = if current_head.is_none() {
            sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM events WHERE community_id=$1 AND kind=$2 \
                 AND pubkey=$3 AND d_tag=$4 AND deleted_at IS NULL)",
            )
            .bind(community_id.as_uuid())
            .bind(KIND_PROJECT as i32)
            .bind(owner.as_slice())
            .bind(&d_tag)
            .fetch_one(&mut *tx)
            .await?
        } else {
            false
        };
        let persisted = self
            .replace_parameterized_event_in_transaction(
                &mut tx,
                community_id,
                event,
                &d_tag,
                None,
                ParameterizedReplacePrecondition::Unconditional,
            )
            .await?;
        if persisted.status != ParameterizedReplaceStatus::Inserted {
            tx.rollback().await?;
            let status = match persisted.status {
                ParameterizedReplaceStatus::Duplicate => ProjectLifecycleStatus::Duplicate,
                _ => ProjectLifecycleStatus::Superseded,
            };
            return Ok(ProjectLifecycleApplyResult {
                event: persisted.event,
                status,
                snapshot: None,
            });
        }

        let revision = match current_head {
            None if had_unmaterialized_identity => 2,
            None => 1,
            Some(head) => head
                .try_get::<i64, _>("revision")?
                .checked_add(1)
                .ok_or_else(|| DbError::InvalidData("Project revision overflow".into()))?,
        };
        sqlx::query(
            "INSERT INTO project_state_heads \
               (community_id, project_owner, project_d_tag, revision, deleted, identity_event_id, last_event_id) \
             VALUES ($1,$2,$3,$4,FALSE,$5,$5) \
             ON CONFLICT (community_id, project_owner, project_d_tag) DO UPDATE SET \
               revision=EXCLUDED.revision, deleted=FALSE, identity_event_id=EXCLUDED.identity_event_id, \
               last_event_id=EXCLUDED.last_event_id, updated_at=transaction_timestamp()",
        )
        .bind(community_id.as_uuid())
        .bind(owner.as_slice())
        .bind(&d_tag)
        .bind(revision)
        .bind(event.id.as_bytes().as_slice())
        .execute(&mut *tx)
        .await?;
        replace_related_channels(&mut tx, community_id, owner.as_slice(), &d_tag, &related).await?;

        let snapshot = ProjectStateSnapshot {
            revision,
            identity_event_id: event.id.as_bytes().to_vec(),
            change_event_id: event.id.as_bytes().to_vec(),
            related_channels: related.into_iter().collect(),
        };
        tx.commit().await?;
        Ok(ProjectLifecycleApplyResult {
            event: persisted.event,
            status: ProjectLifecycleStatus::Applied,
            snapshot: Some(snapshot),
        })
    }

    /// Atomically store an owner-authorized NIP-09 coordinate deletion and,
    /// when it covers the live identity, advance Project state to a tombstone.
    pub async fn apply_project_deletion_event(
        &self,
        community_id: CommunityId,
        event: &Event,
        project_owner: &[u8],
        project_d_tag: &str,
        expected_identity_event_id: Option<&[u8]>,
    ) -> Result<ProjectLifecycleApplyResult> {
        if event_kind_u32(event) != KIND_DELETION
            || project_owner.len() != 32
            || project_d_tag.is_empty()
            || project_d_tag.len() > crate::event::D_TAG_MAX_LEN
            || expected_identity_event_id.is_some_and(|event_id| event_id.len() != 32)
        {
            return Err(DbError::InvalidData("invalid Project deletion".into()));
        }
        let deletion_created_at =
            DateTime::<Utc>::from_timestamp(event.created_at.as_secs() as i64, 0)
                .ok_or_else(|| DbError::InvalidTimestamp(event.created_at.as_secs() as i64))?;
        let mut tx = self.begin_transaction().await?;
        self.deletion_store()
            .guard_transaction(&mut tx, community_id)
            .await?;
        let lock = event_replacement_lock_key(
            community_id,
            KIND_PROJECT as i32,
            project_owner,
            Some(project_d_tag.as_bytes()),
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock)
            .execute(&mut *tx)
            .await?;
        let (stored_event, inserted) =
            insert_event_in_transaction(&mut tx, community_id, event, None).await?;
        if !inserted {
            tx.rollback().await?;
            return Ok(ProjectLifecycleApplyResult {
                event: stored_event,
                status: ProjectLifecycleStatus::Duplicate,
                snapshot: None,
            });
        }

        let live = sqlx::query(
            "SELECT id, created_at FROM events WHERE community_id=$1 AND kind=$2 \
             AND pubkey=$3 AND d_tag=$4 AND deleted_at IS NULL \
             ORDER BY created_at DESC, id ASC LIMIT 1 FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(KIND_PROJECT as i32)
        .bind(project_owner)
        .bind(project_d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(live) = live else {
            tx.commit().await?;
            return Ok(ProjectLifecycleApplyResult {
                event: stored_event,
                status: ProjectLifecycleStatus::NoEffect,
                snapshot: None,
            });
        };
        let identity_event_id: Vec<u8> = live.try_get("id")?;
        let identity_created_at: DateTime<Utc> = live.try_get("created_at")?;
        if identity_created_at > deletion_created_at
            || expected_identity_event_id
                .is_some_and(|expected| expected != identity_event_id.as_slice())
        {
            tx.commit().await?;
            return Ok(ProjectLifecycleApplyResult {
                event: stored_event,
                status: ProjectLifecycleStatus::NoEffect,
                snapshot: None,
            });
        }

        let head = sqlx::query(
            "SELECT revision, deleted, identity_event_id FROM project_state_heads \
             WHERE community_id=$1 AND project_owner=$2 AND project_d_tag=$3 FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(project_owner)
        .bind(project_d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        let revision = if let Some(head) = head {
            let materialized: Vec<u8> = head.try_get("identity_event_id")?;
            if head.try_get::<bool, _>("deleted")? || materialized != identity_event_id {
                return Err(DbError::InvalidData(
                    "Project lifecycle state does not match live identity".into(),
                ));
            }
            head.try_get::<i64, _>("revision")?
                .checked_add(1)
                .ok_or_else(|| DbError::InvalidData("Project revision overflow".into()))?
        } else {
            // Materialize the pre-existing identity at revision 1 before applying
            // its first relational lifecycle event.
            2
        };

        sqlx::query(
            "UPDATE events SET deleted_at=transaction_timestamp() WHERE community_id=$1 \
             AND id=$2 AND deleted_at IS NULL AND created_at <= $3",
        )
        .bind(community_id.as_uuid())
        .bind(&identity_event_id)
        .bind(deletion_created_at)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO project_state_heads \
               (community_id, project_owner, project_d_tag, revision, deleted, identity_event_id, last_event_id) \
             VALUES ($1,$2,$3,$4,TRUE,$5,$6) \
             ON CONFLICT (community_id, project_owner, project_d_tag) DO UPDATE SET \
               revision=EXCLUDED.revision, deleted=TRUE, last_event_id=EXCLUDED.last_event_id, \
               updated_at=transaction_timestamp()",
        )
        .bind(community_id.as_uuid())
        .bind(project_owner)
        .bind(project_d_tag)
        .bind(revision)
        .bind(&identity_event_id)
        .bind(event.id.as_bytes().as_slice())
        .execute(&mut *tx)
        .await?;
        replace_related_channels(
            &mut tx,
            community_id,
            project_owner,
            project_d_tag,
            &BTreeSet::new(),
        )
        .await?;
        let snapshot = ProjectStateSnapshot {
            revision,
            identity_event_id,
            change_event_id: event.id.as_bytes().to_vec(),
            related_channels: Vec::new(),
        };
        tx.commit().await?;
        Ok(ProjectLifecycleApplyResult {
            event: stored_event,
            status: ProjectLifecycleStatus::Applied,
            snapshot: Some(snapshot),
        })
    }

    /// Authorize and atomically apply one v1 Project related-channel command.
    ///
    /// Lock order is community deletion fence, Project coordinate, then current
    /// home-channel membership. The accepted command event, normalized state,
    /// CAS advance, and durable replay receipt share one transaction.
    pub async fn apply_project_related_channel_change(
        &self,
        community_id: CommunityId,
        event: &Event,
        change: ProjectRelatedChannelChange<'_>,
    ) -> Result<ProjectChangeApplyResult> {
        if event_kind_u32(event) != KIND_PROJECT_CHANGE {
            return Err(DbError::InvalidData(
                "Project change persistence requires kind 47010".into(),
            ));
        }
        if let Some(message) = validate_patch(change) {
            return Ok(ProjectChangeApplyResult::InvalidMutation(message));
        }

        let mut tx = self.begin_transaction().await?;
        self.deletion_store()
            .guard_transaction(&mut tx, community_id)
            .await?;
        let coordinate_lock = event_replacement_lock_key(
            community_id,
            KIND_PROJECT as i32,
            change.project_owner,
            Some(change.project_d_tag.as_bytes()),
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(coordinate_lock)
            .execute(&mut *tx)
            .await?;

        let receipt = sqlx::query(
            "SELECT project_owner, project_d_tag, applied_revision \
             FROM project_change_receipts WHERE community_id=$1 AND command_event_id=$2",
        )
        .bind(community_id.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(receipt) = receipt {
            let owner: Vec<u8> = receipt.try_get("project_owner")?;
            let d_tag: String = receipt.try_get("project_d_tag")?;
            if owner != change.project_owner || d_tag != change.project_d_tag {
                return Err(DbError::InvalidData(
                    "Project command receipt coordinate mismatch".into(),
                ));
            }
            return Ok(ProjectChangeApplyResult::Duplicate {
                applied_revision: receipt.try_get("applied_revision")?,
            });
        }

        let head = sqlx::query(
            "SELECT revision, deleted, identity_event_id FROM project_state_heads \
             WHERE community_id=$1 AND project_owner=$2 AND project_d_tag=$3 FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(change.project_owner)
        .bind(change.project_d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        if head.as_ref().is_some_and(|row| row.get("deleted")) {
            return Ok(ProjectChangeApplyResult::ProjectDeleted);
        }

        let base = sqlx::query(
            "SELECT id, tags FROM events WHERE community_id=$1 AND kind=$2 AND pubkey=$3 \
               AND d_tag=$4 AND deleted_at IS NULL ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(community_id.as_uuid())
        .bind(KIND_PROJECT as i32)
        .bind(change.project_owner)
        .bind(change.project_d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(base) = base else {
            return Ok(ProjectChangeApplyResult::ProjectNotFound);
        };
        let identity_event_id: Vec<u8> = base.try_get("id")?;
        let base_tags: serde_json::Value = base.try_get("tags")?;
        let (home_channel, base_related) = match parse_base_state(&base_tags) {
            Ok(state) => state,
            Err(message) => return Ok(ProjectChangeApplyResult::InvalidBase(message)),
        };

        let current_revision = if let Some(head) = head {
            let revision: i64 = head.try_get("revision")?;
            let materialized_identity: Vec<u8> = head.try_get("identity_event_id")?;
            if materialized_identity != identity_event_id {
                return Ok(ProjectChangeApplyResult::Conflict {
                    current_revision: revision,
                });
            }
            revision
        } else {
            sqlx::query(
                "INSERT INTO project_state_heads \
                   (community_id, project_owner, project_d_tag, revision, identity_event_id, last_event_id) \
                 VALUES ($1,$2,$3,1,$4,$4)",
            )
            .bind(community_id.as_uuid())
            .bind(change.project_owner)
            .bind(change.project_d_tag)
            .bind(&identity_event_id)
            .execute(&mut *tx)
            .await?;
            for channel in &base_related {
                sqlx::query(
                    "INSERT INTO project_related_channels \
                       (community_id, project_owner, project_d_tag, channel_id) VALUES ($1,$2,$3,$4)",
                )
                .bind(community_id.as_uuid())
                .bind(change.project_owner)
                .bind(change.project_d_tag)
                .bind(channel)
                .execute(&mut *tx)
                .await?;
            }
            1
        };

        let actor = event.pubkey.to_bytes();
        if actor.as_slice() != change.project_owner {
            let Some(home_channel) = home_channel else {
                return Ok(ProjectChangeApplyResult::Forbidden);
            };
            crate::channel_members::acquire_channel_membership_lock(
                &mut tx,
                community_id,
                home_channel,
            )
            .await?;
            let role: Option<String> = sqlx::query_scalar(
                "SELECT member.role::text FROM channel_members member \
                 JOIN channels channel ON channel.community_id=member.community_id \
                   AND channel.id=member.channel_id \
                 WHERE member.community_id=$1 AND member.channel_id=$2 AND member.pubkey=$3 \
                   AND member.removed_at IS NULL AND channel.archived_at IS NULL \
                   AND channel.deleted_at IS NULL FOR SHARE OF channel, member",
            )
            .bind(community_id.as_uuid())
            .bind(home_channel)
            .bind(actor.as_slice())
            .fetch_optional(&mut *tx)
            .await?;
            if !matches!(role.as_deref(), Some("owner" | "admin")) {
                return Ok(ProjectChangeApplyResult::Forbidden);
            }
        }
        if change.expected_revision != current_revision {
            return Ok(ProjectChangeApplyResult::Conflict { current_revision });
        }

        let rows = sqlx::query_scalar::<_, Uuid>(
            "SELECT channel_id FROM project_related_channels \
             WHERE community_id=$1 AND project_owner=$2 AND project_d_tag=$3 ORDER BY channel_id",
        )
        .bind(community_id.as_uuid())
        .bind(change.project_owner)
        .bind(change.project_d_tag)
        .fetch_all(&mut *tx)
        .await?;
        let mut related = rows.into_iter().collect::<BTreeSet<_>>();
        for channel in change.add {
            if Some(*channel) == home_channel || !related.insert(*channel) {
                return Ok(ProjectChangeApplyResult::InvalidMutation(
                    "cannot add the home channel or an already-related channel".into(),
                ));
            }
        }
        for channel in change.remove {
            if !related.remove(channel) {
                return Ok(ProjectChangeApplyResult::InvalidMutation(
                    "cannot remove a channel that is not related".into(),
                ));
            }
        }
        if related.len() > RELATED_CHANNEL_CAP {
            return Ok(ProjectChangeApplyResult::InvalidMutation(
                "effective Project state exceeds 64 related channels".into(),
            ));
        }

        let (_, inserted) = insert_event_in_transaction(&mut tx, community_id, event, None).await?;
        if !inserted {
            return Err(DbError::InvalidData(
                "Project command event exists without a durable receipt".into(),
            ));
        }
        for channel in change.remove {
            sqlx::query(
                "DELETE FROM project_related_channels WHERE community_id=$1 \
                   AND project_owner=$2 AND project_d_tag=$3 AND channel_id=$4",
            )
            .bind(community_id.as_uuid())
            .bind(change.project_owner)
            .bind(change.project_d_tag)
            .bind(channel)
            .execute(&mut *tx)
            .await?;
        }
        for channel in change.add {
            sqlx::query(
                "INSERT INTO project_related_channels \
                   (community_id, project_owner, project_d_tag, channel_id) VALUES ($1,$2,$3,$4)",
            )
            .bind(community_id.as_uuid())
            .bind(change.project_owner)
            .bind(change.project_d_tag)
            .bind(channel)
            .execute(&mut *tx)
            .await?;
        }
        let revision: i64 = sqlx::query_scalar(
            "UPDATE project_state_heads SET revision=revision+1, last_event_id=$4, \
               updated_at=transaction_timestamp() WHERE community_id=$1 AND project_owner=$2 \
               AND project_d_tag=$3 AND revision=$5 AND revision < 9223372036854775807 \
             RETURNING revision",
        )
        .bind(community_id.as_uuid())
        .bind(change.project_owner)
        .bind(change.project_d_tag)
        .bind(event.id.as_bytes().as_slice())
        .bind(current_revision)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| DbError::InvalidData("Project revision overflow or CAS failure".into()))?;
        sqlx::query(
            "INSERT INTO project_change_receipts \
               (community_id, command_event_id, project_owner, project_d_tag, applied_revision) \
             VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(community_id.as_uuid())
        .bind(event.id.as_bytes().as_slice())
        .bind(change.project_owner)
        .bind(change.project_d_tag)
        .bind(revision)
        .execute(&mut *tx)
        .await?;

        let snapshot = ProjectStateSnapshot {
            revision,
            identity_event_id,
            change_event_id: event.id.as_bytes().to_vec(),
            related_channels: related.into_iter().collect(),
        };
        tx.commit().await?;
        Ok(ProjectChangeApplyResult::Applied(snapshot))
    }

    /// Load coherent Project states that require publication by `projection_pubkey`.
    ///
    /// A Project is pending when its relational revision has not been projected,
    /// or when a relay-key rotation requires the same revision to be republished.
    /// Every returned candidate is assembled while holding the Project coordinate
    /// lock; [`Self::commit_project_state_projection`] rejects it if state changes
    /// after this method returns.
    pub async fn load_pending_project_state_projections(
        &self,
        projection_pubkey: &[u8],
        limit: i64,
    ) -> Result<Vec<ProjectStateProjectionCandidate>> {
        if projection_pubkey.len() != 32 {
            return Err(DbError::InvalidData(
                "Project projection pubkey must be 32 bytes".into(),
            ));
        }
        if !(1..=1_000).contains(&limit) {
            return Err(DbError::InvalidData(
                "Project projection candidate limit must be between 1 and 1000".into(),
            ));
        }
        let coordinates = sqlx::query(
            "SELECT head.community_id, head.project_owner, head.project_d_tag \
             FROM project_state_heads head JOIN communities community ON community.id=head.community_id \
             WHERE community.deletion_state='active' AND community.deleted_at IS NULL \
               AND (head.projected_revision < head.revision \
                    OR head.projection_pubkey IS DISTINCT FROM $1) \
             ORDER BY head.updated_at, head.community_id, head.project_owner, head.project_d_tag \
             LIMIT $2",
        )
        .bind(projection_pubkey)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let mut candidates = Vec::with_capacity(coordinates.len());
        for coordinate in coordinates {
            let community_id = CommunityId::from_uuid(coordinate.try_get("community_id")?);
            let project_owner: Vec<u8> = coordinate.try_get("project_owner")?;
            let project_d_tag: String = coordinate.try_get("project_d_tag")?;
            if let Some(candidate) = self
                .load_pending_project_state_projection(
                    community_id,
                    &project_owner,
                    &project_d_tag,
                    projection_pubkey,
                )
                .await?
            {
                candidates.push(candidate);
            }
        }
        Ok(candidates)
    }

    /// Load one coherent Project state when it requires publication by
    /// `projection_pubkey`.
    ///
    /// The returned candidate is safe to sign outside the transaction because
    /// [`Self::commit_project_state_projection`] revalidates every observed
    /// head field while holding the same Project coordinate lock.
    pub async fn load_pending_project_state_projection(
        &self,
        community_id: CommunityId,
        project_owner: &[u8],
        project_d_tag: &str,
        projection_pubkey: &[u8],
    ) -> Result<Option<ProjectStateProjectionCandidate>> {
        let mut tx = self.begin_transaction().await?;
        let coordinate_lock = event_replacement_lock_key(
            community_id,
            KIND_PROJECT as i32,
            project_owner,
            Some(project_d_tag.as_bytes()),
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(coordinate_lock)
            .execute(&mut *tx)
            .await?;
        let head = sqlx::query(
            "SELECT revision, projected_revision, projection_pubkey, deleted, \
                    identity_event_id, last_event_id \
             FROM project_state_heads WHERE community_id=$1 AND project_owner=$2 \
               AND project_d_tag=$3 FOR SHARE",
        )
        .bind(community_id.as_uuid())
        .bind(project_owner)
        .bind(project_d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(head) = head else {
            return Ok(None);
        };
        let revision: i64 = head.try_get("revision")?;
        let projected_revision: i64 = head.try_get("projected_revision")?;
        let observed_projection_pubkey: Option<Vec<u8>> = head.try_get("projection_pubkey")?;
        if projected_revision == revision
            && observed_projection_pubkey.as_deref() == Some(projection_pubkey)
        {
            return Ok(None);
        }
        let identity_event_id: Vec<u8> = head.try_get("identity_event_id")?;
        let change_event_id: Vec<u8> = head.try_get("last_event_id")?;
        let deleted: bool = head.try_get("deleted")?;
        let identity_row = sqlx::query(
            "SELECT id, pubkey, created_at, kind, tags, content, sig, received_at, channel_id \
             FROM events WHERE community_id=$1 AND id=$2 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(community_id.as_uuid())
        .bind(&identity_event_id)
        .fetch_optional(&mut *tx)
        .await?;
        let identity_event = match identity_row {
            Some(row) => {
                crate::event::row_to_stored_event(row)?
                    .ok_or_else(|| DbError::InvalidData("invalid Project identity event".into()))?
                    .event
            }
            None => {
                return Err(DbError::InvalidData(
                    "Project state references a missing identity event".into(),
                ))
            }
        };
        let related_channels = sqlx::query_scalar::<_, Uuid>(
            "SELECT channel_id FROM project_related_channels WHERE community_id=$1 \
               AND project_owner=$2 AND project_d_tag=$3 ORDER BY channel_id",
        )
        .bind(community_id.as_uuid())
        .bind(project_owner)
        .bind(project_d_tag)
        .fetch_all(&mut *tx)
        .await?;
        let change_id = EventId::from_hex(&hex::encode(&change_event_id))
            .map_err(|error| DbError::InvalidData(format!("invalid Project change id: {error}")))?;
        let coordinate = format!("30621:{}:{project_d_tag}", hex::encode(project_owner));
        let template = project_state_template(ProjectStateProjectionInput {
            coordinate: &coordinate,
            revision,
            identity_event: &identity_event,
            change_event_id: &change_id,
            deleted,
            related_channels: &related_channels,
        })
        .map_err(|error| DbError::InvalidData(error.to_string()))?;
        let projection_d_tag = projection_d_tag(&template)?;
        let previous_created_at: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
            "SELECT created_at FROM events WHERE community_id=$1 AND kind=$2 AND pubkey=$3 \
               AND d_tag=$4 AND deleted_at IS NULL ORDER BY created_at DESC, id ASC LIMIT 1",
        )
        .bind(community_id.as_uuid())
        .bind(KIND_PROJECT_STATE as i32)
        .bind(projection_pubkey)
        .bind(projection_d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        let previous_created_at = previous_created_at
            .map(|value| {
                u64::try_from(value.timestamp()).map_err(|_| {
                    DbError::InvalidData("Project projection has a negative timestamp".into())
                })
            })
            .transpose()?;
        tx.commit().await?;
        Ok(Some(ProjectStateProjectionCandidate {
            community_id,
            template,
            previous_created_at,
            project_owner: project_owner.to_vec(),
            project_d_tag: project_d_tag.to_owned(),
            revision,
            identity_event_id,
            change_event_id,
            observed_projected_revision: projected_revision,
            observed_projection_pubkey,
            projection_pubkey: projection_pubkey.to_vec(),
        }))
    }

    /// Atomically publish a relay-signed projection and advance its retry marker.
    ///
    /// The candidate must be passed back unchanged. The Project coordinate is
    /// locked before the projection replacement coordinate, and every observed
    /// head field is revalidated before the event is stored.
    pub async fn commit_project_state_projection(
        &self,
        candidate: &ProjectStateProjectionCandidate,
        event: &Event,
    ) -> Result<ProjectStateProjectionCommitResult> {
        event.verify().map_err(|error| {
            DbError::InvalidData(format!("invalid signed Project projection: {error}"))
        })?;
        if event_kind_u32(event) != KIND_PROJECT_STATE
            || event.pubkey.to_bytes().as_slice() != candidate.projection_pubkey
            || event.tags.as_slice() != candidate.template.tags
            || event.content != candidate.template.content
        {
            return Err(DbError::InvalidData(
                "signed Project projection does not match its candidate".into(),
            ));
        }
        if candidate
            .previous_created_at
            .is_some_and(|previous| event.created_at.as_secs() <= previous)
        {
            return Err(DbError::InvalidData(
                "Project projection timestamp must advance the live projection".into(),
            ));
        }
        let projection_d_tag = projection_d_tag(&candidate.template)?;
        let mut tx = self.begin_transaction().await?;
        self.deletion_store()
            .guard_transaction(&mut tx, candidate.community_id)
            .await?;
        let coordinate_lock = event_replacement_lock_key(
            candidate.community_id,
            KIND_PROJECT as i32,
            &candidate.project_owner,
            Some(candidate.project_d_tag.as_bytes()),
        );
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(coordinate_lock)
            .execute(&mut *tx)
            .await?;
        let head = sqlx::query(
            "SELECT revision, projected_revision, projection_pubkey, identity_event_id, \
                    last_event_id FROM project_state_heads WHERE community_id=$1 \
               AND project_owner=$2 AND project_d_tag=$3 FOR UPDATE",
        )
        .bind(candidate.community_id.as_uuid())
        .bind(&candidate.project_owner)
        .bind(&candidate.project_d_tag)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(head) = head else {
            return Ok(ProjectStateProjectionCommitResult::Stale);
        };
        let projection_pubkey: Option<Vec<u8>> = head.try_get("projection_pubkey")?;
        let matches_candidate = head.try_get::<i64, _>("revision")? == candidate.revision
            && head.try_get::<i64, _>("projected_revision")?
                == candidate.observed_projected_revision
            && projection_pubkey == candidate.observed_projection_pubkey
            && head.try_get::<Vec<u8>, _>("identity_event_id")? == candidate.identity_event_id
            && head.try_get::<Vec<u8>, _>("last_event_id")? == candidate.change_event_id;
        if !matches_candidate {
            return Ok(ProjectStateProjectionCommitResult::Stale);
        }
        let replaced = self
            .replace_parameterized_event_in_transaction(
                &mut tx,
                candidate.community_id,
                event,
                projection_d_tag,
                None,
                ParameterizedReplacePrecondition::Unconditional,
            )
            .await?;
        if replaced.status != ParameterizedReplaceStatus::Inserted {
            tx.rollback().await?;
            return Ok(ProjectStateProjectionCommitResult::Stale);
        }
        sqlx::query(
            "UPDATE project_state_heads SET projected_revision=$4, projection_pubkey=$5, \
               updated_at=transaction_timestamp() WHERE community_id=$1 AND project_owner=$2 \
               AND project_d_tag=$3",
        )
        .bind(candidate.community_id.as_uuid())
        .bind(&candidate.project_owner)
        .bind(&candidate.project_d_tag)
        .bind(candidate.revision)
        .bind(&candidate.projection_pubkey)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(ProjectStateProjectionCommitResult::Committed)
    }
}

fn projection_d_tag(template: &ProjectStateTemplate) -> Result<&str> {
    template
        .tags
        .iter()
        .find_map(|tag| match tag.as_slice() {
            [name, value] if name == "d" => Some(value.as_str()),
            _ => None,
        })
        .ok_or_else(|| DbError::InvalidData("Project projection template has no d tag".into()))
}

#[cfg(test)]
mod tests {
    use buzz_core::channel::{ChannelType, ChannelVisibility};
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use sqlx::postgres::PgPoolOptions;
    use sqlx::PgPool;

    use super::*;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1 -- local test-only credentials

    async fn scratch_db(prefix: &str) -> (PgPool, PgPool, String) {
        let database_url =
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.into());
        let admin = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .expect("connect admin database");
        let name = format!("{prefix}_{}", Uuid::new_v4().simple());
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
            .execute(&admin)
            .await
            .expect("create scratch database");
        let slash = database_url.rfind('/').expect("database URL path");
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&format!("{}/{name}", &database_url[..slash]))
            .await
            .expect("connect scratch database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("migrate scratch database");
        (admin, pool, name)
    }

    async fn drop_scratch_db(admin: PgPool, pool: PgPool, name: &str) {
        pool.close().await;
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
        )))
        .execute(&admin)
        .await
        .expect("drop scratch database");
    }

    fn command(
        actor: &Keys,
        owner: &Keys,
        slug: &str,
        expected: i64,
        add: &[Uuid],
        remove: &[Uuid],
    ) -> Event {
        let content = serde_json::json!({
            "v": 1,
            "patch": {"related_channels": {"add": add, "remove": remove}}
        })
        .to_string();
        EventBuilder::new(Kind::Custom(KIND_PROJECT_CHANGE as u16), content)
            .tags([
                Tag::parse([
                    "a",
                    &format!("30621:{}:{slug}", owner.public_key().to_hex()),
                ])
                .expect("a tag"),
                Tag::parse(["expected-revision", &expected.to_string()]).expect("revision tag"),
            ])
            .sign_with_keys(actor)
            .expect("sign command")
    }

    fn projection(
        candidate: &ProjectStateProjectionCandidate,
        relay: &Keys,
        created_at: u64,
    ) -> Event {
        EventBuilder::new(
            candidate.template().kind,
            candidate.template().content.clone(),
        )
        .tags(candidate.template().tags.clone())
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(relay)
        .expect("sign projection")
    }

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transaction_enforces_authority_cas_replay_and_atomic_attribution() {
        let (admin_pool, pool, scratch_name) = scratch_db("project_state").await;
        let db = Db::from_pool(pool.clone());
        let community = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("project-state-{}.example", community.as_uuid()))
            .execute(&pool)
            .await
            .expect("insert community");

        let owner = Keys::generate();
        let admin = Keys::generate();
        let member = Keys::generate();
        let home = Uuid::new_v4();
        crate::channel::create_channel_with_id(
            &pool,
            community,
            home,
            "project-home",
            ChannelType::Stream,
            ChannelVisibility::Private,
            None,
            owner.public_key().to_bytes().as_slice(),
            None,
        )
        .await
        .expect("create home channel");
        for (keys, role) in [(&admin, "admin"), (&member, "member")] {
            sqlx::query(
                "INSERT INTO channel_members (community_id,channel_id,pubkey,role,invited_by) \
                 VALUES ($1,$2,$3,$4::member_role,$5)",
            )
            .bind(community.as_uuid())
            .bind(home)
            .bind(keys.public_key().to_bytes().as_slice())
            .bind(role)
            .bind(owner.public_key().to_bytes().as_slice())
            .execute(&pool)
            .await
            .expect("insert member");
        }
        let seeded = Uuid::new_v4();
        let identity_time = Utc::now().timestamp() as u64 - 100;
        let base = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags([
                Tag::parse(["d", "shared"]).expect("d tag"),
                Tag::parse(["buzz-channel", &home.to_string()]).expect("home tag"),
                Tag::parse(["buzz-related-channel", &seeded.to_string()]).expect("related tag"),
            ])
            .custom_created_at(Timestamp::from(identity_time))
            .sign_with_keys(&owner)
            .expect("sign Project");
        let initial = db
            .apply_project_identity_event(community, &base)
            .await
            .expect("insert Project identity");
        assert_eq!(initial.status, ProjectLifecycleStatus::Applied);
        assert_eq!(
            initial.snapshot.as_ref().map(|state| state.revision),
            Some(1)
        );

        let added = Uuid::new_v4();
        let first = command(&admin, &owner, "shared", 1, &[added], &[]);
        let owner_bytes = owner.public_key().to_bytes();
        let applied = db
            .apply_project_related_channel_change(
                community,
                &first,
                ProjectRelatedChannelChange {
                    project_owner: owner_bytes.as_slice(),
                    project_d_tag: "shared",
                    expected_revision: 1,
                    add: &[added],
                    remove: &[],
                },
            )
            .await
            .expect("apply admin command");
        let ProjectChangeApplyResult::Applied(snapshot) = applied else {
            panic!("expected applied command: {applied:?}");
        };
        assert_eq!(snapshot.revision, 2);
        assert_eq!(snapshot.related_channels.len(), 2);
        assert!(snapshot.related_channels.contains(&seeded));
        assert!(snapshot.related_channels.contains(&added));
        let later = command(&owner, &owner, "shared", 2, &[], &[seeded]);
        assert!(matches!(
            db.apply_project_related_channel_change(
                community,
                &later,
                ProjectRelatedChannelChange {
                    project_owner: owner_bytes.as_slice(),
                    project_d_tag: "shared",
                    expected_revision: 2,
                    add: &[],
                    remove: &[seeded],
                },
            )
            .await
            .expect("apply owner command"),
            ProjectChangeApplyResult::Applied(ProjectStateSnapshot { revision: 3, .. })
        ));
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &first,
                ProjectRelatedChannelChange {
                    project_owner: owner_bytes.as_slice(),
                    project_d_tag: "shared",
                    expected_revision: 1,
                    add: &[added],
                    remove: &[],
                },
            )
            .await
            .expect("replay accepted command"),
            ProjectChangeApplyResult::Duplicate {
                applied_revision: 2
            }
        );

        let denied_channel = Uuid::new_v4();
        let denied = command(&member, &owner, "shared", 3, &[denied_channel], &[]);
        let denied_id = denied.id.as_bytes().to_vec();
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &denied,
                ProjectRelatedChannelChange {
                    project_owner: owner_bytes.as_slice(),
                    project_d_tag: "shared",
                    expected_revision: 3,
                    add: &[denied_channel],
                    remove: &[],
                },
            )
            .await
            .expect("reject member"),
            ProjectChangeApplyResult::Forbidden
        );
        let rejected_rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
                .bind(community.as_uuid())
                .bind(denied_id)
                .fetch_one(&pool)
                .await
                .expect("count rejected event");
        assert_eq!(rejected_rows, 0);

        let stale_channel = Uuid::new_v4();
        let stale = command(&owner, &owner, "shared", 2, &[stale_channel], &[]);
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &stale,
                ProjectRelatedChannelChange {
                    project_owner: owner_bytes.as_slice(),
                    project_d_tag: "shared",
                    expected_revision: 2,
                    add: &[stale_channel],
                    remove: &[],
                },
            )
            .await
            .expect("reject stale CAS"),
            ProjectChangeApplyResult::Conflict {
                current_revision: 3
            }
        );
        let relay_a = Keys::generate();
        let relay_a_bytes = relay_a.public_key().to_bytes();
        let candidates = db
            .load_pending_project_state_projections(relay_a_bytes.as_slice(), 10)
            .await
            .expect("load initial pending projection");
        assert_eq!(candidates.len(), 1);
        let initial = candidates[0].clone();
        assert_eq!(initial.previous_created_at(), None);
        let initial_timestamp = Timestamp::now().as_secs();
        let initial_event = projection(&initial, &relay_a, initial_timestamp);
        assert_eq!(
            db.commit_project_state_projection(&initial, &initial_event)
                .await
                .expect("commit initial projection"),
            ProjectStateProjectionCommitResult::Committed
        );
        let marker: (i64, Vec<u8>) = sqlx::query_as(
            "SELECT projected_revision, projection_pubkey FROM project_state_heads \
             WHERE community_id=$1 AND project_owner=$2 AND project_d_tag='shared'",
        )
        .bind(community.as_uuid())
        .bind(owner_bytes.as_slice())
        .fetch_one(&pool)
        .await
        .expect("read committed projection marker");
        assert_eq!(marker, (3, relay_a_bytes.to_vec()));
        assert!(db
            .load_pending_project_state_projections(relay_a_bytes.as_slice(), 10)
            .await
            .expect("check settled projection")
            .is_empty());

        let relay_b = Keys::generate();
        let relay_b_bytes = relay_b.public_key().to_bytes();
        let rotation_candidate = db
            .load_pending_project_state_projections(relay_b_bytes.as_slice(), 10)
            .await
            .expect("load key rotation projection")
            .pop()
            .expect("key rotation is pending");
        assert_eq!(rotation_candidate.previous_created_at(), None);

        let newest = Uuid::new_v4();
        let next = command(&owner, &owner, "shared", 3, &[newest], &[]);
        assert!(matches!(
            db.apply_project_related_channel_change(
                community,
                &next,
                ProjectRelatedChannelChange {
                    project_owner: owner_bytes.as_slice(),
                    project_d_tag: "shared",
                    expected_revision: 3,
                    add: &[newest],
                    remove: &[],
                },
            )
            .await
            .expect("advance Project after candidate load"),
            ProjectChangeApplyResult::Applied(ProjectStateSnapshot { revision: 4, .. })
        ));
        let stale_event = projection(&rotation_candidate, &relay_b, initial_timestamp + 1);
        assert_eq!(
            db.commit_project_state_projection(&rotation_candidate, &stale_event)
                .await
                .expect("reject stale projection"),
            ProjectStateProjectionCommitResult::Stale
        );
        let pending = db
            .load_pending_project_state_projections(relay_a_bytes.as_slice(), 10)
            .await
            .expect("reload advanced projection")
            .pop()
            .expect("advanced revision is pending");
        assert_eq!(pending.previous_created_at(), Some(initial_timestamp));
        let advanced_event = projection(&pending, &relay_a, initial_timestamp + 1);
        assert_eq!(
            db.commit_project_state_projection(&pending, &advanced_event)
                .await
                .expect("commit monotonic projection"),
            ProjectStateProjectionCommitResult::Committed
        );

        let rotated = db
            .load_pending_project_state_projections(relay_b_bytes.as_slice(), 10)
            .await
            .expect("reload key rotation")
            .pop()
            .expect("rotation remains pending");
        let rotated_event = projection(&rotated, &relay_b, initial_timestamp + 2);
        assert_eq!(
            db.commit_project_state_projection(&rotated, &rotated_event)
                .await
                .expect("commit key rotation"),
            ProjectStateProjectionCommitResult::Committed
        );
        let live_projection_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events WHERE community_id=$1 AND kind=$2 \
               AND pubkey=$3 AND deleted_at IS NULL",
        )
        .bind(community.as_uuid())
        .bind(KIND_PROJECT_STATE as i32)
        .bind(relay_b_bytes.as_slice())
        .fetch_one(&pool)
        .await
        .expect("count rotated projection");
        assert_eq!(live_projection_count, 1);

        let recovered = Uuid::new_v4();
        let recovery = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags([
                Tag::parse(["d", "shared"]).expect("d tag"),
                Tag::parse(["buzz-channel", &home.to_string()]).expect("home tag"),
                Tag::parse(["buzz-related-channel", &recovered.to_string()]).expect("related tag"),
            ])
            .custom_created_at(Timestamp::from(identity_time + 1))
            .sign_with_keys(&owner)
            .expect("sign recovery");
        let recovery_result = db
            .apply_project_identity_event(community, &recovery)
            .await
            .expect("apply recovery");
        assert_eq!(recovery_result.status, ProjectLifecycleStatus::Applied);
        let recovery_state = recovery_result.snapshot.expect("recovery state");
        assert_eq!(recovery_state.revision, 5);
        assert_eq!(recovery_state.related_channels, vec![recovered]);
        assert_eq!(
            db.apply_project_identity_event(community, &recovery)
                .await
                .expect("duplicate recovery")
                .status,
            ProjectLifecycleStatus::Duplicate
        );

        let stale_identity_deletion = EventBuilder::new(Kind::EventDeletion, "")
            .tag(Tag::event(base.id))
            .custom_created_at(Timestamp::from(identity_time + 2))
            .sign_with_keys(&owner)
            .expect("sign stale identity deletion");
        assert_eq!(
            db.apply_project_deletion_event(
                community,
                &stale_identity_deletion,
                owner_bytes.as_slice(),
                "shared",
                Some(base.id.as_bytes()),
            )
            .await
            .expect("ignore stale identity deletion")
            .status,
            ProjectLifecycleStatus::NoEffect
        );
        assert!(db
            .get_event_by_id(community, recovery.id.as_bytes())
            .await
            .expect("load recovered identity")
            .is_some());

        let coordinate = format!("30621:{}:shared", owner.public_key().to_hex());
        let deletion = EventBuilder::new(Kind::EventDeletion, "")
            .tag(Tag::parse(["a", &coordinate]).expect("coordinate tag"))
            .custom_created_at(Timestamp::from(identity_time + 2))
            .sign_with_keys(&owner)
            .expect("sign deletion");
        let deleted = db
            .apply_project_deletion_event(
                community,
                &deletion,
                owner_bytes.as_slice(),
                "shared",
                None,
            )
            .await
            .expect("delete Project");
        assert_eq!(deleted.status, ProjectLifecycleStatus::Applied);
        assert_eq!(
            deleted.snapshot.as_ref().map(|state| state.revision),
            Some(6)
        );
        assert_eq!(
            db.apply_project_deletion_event(
                community,
                &deletion,
                owner_bytes.as_slice(),
                "shared",
                None,
            )
            .await
            .expect("duplicate deletion")
            .status,
            ProjectLifecycleStatus::Duplicate
        );

        let same_second_recreation = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "stale")
            .tags([Tag::parse(["d", "shared"]).expect("d tag")])
            .custom_created_at(Timestamp::from(identity_time + 2))
            .sign_with_keys(&owner)
            .expect("sign same-second recreation");
        assert_eq!(
            db.apply_project_identity_event(community, &same_second_recreation)
                .await
                .expect("reject same-second recreation")
                .status,
            ProjectLifecycleStatus::Superseded
        );
        assert!(db
            .get_event_by_id(community, same_second_recreation.id.as_bytes())
            .await
            .expect("check same-second recreation")
            .is_none());

        let recreation = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags([Tag::parse(["d", "shared"]).expect("d tag")])
            .custom_created_at(Timestamp::from(identity_time + 3))
            .sign_with_keys(&owner)
            .expect("sign recreation");
        let recreated = db
            .apply_project_identity_event(community, &recreation)
            .await
            .expect("recreate Project");
        assert_eq!(recreated.status, ProjectLifecycleStatus::Applied);
        assert_eq!(
            recreated.snapshot.as_ref().map(|state| state.revision),
            Some(7)
        );

        let live_identity_deletion = EventBuilder::new(Kind::EventDeletion, "")
            .tag(Tag::event(recreation.id))
            .custom_created_at(Timestamp::from(identity_time + 4))
            .sign_with_keys(&owner)
            .expect("sign live identity deletion");
        let deleted_by_id = db
            .apply_project_deletion_event(
                community,
                &live_identity_deletion,
                owner_bytes.as_slice(),
                "shared",
                Some(recreation.id.as_bytes()),
            )
            .await
            .expect("delete live identity by id");
        assert_eq!(deleted_by_id.status, ProjectLifecycleStatus::Applied);
        assert_eq!(
            deleted_by_id.snapshot.as_ref().map(|state| state.revision),
            Some(8)
        );

        drop_scratch_db(admin_pool, pool, &scratch_name).await;
    }
}
