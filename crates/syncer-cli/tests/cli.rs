use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_syncer"))
        .env_remove("SYNCER_DEV_EXTENSIONS")
        .args([
            "--state-dir",
            root.join("state").to_str().unwrap(),
            "--project",
            root.to_str().unwrap(),
        ])
        .args(args)
        .output()
        .unwrap()
}
fn success(out: Output) -> Output {
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    out
}
fn fixture(root: &Path) {
    fs::write(root.join("policy.hcl"),"schema_version=1\nname=\"personal\"\nrule \"a\" {\ntarget=\"settings\"\nkind=\"file\"\noperation=\"replace\"\nvalue=\"managed\"\n}").unwrap();
}
#[test]
fn enrollment_dry_run_check_and_daemon_once() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fixture(&root);
    success(run(&root, &["add", "personal", "policy.hcl"]));
    let before = fs::read(root.join("state/config.json")).unwrap();
    let cache = fs::read(root.join("state/cache/personal.json")).unwrap();
    success(run(&root, &["apply", "--dry-run", "--diff"]));
    assert!(!root.join("settings").exists());
    assert_eq!(before, fs::read(root.join("state/config.json")).unwrap());
    assert_eq!(
        cache,
        fs::read(root.join("state/cache/personal.json")).unwrap()
    );
    assert!(!root.join("state/backups").exists());
    assert_eq!(run(&root, &["apply", "--check"]).status.code(), Some(2));
    success(run(&root, &["daemon", "--once"]));
    assert_eq!(
        fs::read_to_string(root.join("settings")).unwrap(),
        "managed"
    );
    success(run(&root, &["apply", "--check"]));
}
#[test]
fn read_only_commands_do_not_create_state() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fixture(&root);
    success(run(&root, &["list"]));
    success(run(
        &root,
        &["validate", root.join("policy.hcl").to_str().unwrap()],
    ));
    assert!(!root.join("state").exists());
}
#[test]
fn remote_policy_cannot_change_enrollment() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fixture(&root);
    let policy = fs::read_to_string(root.join("policy.hcl"))
        .unwrap()
        .replace("target=\"settings\"", "target=\"state/config.json\"");
    fs::write(root.join("policy.hcl"), policy).unwrap();
    success(run(&root, &["add", "personal", "policy.hcl"]));
    let before = fs::read(root.join("state/config.json")).unwrap();
    assert!(!run(&root, &["apply"]).status.success());
    assert_eq!(before, fs::read(root.join("state/config.json")).unwrap());
}
#[test]
fn push_detects_remote_change() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fixture(&root);
    success(run(&root, &["add", "personal", "policy.hcl"]));
    fs::copy(root.join("policy.hcl"), root.join("updated.hcl")).unwrap();
    let original = fs::read_to_string(root.join("policy.hcl")).unwrap();
    fs::write(
        root.join("policy.hcl"),
        original + "\n# someone else edited\n",
    )
    .unwrap();
    assert!(
        !run(&root, &["push", "personal", "updated.hcl"])
            .status
            .success()
    );
    assert!(
        fs::read_to_string(root.join("policy.hcl"))
            .unwrap()
            .contains("someone else")
    );
}

#[test]
fn policy_cannot_rewrite_an_active_source_for_next_cycle() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fixture(&root);
    let policy = fs::read_to_string(root.join("policy.hcl"))
        .unwrap()
        .replace("target=\"settings\"", "target=\"policy.hcl\"");
    fs::write(root.join("policy.hcl"), &policy).unwrap();
    success(run(&root, &["add", "personal", "policy.hcl"]));
    assert!(!run(&root, &["apply"]).status.success());
    assert_eq!(fs::read_to_string(root.join("policy.hcl")).unwrap(), policy);
}
