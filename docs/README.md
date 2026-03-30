# Docs

Project documentation. This is the reference layer beneath `SPEC.md`.

## Structure

```
docs/
  README.md          — this file, index of all docs
  features/          — one file per cross-cutting feature or capability
  architecture/      — system design, data flow, infrastructure decisions
```

For app views/screens, add a subdirectory named after the app:

```
docs/
  <appname>/         — one file per view or major UI section
```

## How This Relates to SPEC.md

`SPEC.md` is the contract — what the app must do, at acceptance-criteria granularity.

These docs are the detail layer — the "how and why" behind each requirement. When a spec item needs more context than fits in a bullet point, it lives here and `SPEC.md` links to it.

## Doc Governance

- Docs describe target state, not current state
- When a feature ships or changes, update the relevant doc
- Keep docs scoped — one concern per file
- Link from `SPEC.md` to the relevant doc, not the other way around
