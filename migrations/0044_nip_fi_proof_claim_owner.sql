-- NIP-FI proof replay-claim: add connection_id owner column.
--
-- Each proof claim now records the WebSocket connection UUID that first
-- admitted the proof event, enabling per-connection ownership checks at
-- admission time.  The primary key remains (community_id, proof_event_id)
-- and that constraint name remains the exact string mapped to
-- AdmissionError::ProofReplayed in the Rust admission path.
--
-- A claim is inserted only during final admission (never at AUTH), after
-- all other authority mutations succeed.  The append-only immutability
-- trigger from migration 0043 continues to hold: once a row is committed,
-- connection_id cannot be changed.
--
-- This column enables the amended Design C ownership protocol:
--   1. SELECT connection_id FOR SHARE on (community_id, proof_event_id)
--   2. Same conn_id → same-connection reuse, continue
--   3. Different conn_id → ProofReplayed (cross-connection reuse)
--   4. No row → proceed; INSERT this row at step 9 of commit_admission_body

-- Existing 0043-shape rows (if any) receive a synthetic owner UUID matching no
-- live connection.  This is deliberate fail-closed: replays of pre-0044 legacy
-- proofs from any connection are rejected as cross-connection reuse (the stored
-- sentinel never matches a live conn_id), preventing any legacy claim from being
-- reused post-migration.  The ownership protocol applies only to claims written
-- after this migration; the append-only trigger continues to hold for all rows.
--
-- Three-step safe upgrade for populated tables:
--   1. Add nullable column (no constraint yet — existing rows get NULL).
--   2. Backfill NULLs with gen_random_uuid() — one random sentinel per row.
--   3. Set NOT NULL (no DEFAULT) so future inserts must supply connection_id
--      explicitly; any insert that omits it errors immediately rather than
--      silently getting a wrong owner.
ALTER TABLE nip_fi_proof_replay_claims
    ADD COLUMN IF NOT EXISTS connection_id UUID;

UPDATE nip_fi_proof_replay_claims
    SET connection_id = gen_random_uuid()
    WHERE connection_id IS NULL;

ALTER TABLE nip_fi_proof_replay_claims
    ALTER COLUMN connection_id SET NOT NULL;
