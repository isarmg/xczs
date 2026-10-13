#!/usr/bin/env bash
# This file is sourced by package-release.sh after its validated constants,
# helper functions, traps and cleanup state have been established.
# shellcheck disable=SC2016,SC2034,SC2154,SC2164

run_publication_self_test() {
  local test_root
  local test_output
  local unsupported_output
  local test_alias
  local physical_output
  local staged_parent
  local staged_release
  local final_release
  local collision_source
  local collision_destination
  local protected_directory
  local symlink_source
  local symlink_destination
  local current_uid
  local test_lock_fd
  local checksum
  local signature
  local public_key
  local rsa_key
  local weak_rsa_key
  local ed25519_key
  local ed448_key
  local ec_key
  local unapproved_ec_key
  local dsa_parameters
  local dsa_key
  local unknown_key
  local linked_key
  local signature_mode_file
  local locked_output
  local locked_staged_release
  local locked_final_release
  local original_output_identity
  local original_output_metadata
  local moved_output
  local replacement_output
  local test_repository
  local test_git_template
  local original_commit
  local replacement_commit
  local repository_git_directory
  local snapshot_directory
  local snapshot_template
  local first_source_archive
  local second_source_archive
  local first_source_tree
  local second_source_tree
  local first_archive_checksum
  local second_archive_checksum
  local unsafe_commit
  local unsafe_parent
  local unsafe_path
  local previous_unsafe_path=""
  local unsafe_target
  local extracted_safety_test
  local notice_test_root
  local notice_test_file
  local notice_test_checksum
  local notice_test_outside
  local notice_test_link
  local build_environment_manifest
  local documentation_package
  local documentation_package_name
  local documentation_sentinel
  local web_manifest_source
  local documentation_archive_one
  local documentation_archive_two
  local documentation_archive_checksum_one
  local documentation_archive_checksum_two
  local documentation_unpack_root
  local advisory_database_test
  local advisory_database_test_revision
  local advisory_database_test_fetch_epoch
  local advisory_database_test_classification
  local advisory_database_test_status
  local advisory_database_test_state
  local advisory_database_test_first_state
  local advisory_database_test_second_state
  local advisory_database_test_index_checksum
  local advisory_database_test_config_checksum
  local advisory_database_test_changed_fetch_epoch
  local advisory_database_stability_iteration
  local advisory_database_filter_marker
  local advisory_database_test_mode
  local advisory_database_untracked_directory
  local freshness_current_epoch
  local freshness_expected_status
  local freshness_fetch_epoch
  local freshness_status
  local partial_git
  local partial_find_bin
  local partial_find
  local verifier_temporary_directory
  local verifier_leak

  current_uid="$(id -u)"
  test_root="$(mktemp -d -t xczs-release-self-test.XXXXXXXX)"
  chmod 0700 "$test_root"
  cleanup_path="$test_root"
  cleanup_parent="${test_root%/*}"
  cleanup_prefix="xczs-release-self-test."

  validate_cargo_audit_version_line \
    'cargo-audit-audit 0.22.2' \
    "$required_cargo_audit_version" || {
    printf 'release self-test rejected the pinned cargo-audit version\n' >&2
    return 1
  }
  if validate_cargo_audit_version_line \
    'cargo-audit-audit 0.22.1' \
    "$required_cargo_audit_version"
  then
    printf 'release self-test accepted an unpinned cargo-audit version\n' >&2
    return 1
  fi

  while read -r \
    freshness_fetch_epoch freshness_current_epoch freshness_expected_status
  do
    freshness_status=0
    validate_advisory_database_freshness \
      "$freshness_fetch_epoch" \
      "$freshness_current_epoch" \
      "$rustsec_advisory_database_maximum_age_seconds" \
      "$rustsec_advisory_database_maximum_future_skew_seconds" || \
      freshness_status=$?
    [[ "$freshness_status" == "$freshness_expected_status" ]] || {
      printf \
        'release self-test classified RustSec freshness case %s/%s as %s, expected %s\n' \
        "$freshness_fetch_epoch" \
        "$freshness_current_epoch" \
        "$freshness_status" \
        "$freshness_expected_status" >&2
      return 1
    }
  done <<EOF
1000000 1000000 0
$((1000000 - rustsec_advisory_database_maximum_age_seconds)) 1000000 0
$((999999 - rustsec_advisory_database_maximum_age_seconds)) 1000000 1
$((1000000 + rustsec_advisory_database_maximum_future_skew_seconds)) 1000000 0
$((1000001 + rustsec_advisory_database_maximum_future_skew_seconds)) 1000000 2
invalid 1000000 2
EOF

  advisory_database_test="$test_root/advisory-database"
  run_git_isolated init --quiet "$advisory_database_test"
  printf 'README.md filter=release-self-test-filter\n' > \
    "$advisory_database_test/.gitattributes"
  printf 'self-test\n' > "$advisory_database_test/README.md"
  run_git_isolated \
    -C "$advisory_database_test" \
    add .gitattributes README.md
  run_git_isolated \
    -C "$advisory_database_test" \
    -c user.name=release-self-test \
    -c user.email=release-self-test@example.invalid \
    commit --quiet -m initial
  run_git_isolated \
    -C "$advisory_database_test" \
    remote add origin "$rustsec_advisory_database_url"
  advisory_database_test_revision="$(
    advisory_database_revision "$advisory_database_test"
  )"
  printf '%s\t\t%s\n' \
    "$advisory_database_test_revision" \
    "$rustsec_advisory_database_url" > \
    "$advisory_database_test/.git/FETCH_HEAD"
  advisory_database_test_classification="$(
    classify_advisory_database_identity "$advisory_database_test"
  )"
  read -r \
    advisory_database_test_status \
    advisory_database_test_revision \
    advisory_database_test_fetch_epoch <<< \
    "$advisory_database_test_classification"
  [[ "$advisory_database_test_status" == "reusable" &&
    "$advisory_database_test_revision" =~ ^[0-9a-f]{40}$ &&
    "$advisory_database_test_fetch_epoch" =~ ^[0-9]{1,12}$ ]] || {
    printf 'release self-test produced an invalid RustSec database identity\n' >&2
    return 1
  }
  advisory_database_filter_marker="$test_root/rustsec-filter-ran"
  run_git_isolated \
    -C "$advisory_database_test" \
    config \
    filter.release-self-test-filter.clean \
    "tee -- $advisory_database_filter_marker"
  validate_advisory_database_state \
    "$advisory_database_test" >/dev/null || return $?
  [[ ! -e "$advisory_database_filter_marker" ]] || {
    printf 'release self-test executed a RustSec repository clean filter\n' >&2
    return 1
  }
  run_git_isolated \
    -C "$advisory_database_test" \
    config --unset-all filter.release-self-test-filter.clean
  advisory_database_test_state="$(
    seal_fresh_advisory_database_state "$advisory_database_test"
  )" || return $?
  read -r \
    advisory_database_test_revision \
    advisory_database_test_fetch_epoch \
    advisory_database_test_index_checksum \
    advisory_database_test_config_checksum <<< \
    "$advisory_database_test_state"

  # Verify a stable seal once, then exercise a real FETCH_HEAD state change.
  seal_fresh_advisory_database_state \
    "$advisory_database_test" >/dev/null || return $?
  advisory_database_test_first_state="$(
    capture_fresh_advisory_database_state "$advisory_database_test"
  )" || return $?
  advisory_database_test_changed_fetch_epoch="$((
    advisory_database_test_fetch_epoch - 1
  ))"
  touch \
    --date="@$advisory_database_test_changed_fetch_epoch" \
    -- \
    "$advisory_database_test/.git/FETCH_HEAD"
  advisory_database_test_second_state="$(
    capture_fresh_advisory_database_state "$advisory_database_test"
  )" || return $?
  if require_matching_advisory_database_snapshots \
    "$advisory_database_test_first_state" \
    "$advisory_database_test_second_state" >/dev/null 2>&1
  then
    printf '%s\n' \
      'release self-test accepted different complete RustSec snapshots' >&2
    return 1
  fi
  touch \
    --date="@$advisory_database_test_fetch_epoch" \
    -- \
    "$advisory_database_test/.git/FETCH_HEAD"
  validate_sealed_advisory_database_state \
    "$advisory_database_test" \
    "$advisory_database_test_revision" \
    "$advisory_database_test_fetch_epoch" \
    "$advisory_database_test_index_checksum" \
    "$advisory_database_test_config_checksum" || return $?

  printf 'changed after the RustSec seal\n' > "$advisory_database_test/README.md"
  if validate_advisory_database_state \
    "$advisory_database_test" \
    "$advisory_database_test_revision" \
    "$advisory_database_test_fetch_epoch" \
    "$advisory_database_test_index_checksum" \
    "$advisory_database_test_config_checksum" >/dev/null 2>&1
  then
    printf 'release self-test accepted a modified tracked RustSec advisory\n' >&2
    return 1
  fi
  printf 'self-test\n' > "$advisory_database_test/README.md"

  advisory_database_test_mode="$(
    stat -Lc '%a' -- "$advisory_database_test/README.md"
  )"
  chmod u+x -- "$advisory_database_test/README.md"
  if validate_advisory_database_state \
    "$advisory_database_test" \
    "$advisory_database_test_revision" \
    "$advisory_database_test_fetch_epoch" \
    "$advisory_database_test_index_checksum" \
    "$advisory_database_test_config_checksum" >/dev/null 2>&1
  then
    printf 'release self-test accepted a RustSec tracked-mode change\n' >&2
    return 1
  fi
  chmod "$advisory_database_test_mode" -- "$advisory_database_test/README.md"

  advisory_database_untracked_directory="$advisory_database_test/crates/release-self-test"
  install -d -m 0700 "$advisory_database_untracked_directory"
  printf 'untracked advisory\n' > \
    "$advisory_database_untracked_directory/RUSTSEC-9999-9999.toml"
  if validate_advisory_database_state \
    "$advisory_database_test" \
    "$advisory_database_test_revision" \
    "$advisory_database_test_fetch_epoch" \
    "$advisory_database_test_index_checksum" \
    "$advisory_database_test_config_checksum" >/dev/null 2>&1
  then
    printf 'release self-test accepted an untracked RustSec advisory\n' >&2
    return 1
  fi
  rm -f -- "$advisory_database_untracked_directory/RUSTSEC-9999-9999.toml"
  rmdir -- "$advisory_database_untracked_directory" \
    "$advisory_database_test/crates"

  printf 'staged index change\n' > "$advisory_database_test/README.md"
  run_git_isolated -C "$advisory_database_test" add README.md
  printf 'self-test\n' > "$advisory_database_test/README.md"
  if validate_advisory_database_state \
    "$advisory_database_test" \
    "$advisory_database_test_revision" \
    "$advisory_database_test_fetch_epoch" \
    "$advisory_database_test_index_checksum" \
    "$advisory_database_test_config_checksum" >/dev/null 2>&1
  then
    printf 'release self-test accepted a modified RustSec Git index\n' >&2
    return 1
  fi
  run_git_isolated \
    -C "$advisory_database_test" \
    reset --quiet HEAD -- README.md
  advisory_database_test_state="$(
    validate_advisory_database_state \
      "$advisory_database_test" \
      "$advisory_database_test_revision" \
      "$advisory_database_test_fetch_epoch"
  )" || return $?
  read -r \
    advisory_database_test_revision \
    advisory_database_test_fetch_epoch \
    advisory_database_test_index_checksum \
    advisory_database_test_config_checksum <<< \
    "$advisory_database_test_state"

  printf 'README.md export-ignore\n' > \
    "$advisory_database_test/.git/info/attributes"
  if validate_advisory_database_state \
    "$advisory_database_test" \
    "$advisory_database_test_revision" \
    "$advisory_database_test_fetch_epoch" \
    "$advisory_database_test_index_checksum" \
    "$advisory_database_test_config_checksum" >/dev/null 2>&1
  then
    printf 'release self-test accepted unsafe RustSec Git metadata\n' >&2
    return 1
  fi
  rm -f -- "$advisory_database_test/.git/info/attributes"
  validate_advisory_database_state \
    "$advisory_database_test" \
    "$advisory_database_test_revision" \
    "$advisory_database_test_fetch_epoch" \
    "$advisory_database_test_index_checksum" \
    "$advisory_database_test_config_checksum" >/dev/null

  # Exercise the formal-release sequence as one chain: a successful pre-gate
  # seal must still reject an otherwise-successful gate that leaves any new
  # database state behind.
  advisory_database_test_state="$(
    validate_advisory_database_state "$advisory_database_test"
  )" || return $?
  read -r \
    advisory_database_test_revision \
    advisory_database_test_fetch_epoch \
    advisory_database_test_index_checksum \
    advisory_database_test_config_checksum <<< \
    "$advisory_database_test_state"
  (
    printf 'simulated quality-gate output\n' > \
      "$advisory_database_test/quality-gate-output"
  )
  if validate_advisory_database_state \
    "$advisory_database_test" \
    "$advisory_database_test_revision" \
    "$advisory_database_test_fetch_epoch" \
    "$advisory_database_test_index_checksum" \
    "$advisory_database_test_config_checksum" >/dev/null 2>&1
  then
    printf '%s\n' \
      'release self-test accepted RustSec state changed by the simulated quality gate' >&2
    return 1
  fi
  rm -f -- "$advisory_database_test/quality-gate-output"
  validate_advisory_database_state \
    "$advisory_database_test" \
    "$advisory_database_test_revision" \
    "$advisory_database_test_fetch_epoch" \
    "$advisory_database_test_index_checksum" \
    "$advisory_database_test_config_checksum" >/dev/null

  run_git_isolated \
    -C "$advisory_database_test" \
    remote set-url origin https://example.invalid/advisory-db.git
  [[ "$(
    classify_advisory_database_identity \
      "$advisory_database_test" 2>/dev/null
  )" == "unavailable" ]] || {
    printf 'release self-test classified an untrusted RustSec origin as reusable\n' >&2
    return 1
  }
  run_git_isolated \
    -C "$advisory_database_test" \
    remote set-url origin "$rustsec_advisory_database_url"
  rm -f -- "$advisory_database_test/.git/FETCH_HEAD"
  [[ "$(
    classify_advisory_database_identity \
      "$advisory_database_test" 2>/dev/null
  )" == "unavailable" ]] || {
    printf 'release self-test classified a missing RustSec FETCH_HEAD as reusable\n' >&2
    return 1
  }
  printf '%s\t\t%s\n' \
    0000000000000000000000000000000000000000 \
    "$rustsec_advisory_database_url" > \
    "$advisory_database_test/.git/FETCH_HEAD"
  [[ "$(
    classify_advisory_database_identity \
      "$advisory_database_test" 2>/dev/null
  )" == "unavailable" ]] || {
    printf 'release self-test classified a mismatched RustSec FETCH_HEAD as reusable\n' >&2
    return 1
  }

  build_environment_manifest="$test_root/BUILD-ENVIRONMENT.txt"
  write_build_environment_manifest \
    "$build_environment_manifest" \
    0123456789abcdef0123456789abcdef01234567 \
    0.0.0-test \
    1234567890 \
    x86_64-unknown-linux-gnu \
    'rustc 1.99.0 (test)' \
    'cargo 1.99.0 (test)' \
    'cargo-cyclonedx-cyclonedx 0.5.9' \
    'cargo-audit-audit 0.22.2' \
    0123456789abcdef0123456789abcdef01234567 \
    1234567890 \
    "$node_command" \
    "$npm_command"
  grep -Fxq \
    'format=xczs-build-environment-v1' \
    "$build_environment_manifest"
  grep -Fxq \
    'source_sha=0123456789abcdef0123456789abcdef01234567' \
    "$build_environment_manifest"
  grep -Fxq 'source_version=0.0.0-test' "$build_environment_manifest"
  grep -Fxq 'source_date_epoch=1234567890' "$build_environment_manifest"
  grep -Fxq 'target=x86_64-unknown-linux-gnu' "$build_environment_manifest"
  local manifest_key
  for manifest_key in \
    bash \
    rustc \
    cargo \
    cargo_cyclonedx \
    cargo_audit \
    rustsec_advisory_db_revision \
    rustsec_advisory_db_fetch_epoch \
    node \
    npm \
    git \
    openssl \
    tar \
    gzip \
    mv \
    sha256sum
  do
    grep -Eq "^${manifest_key}=.+$" "$build_environment_manifest" || {
      printf 'release self-test build manifest omitted %s\n' \
        "$manifest_key" >&2
      return 1
    }
  done
  [[ "$(stat -Lc '%a' -- "$build_environment_manifest")" == "644" ]] || {
    printf 'release self-test produced an unsafe build manifest mode\n' >&2
    return 1
  }

  documentation_package_name="documentation-package"
  documentation_package="$test_root/$documentation_package_name"
  install -d -m 0700 "$documentation_package"
  install_release_support_tree "$project_dir" "$documentation_package"
  verify_release_documentation_layout \
    "$documentation_package" \
    "$node_command"
  web_manifest_source="$test_root/web-manifest-source"
  printf '%s\n' \
    '#!/usr/bin/env bash' \
    '[[ "$1" == "web-assets" ]] || exit 1' \
    'printf '\''{"format":"web-assets-v1","files":[]}'\''' \
    > "$web_manifest_source"
  chmod 0755 "$web_manifest_source"
  install -d -m 0755 "$documentation_package/bin"
  install -m 0755 "$web_manifest_source" "$documentation_package/bin/xczs"
  write_release_web_assets_manifest "$web_manifest_source" "$documentation_package"
  [[ "$(stat -Lc '%a' -- "$documentation_package/WEB-ASSETS.json")" == "644" ]] || {
    printf 'release self-test produced an unsafe Web manifest mode\n' >&2
    return 1
  }
  printf '%s\n' '#!/usr/bin/env bash' 'printf tampered' > "$documentation_package/bin/xczs"
  if write_release_web_assets_manifest "$web_manifest_source" "$documentation_package" 2>/dev/null; then
    printf 'release self-test accepted an inventory from another executable\n' >&2
    return 1
  fi
  install -m 0755 "$web_manifest_source" "$documentation_package/bin/xczs"
  write_release_web_assets_manifest "$web_manifest_source" "$documentation_package"
  documentation_sentinel="$documentation_package/config/.release-self-test-sentinel"
  printf 'recursive release checksum sentinel\n' > "$documentation_sentinel"
  chmod 0644 "$documentation_sentinel"
  write_release_package_checksums "$documentation_package"
  verify_release_package_checksum_coverage "$documentation_package"
  printf 'tampered\n' >> "$documentation_sentinel"
  if (
    cd "$documentation_package"
    sha256sum --quiet --check SHA256SUMS >/dev/null 2>&1
  ); then
    printf 'release self-test checksum did not detect tampering\n' >&2
    return 1
  fi
  printf 'recursive release checksum sentinel\n' > "$documentation_sentinel"
  verify_release_package_checksum_coverage "$documentation_package"

  documentation_archive_one="$test_root/documentation-package-one.tar.gz"
  documentation_archive_two="$test_root/documentation-package-two.tar.gz"
  write_reproducible_release_archive \
    "$test_root" \
    "$documentation_package_name" \
    1234567890 \
    "$documentation_archive_one"
  write_reproducible_release_archive \
    "$test_root" \
    "$documentation_package_name" \
    1234567890 \
    "$documentation_archive_two"
  documentation_archive_checksum_one="$(sha256sum < "$documentation_archive_one")"
  documentation_archive_checksum_two="$(sha256sum < "$documentation_archive_two")"
  [[ "$documentation_archive_checksum_one" == \
    "$documentation_archive_checksum_two" ]] || {
    printf 'release self-test documentation archives were not reproducible\n' >&2
    return 1
  }
  documentation_unpack_root="$test_root/documentation-unpacked"
  install -d -m 0700 "$documentation_unpack_root"
  gzip --decompress --stdout -- "$documentation_archive_one" |
    tar \
      --extract \
      --file=- \
      --directory="$documentation_unpack_root" \
      --no-same-owner \
      --same-permissions
  verify_release_documentation_layout \
    "$documentation_unpack_root/$documentation_package_name" \
    "$node_command"
  verify_release_package_checksum_coverage \
    "$documentation_unpack_root/$documentation_package_name"

  unsupported_output="$test_root/output\\unsupported"
  install -d -m 0700 "$unsupported_output"
  if validate_output_directory \
    "$unsupported_output" \
    "$current_uid" 2>/dev/null
  then
    printf 'release self-test accepted an output path containing a backslash\n' >&2
    return 1
  fi

  test_output="$test_root/output"
  test_alias="$test_root/output-alias"
  install -d -m 0700 "$test_output"
  ln -s -- "$test_output" "$test_alias"
  physical_output="$(
    cd -P -- "$test_alias"
    pwd -P
  )"
  [[ "$(realpath -e -- "$physical_output")" == \
    "$(realpath -e -- "$test_output")" ]] || {
    printf 'release self-test did not resolve the physical output directory\n' >&2
    return 1
  }
  validate_output_directory "$physical_output" "$current_uid"
  if validate_output_directory \
    "$physical_output" \
    "$((current_uid + 1))" 2>/dev/null
  then
    printf 'release self-test accepted an output owned by another uid\n' >&2
    return 1
  fi

  chmod 0770 "$physical_output"
  if validate_output_directory "$physical_output" "$current_uid" 2>/dev/null; then
    printf 'release self-test accepted a group-writable output directory\n' >&2
    return 1
  fi
  chmod 0700 "$physical_output"

  exec {test_lock_fd}<"$physical_output"
  flock --exclusive "$test_lock_fd"
  locked_output="/proc/$packager_pid/fd/$test_lock_fd"
  original_output_identity="$(stat -Lc '%d:%i' -- "$locked_output")"
  original_output_metadata="$(stat -Lc '%u:%a:%d:%i' -- "$locked_output")"
  if ! (
    exec {test_lock_fd}>&-
    [[ "$(stat -Lc '%d:%i' -- "$locked_output")" == \
      "$original_output_identity" ]]
  ); then
    printf '%s\n' \
      'release self-test lost its fixed-process output anchor after closing the inherited fd' >&2
    return 1
  fi
  validate_private_directory_binding \
    "$locked_output" \
    "$physical_output" \
    "$original_output_metadata" \
    'release self-test output'
  if flock --exclusive --nonblock "$physical_output" true 2>/dev/null; then
    printf 'release self-test did not serialize the output directory\n' >&2
    return 1
  fi

  # Rebind the public pathname before creating any stage content. Stage
  # creation and every later write must still resolve below the locked fd.
  moved_output="$test_root/original-output"
  replacement_output="$physical_output"
  mv -T -- "$physical_output" "$moved_output"
  install -d -m 0700 "$replacement_output"
  if validate_private_directory_binding \
    "$locked_output" \
    "$physical_output" \
    "$original_output_metadata" \
    'release self-test output' 2>/dev/null
  then
    printf 'release self-test did not detect a private path rebinding\n' >&2
    return 1
  fi

  staged_parent="$(
    mktemp -d --tmpdir="$locked_output" .xczs-release-stage.XXXXXXXX
  )"
  chmod 0700 "$staged_parent"
  [[ -d "$moved_output/${staged_parent##*/}" ]] || {
    printf 'release self-test did not create its stage through the locked fd\n' >&2
    return 1
  }
  [[ ! -e "$replacement_output/${staged_parent##*/}" ]] || {
    printf 'release self-test created a stage through the rebound path\n' >&2
    return 1
  }
  staged_release="$staged_parent/example.release"
  final_release="$physical_output/example.release"
  locked_staged_release="$staged_release"
  locked_final_release="$locked_output/example.release"
  install -d -m 0755 "$staged_release"
  printf 'archive\n' > "$staged_release/example.tar.gz"
  printf 'checksum\n' > "$staged_release/example.tar.gz.sha256"
  printf 'signature\n' > "$staged_release/example.tar.gz.sha256.sig"
  printf 'public key\n' > "$staged_release/example.tar.gz.sha256.pub.pem"
  sync -- "$staged_release"/*
  sync -- "$staged_release"

  # Publication must target the original inode through /proc/self/fd, and the
  # public identity check must report the earlier path substitution.
  # Send TERM at the exact former gap between rename and parent-directory
  # sync. The durable publication helper must finish and preserve the release.
  publish_release_directory_durably \
    "$locked_staged_release" \
    "$locked_final_release" \
    "$locked_output" \
    true
  if validate_public_output_binding \
    "$replacement_output" \
    "$locked_output" \
    "$original_output_identity" 2>/dev/null
  then
    printf 'release self-test did not detect output path rebinding\n' >&2
    return 1
  fi
  [[ -d "$moved_output/example.release" ]] || {
    printf 'release self-test did not publish through the locked directory fd\n' >&2
    return 1
  }
  [[ ! -e "$replacement_output/example.release" ]] || {
    printf 'release self-test published through the rebound string path\n' >&2
    return 1
  }
  rm -rf --one-file-system -- "$replacement_output"
  mv -T -- "$moved_output" "$physical_output"
  validate_public_output_binding \
    "$physical_output" \
    "$locked_output" \
    "$original_output_identity"
  validate_private_directory_binding \
    "$locked_output" \
    "$physical_output" \
    "$original_output_metadata" \
    'release self-test output'
  [[ -d "$final_release" && ! -e "$staged_release" ]] || {
    printf 'release self-test did not publish one complete directory\n' >&2
    return 1
  }

  collision_source="$staged_parent/collision-source.release"
  collision_destination="$physical_output/collision.release"
  install -d -m 0755 "$collision_source" "$collision_destination"
  printf 'source\n' > "$collision_source/value"
  # Keep the destination empty: an ordinary directory rename could replace
  # it, so this proves the no-replace option rather than relying on ENOTEMPTY.
  if atomic_publish_directory \
    "$collision_source" \
    "$collision_destination" 2>/dev/null
  then
    printf 'release self-test overwrote an existing directory\n' >&2
    return 1
  fi
  [[ "$(<"$collision_source/value")" == "source" ]] || return 1
  [[ -d "$collision_destination" && ! -L "$collision_destination" ]] || \
    return 1
  [[ ! -e "$collision_destination/value" && \
    ! -L "$collision_destination/value" ]] || return 1

  protected_directory="$test_root/protected"
  symlink_source="$staged_parent/symlink-source.release"
  symlink_destination="$physical_output/symlink.release"
  install -d -m 0700 "$protected_directory"
  install -d -m 0755 "$symlink_source"
  printf 'protected\n' > "$protected_directory/value"
  ln -s -- "$protected_directory" "$symlink_destination"
  if atomic_publish_directory \
    "$symlink_source" \
    "$symlink_destination" 2>/dev/null
  then
    printf 'release self-test followed an existing destination symlink\n' >&2
    return 1
  fi
  [[ "$(<"$protected_directory/value")" == "protected" ]] || return 1
  [[ -d "$symlink_source" && -L "$symlink_destination" ]] || return 1

  checksum="$test_root/checksum"
  printf '0123456789abcdef  example.tar.gz\n' > "$checksum"
  rsa_key="$test_root/rsa.pem"
  signature="$test_root/rsa.sig"
  public_key="$test_root/rsa.pub.pem"
  openssl genpkey \
    -algorithm RSA \
    -pkeyopt rsa_keygen_bits:3072 \
    -out "$rsa_key" 2>/dev/null
  chmod 0600 "$rsa_key"
  validate_signing_key "$rsa_key" "$current_uid"
  if validate_signing_key "$rsa_key" "$((current_uid + 1))" 2>/dev/null; then
    printf 'release self-test accepted a key owned by another uid\n' >&2
    return 1
  fi
  chmod 0640 "$rsa_key"
  if validate_signing_key "$rsa_key" "$current_uid" 2>/dev/null; then
    printf 'release self-test accepted an exposed signing key\n' >&2
    return 1
  fi
  chmod 0600 "$rsa_key"
  linked_key="$test_root/rsa-linked.pem"
  ln -- "$rsa_key" "$linked_key"
  if validate_signing_key "$rsa_key" "$current_uid" 2>/dev/null; then
    printf 'release self-test accepted a multiply linked signing key\n' >&2
    return 1
  fi
  rm -f -- "$linked_key"
  signature_mode_file="$test_root/rsa.mode"
  sign_checksum_with_validated_key \
    "$rsa_key" \
    "$current_uid" \
    "$checksum" \
    "$signature" \
    "$public_key" \
    "$signature_mode_file"
  signature_mode="$(<"$signature_mode_file")"
  rm -f -- "$signature_mode_file"
  [[ "$signature_mode" == "SHA-256 digest" ]] || return 1

  weak_rsa_key="$test_root/weak-rsa.pem"
  openssl genpkey \
    -algorithm RSA \
    -pkeyopt rsa_keygen_bits:1024 \
    -out "$weak_rsa_key" 2>/dev/null
  chmod 0600 "$weak_rsa_key"
  if sign_checksum_with_validated_key \
    "$weak_rsa_key" \
    "$current_uid" \
    "$checksum" \
    "$test_root/weak-rsa.sig" \
    "$test_root/weak-rsa.pub.pem" \
    "$test_root/weak-rsa.mode" 2>/dev/null
  then
    printf 'release self-test accepted a weak RSA signing key\n' >&2
    return 1
  fi

  ed25519_key="$test_root/ed25519.pem"
  signature="$test_root/ed25519.sig"
  public_key="$test_root/ed25519.pub.pem"
  openssl genpkey -algorithm ED25519 -out "$ed25519_key"
  chmod 0400 "$ed25519_key"
  validate_signing_key "$ed25519_key" "$current_uid"
  signature_mode_file="$test_root/ed25519.mode"
  sign_checksum_with_validated_key \
    "$ed25519_key" \
    "$current_uid" \
    "$checksum" \
    "$signature" \
    "$public_key" \
    "$signature_mode_file"
  signature_mode="$(<"$signature_mode_file")"
  rm -f -- "$signature_mode_file"
  [[ "$signature_mode" == "EdDSA raw message" ]] || return 1
  if sign_checksum_with_validated_key \
    "$ed25519_key" \
    "$current_uid" \
    "$checksum" \
    "$test_root/missing/signature" \
    "$test_root/failed-signature.pub.pem" \
    "$test_root/failed-signature.mode" 2>/dev/null
  then
    printf 'release self-test masked a signature-output failure\n' >&2
    return 1
  fi
  if sign_checksum_with_validated_key \
    "$ed25519_key" \
    "$current_uid" \
    "$checksum" \
    "$test_root/failed-mode.sig" \
    "$test_root/failed-mode.pub.pem" \
    "$test_root" 2>/dev/null
  then
    printf 'release self-test masked a signature-mode write failure\n' >&2
    return 1
  fi

  ed448_key="$test_root/ed448.pem"
  openssl genpkey -algorithm ED448 -out "$ed448_key"
  chmod 0400 "$ed448_key"
  signature_mode_file="$test_root/ed448.mode"
  sign_checksum_with_validated_key \
    "$ed448_key" \
    "$current_uid" \
    "$checksum" \
    "$test_root/ed448.sig" \
    "$test_root/ed448.pub.pem" \
    "$signature_mode_file"
  signature_mode="$(<"$signature_mode_file")"
  rm -f -- "$signature_mode_file"
  [[ "$signature_mode" == "EdDSA raw message" ]] || return 1

  ec_key="$test_root/ec-p256.pem"
  openssl genpkey \
    -algorithm EC \
    -pkeyopt ec_paramgen_curve:prime256v1 \
    -out "$ec_key"
  chmod 0400 "$ec_key"
  signature_mode_file="$test_root/ec-p256.mode"
  sign_checksum_with_validated_key \
    "$ec_key" \
    "$current_uid" \
    "$checksum" \
    "$test_root/ec-p256.sig" \
    "$test_root/ec-p256.pub.pem" \
    "$signature_mode_file"
  signature_mode="$(<"$signature_mode_file")"
  rm -f -- "$signature_mode_file"
  [[ "$signature_mode" == "SHA-256 digest" ]] || return 1

  unapproved_ec_key="$test_root/ec-secp256k1.pem"
  openssl genpkey \
    -algorithm EC \
    -pkeyopt ec_paramgen_curve:secp256k1 \
    -out "$unapproved_ec_key"
  chmod 0400 "$unapproved_ec_key"
  if sign_checksum_with_validated_key \
    "$unapproved_ec_key" \
    "$current_uid" \
    "$checksum" \
    "$test_root/ec-secp256k1.sig" \
    "$test_root/ec-secp256k1.pub.pem" \
    "$test_root/ec-secp256k1.mode" 2>/dev/null
  then
    printf 'release self-test accepted an unapproved EC curve\n' >&2
    return 1
  fi

  dsa_parameters="$test_root/dsa-parameters.pem"
  dsa_key="$test_root/dsa.pem"
  openssl genpkey \
    -genparam \
    -algorithm DSA \
    -pkeyopt dsa_paramgen_bits:1024 \
    -out "$dsa_parameters" 2>/dev/null
  openssl genpkey \
    -paramfile "$dsa_parameters" \
    -out "$dsa_key" 2>/dev/null
  chmod 0400 "$dsa_key"
  if sign_checksum_with_validated_key \
    "$dsa_key" \
    "$current_uid" \
    "$checksum" \
    "$test_root/dsa.sig" \
    "$test_root/dsa.pub.pem" \
    "$test_root/dsa.mode" 2>/dev/null
  then
    printf 'release self-test accepted a DSA signing key\n' >&2
    return 1
  fi

  unknown_key="$test_root/x25519.pem"
  openssl genpkey -algorithm X25519 -out "$unknown_key"
  chmod 0400 "$unknown_key"
  if sign_checksum_with_validated_key \
    "$unknown_key" \
    "$current_uid" \
    "$checksum" \
    "$test_root/x25519.sig" \
    "$test_root/x25519.pub.pem" \
    "$test_root/x25519.mode" 2>/dev/null
  then
    printf 'release self-test accepted an unknown signing-key algorithm\n' >&2
    return 1
  fi

  test_repository="$test_root/source-repository"
  test_git_template="$test_root/source-template"
  install -d -m 0700 "$test_git_template"
  run_git_isolated init \
    --quiet \
    --template="$test_git_template" \
    "$test_repository"
  printf 'original\n' > "$test_repository/tracked.txt"
  printf '[package]\nname = "release-self-test"\nversion = "0.0.0-test"\n' \
    > "$test_repository/Cargo.toml"
  run_source_git "$test_repository" add -- Cargo.toml tracked.txt
  run_source_git "$test_repository" \
    -c user.name=release-self-test \
    -c user.email=release-self-test.invalid \
    -c commit.gpgSign=false \
    commit --quiet --message=original
  original_commit="$(
    run_source_git "$test_repository" rev-parse --verify "HEAD^{commit}"
  )"
  printf 'replacement\n' > "$test_repository/tracked.txt"
  run_source_git "$test_repository" add -- tracked.txt
  run_source_git "$test_repository" \
    -c user.name=release-self-test \
    -c user.email=release-self-test.invalid \
    -c commit.gpgSign=false \
    commit --quiet --message=replacement
  replacement_commit="$(
    run_source_git "$test_repository" rev-parse --verify "HEAD^{commit}"
  )"
  run_source_git "$test_repository" \
    checkout --quiet --detach "$original_commit"
  run_source_git "$test_repository" tag v0.0.0-test "$original_commit"
  validate_release_source_state \
    "$test_repository" \
    "$original_commit" \
    v0.0.0-test \
    0.0.0-test
  if validate_release_source_state \
    "$test_repository" \
    "$original_commit" \
    v0.0.0-test \
    9.9.9 2>/dev/null
  then
    printf 'release self-test accepted a mismatched Cargo version\n' >&2
    return 1
  fi
  printf 'dirty\n' > "$test_repository/tracked.txt"
  if validate_release_source_state \
    "$test_repository" \
    "$original_commit" \
    v0.0.0-test \
    0.0.0-test 2>/dev/null
  then
    printf 'release self-test accepted a dirty worktree\n' >&2
    return 1
  fi
  printf 'original\n' > "$test_repository/tracked.txt"

  run_source_git "$test_repository" \
    replace "$original_commit" "$replacement_commit"
  if validate_source_git_metadata "$test_repository" 2>/dev/null; then
    printf 'release self-test accepted refs/replace metadata\n' >&2
    return 1
  fi
  run_source_git "$test_repository" replace -d "$original_commit" >/dev/null

  repository_git_directory="$(
    run_source_git "$test_repository" \
      rev-parse --path-format=absolute --git-dir
  )"
  install -d -m 0700 "$repository_git_directory/info"
  printf 'tracked.txt export-ignore\n' \
    > "$repository_git_directory/info/attributes"
  if validate_source_git_metadata "$test_repository" 2>/dev/null; then
    printf 'release self-test accepted info/attributes metadata\n' >&2
    return 1
  fi
  rm -f -- "$repository_git_directory/info/attributes"
  validate_source_git_metadata "$test_repository"

  snapshot_directory="$test_root/source-snapshot.git"
  snapshot_template="$test_root/snapshot-template"
  source_sha="$original_commit"
  initialize_source_snapshot \
    "$test_repository" \
    "$snapshot_directory" \
    "$snapshot_template" \
    "$source_sha"

  # Mutate both repository-local mechanisms after the isolated object view has
  # been created. The snapshot archive must remain byte-stable and must still
  # materialize the original commit tree.
  run_source_git "$test_repository" \
    replace "$original_commit" "$replacement_commit"
  printf 'tracked.txt export-ignore\n' \
    > "$repository_git_directory/info/attributes"
  first_source_archive="$test_root/source-first.tar"
  second_source_archive="$test_root/source-second.tar"
  first_source_tree="$test_root/source-first"
  second_source_tree="$test_root/source-second"
  install -d -m 0700 "$first_source_tree" "$second_source_tree"
  create_and_verify_source_archive \
    "$first_source_archive" \
    "$first_source_tree" \
    "$test_root/source-first.index" \
    "$test_root/source-first.untracked"
  create_and_verify_source_archive \
    "$second_source_archive" \
    "$second_source_tree" \
    "$test_root/source-second.index" \
    "$test_root/source-second.untracked"
  [[ "$(<"$first_source_tree/tracked.txt")" == "original" ]] || {
    printf 'isolated Git archive followed a replace object\n' >&2
    return 1
  }
  [[ "$(<"$second_source_tree/tracked.txt")" == "original" ]] || {
    printf 'second isolated Git archive followed a replace object\n' >&2
    return 1
  }
  first_archive_checksum="$(sha256sum "$first_source_archive")"
  first_archive_checksum="${first_archive_checksum%% *}"
  second_archive_checksum="$(sha256sum "$second_source_archive")"
  second_archive_checksum="${second_archive_checksum%% *}"
  [[ "$first_archive_checksum" == "$second_archive_checksum" ]] || {
    printf 'isolated Git archives were not reproducible\n' >&2
    return 1
  }
  verify_quality_source_after_gate \
    "$first_source_tree" \
    "$test_root/quality.index" \
    "$test_root/quality.untracked"
  printf 'changed by quality gate\n' > "$first_source_tree/tracked.txt"
  if verify_quality_source_after_gate \
    "$first_source_tree" \
    "$test_root/quality.index" \
    "$test_root/quality.untracked" 2>/dev/null
  then
    printf 'release self-test missed a quality-gate source mutation\n' >&2
    return 1
  fi
  printf 'original\n' > "$first_source_tree/tracked.txt"
  printf 'unexpected\n' > "$first_source_tree/unexpected.txt"
  if verify_quality_source_after_gate \
    "$first_source_tree" \
    "$test_root/quality.index" \
    "$test_root/quality.untracked" 2>/dev/null
  then
    printf 'release self-test missed an unexpected quality-gate path\n' >&2
    return 1
  fi
  rm -f -- "$first_source_tree/unexpected.txt"
  if validate_source_git_metadata "$test_repository" 2>/dev/null; then
    printf 'release self-test missed post-snapshot Git metadata changes\n' >&2
    return 1
  fi
  run_source_git "$test_repository" replace -d "$original_commit" >/dev/null
  rm -f -- "$repository_git_directory/info/attributes"

  verifier_temporary_directory="$test_root/verifier-temporary"
  install -d -m 0700 "$verifier_temporary_directory"
  TMPDIR="$verifier_temporary_directory" \
    validate_source_tree_entries "$test_repository" "$original_commit"
  verifier_leak="$(
    find -P "$verifier_temporary_directory" -mindepth 1 -print -quit
  )" || {
    printf 'release self-test could not inspect verifier cleanup\n' >&2
    return 1
  }
  [[ -z "$verifier_leak" ]] || {
    printf 'release self-test leaked a successful verifier stream\n' >&2
    return 1
  }

  partial_git="$test_root/partial-git"
  printf '%s\n' \
    '#!/usr/bin/env bash' \
    "printf '100644 blob 0123456789abcdef0123456789abcdef01234567\\ttracked.txt\\0'" \
    "printf 'simulated partial Git listing failure\\n' >&2" \
    'exit 73' > "$partial_git"
  chmod 0700 "$partial_git"
  if (
    TMPDIR="$verifier_temporary_directory"
    git_command="$partial_git"
    validate_source_tree_entries "$test_repository" "$original_commit"
  ) 2>/dev/null
  then
    printf 'release self-test masked a partial source ls-tree failure\n' >&2
    return 1
  fi
  if (
    TMPDIR="$verifier_temporary_directory"
    git_command="$partial_git"
    validate_snapshot_tree_entries "$original_commit"
  ) 2>/dev/null
  then
    printf 'release self-test masked a partial snapshot ls-tree failure\n' >&2
    return 1
  fi
  printf '%s\n' \
    '#!/usr/bin/env bash' \
    "printf '100644 blob 0123456789abcdef0123456789abcdef01234567\\ttruncated.txt'" \
    'exit 0' > "$partial_git"
  chmod 0700 "$partial_git"
  if (
    TMPDIR="$verifier_temporary_directory"
    git_command="$partial_git"
    validate_source_tree_entries "$test_repository" "$original_commit"
  ) 2>/dev/null
  then
    printf 'release self-test accepted a truncated source ls-tree record\n' >&2
    return 1
  fi
  if (
    TMPDIR="$verifier_temporary_directory"
    git_command="$partial_git"
    validate_snapshot_tree_entries "$original_commit"
  ) 2>/dev/null
  then
    printf 'release self-test accepted a truncated snapshot ls-tree record\n' >&2
    return 1
  fi

  partial_find_bin="$test_root/partial-find-bin"
  partial_find="$partial_find_bin/find"
  install -d -m 0700 "$partial_find_bin"
  printf '%s\n' \
    '#!/usr/bin/env bash' \
    'if [[ -f SHA256SUMS ]]; then' \
    '  while IFS= read -r checksum_line; do' \
    '    package_file=${checksum_line#*  }' \
    "    printf '%s\\0' \"\$package_file\"" \
    '  done < SHA256SUMS' \
    'fi' \
    "printf 'simulated partial find failure\\n' >&2" \
    'exit 73' > "$partial_find"
  chmod 0700 "$partial_find"
  if (
    TMPDIR="$verifier_temporary_directory"
    PATH="$partial_find_bin:/usr/bin:/bin"
    hash -r
    validate_extracted_source_tree "$first_source_tree"
  ) 2>/dev/null
  then
    printf 'release self-test masked a partial extraction scan failure\n' >&2
    return 1
  fi
  if (
    TMPDIR="$verifier_temporary_directory"
    PATH="$partial_find_bin:/usr/bin:/bin"
    hash -r
    verify_release_package_checksum_coverage "$documentation_package"
  ) 2>/dev/null
  then
    printf 'release self-test masked a partial checksum traversal failure\n' >&2
    return 1
  fi
  printf '%s\n' \
    '#!/usr/bin/env bash' \
    'last_package_file=' \
    'while IFS= read -r checksum_line; do' \
    '  package_file=${checksum_line#*  }' \
    '  if [[ -n "$last_package_file" ]]; then' \
    "    printf '%s\\0' \"\$last_package_file\"" \
    '  fi' \
    '  last_package_file=$package_file' \
    'done < SHA256SUMS' \
    "printf '%s' \"\$last_package_file\"" \
    'exit 0' > "$partial_find"
  chmod 0700 "$partial_find"
  if (
    TMPDIR="$verifier_temporary_directory"
    PATH="$partial_find_bin:/usr/bin:/bin"
    hash -r
    verify_release_package_checksum_coverage "$documentation_package"
  ) 2>/dev/null
  then
    printf 'release self-test accepted a truncated checksum traversal record\n' >&2
    return 1
  fi
  verifier_leak="$(
    find -P "$verifier_temporary_directory" -mindepth 1 -print -quit
  )" || {
    printf 'release self-test could not re-inspect verifier cleanup\n' >&2
    return 1
  }
  [[ -z "$verifier_leak" ]] || {
    printf 'release self-test leaked a failed verifier stream\n' >&2
    return 1
  }

  for unsafe_case in \
    'README.md|../outside-release-tree' \
    'docs/manual.md|/etc/passwd' \
    'build.rs|../../outside-build-input'
  do
    IFS='|' read -r unsafe_path unsafe_target <<< "$unsafe_case"
    if [[ -n "$previous_unsafe_path" ]]; then
      run_source_git "$test_repository" rm -q -f -- "$previous_unsafe_path"
    fi
    unsafe_parent="${unsafe_path%/*}"
    if [[ "$unsafe_parent" != "$unsafe_path" ]]; then
      mkdir -p -- "$test_repository/$unsafe_parent"
    fi
    ln -s -- "$unsafe_target" "$test_repository/$unsafe_path"
    run_source_git "$test_repository" add -- "$unsafe_path"
    run_source_git "$test_repository" \
      -c user.name=release-self-test \
      -c user.email=release-self-test.invalid \
      -c commit.gpgSign=false \
      commit --quiet --message="unsafe symlink $unsafe_path"
    unsafe_commit="$(
      run_source_git "$test_repository" rev-parse --verify "HEAD^{commit}"
    )"
    if validate_snapshot_tree_entries "$unsafe_commit" 2>/dev/null; then
      printf 'release self-test accepted tracked symlink %s\n' \
        "$unsafe_path" >&2
      return 1
    fi
    if validate_source_tree_entries \
      "$test_repository" \
      "$unsafe_commit" 2>/dev/null
    then
      printf 'release preflight accepted tracked symlink %s\n' \
        "$unsafe_path" >&2
      return 1
    fi
    previous_unsafe_path="$unsafe_path"
  done

  run_source_git "$test_repository" rm -q -f -- "$previous_unsafe_path"
  run_source_git "$test_repository" update-index \
    --add \
    --cacheinfo "160000,$original_commit,submodule"
  run_source_git "$test_repository" \
    -c user.name=release-self-test \
    -c user.email=release-self-test.invalid \
    -c commit.gpgSign=false \
    commit --quiet --message='unsafe submodule'
  unsafe_commit="$(
    run_source_git "$test_repository" rev-parse --verify "HEAD^{commit}"
  )"
  if validate_snapshot_tree_entries "$unsafe_commit" 2>/dev/null; then
    printf 'release self-test accepted a submodule entry\n' >&2
    return 1
  fi
  if validate_source_tree_entries \
    "$test_repository" \
    "$unsafe_commit" 2>/dev/null
  then
    printf 'release preflight accepted a submodule entry\n' >&2
    return 1
  fi

  extracted_safety_test="$test_root/extracted-safety"
  install -d -m 0700 "$extracted_safety_test"
  ln -s -- /etc/passwd "$extracted_safety_test/link"
  if validate_extracted_source_tree "$extracted_safety_test" 2>/dev/null; then
    printf 'release self-test accepted an extracted symbolic link\n' >&2
    return 1
  fi
  rm -f -- "$extracted_safety_test/link"
  mkfifo "$extracted_safety_test/fifo"
  if validate_extracted_source_tree "$extracted_safety_test" 2>/dev/null; then
    printf 'release self-test accepted an extracted special file\n' >&2
    return 1
  fi

  notice_test_root="$test_root/notice-root"
  notice_test_file="$notice_test_root/share/doc/rust/COPYRIGHT-library.html"
  notice_test_outside="$test_root/outside-notice"
  notice_test_link="$notice_test_root/share/doc/rust/symlink.html"
  install -d -m 0700 "${notice_test_file%/*}"
  printf 'reviewed standard-library notice\n' > "$notice_test_file"
  printf 'outside notice\n' > "$notice_test_outside"
  notice_test_checksum="$(sha256sum < "$notice_test_file")"
  notice_test_checksum="${notice_test_checksum%% *}"
  [[ "$(
    validate_contained_notice_file \
      "$notice_test_root" \
      "$notice_test_file" \
      "$notice_test_checksum"
  )" == "$(realpath -e -- "$notice_test_file")" ]] || {
    printf 'release self-test rejected a valid contained notice\n' >&2
    return 1
  }
  if validate_contained_notice_file \
    "$notice_test_root" \
    "$notice_test_file" \
    "${notice_test_checksum%?}0" >/dev/null 2>&1
  then
    printf 'release self-test accepted a notice checksum mismatch\n' >&2
    return 1
  fi
  if validate_contained_notice_file \
    "$notice_test_root" \
    "$notice_test_outside" \
    "$(sha256sum < "$notice_test_outside")" >/dev/null 2>&1
  then
    printf 'release self-test accepted a notice outside its root\n' >&2
    return 1
  fi
  ln -s -- "$notice_test_file" "$notice_test_link"
  if validate_contained_notice_file \
    "$notice_test_root" \
    "$notice_test_link" \
    "$notice_test_checksum" >/dev/null 2>&1
  then
    printf 'release self-test accepted a symbolic-link notice\n' >&2
    return 1
  fi
  [[ "$(expected_rust_library_notice_sha256 1.99.0)" == \
    '5647be074c8edf7339fd863055923a8fc80bc5610a8d4661ec3b767b9d392c27' ]] || {
    printf 'release self-test found the wrong pinned Rust notice digest\n' >&2
    return 1
  }
  if expected_rust_library_notice_sha256 0.0.0 >/dev/null 2>&1; then
    printf 'release self-test accepted an unreviewed Rust toolchain notice\n' >&2
    return 1
  fi

  run_node_entrypoint \
    "$node_command" \
    "$project_dir/scripts/normalize-sbom.mjs" \
    --self-test
  run_node_entrypoint \
    "$node_command" \
    "$project_dir/scripts/generate-third-party-notices.mjs" \
    --self-test
  run_node_entrypoint "$node_command" "$project_dir/scripts/seed-npm-cache.mjs" \
    --self-test \
    "$npm_command"

  exec {test_lock_fd}>&-
  rm -rf --one-file-system -- "$test_root"
  cleanup_path=""
  printf 'atomic release-directory publication self-test passed\n'
}
