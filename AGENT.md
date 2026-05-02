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

## Development Flow

After changing app behavior, always reinstall the local binary so `den` runs the latest code without extra user steps:

```bash
cargo install --path . --force
```

Run this after validation (`cargo fmt`, `cargo test`, `cargo build`, and clippy when practical), especially before asking the user to manually test TUI behavior.

## Current Install Path

```bash
brew install nothingdao/tap/den
den
```

## Theme

Colors are loaded from `~/.config/den/theme.toml` at startup via `init_den_theme()` and hot-reloaded on file change (~250ms detection via mtime in the main loop). The global theme is stored in `thread_local! { Cell<Option<DenTheme>> }`. Theme fields: `bg`, `fg`, `accent`, `sel_fg`, `fg_dim`, `fg_xdim`, `border`, `surface`, `green`, `red`, `yellow`. Edit `~/.config/den/theme.toml` directly and the running TUI updates without restart. Den intentionally has its own theme, separate from the whaleen dotfiles palette.

## Key Files

```text
src/main.rs                           — main application
src/theme.rs                          — Den theme loading/hot-reload helpers
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

- Den is the planned local wallet authority for the ecosystem; browser clients should connect to Den/daemon and should not store private keys.
- The daemon API and key-storage provider API are design targets documented under `docs/architecture/`.
- `src/main.rs` is still the primary implementation file, with theme helpers split into `src/theme.rs`.
- Network fetches are blocking, so refreshes can freeze the UI briefly.
- `--set-api-key` stores the API key in config, not Keychain.
- Contacts are persisted now; they are no longer placeholder-only.
- Homebrew is the primary release channel.
