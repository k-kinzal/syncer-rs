use serde_json::{Value, json};
use syncer_extension_sdk::Request;

fn call(
    kind: &str,
    source: &str,
    pointer: &str,
    operation: &str,
    value: Value,
    constraints: Value,
    action: &str,
) -> anyhow::Result<Value> {
    syncer_extension_structured::dispatch(Request {
        method: "document".into(),
        params: json!({"content":source,"action":action,"rule":{"target":"fixture","kind":kind,"pointer":pointer,"operation":operation,"value":value,"constraints":constraints}}),
    })
}
fn set(kind: &str, source: &str, pointer: &str, value: Value) -> String {
    let before = call(
        kind,
        source,
        pointer,
        "set",
        value.clone(),
        json!({}),
        "check",
    )
    .unwrap();
    assert_eq!(before["content"], source);
    assert_eq!(before["compliant"], false);
    let out = call(
        kind,
        source,
        pointer,
        "set",
        value.clone(),
        json!({}),
        "apply",
    )
    .unwrap();
    let result = out["content"].as_str().unwrap();
    let again = call(
        kind,
        result,
        pointer,
        "set",
        value.clone(),
        json!({}),
        "apply",
    )
    .unwrap();
    assert_eq!(again["content"], result, "{kind} must be idempotent");
    assert_eq!(again["compliant"], true);
    result.into()
}

#[test]
fn edits_selected_fields_in_every_format() {
    let cases = [
        (
            "yaml",
            "# personal\ntheme: dark\nserver:\n  port: 80 # managed\n",
            "/server/port",
            json!(443),
            "theme: dark",
        ),
        (
            "toml",
            "# personal\ntheme = 'dark'\n[server]\nport = 80 # managed\n",
            "/server/port",
            json!(443),
            "theme = 'dark'",
        ),
        (
            "hcl",
            "# personal\ntheme = \"dark\"\nserver \"api\" {\n port = 80\n dynamic = var.keep\n}\n",
            "/server/api/port",
            json!(443),
            "dynamic = var.keep",
        ),
        (
            "xml",
            "<?xml version=\"1.0\"?><config><!-- personal --><theme>dark</theme><server port='80'/></config>",
            "/config/server/0/@port",
            json!("443"),
            "<!-- personal --><theme>dark</theme>",
        ),
        (
            "jsonc",
            "{\n// personal\n\"theme\": \"dark\", \"port\": 80,\n}",
            "/port",
            json!(443),
            "// personal",
        ),
        (
            "json5",
            "{ // personal\n theme: 'dark', port: +80, }",
            "/port",
            json!(443),
            "theme: 'dark'",
        ),
        (
            "ini",
            "; personal\ntheme=dark\n[server]\nport = 80 ; managed\n",
            "/server/port",
            json!("443"),
            "theme=dark",
        ),
        (
            "dotenv",
            "# personal\nTHEME=dark\nexport PORT = 80 # managed\n",
            "/PORT",
            json!("443"),
            "THEME=dark",
        ),
        (
            "properties",
            "# personal\ntheme=dark\nserver.port: 80\n",
            "/server.port",
            json!("443"),
            "theme=dark",
        ),
        (
            "csv",
            "name,port\npersonal,80\napi,80\n",
            "/1/port",
            json!("443"),
            "personal,80\n",
        ),
        (
            "tsv",
            "name\tport\npersonal\t80\napi\t80\n",
            "/1/port",
            json!("443"),
            "personal\t80\n",
        ),
        (
            "plist",
            "<plist version=\"1.0\"><dict><key>theme</key><string>dark</string><key>port</key><integer>80</integer></dict></plist>",
            "/port",
            json!(443),
            "<string>dark</string>",
        ),
    ];
    for (kind, source, pointer, value, personal) in cases {
        let out = set(kind, source, pointer, value);
        assert!(out.contains(personal), "{kind} lost unmanaged data: {out}");
    }
}

#[test]
fn constraints_keep_personal_values_and_array_entries() {
    for (kind, source) in [
        ("yaml", "domains: [mine, bad]\nn: 7\n"),
        ("toml", "domains = ['mine', 'bad']\nn = 7\n"),
        ("json5", "{domains: ['mine', 'bad'], n: 7}"),
        ("hcl", "domains = [\"mine\", \"bad\"]\nn = 7\n"),
    ] {
        let keep = call(
            kind,
            source,
            "/n",
            "ensure",
            json!(1),
            json!({"min":1}),
            "apply",
        )
        .unwrap();
        assert_eq!(keep["content"], source);
        let rules = json!({"required_items":["corp"],"forbidden_items":["bad"]});
        let out = call(
            kind,
            source,
            "/domains",
            "array",
            Value::Null,
            rules.clone(),
            "apply",
        )
        .unwrap();
        let text = out["content"].as_str().unwrap();
        assert!(
            text.contains("mine") && text.contains("corp") && !text.contains("bad"),
            "{kind}: {text}"
        );
        assert_eq!(
            call(kind, text, "/domains", "array", Value::Null, rules, "check").unwrap()["compliant"],
            true
        );
    }
}

#[test]
fn insertion_and_removal_preserve_other_settings() {
    for (kind, source, pointer, value) in [
        ("yaml", "theme: dark\n", "/enabled", json!(true)),
        ("toml", "theme='dark'\n", "/server/enabled", json!(true)),
        (
            "jsonc",
            "{\"theme\":\"dark\"}",
            "/server/enabled",
            json!(true),
        ),
        ("json5", "{theme:'dark'}", "/server/enabled", json!(true)),
        ("hcl", "theme = \"dark\"\n", "/enabled", json!(true)),
        (
            "ini",
            "theme=dark\n[personal]\ncolor=blue\n",
            "/server/enabled",
            json!("true"),
        ),
        ("dotenv", "THEME=dark\n", "/ENABLED", json!("true")),
        ("properties", "theme=dark\n", "/enabled", json!("true")),
        (
            "xml",
            "<config><theme>dark</theme><server/></config>",
            "/config/server/0/@enabled",
            json!("true"),
        ),
        (
            "plist",
            "<plist><dict><key>theme</key><string>dark</string></dict></plist>",
            "/server/enabled",
            json!(true),
        ),
    ] {
        let out = set(kind, source, pointer, value);
        let removed = call(
            kind,
            &out,
            pointer,
            "remove",
            Value::Null,
            json!({}),
            "apply",
        )
        .unwrap();
        let text = removed["content"].as_str().unwrap();
        assert!(text.contains("dark"), "{kind}: {text}");
        assert_eq!(
            call(
                kind,
                text,
                pointer,
                "remove",
                Value::Null,
                json!({}),
                "check"
            )
            .unwrap()["compliant"],
            true
        );
    }
}

#[test]
fn ambiguous_or_unsupported_input_is_never_rewritten() {
    for (kind, source, pointer, value) in [
        ("json5", "{port:80, port:81}", "/port", json!(443)),
        ("jsonc", "{port:80}", "/port", json!(443)),
        ("yaml", "port: 80\nport: 81\n", "/port", json!(443)),
        ("ini", "port=80\nport=81\n", "/port", json!("443")),
        ("dotenv", "PORT=80\nPORT=81\n", "/PORT", json!("443")),
        ("properties", "port=80\nport=81\n", "/port", json!("443")),
        ("csv", "name,name\na,b\n", "/0/name", json!("x")),
        (
            "hcl",
            "server \"api\" { port = 80 }\nserver \"api\" { port = 81 }\n",
            "/server/api/port",
            json!(443),
        ),
        ("hcl", "port = var.port\n", "/port", json!(443)),
        ("toml", "parent = 1\n", "/parent/child", json!(2)),
        (
            "xml",
            "<!DOCTYPE config [<!ENTITY secret SYSTEM 'file:///etc/passwd'>]><config>&secret;</config>",
            "/config",
            json!("x"),
        ),
        (
            "xml",
            "<config>Hello <b>world</b></config>",
            "/config/#text",
            json!("x"),
        ),
    ] {
        assert!(
            call(kind, source, pointer, "set", value, json!({}), "apply").is_err(),
            "accepted ambiguous {kind}: {source}"
        );
    }
}

#[test]
fn escapes_and_special_types_round_trip() {
    let out = set(
        "dotenv",
        "VALUE=old\n",
        "/VALUE",
        json!("a\"b\nc\\d ${HOME}"),
    );
    assert!(out.contains("\\${HOME}"));
    set(
        "properties",
        "value=old\n",
        "/value",
        json!("日本語 🦀\n=next"),
    );
    let out = set(
        "properties",
        "other=personal\nvalue=one\\\n  two\n",
        "/value",
        json!("next"),
    );
    assert!(out.contains("other=personal\n") && !out.contains("two"));
    let out = set(
        "toml",
        "created = 2026-09-27T00:00:00Z\nport = 80\n",
        "/port",
        json!(443),
    );
    assert!(out.contains("created = 2026-09-27T00:00:00Z"));
    let out = set(
        "xml",
        "<c xmlns:a=\"urn:app\"><a:s p='old'/></c>",
        "/c/{urn:app}s/0/@p",
        json!("a<&\"\n"),
    );
    assert!(out.contains("xmlns:a=\"urn:app\""));
    set(
        "xml",
        "<config><name/></config>",
        "/config/name/0",
        json!("hello & world"),
    );
    set("xml", "<config/>", "/config/name/0", json!("hello"));
    set(
        "csv",
        "name,note\na,old\n",
        "/0/note",
        json!("one, two\nthree"),
    );
    let out = set(
        "plist",
        "<plist><dict><key>blob</key><data>AQID</data><key>port</key><integer>80</integer></dict></plist>",
        "/port",
        json!(443),
    );
    let doc = plist::Value::from_reader_xml(out.as_bytes()).unwrap();
    assert_eq!(
        doc.as_dictionary().unwrap()["blob"].as_data().unwrap(),
        &[1, 2, 3]
    );
}

#[test]
fn nested_literal_edits_preserve_dynamic_hcl_and_toml_table_arrays() {
    let source = "settings = { enabled = false, custom = var.personal }\n";
    let out = set("hcl", source, "/settings/enabled", json!(true));
    assert!(out.contains("custom = var.personal"));
    let source = "[[servers]]\nname = 'managed'\nport = 80\n[[servers]]\nname = 'PERSONAL'\nport = 8080 # personal port\n";
    let out = set("toml", source, "/servers/0/port", json!(443));
    assert!(out.contains("name = 'PERSONAL'\nport = 8080 # personal port"));
    set(
        "json5",
        "{ a: 0xFF, b: .5, c: 'PERSONAL' }",
        "/b",
        json!(0.75),
    );
    assert!(
        call(
            "json5",
            "{a:1e999}",
            "/a",
            "set",
            json!(1),
            json!({}),
            "apply"
        )
        .is_err()
    );
}

#[test]
fn yaml_anchor_edits_cannot_change_unselected_alias_values() {
    let source = "# personal\nbase: &base {port: 80}\ncopy: *base\n";
    let out = set("yaml", source, "/base/port", json!(443));
    let copy = call(
        "yaml",
        &out,
        "/copy/port",
        "set",
        json!(80),
        json!({}),
        "check",
    )
    .unwrap();
    assert_eq!(copy["compliant"], true);
    assert!(out.contains("# personal"));
}

#[test]
fn block_comments_survive_jsonc_and_json5_edits() {
    for kind in ["jsonc", "json5"] {
        let source = "{/* PERSONAL */ \"personal\":\"PERSONAL\", \"managed\":false}";
        let out = set(kind, source, "/managed", json!(true));
        assert!(out.contains("/* PERSONAL */"));
    }
}

#[test]
fn plist_refuses_duplicate_keys_and_external_entity_values() {
    for source in [
        "<plist><dict><key>port</key><integer>80</integer><key>port</key><integer>81</integer></dict></plist>",
        "<!DOCTYPE plist [<!ENTITY secret SYSTEM 'file:///etc/passwd'>]><plist><dict><key>secret</key><string>&secret;</string></dict></plist>",
    ] {
        assert!(
            call(
                "plist",
                source,
                "/port",
                "set",
                json!(443),
                json!({}),
                "apply"
            )
            .is_err()
        );
    }
}
