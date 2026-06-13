#!/bin/bash
set -euo pipefail

IFS=$'\n\t'

SCRIPT_NAME="scripts/run-xwin-screenshot-isolated-manifest.sh"

run_root_arg=""

usage() {
    cat <<'EOF'
Usage: scripts/run-xwin-screenshot-isolated-manifest.sh [--run-root PATH]

Run the TUFF-Xwin screenshot isolated manifest inside repo-local target/.
EOF
}

fail() {
    printf 'Error: %s\n' "$*" >&2
    exit 1
}

append_report() {
    printf '%s\n' "$*" >>"$REPORT_FILE"
}

append_blank_line() {
    printf '\n' >>"$REPORT_FILE"
}

record_step_header() {
    append_blank_line
    append_report "## $1"
}

record_kv() {
    append_report "- $1: $2"
}

record_command() {
    append_report "- command: \`$1\`"
}

record_step_result() {
    append_report "- result: $1"
}

record_log_path() {
    append_report "- log: \`$1\`"
}

record_artifact_inventory() {
    append_report "- artifact inventory: \`$1\`"
}

cleanup() {
    local pid
    for pid in "${CHILD_PIDS[@]:-}"; do
        if kill -0 "$pid" >/dev/null 2>&1; then
            kill "$pid" >/dev/null 2>&1 || true
        fi
    done
}

trap cleanup EXIT INT TERM

declare -a CHILD_PIDS=()

resolve_run_root() {
    local raw="$1"
    if [[ -z "$raw" ]]; then
        local run_id
        run_id="$(date +%Y%m%dT%H%M%S)-$$"
        printf '%s/target/xsm/%s\n' "$REPO_ROOT" "$run_id"
        return 0
    fi

    if [[ "$raw" == /* ]]; then
        printf '%s\n' "$raw"
    else
        printf '%s/%s\n' "$REPO_ROOT" "$raw"
    fi
}

ensure_clean_workspace() {
    local status
    status="$(git status --short --branch)"
    if [[ -n "$(printf '%s\n' "$status" | sed -n '2,$p')" ]]; then
        fail "workspace is not clean"
    fi
}

wait_for_socket() {
    local socket_path="$1"
    local pid="$2"
    local attempt
    for attempt in $(seq 1 300); do
        if [[ -S "$socket_path" ]]; then
            return 0
        fi
        if ! kill -0 "$pid" >/dev/null 2>&1; then
            wait "$pid" || true
            fail "harness exited before socket appeared: $socket_path"
        fi
        sleep 0.05
    done
    fail "socket did not appear: $socket_path"
}

snapshot_inventory() {
    local output="$1"
    shift
    : >"$output"
    local dir
    for dir in "$@"; do
        if [[ -d "$dir" ]]; then
            find "$dir" -type f -printf '%p\t%s\n'
        fi
    done | sort >"$output"
}

start_harness() {
    local log_file="$1"
    local artifact_root="$2"
    local reject_reason="${3:-}"
    local -a args=(
        cargo run -p xwin-screenshot --features dev-harness --bin xwin-screenshot-harness-displayd
        -- --socket "$SOCKET_PATH" --artifact-root "$artifact_root" --width 2 --height 2 --serve-once
    )
    if [[ -n "$reject_reason" ]]; then
        args+=(--reject "$reject_reason")
    fi

    rm -f "$SOCKET_PATH"
    "${args[@]}" >"$log_file" 2>&1 &
    local pid=$!
    CHILD_PIDS+=("$pid")
    wait_for_socket "$SOCKET_PATH" "$pid"
    printf '%s\n' "$pid"
}

run_simple_step() {
    local step_name="$1"
    local command_display="$2"
    local log_file="$3"
    shift 3

    : >"$log_file"
    record_step_header "$step_name"
    record_command "$command_display"
    record_log_path "$log_file"

    if "$@" >"$log_file" 2>&1; then
        record_step_result "PASS"
    else
        local rc=$?
        record_step_result "FAIL (exit $rc)"
        exit "$rc"
    fi
}

run_logged_shell() {
    local step_name="$1"
    local command_display="$2"
    local log_file="$3"
    local shell_command="$4"

    : >"$log_file"
    record_step_header "$step_name"
    record_command "$command_display"
    record_log_path "$log_file"

    if bash -lc "set -euo pipefail; $shell_command" >"$log_file" 2>&1; then
        record_step_result "PASS"
    else
        local rc=$?
        record_step_result "FAIL (exit $rc)"
        exit "$rc"
    fi
}

run_expected_failure_shell() {
    local step_name="$1"
    local command_display="$2"
    local log_file="$3"
    local shell_command="$4"

    record_step_header "$step_name"
    record_command "$command_display"
    record_log_path "$log_file"

    if bash -lc "set -euo pipefail; $shell_command" >"$log_file" 2>&1; then
        record_step_result "FAIL (unexpected success)"
        exit 1
    fi

    local rc=$?
    record_step_result "PASS (expected failure, exit $rc)"
}

step0_identity_workspace() {
    local pwd_log="$LOG_DIR/step0-pwd.log"
    local show_root_log="$LOG_DIR/step0-show-toplevel.log"
    local head_log="$LOG_DIR/step0-head.log"
    local status_log="$LOG_DIR/step0-status.log"

    run_simple_step "Step 0.1 Identity: pwd" "pwd" "$pwd_log" pwd
    run_simple_step "Step 0.2 Identity: git rev-parse --show-toplevel" "git rev-parse --show-toplevel" "$show_root_log" git rev-parse --show-toplevel
    run_simple_step "Step 0.3 Identity: git rev-parse HEAD" "git rev-parse HEAD" "$head_log" git rev-parse HEAD
    run_simple_step "Step 0.4 Identity: git status --short --branch" "git status --short --branch" "$status_log" git status --short --branch

    if [[ -n "$(sed -n '2,$p' "$status_log")" ]]; then
        fail "workspace is not clean"
    fi

    record_step_header "Step 0 Workspace Identity Summary"
    record_kv "cwd" "$PWD"
    record_kv "repo_root" "$REPO_ROOT"
    record_kv "current_head" "$(cat "$head_log")"
}

step1_repo_validation() {
    run_simple_step "Step 1.1 Repo Validation: cargo fmt --check" "cargo fmt --check" "$LOG_DIR/step1-cargo-fmt.log" cargo fmt --check
    run_simple_step "Step 1.2 Repo Validation: cargo check --workspace" "cargo check --workspace" "$LOG_DIR/step1-cargo-check.log" cargo check --workspace
    run_simple_step "Step 1.3 Repo Validation: cargo test --workspace" "cargo test --workspace" "$LOG_DIR/step1-cargo-test.log" cargo test --workspace
    run_simple_step "Step 1.4 Repo Validation: git diff --check" "git diff --check" "$LOG_DIR/step1-git-diff-check.log" git diff --check
}

step1b_browser_surface_boundary_regression() {
    run_simple_step "Step 1b Browser Surface Boundary Regression" "cargo test -p xwin-sec --test browser_surface_boundary" "$LOG_DIR/step1b-browser-surface-boundary.log" cargo test -p xwin-sec --test browser_surface_boundary
}

step2_fake_png() {
    local step_dir="$OUT_DIR/step2-fake-png"
    local before="$RUN_ROOT/step2-before.txt"
    local after="$RUN_ROOT/step2-after.txt"
    local inventory="$LOG_DIR/step2-artifacts.txt"
    mkdir -p "$step_dir"
    snapshot_inventory "$before" "$step_dir"
    run_logged_shell \
        "Step 2 Fake Backend PNG" \
        "cargo run -p xwin-screenshot -- --backend fake --format png --save-dir ${step_dir}" \
        "$LOG_DIR/step2-fake-png.log" \
        "cargo run -p xwin-screenshot -- --backend fake --format png --save-dir \"${step_dir}\""
    snapshot_inventory "$after" "$step_dir"
    comm -13 "$before" "$after" >"$inventory"
    record_artifact_inventory "$inventory"
    if [[ -s "$inventory" ]]; then
        append_report "- generated files:"
        while IFS=$'\t' read -r path size; do
            append_report "  - \`$path\` ($size bytes)"
        done <"$inventory"
    fi
}

step3_fake_jpeg() {
    local step_dir="$OUT_DIR/step3-fake-jpeg"
    local before="$RUN_ROOT/step3-before.txt"
    local after="$RUN_ROOT/step3-after.txt"
    local inventory="$LOG_DIR/step3-artifacts.txt"
    mkdir -p "$step_dir"
    snapshot_inventory "$before" "$step_dir"
    run_logged_shell \
        "Step 3 Fake Backend JPEG" \
        "cargo run -p xwin-screenshot -- --backend fake --format jpeg --save-dir ${step_dir}" \
        "$LOG_DIR/step3-fake-jpeg.log" \
        "cargo run -p xwin-screenshot -- --backend fake --format jpeg --save-dir \"${step_dir}\""
    snapshot_inventory "$after" "$step_dir"
    comm -13 "$before" "$after" >"$inventory"
    record_artifact_inventory "$inventory"
    if [[ -s "$inventory" ]]; then
        append_report "- generated files:"
        while IFS=$'\t' read -r path size; do
            append_report "  - \`$path\` ($size bytes)"
        done <"$inventory"
    fi
}

step4_dev_harness_png() {
    local step_dir="$OUT_DIR/step4-dev-harness-png"
    local artifact_dir="$ARTIFACT_ROOT/step4-dev-harness-png"
    local before="$RUN_ROOT/step4-before.txt"
    local after="$RUN_ROOT/step4-after.txt"
    local inventory="$LOG_DIR/step4-artifacts.txt"
    local harness_log="$LOG_DIR/step4-harness.log"
    local client_log="$LOG_DIR/step4-client.log"
    mkdir -p "$step_dir" "$artifact_dir"
    snapshot_inventory "$before" "$step_dir" "$artifact_dir"
    local harness_pid
    harness_pid="$(start_harness "$harness_log" "$artifact_dir")"
    run_logged_shell \
        "Step 4 Dev Harness PNG Client" \
        "cargo run -p xwin-screenshot -- --backend isolated-displayd --displayd-socket ${SOCKET_PATH} --artifact-root ${artifact_dir} --format png --save-dir ${step_dir}" \
        "$client_log" \
        "cargo run -p xwin-screenshot -- --backend isolated-displayd --displayd-socket \"${SOCKET_PATH}\" --artifact-root \"${artifact_dir}\" --format png --save-dir \"${step_dir}\""
    wait "$harness_pid"
    rm -f "$SOCKET_PATH"
    snapshot_inventory "$after" "$step_dir" "$artifact_dir"
    comm -13 "$before" "$after" >"$inventory"
    record_log_path "$harness_log"
    record_artifact_inventory "$inventory"
    if [[ -s "$inventory" ]]; then
        append_report "- generated files:"
        while IFS=$'\t' read -r path size; do
            append_report "  - \`$path\` ($size bytes)"
        done <"$inventory"
    fi
}

step5_dev_harness_jpeg() {
    local step_dir="$OUT_DIR/step5-dev-harness-jpeg"
    local artifact_dir="$ARTIFACT_ROOT/step5-dev-harness-jpeg"
    local before="$RUN_ROOT/step5-before.txt"
    local after="$RUN_ROOT/step5-after.txt"
    local inventory="$LOG_DIR/step5-artifacts.txt"
    local harness_log="$LOG_DIR/step5-harness.log"
    local client_log="$LOG_DIR/step5-client.log"
    mkdir -p "$step_dir" "$artifact_dir"
    snapshot_inventory "$before" "$step_dir" "$artifact_dir"
    local harness_pid
    harness_pid="$(start_harness "$harness_log" "$artifact_dir")"
    run_logged_shell \
        "Step 5 Dev Harness JPEG Client" \
        "cargo run -p xwin-screenshot -- --backend isolated-displayd --displayd-socket ${SOCKET_PATH} --artifact-root ${artifact_dir} --format jpeg --save-dir ${step_dir}" \
        "$client_log" \
        "cargo run -p xwin-screenshot -- --backend isolated-displayd --displayd-socket \"${SOCKET_PATH}\" --artifact-root \"${artifact_dir}\" --format jpeg --save-dir \"${step_dir}\""
    wait "$harness_pid"
    rm -f "$SOCKET_PATH"
    snapshot_inventory "$after" "$step_dir" "$artifact_dir"
    comm -13 "$before" "$after" >"$inventory"
    record_log_path "$harness_log"
    record_artifact_inventory "$inventory"
    if [[ -s "$inventory" ]]; then
        append_report "- generated files:"
        while IFS=$'\t' read -r path size; do
            append_report "  - \`$path\` ($size bytes)"
        done <"$inventory"
    fi
}

write_config() {
    local path="$1"
    shift
    cat >"$path" <<EOF
$*
EOF
}

step6_config_fake_flow() {
    local png_dir="$OUT_DIR/step6-config-fake-png"
    local jpeg_dir="$OUT_DIR/step6-config-fake-jpeg"
    local png_cfg="$CONFIG_DIR/fake-png.toml"
    local jpeg_cfg="$CONFIG_DIR/fake-jpeg.toml"
    local before="$RUN_ROOT/step6-before.txt"
    local after="$RUN_ROOT/step6-after.txt"
    local inventory="$LOG_DIR/step6-artifacts.txt"
    local png_log="$LOG_DIR/step6-config-fake-png.log"
    local jpeg_log="$LOG_DIR/step6-config-fake-jpeg.log"
    mkdir -p "$png_dir" "$jpeg_dir"
    snapshot_inventory "$before" "$png_dir" "$jpeg_dir"
    cat >"$png_cfg" <<EOF
backend = "fake"
format = "png"
save_dir = "$png_dir"
png_compression = 6
jpeg_quality = 90
EOF
    cat >"$jpeg_cfg" <<EOF
backend = "fake"
format = "jpeg"
save_dir = "$jpeg_dir"
png_compression = 6
jpeg_quality = 90
EOF
    run_logged_shell \
        "Step 6.1 Config Fake PNG" \
        "cargo run -p xwin-screenshot -- --config ${png_cfg}" \
        "$png_log" \
        "cargo run -p xwin-screenshot -- --config \"${png_cfg}\""
    run_logged_shell \
        "Step 6.2 Config Fake JPEG" \
        "cargo run -p xwin-screenshot -- --config ${jpeg_cfg}" \
        "$jpeg_log" \
        "cargo run -p xwin-screenshot -- --config \"${jpeg_cfg}\""
    snapshot_inventory "$after" "$png_dir" "$jpeg_dir"
    comm -13 "$before" "$after" >"$inventory"
    record_artifact_inventory "$inventory"
    if [[ -s "$inventory" ]]; then
        append_report "- generated files:"
        while IFS=$'\t' read -r path size; do
            append_report "  - \`$path\` ($size bytes)"
        done <"$inventory"
    fi
}

step7_config_isolated_displayd_flow() {
    local png_dir="$OUT_DIR/step7-config-isolated-displayd-png"
    local jpeg_dir="$OUT_DIR/step7-config-isolated-displayd-jpeg"
    local png_artifact_dir="$ARTIFACT_ROOT/step7-config-isolated-displayd-png"
    local jpeg_artifact_dir="$ARTIFACT_ROOT/step7-config-isolated-displayd-jpeg"
    local png_cfg="$CONFIG_DIR/isolated-displayd-png.toml"
    local jpeg_cfg="$CONFIG_DIR/isolated-displayd-jpeg.toml"
    local before="$RUN_ROOT/step7-before.txt"
    local after="$RUN_ROOT/step7-after.txt"
    local inventory="$LOG_DIR/step7-artifacts.txt"
    local harness_log_png="$LOG_DIR/step7-harness-png.log"
    local harness_log_jpeg="$LOG_DIR/step7-harness-jpeg.log"
    local client_log_png="$LOG_DIR/step7-config-isolated-displayd-png.log"
    local client_log_jpeg="$LOG_DIR/step7-config-isolated-displayd-jpeg.log"
    mkdir -p "$png_dir" "$jpeg_dir" "$png_artifact_dir" "$jpeg_artifact_dir"
    snapshot_inventory "$before" "$png_dir" "$jpeg_dir" "$png_artifact_dir" "$jpeg_artifact_dir"
    cat >"$png_cfg" <<EOF
backend = "isolated-displayd"
displayd_socket = "$SOCKET_PATH"
artifact_root = "$png_artifact_dir"
format = "png"
save_dir = "$png_dir"
png_compression = 6
jpeg_quality = 90
EOF
    cat >"$jpeg_cfg" <<EOF
backend = "isolated-displayd"
displayd_socket = "$SOCKET_PATH"
artifact_root = "$jpeg_artifact_dir"
format = "jpeg"
save_dir = "$jpeg_dir"
png_compression = 6
jpeg_quality = 90
EOF
    local harness_pid
    harness_pid="$(start_harness "$harness_log_png" "$png_artifact_dir")"
    run_logged_shell \
        "Step 7.1 Config Isolated Displayd PNG" \
        "cargo run -p xwin-screenshot -- --config ${png_cfg}" \
        "$client_log_png" \
        "cargo run -p xwin-screenshot -- --config \"${png_cfg}\""
    wait "$harness_pid"
    harness_pid="$(start_harness "$harness_log_jpeg" "$jpeg_artifact_dir")"
    run_logged_shell \
        "Step 7.2 Config Isolated Displayd JPEG" \
        "cargo run -p xwin-screenshot -- --config ${jpeg_cfg}" \
        "$client_log_jpeg" \
        "cargo run -p xwin-screenshot -- --config \"${jpeg_cfg}\""
    wait "$harness_pid"
    rm -f "$SOCKET_PATH"
    snapshot_inventory "$after" "$png_dir" "$jpeg_dir" "$png_artifact_dir" "$jpeg_artifact_dir"
    comm -13 "$before" "$after" >"$inventory"
    record_log_path "$harness_log_png"
    record_log_path "$harness_log_jpeg"
    record_artifact_inventory "$inventory"
    if [[ -s "$inventory" ]]; then
        append_report "- generated files:"
        while IFS=$'\t' read -r path size; do
            append_report "  - \`$path\` ($size bytes)"
        done <"$inventory"
    fi
}

step8_negative_contract_checks() {
    local test_log1="$LOG_DIR/step8-openat-rejects.log"
    local test_log2="$LOG_DIR/step8-dev-harness-arg-parser.log"
    local runtime_log="$LOG_DIR/step8-dev-harness-reject-runtime.log"
    local reject_artifact_dir="$ARTIFACT_ROOT/step8-negative-contract"
    local reject_out_dir="$OUT_DIR/step8-negative-contract"
    local before="$RUN_ROOT/step8-before.txt"
    local after="$RUN_ROOT/step8-after.txt"
    local inventory="$LOG_DIR/step8-artifacts.txt"
    mkdir -p "$reject_artifact_dir" "$reject_out_dir"
    snapshot_inventory "$before" "$reject_artifact_dir" "$reject_out_dir"
    run_simple_step \
        "Step 8.1 Negative Tests: openat_reader_rejects" \
        "cargo test -p xwin-screenshot openat_reader_rejects -- --nocapture" \
        "$test_log1" \
        cargo test -p xwin-screenshot openat_reader_rejects -- --nocapture
    run_simple_step \
        "Step 8.2 Negative Tests: dev_harness_binary_arg_parser_rejects" \
        "cargo test -p xwin-screenshot --features dev-harness dev_harness_binary_arg_parser_rejects -- --nocapture" \
        "$test_log2" \
        cargo test -p xwin-screenshot --features dev-harness dev_harness_binary_arg_parser_rejects -- --nocapture

    record_step_header "Step 8.3 Negative Runtime: dev harness reject mode"
    record_command "cargo run -p xwin-screenshot --features dev-harness --bin xwin-screenshot-harness-displayd -- --socket ${SOCKET_PATH} --artifact-root ${reject_artifact_dir} --width 2 --height 2 --serve-once --reject policy denied"
    record_command "cargo run -p xwin-screenshot -- --backend isolated-displayd --displayd-socket ${SOCKET_PATH} --artifact-root ${reject_artifact_dir} --format png --save-dir ${reject_out_dir}"
    record_log_path "$runtime_log"
    local harness_pid
    harness_pid="$(start_harness "$runtime_log" "$reject_artifact_dir" "policy denied")"
    set +e
    cargo run -p xwin-screenshot -- --backend isolated-displayd --displayd-socket "$SOCKET_PATH" --artifact-root "$reject_artifact_dir" --format png --save-dir "$reject_out_dir" >>"$runtime_log" 2>&1
    client_rc=$?
    set -e
    wait "$harness_pid"
    rm -f "$SOCKET_PATH"
    if [[ $client_rc -eq 0 ]]; then
        echo "unexpected success for reject mode" >&2
        exit 1
    fi
    if ! grep -q "displayd rejected capture request" "$runtime_log"; then
        echo "reject-mode runtime did not report displayd rejection" >&2
        exit 1
    fi
    snapshot_inventory "$after" "$reject_artifact_dir" "$reject_out_dir"
    comm -13 "$before" "$after" >"$inventory"
    record_artifact_inventory "$inventory"
    if [[ -s "$inventory" ]]; then
        append_report "- generated files:"
        while IFS=$'\t' read -r path size; do
            append_report "  - \`$path\` ($size bytes)"
        done <"$inventory"
    fi
    record_step_result "PASS"
}

write_report_footer() {
    append_blank_line
    append_report "## Summary"
    record_kv "run_root" "$RUN_ROOT"
    record_kv "tmp_root" "$TMP_ROOT"
    record_kv "socket_path" "$SOCKET_PATH"
    record_kv "artifact_root" "$ARTIFACT_ROOT"
    record_kv "out_dir" "$OUT_DIR"
    record_kv "config_dir" "$CONFIG_DIR"
    record_kv "log_dir" "$LOG_DIR"
    record_kv "report_file" "$REPORT_FILE"
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --run-root)
                [[ $# -ge 2 ]] || fail "--run-root requires a value"
                run_root_arg="$2"
                shift 2
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            *)
                fail "unknown argument: $1"
                ;;
        esac
    done
}

main() {
    parse_args "$@"

    REPO_ROOT="$(git rev-parse --show-toplevel)"
    cd "$REPO_ROOT"

    ensure_clean_workspace

    RUN_ROOT="$(resolve_run_root "$run_root_arg")"
    case "$RUN_ROOT" in
        /run/user|/run/user/*)
            fail "run root must not be under /run/user"
            ;;
    esac

    TMP_ROOT="$RUN_ROOT/tmp"
    SOCKET_PATH="$TMP_ROOT/xwin-displayd-harness.sock"
    ARTIFACT_ROOT="$TMP_ROOT/artifacts"
    OUT_DIR="$TMP_ROOT/out"
    CONFIG_DIR="$TMP_ROOT/config"
    LOG_DIR="$RUN_ROOT/logs"
    REPORT_FILE="$RUN_ROOT/XWIN_SCREENSHOT_RUN_REPORT.md"

    mkdir -p "$RUN_ROOT" "$TMP_ROOT" "$ARTIFACT_ROOT" "$OUT_DIR" "$CONFIG_DIR" "$LOG_DIR"
    : >"$REPORT_FILE"

    append_report "# TUFF-Xwin Screenshot Isolated Manifest Run"
    record_kv "script" "$SCRIPT_NAME"
    record_kv "repo_root" "$REPO_ROOT"
    record_kv "run_root" "$RUN_ROOT"
    record_kv "tmp_root" "$TMP_ROOT"
    record_kv "socket_path" "$SOCKET_PATH"
    record_kv "artifact_root" "$ARTIFACT_ROOT"
    record_kv "out_dir" "$OUT_DIR"
    record_kv "config_dir" "$CONFIG_DIR"
    record_kv "log_dir" "$LOG_DIR"
    record_kv "report_file" "$REPORT_FILE"

    step0_identity_workspace
    step1_repo_validation
    step1b_browser_surface_boundary_regression
    step2_fake_png
    step3_fake_jpeg
    step4_dev_harness_png
    step5_dev_harness_jpeg
    step6_config_fake_flow
    step7_config_isolated_displayd_flow
    step8_negative_contract_checks
    write_report_footer

    printf 'Run report written to %s\n' "$REPORT_FILE"
}

main "$@"
