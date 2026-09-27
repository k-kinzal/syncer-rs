schema_version = 1
name = "personal"
description = "A local-only example; no extensions needed"

rule "agent-instructions" {
  target = "AGENTS.md"
  kind = "file"
  operation = "replace"
  value = <<-TEXT
    # Project instructions

    Run the relevant tests before committing.
  TEXT
  rationale = "Keep project setup reproducible"
  added = "2026-09-27"
}
