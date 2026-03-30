# Den — Agent Context

## What This Project Is

Den is a Rust terminal wallet dashboard for Solana, built with Ratatui. It supports multiple wallets, watch-only wallets, persistent contacts, live balances and transaction history via Helius, and a first-launch onboarding flow for local or Bitwarden-backed config.

## Stack

- Language: Rust
- TUI: Ratatui + Crossterm
- HTTP: reqwest (blocking)
- Solana: solana-sdk, bs58
- Secret storage: keyring (macOS Keychain)
- Config: TOML/JSON local files plus optional Bitwarden sync
- Distribution: Homebrew via GitHub Releases

## Running Locally

```bash
cargo run
den --help
cargo run -- --status
DEN_SECRET_KEY=<key> cargo run -- --add-wallet main
```

## Current Install Path

```bash
brew install nothingdao/tap/den
den
```

## Key Files

```text
src/main.rs                           — main application
Cargo.toml                            — package/binary metadata
.github/workflows/release.yml         — release automation
packaging/homebrew/den.rb.template    — Homebrew formula template
README.md                             — user-facing docs
SPEC.md                               — feature/status source of truth
docs/architecture/overview.md         — architecture notes
```

## Runtime Storage

- Wallet private keys: macOS Keychain
- Local config: `~/.config/den/config.toml`
- Contacts: local JSON under the Den config directory
- Optional synced config: Bitwarden item referenced by `DEN_BW_CONFIG_ITEM_ID`

## Important Behaviors

- `src/main.rs` is still the primary implementation file.
- Network fetches are blocking, so refreshes can freeze the UI briefly.
- `--set-api-key` stores the API key in config, not Keychain.
- Contacts are persisted now; they are no longer placeholder-only.
- Homebrew is the primary release channel.
