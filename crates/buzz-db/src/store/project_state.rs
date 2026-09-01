//! Transactional persistence for collaborative Project state.

use std::collections::BTreeSet;

use buzz_core::kind::{event_kind_u32, KIND_PROJECT, KIND_PROJECT_CHANGE};
use buzz_core::CommunityId;
use nostr::Event;
use sqlx::Row;
use uuid::Uuid;

use crate::event::insert_event_in_transaction;
use crate::replaceable::event_replacement_lock_key;
use crate::{Db, DbError, Result};

const RELATED_CHANNEL_CAP: usize = 64;

/// A validated Project related-channel mutation supplied by the relay parser.
#[derive(Clone, Copy, Debug)]
pub struct ProjectRelatedChannelChange<'a> {
    /// Owner pubkey from the canonical kind:30621 coordinate.
    pub project_owner: &'a [u8],
    /// Owner currently attested by the actor, after event-level verification.
    pub delegated_owner: Option<&'a [u8]>,
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
    if change
        .delegated_owner
        .is_some_and(|owner| owner.len() != 32)
    {
        return Some("delegated Project owner must be 32 bytes".into());
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

impl Db {
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

        let actor = event.pubkey.to_bytes();
        let delegated_owner = if let Some(owner) = change.delegated_owner {
            let registered: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM users WHERE community_id=$1 AND pubkey=$2 \
                   AND agent_owner_pubkey=$3)",
            )
            .bind(community_id.as_uuid())
            .bind(actor.as_slice())
            .bind(owner)
            .fetch_one(&mut *tx)
            .await?;
            if !registered {
                return Ok(ProjectChangeApplyResult::Forbidden);
            }
            Some(owner)
        } else {
            None
        };

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

        let actor_owns_project = actor.as_slice() == change.project_owner;
        let delegate_owns_project = delegated_owner == Some(change.project_owner);
        if !actor_owns_project && !delegate_owns_project {
            let Some(home_channel) = home_channel else {
                return Ok(ProjectChangeApplyResult::Forbidden);
            };
            crate::channel_members::acquire_channel_membership_lock(
                &mut tx,
                community_id,
                home_channel,
            )
            .await?;
            let authorized_role: Option<String> = sqlx::query_scalar(
                "SELECT member.role::text FROM channel_members member \
                 JOIN channels channel ON channel.community_id=member.community_id \
                   AND channel.id=member.channel_id \
                 WHERE member.community_id=$1 AND member.channel_id=$2 \
                   AND (member.pubkey=$3 OR member.pubkey=$4) \
                   AND member.role IN ('owner', 'admin') \
                   AND member.removed_at IS NULL AND channel.archived_at IS NULL \
                   AND channel.deleted_at IS NULL LIMIT 1 FOR SHARE OF channel, member",
            )
            .bind(community_id.as_uuid())
            .bind(home_channel)
            .bind(actor.as_slice())
            .bind(delegated_owner)
            .fetch_optional(&mut *tx)
            .await?;
            if authorized_role.is_none() {
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
}

#[cfg(test)]
mod tests {
    use buzz_core::channel::{ChannelType, ChannelVisibility};
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use sqlx::postgres::PgPoolOptions;

    use super::*;

    const TEST_DB_URL: &str = "postgres://buzz:buzz_dev@localhost:5432/buzz"; // sadscan:disable np.postgres.1 -- local test-only credentials

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

    #[tokio::test]
    #[ignore = "requires Postgres"]
    async fn transaction_enforces_authority_cas_replay_and_atomic_attribution() {
        let database_url =
            std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| TEST_DB_URL.into());
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("connect test database");
        crate::migration::run_migrations(&pool)
            .await
            .expect("run migrations");
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
        let owner_agent = Keys::generate();
        let admin_agent = Keys::generate();
        for principal in [&owner, &admin, &owner_agent, &admin_agent] {
            db.ensure_user(community, principal.public_key().to_bytes().as_slice())
                .await
                .expect("ensure authorization principal");
        }
        db.set_agent_owner(
            community,
            owner_agent.public_key().to_bytes().as_slice(),
            owner.public_key().to_bytes().as_slice(),
        )
        .await
        .expect("register Project owner's agent");
        db.set_agent_owner(
            community,
            admin_agent.public_key().to_bytes().as_slice(),
            admin.public_key().to_bytes().as_slice(),
        )
        .await
        .expect("register home admin's agent");
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
        let base = EventBuilder::new(Kind::Custom(KIND_PROJECT as u16), "")
            .tags([
                Tag::parse(["d", "shared"]).expect("d tag"),
                Tag::parse(["buzz-channel", &home.to_string()]).expect("home tag"),
                Tag::parse(["buzz-related-channel", &seeded.to_string()]).expect("related tag"),
            ])
            .sign_with_keys(&owner)
            .expect("sign Project");
        db.insert_event(community, &base, None)
            .await
            .expect("insert Project");

        let added = Uuid::new_v4();
        let first = command(&admin, &owner, "shared", 1, &[added], &[]);
        let owner_bytes = owner.public_key().to_bytes();
        let applied = db
            .apply_project_related_channel_change(
                community,
                &first,
                ProjectRelatedChannelChange {
                    project_owner: owner_bytes.as_slice(),
                    delegated_owner: None,
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
                    delegated_owner: None,
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
                    delegated_owner: None,
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
                    delegated_owner: None,
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

        let stale_mapping_channel = Uuid::new_v4();
        let stale_mapping = command(
            &owner_agent,
            &owner,
            "shared",
            3,
            &[stale_mapping_channel],
            &[],
        );
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &stale_mapping,
                ProjectRelatedChannelChange {
                    project_owner: owner_bytes.as_slice(),
                    delegated_owner: None,
                    project_d_tag: "shared",
                    expected_revision: 3,
                    add: &[stale_mapping_channel],
                    remove: &[],
                },
            )
            .await
            .expect("stale mapping alone cannot authorize"),
            ProjectChangeApplyResult::Forbidden
        );

        let mismatched_owner = admin.public_key().to_bytes();
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &stale_mapping,
                ProjectRelatedChannelChange {
                    project_owner: owner_bytes.as_slice(),
                    delegated_owner: Some(mismatched_owner.as_slice()),
                    project_d_tag: "shared",
                    expected_revision: 3,
                    add: &[stale_mapping_channel],
                    remove: &[],
                },
            )
            .await
            .expect("mismatched registered owner cannot authorize"),
            ProjectChangeApplyResult::Forbidden
        );

        let delegated_owner = owner.public_key().to_bytes();
        assert!(matches!(
            db.apply_project_related_channel_change(
                community,
                &stale_mapping,
                ProjectRelatedChannelChange {
                    project_owner: owner_bytes.as_slice(),
                    delegated_owner: Some(delegated_owner.as_slice()),
                    project_d_tag: "shared",
                    expected_revision: 3,
                    add: &[stale_mapping_channel],
                    remove: &[],
                },
            )
            .await
            .expect("registered owner's current credential authorizes"),
            ProjectChangeApplyResult::Applied(ProjectStateSnapshot { revision: 4, .. })
        ));

        let delegated_admin_channel = Uuid::new_v4();
        let delegated_admin = command(
            &admin_agent,
            &owner,
            "shared",
            4,
            &[delegated_admin_channel],
            &[],
        );
        let admin_bytes = admin.public_key().to_bytes();
        assert!(matches!(
            db.apply_project_related_channel_change(
                community,
                &delegated_admin,
                ProjectRelatedChannelChange {
                    project_owner: owner_bytes.as_slice(),
                    delegated_owner: Some(admin_bytes.as_slice()),
                    project_d_tag: "shared",
                    expected_revision: 4,
                    add: &[delegated_admin_channel],
                    remove: &[],
                },
            )
            .await
            .expect("registered home admin credential authorizes"),
            ProjectChangeApplyResult::Applied(ProjectStateSnapshot { revision: 5, .. })
        ));
        let stored_author: Vec<u8> =
            sqlx::query_scalar("SELECT pubkey FROM events WHERE community_id=$1 AND id=$2")
                .bind(community.as_uuid())
                .bind(delegated_admin.id.as_bytes().as_slice())
                .fetch_one(&pool)
                .await
                .expect("load delegated command author");
        assert_eq!(stored_author, admin_agent.public_key().to_bytes());

        let stale_channel = Uuid::new_v4();
        let stale = command(&owner, &owner, "shared", 2, &[stale_channel], &[]);
        assert_eq!(
            db.apply_project_related_channel_change(
                community,
                &stale,
                ProjectRelatedChannelChange {
                    project_owner: owner_bytes.as_slice(),
                    delegated_owner: None,
                    project_d_tag: "shared",
                    expected_revision: 2,
                    add: &[stale_channel],
                    remove: &[],
                },
            )
            .await
            .expect("reject stale CAS"),
            ProjectChangeApplyResult::Conflict {
                current_revision: 5
            }
        );
    }
}
