#!/usr/bin/env bash
# Fake `claude` binary for the hydra integration test harness.
#
# Stands in for the real `claude` CLI so tests don't need an API key or
# network access. Reads a list of shell commands from
# $HYDRA_TEST_FAKE_CLAUDE_COMMANDS_FILE (one per line), runs each via
# `bash -c` in the inherited cwd/env, writes per-command outputs to
# $HYDRA_TEST_FAKE_CLAUDE_OUTPUTS_DIR/<index>/{command,stdout,stderr,status},
# then emits one stream-json line on stdout so `claude.rs` can parse it.
#
# Exits non-zero on the first command failure, or always after the last
# command when $HYDRA_TEST_FAKE_CLAUDE_FAIL_AFTER_RUN=1 (used by the
# expect-failure-after-success tests).

set -uo pipefail

if [[ -z "${HYDRA_TEST_FAKE_CLAUDE_COMMANDS_FILE:-}" ]]; then
    echo "fake-claude: HYDRA_TEST_FAKE_CLAUDE_COMMANDS_FILE not set" >&2
    exit 2
fi
if [[ -z "${HYDRA_TEST_FAKE_CLAUDE_OUTPUTS_DIR:-}" ]]; then
    echo "fake-claude: HYDRA_TEST_FAKE_CLAUDE_OUTPUTS_DIR not set" >&2
    exit 2
fi

commands_file="$HYDRA_TEST_FAKE_CLAUDE_COMMANDS_FILE"
outputs_dir="$HYDRA_TEST_FAKE_CLAUDE_OUTPUTS_DIR"
fail_after_run="${HYDRA_TEST_FAKE_CLAUDE_FAIL_AFTER_RUN:-0}"

mkdir -p "$outputs_dir"

idx=0
while IFS= read -r cmd || [[ -n "$cmd" ]]; do
    if [[ -z "$cmd" ]]; then
        continue
    fi
    cmd_dir="$outputs_dir/$idx"
    mkdir -p "$cmd_dir"
    printf '%s' "$cmd" > "$cmd_dir/command"
    bash -c "$cmd" >"$cmd_dir/stdout" 2>"$cmd_dir/stderr"
    status=$?
    printf '%d' "$status" > "$cmd_dir/status"
    idx=$((idx + 1))
    if [[ $status -ne 0 ]]; then
        echo "fake-claude: command failed: $cmd (status $status)" >&2
        exit "$status"
    fi
done < "$commands_file"

if [[ "$fail_after_run" == "1" ]]; then
    echo "fake-claude: configured to fail after running commands" >&2
    exit 1
fi

# Emit one stream-json assistant line so claude.rs's parser sees valid output.
printf '%s\n' '{"type":"assistant","session_id":"fake-claude-session","message":{"role":"assistant","content":[{"type":"text","text":"ok"}]}}'
exit 0
