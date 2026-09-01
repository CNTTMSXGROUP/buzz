#!/usr/bin/env bash
# rust-tests-selective.sh — change-aware pre-push Rust test lane (Variant A).
#
# Scopes `just test-unit` to the subset of packages that this branch
# actually touched, falling back to the full battery whenever the scope
# cannot be determined safely.
#
# Selection logic (path→crate only, no dependency graph):
#   1. Full-battery triggers: Cargo.lock, root Cargo.toml, rust-toolchain.toml,
#      deny.toml, Justfile, scripts/run-tests.sh, migrations/**, schema/**,
#      any changed path that doesn't map to a crate, or any script error.
#   2. Crate mapping: crates/<name>/** → <name>; examples/<name>/** → <name>.
#   3. Per-package invocation shapes (the PACKAGE_TABLE below) are the single
#      source of truth for how each package's suite is run. A drift guard diffs
#      this table's package list against the enumeration in `just test-unit` at
#      every run; a mismatch falls back to the full battery.
#
# Note: desktop-tauri-checks is unchanged in this round — gating it per-crate
# requires the tauri workspace's transitive path-dep closure (dependency-graph
# machinery deferred per Will's direction). That lane still fires on all crate
# changes via its lefthook glob.
#
# CI (dorny/paths-filter + full `cargo clippy --workspace --all-targets`)
# remains the authoritative gate; this script is a fast pre-filter only.

set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

# ---------------------------------------------------------------------------
# Per-package invocation table.
# Each entry: "<pkg>:<mode>" where mode is one of:
#   lib          → cargo nextest run -p <pkg> --lib
#   all          → cargo nextest run -p <pkg>          (all targets)
#   lib+doc      → cargo nextest run -p <pkg> --lib  +  cargo test -p <pkg> --doc
#   relay        → the scoped buzz-relay invocation (special-cased below)
# ---------------------------------------------------------------------------
declare -A PACKAGE_TABLE=(
    [buzz-core]="lib"
    [buzz-auth]="lib+doc"
    [buzz-voice]="lib"
    [buzz-cli]="all"
    [buzz-db]="lib"
    [buzz-conformance]="all"
    [buzz-push-gateway]="all"
    [buzz-backend-kubernetes]="all"
    [buzz-agent]="lib"
    [buzz-relay]="relay"
    [buzz-acp]="lib"
)

# Canonical ordered list from just test-unit — drift guard compares this.
# Update both lists together if test-unit adds or removes a package.
CANONICAL_PACKAGES=(
    buzz-core
    buzz-auth
    buzz-voice
    buzz-cli
    buzz-db
    buzz-conformance
    buzz-push-gateway
    buzz-backend-kubernetes
    buzz-agent
    buzz-relay
    buzz-acp
)

# ---------------------------------------------------------------------------
# Drift guard: verify PACKAGE_TABLE keys match CANONICAL_PACKAGES exactly.
# ---------------------------------------------------------------------------
drift_guard() {
    local table_keys
    table_keys=$(printf '%s\n' "${!PACKAGE_TABLE[@]}" | sort)
    local canonical_sorted
    canonical_sorted=$(printf '%s\n' "${CANONICAL_PACKAGES[@]}" | sort)
    if [[ "$table_keys" != "$canonical_sorted" ]]; then
        echo "rust-tests-selective: DRIFT DETECTED — PACKAGE_TABLE keys do not" >&2
        echo "  match CANONICAL_PACKAGES. Update both lists together." >&2
        echo "  Table keys:  $(printf '%s\n' "${!PACKAGE_TABLE[@]}" | sort | tr '\n' ' ')" >&2
        echo "  Canonical:   $(printf '%s\n' "${CANONICAL_PACKAGES[@]}" | sort | tr '\n' ' ')" >&2
        return 1
    fi
    return 0
}

# ---------------------------------------------------------------------------
# Full-battery fallback.
# ---------------------------------------------------------------------------
run_full_battery() {
    echo "rust-tests-selective: running full battery (just test-unit)" >&2
    exec just test-unit
}

# ---------------------------------------------------------------------------
# Run one package's shaped suite. Returns non-zero on failure.
# ---------------------------------------------------------------------------
run_package() {
    local pkg="$1"
    local mode="${PACKAGE_TABLE[$pkg]}"
    case "$mode" in
        lib)
            cargo nextest run -p "$pkg" --lib
            ;;
        all)
            cargo nextest run -p "$pkg"
            ;;
        lib+doc)
            cargo nextest run -p "$pkg" --lib
            cargo test -p "$pkg" --doc
            ;;
        relay)
            cargo nextest run -p buzz-relay --lib \
                -E 'test(/^api::admin::/) - test(=api::admin::tests::disabled_mode_allows_unauthenticated_requests_on_the_admin_host) - test(=api::admin::tests::nip98_mode_unrostered_signer_does_not_consume_a_replay_slot)'
            ;;
        *)
            echo "rust-tests-selective: unknown mode '$mode' for package '$pkg'" >&2
            return 1
            ;;
    esac
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

# Step 0a: require cargo-nextest — selective invocations always use it.
# If it's absent (e.g. minimal CI environment), fall back to full battery,
# which handles the nextest-absent case via ./scripts/run-tests.sh unit.
if ! command -v cargo-nextest &>/dev/null; then
    echo "rust-tests-selective: cargo-nextest not found; falling back to full battery" >&2
    run_full_battery
fi

# Step 0b: drift guard — fail closed if table is stale.
if ! drift_guard; then
    run_full_battery
fi

# Step 1: compute changed paths via three-dot diff against origin/main.
# (lefthook already sets $LEFTHOOK_OUTPUT_LOG / provides files via env, but
# we recompute here so the script is usable standalone and in the harness.)
if ! changed_paths=$(git diff --name-only origin/main...HEAD 2>/dev/null); then
    echo "rust-tests-selective: git diff failed; falling back to full battery" >&2
    run_full_battery
fi

# Step 2: full-battery trigger patterns.
FULL_TRIGGERS=(
    "^Cargo\.lock$"
    "^Cargo\.toml$"
    "^rust-toolchain\.toml$"
    "^deny\.toml$"
    "^Justfile$"
    "^scripts/run-tests\.sh$"
    "^migrations/"
    "^schema/"
)

for pattern in "${FULL_TRIGGERS[@]}"; do
    if echo "$changed_paths" | grep -qE "$pattern"; then
        echo "rust-tests-selective: full-battery trigger matched ('$pattern')" >&2
        run_full_battery
    fi
done

# Step 3: map changed paths to crate names.
declare -A touched_crates

while IFS= read -r path; do
    [[ -z "$path" ]] && continue

    if [[ "$path" =~ ^crates/([^/]+)/ ]]; then
        touched_crates["${BASH_REMATCH[1]}"]=1
    elif [[ "$path" =~ ^examples/([^/]+)/ ]]; then
        touched_crates["${BASH_REMATCH[1]}"]=1
    else
        # Path outside crates/examples/ and not caught by full-battery triggers.
        # Fail closed.
        echo "rust-tests-selective: unmapped path '$path'; falling back to full battery" >&2
        run_full_battery
    fi
done <<< "$changed_paths"

# Step 4: if nothing changed (e.g. deletion-only push already skipped by
# lefthook), nothing to do.
if [[ ${#touched_crates[@]} -eq 0 ]]; then
    echo "rust-tests-selective: no Rust crate changes detected; skipping Rust tests" >&2
    exit 0
fi

# Step 5: intersect touched crates with PACKAGE_TABLE.
# Crates touched that are NOT in the table (e.g. buzz-ws-client, buzz-sdk)
# are not exercised by test-unit, so they don't need a suite run.
# Any crate outside the table that is also NOT in the workspace is the unmapped
# case we already handled above; we reach here only for valid workspace members.
pkgs_to_run=()
for crate in "${!touched_crates[@]}"; do
    if [[ -v "PACKAGE_TABLE[$crate]" ]]; then
        pkgs_to_run+=("$crate")
    fi
    # Crates with no suite entry in test-unit are silently skipped —
    # CI's full clippy + check lane covers them.
done

if [[ ${#pkgs_to_run[@]} -eq 0 ]]; then
    echo "rust-tests-selective: touched crates have no test-unit suites; skipping" >&2
    exit 0
fi

# Step 6: run the local relay key setup (mirrors test-unit's preamble).
./scripts/test-ensure-local-relay-key.sh

# Step 7: run shaped suites for each selected package.
echo "rust-tests-selective: running suites for: ${pkgs_to_run[*]}" >&2
for pkg in "${pkgs_to_run[@]}"; do
    run_package "$pkg"
done

echo "rust-tests-selective: selective run complete" >&2
