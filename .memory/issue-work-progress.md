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
Commit: eeabbbb feat: make wallet refresh non-blocking (#7)
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
Commit: 112c388 feat: complete contact management flows (#11)
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
Commit: 5129622 feat: add clipboard copy and receive QR (#12)
Blockers: none

## #8 Implement SOL and SPL send flows
Status: completed with approved conservative scope.
Changes:
- Added TUI send flow from the Send tab: choose asset with Up/Down, enter recipient, enter amount, simulate, review, then type `SEND` to sign/broadcast.
- Added SOL transfers using legacy transactions and default Solana fees.
- Added SPL Token transfers with source ATA validation and recipient ATA creation when missing.
- Blocked watch-only wallets before send entry.
- Blocked Token2022/unsupported-token-program sends until asset support is validated.
- Broadcast uses preflight and reports the resulting signature in status/last-signature.
- Updated SPEC.md, README.md, and architecture docs.
Checks:
- cargo fmt: passed
- cargo test: passed (0 tests; warnings)
- cargo build: passed (warnings)
- cargo clippy: passed with warnings
Commit: pending in this run.
Blockers: none for approved #8 scope.

## #9 Add transaction detail, simulation, and confirmation review
Status: completed with #8 vertical slice.
Changes:
- Added reusable transaction review screen for sends.
- Added required simulation before review; simulation failures block sending with no override.
- Added typed confirmation before signing/broadcast.
- Added transaction detail view for history rows with status, slot, summary, amount, and signature.
- Added copy support for selected/detail transaction signatures.
- Updated SPEC.md, README.md, and architecture docs.
Checks:
- cargo fmt: passed
- cargo test: passed (0 tests; warnings)
- cargo build: passed (warnings)
- cargo clippy: passed with warnings
Commit: pending in this run.
Blockers: none for approved #9 scope.

## #10 Expand key management: generation, seed phrases, HD wallets, and backup
Status: blocked/not started after #8/#9.
Blocker:
- Completing #10 safely needs product/security decisions that are not yet approved: mnemonic word count/language, whether mnemonic creation is required vs keypair-only generation, HD derivation defaults and account-index UX, passphrase support, local backup/export format, and exact confirmation language for showing/exporting secrets.
- Secret export/backup can be made confirmation-gated, but the recovery model and derivation defaults should not be guessed.
Checks: not run for #10 because no code changes were made after the #8/#9 validation set.
Commit: none for implementation; progress note pending.

## #13/#14/#15
Status: not started in this run.
Reason: stopped at #10 per safety instruction rather than skipping ahead after identifying key-management security/product blockers.

