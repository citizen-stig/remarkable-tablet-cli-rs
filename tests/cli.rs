//! Binary-level tests for argument parsing and help output.
//!
//! These spawn the actual `remarkable-cli` binary. Anything that requires
//! talking to a tablet lives in the integration tests under `tests/browse.rs`
//! and `tests/output_snapshots.rs`.

use assert_cmd::Command;
use insta::assert_snapshot;
use predicates::str;

fn cli() -> Command {
    Command::cargo_bin("remarkable-cli").expect("cargo_bin remarkable-cli")
}

#[test]
fn root_help_lists_every_subcommand() {
    let out = cli().arg("--help").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    for sub in [
        "connect", "ls", "info", "find", "backup", "download", "upload", "mv", "mkdir", "rename",
        "rm",
    ] {
        assert!(
            stdout.contains(sub),
            "--help should mention `{sub}` subcommand, got:\n{stdout}"
        );
    }
}

#[test]
fn root_help_snapshot() {
    let out = cli().arg("--help").assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_snapshot!(stdout);
}

#[test]
fn version_flag_succeeds() {
    cli()
        .arg("--version")
        .assert()
        .success()
        .stdout(str::contains("remarkable-cli"));
}

#[test]
fn unknown_subcommand_exits_nonzero() {
    cli().arg("blorp").assert().failure();
}

#[test]
fn unknown_flag_exits_nonzero() {
    cli().arg("--no-such-flag").assert().failure();
}

#[test]
fn ls_help_snapshot() {
    let out = cli().args(["ls", "--help"]).assert().success();
    let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
    assert_snapshot!(stdout);
}

#[test]
fn info_help_lists_path_or_uuid_argument() {
    cli()
        .args(["info", "--help"])
        .assert()
        .success()
        .stdout(str::contains("PATH_OR_UUID"));
}

#[test]
fn info_without_required_arg_fails() {
    cli().arg("info").assert().failure();
}

#[test]
fn find_without_pattern_fails() {
    cli().arg("find").assert().failure();
}

#[test]
fn ls_invalid_kind_value_fails() {
    cli()
        .args(["ls", "--kind", "bogus"])
        .assert()
        .failure()
        .stderr(str::contains("invalid value"));
}

#[test]
fn ls_invalid_format_value_fails() {
    cli()
        .args(["--format", "yaml", "ls"])
        .assert()
        .failure()
        .stderr(str::contains("invalid value"));
}

#[test]
fn ls_invalid_sort_value_fails() {
    cli()
        .args(["ls", "--sort", "bogus"])
        .assert()
        .failure()
        .stderr(str::contains("invalid value"));
}
