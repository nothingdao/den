# Issue work progress (#7-#15)

## Plan/order
1. #7 non-blocking refresh/loading states (complete first because it touches core data flow and reduces UI freeze risk).
2. #11 contact add/edit/delete hardening (existing flows are present; complete validation/duplicate handling).
3. #12 clipboard + QR utilities (small UX vertical slice after contact/address primitives).
4. Reassess #8/#9/#10/#13/#14/#15 individually; stop before unsafe send/key-management decisions if required.

Issue #6 is deferred/future and intentionally untouched.

## #7 Make network refresh non-blocking with loading states
Status: completed.
Changes:
- Replaced direct TUI refresh calls with a background worker thread and mpsc result channel.
- Added refresh in-flight tracking, spinner text, and data status display in Overview/Settings/status.
- Preserved blocking reqwest internally but moved it off the UI thread.
- Updated SPEC.md, README.md, and architecture docs.
Checks:
- cargo fmt: passed
- cargo test: passed (0 tests; existing warnings)
- cargo build: passed (existing warnings plus now-unused helper warning)
- cargo clippy: passed with warnings
Commit: pending
Blockers: none

