schema_version = 1
name = "personal"
select_roles = ["preferences"]

role "preferences" {
  rule "personal-domain" {
    target = "claude://project/settings"
    kind = "json"
    operation = "array"
    pointer = "/sandbox/network/allowedDomains"
    constraints {
      required_items = ["docs.rs"]
    }
    rationale = "Rust documentation is useful for my work"
    added = "2026-09-27"
  }
}
