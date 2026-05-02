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

## #11 Complete contact add, edit, and delete flows
Status: completed.
Changes:
- Hardened existing TUI add/edit/delete flows with Solana public-key validation.
- Added duplicate-address checks for add/edit.
- Persist errors are surfaced in status instead of silently ignored for contact changes.
- Contact imports now skip invalid addresses in addition to duplicates.
- Updated SPEC.md and README.md.
Checks:
- cargo fmt: passed
- cargo test: passed (0 tests; existing warnings)
- cargo build: passed (existing warnings)
- cargo clippy: passed with warnings
Commit: pending
Blockers: none

## #12 Add clipboard copy and QR display UX utilities
Status: completed.
Changes:
- Added `c` clipboard-copy shortcut for selected wallet, active receive address, settings/overview active address, and selected contact address.
- Added receive-address QR rendering using the `qrcode` crate.
- Updated receive screen and footer hints.
- Updated SPEC.md and README.md.
Checks:
- cargo fmt: passed
- cargo test: passed (0 tests; existing warnings)
- cargo build: passed (existing warnings)
- cargo clippy: passed with warnings
Commit: pending
Blockers: none

