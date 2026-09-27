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

fn json_output(out: Output) -> serde_json::Value {
    serde_json::from_slice(&success(out).stdout).unwrap()
}

#[test]
fn human_default_and_explicit_formats_share_queryable_data() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let empty = success(run(&root, &["extension", "list"]));
    assert!(
        String::from_utf8(empty.stdout)
            .unwrap()
            .contains("No extensions enabled")
    );
    assert_eq!(
        json_output(run(&root, &["extension", "list", "--output", "json"])),
        serde_json::json!([])
    );
    assert!(!root.join("state").exists());
    fixture(&root);
    let receipt = json_output(run(
        &root,
        &[
            "--output",
            "json",
            "add",
            "team",
            "policy.hcl",
            "--priority",
            "10",
        ],
    ));
    assert_eq!(receipt["source"]["name"], "team");
    success(run(
        &root,
        &["add", "company", "policy.hcl", "--priority", "0"],
    ));
    let human = String::from_utf8(success(run(&root, &["list"])).stdout).unwrap();
    assert!(human.contains("NAME") && human.contains("PRIORITY") && human.contains("company"));
    assert!(!human.trim_start().starts_with('['));
    let all = json_output(run(&root, &["list", "-o", "json"]));
    let yaml = success(run(&root, &["list", "-o", "yaml"]));
    assert_eq!(
        serde_norway::from_slice::<serde_json::Value>(&yaml.stdout).unwrap(),
        all
    );
    let jsonl = success(run(&root, &["list", "-o", "jsonl"]));
    assert_eq!(String::from_utf8_lossy(&jsonl.stdout).lines().count(), 1);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&jsonl.stdout).unwrap(),
        all
    );
    assert_eq!(
        json_output(run(
            &root,
            &[
                "list",
                "-o",
                "json",
                "--query",
                "[?priority >= `10`].{name:name,priority:priority}"
            ]
        )),
        serde_json::json!([{"name":"team","priority":10}])
    );
    let text = success(run(
        &root,
        &[
            "list",
            "--query",
            "sort_by(@, &priority)[].[name, priority]",
            "-o",
            "text",
        ],
    ));
    assert_eq!(
        String::from_utf8(text.stdout).unwrap(),
        "company\t0\nteam\t10\n"
    );
    let table = success(run(
        &root,
        &[
            "list",
            "--query",
            "[].{name:name,priority:priority}",
            "-o",
            "table",
        ],
    ));
    assert!(
        String::from_utf8(table.stdout)
            .unwrap()
            .contains("PRIORITY")
    );
    assert_eq!(
        json_output(run(
            &root,
            &["list", "-o", "json", "--query", "[0].missing"]
        )),
        serde_json::Value::Null
    );
    assert_eq!(
        json_output(run(
            &root,
            &["list", "-o", "json", "--query", "[?name == 'absent']"]
        )),
        serde_json::json!([])
    );
    assert!(success(run(&root, &["list", "--quiet"])).stdout.is_empty());
}

#[test]
fn query_errors_prevent_writes_and_filtering_does_not_change_execution_or_exit_codes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fixture(&root);
    let invalid = run(&root, &["add", "personal", "policy.hcl", "--query", "["]);
    assert!(!invalid.status.success() && invalid.stdout.is_empty());
    assert!(!root.join("state").exists());
    success(run(&root, &["add", "personal", "policy.hcl"]));
    let config = fs::read(root.join("state/config.json")).unwrap();
    let cache = fs::read(root.join("state/cache/personal.json")).unwrap();
    for query in ["length(changed_files)", "unknown_function(@)"] {
        let bad = run(&root, &["apply", "--query", query]);
        assert!(!bad.status.success() && bad.stdout.is_empty());
        assert!(String::from_utf8_lossy(&bad.stderr).contains("--query"));
        assert!(!root.join("settings").exists());
        assert!(!root.join("state/backups").exists());
        assert_eq!(
            fs::read(root.join("state/cache/personal.json")).unwrap(),
            cache
        );
    }
    assert_eq!(fs::read(root.join("state/config.json")).unwrap(), config);
    let check = run(
        &root,
        &[
            "apply",
            "--check",
            "--query",
            "changed_files",
            "--output",
            "json",
        ],
    );
    assert_eq!(check.status.code(), Some(2));
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&check.stdout).unwrap(),
        1
    );
    assert!(!root.join("settings").exists());
    let quiet = run(&root, &["apply", "--check", "--quiet"]);
    assert_eq!(quiet.status.code(), Some(2));
    assert!(quiet.stdout.is_empty());
    let human = success(run(&root, &["apply", "--dry-run"]));
    assert!(String::from_utf8_lossy(&human.stdout).contains("WILL REPAIR"));
    assert_eq!(
        json_output(run(
            &root,
            &[
                "apply",
                "--output",
                "json",
                "--query",
                "files[?path == 'absent']"
            ]
        )),
        serde_json::json!([])
    );
    assert_eq!(
        fs::read_to_string(root.join("settings")).unwrap(),
        "managed"
    );
    let clean = success(run(&root, &["apply", "--check"]));
    assert!(String::from_utf8_lossy(&clean.stdout).contains("No changes needed"));
}

#[test]
fn formatting_and_queries_cannot_reveal_sensitive_diffs_or_private_keys() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::write(root.join("settings"), "OLD_SENSITIVE_VALUE").unwrap();
    fs::write(root.join("policy.hcl"), "schema_version=1\nname=\"personal\"\nrule \"private\" {\n target=\"settings\"\n kind=\"file\"\n operation=\"replace\"\n value=\"NEW_SENSITIVE_VALUE\"\n sensitive=true\n}\n").unwrap();
    success(run(&root, &["add", "personal", "policy.hcl"]));
    for format in ["human", "json", "jsonl", "yaml", "table", "text"] {
        let out = success(run(
            &root,
            &["apply", "--dry-run", "--diff", "--output", format],
        ));
        assert!(!String::from_utf8_lossy(&out.stdout).contains("SENSITIVE_VALUE"));
    }
    assert_eq!(
        json_output(run(
            &root,
            &[
                "apply",
                "--dry-run",
                "--diff",
                "--output",
                "json",
                "--query",
                "files[0].diff"
            ]
        )),
        serde_json::Value::Null
    );
    let identity = root.join("provider.agekey");
    let key = json_output(run(
        &root,
        &[
            "report",
            "keygen",
            identity.to_str().unwrap(),
            "--output",
            "json",
        ],
    ));
    assert!(key["recipient"].as_str().unwrap().starts_with("age1"));
    assert!(!key.to_string().contains("AGE-SECRET-KEY"));
    assert!(
        fs::read_to_string(identity)
            .unwrap()
            .starts_with("AGE-SECRET-KEY")
    );
}

#[test]
fn daemon_streams_complete_json_lines_and_rejects_nonstreaming_formats() {
    use std::{process::Stdio, thread, time::Duration};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let invalid = run(&root, &["daemon", "--output", "json"]);
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("jsonl"));
    assert!(!root.join("state").exists());
    fixture(&root);
    success(run(&root, &["add", "personal", "policy.hcl"]));
    let log = root.join("stream.jsonl");
    let mut child = Command::new(env!("CARGO_BIN_EXE_syncer"))
        .env_remove("SYNCER_DEV_EXTENSIONS")
        .args([
            "--state-dir",
            root.join("state").to_str().unwrap(),
            "--project",
            root.to_str().unwrap(),
            "daemon",
            "--interval",
            "1",
            "--output",
            "jsonl",
            "--query",
            "{changed:changed_files,ok:compliant}",
        ])
        .stdout(fs::File::create(&log).unwrap())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut records = Vec::new();
    for _ in 0..100 {
        thread::sleep(Duration::from_millis(100));
        records = fs::read_to_string(&log)
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .collect();
        if records.len() >= 2 {
            break;
        }
    }
    let _ = child.kill();
    child.wait().unwrap();
    assert!(records.len() >= 2, "daemon did not emit two JSON records");
    assert_eq!(records[0], serde_json::json!({"changed":1,"ok":true}));
    assert_eq!(records[1], serde_json::json!({"changed":0,"ok":true}));
    let once = json_output(run(&root, &["daemon", "--once", "--output", "json"]));
    assert_eq!(once["changed_files"], 0);
}

#[test]
fn hidden_violations_still_block_apply_and_keep_violation_exit_code() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fixture(&root);
    let mut policy = fs::read_to_string(root.join("policy.hcl")).unwrap();
    policy.push_str("\nrule \"audit\" {\n target=\"audit.txt\"\n kind=\"text\"\n operation=\"match\"\n pattern=\"mandatory\"\n}\n");
    fs::write(root.join("policy.hcl"), policy).unwrap();
    success(run(&root, &["add", "personal", "policy.hcl"]));
    let out = run(
        &root,
        &[
            "apply",
            "--check",
            "--query",
            "rules[?after == `true`]",
            "--output",
            "json",
        ],
    );
    assert_eq!(out.status.code(), Some(3));
    assert!(
        serde_json::from_slice::<serde_json::Value>(&out.stdout)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["after"] == true)
    );
    let apply = run(&root, &["apply"]);
    assert_eq!(apply.status.code(), Some(1));
    let text = String::from_utf8_lossy(&apply.stdout);
    assert!(text.contains("Unresolved violations") && text.contains("VIOLATION"));
    assert!(!text.contains("REPAIRED"));
    assert!(!root.join("settings").exists());
    assert!(!root.join("state/backups").exists());
}
