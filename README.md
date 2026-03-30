# Den Wallet TUI

Terminal wallet dashboard for Solana, built with Ratatui. It can show live balances and recent activity via Helius, or run in a placeholder mode when no credentials are set.

## Install

Install via Homebrew:
```bash
brew install nothingdao/tap/den
den
```

Build and run locally during development:
```bash
cargo run
```

Install the CLI from source:
```bash
cargo install --path .
den
```

## Features
- Multi-tab wallet overview (accounts, tokens, history, address book, settings)
- Live balance + token fetch from Helius RPC
- Keychain-backed key import (macOS)
- Message signing from stored key

## Requirements
- Rust stable
- Helius API key (for live data)
- macOS Keychain for secure key storage (Keychain-backed flows)

## Quick Start
```bash
den
```

## Setup

Store your Helius API key (required for live data):
```bash
den --set-api-key YOUR_HELIUS_KEY
```

Import a wallet key (base58 from Phantom, or JSON byte array from Solana CLI):
```bash
den --import  # reads DEN_SECRET_KEY env var
```

Or import interactively: launch the TUI and press `i` to paste a key.

Check current status:
```bash
den --status
```

If you have not installed the binary yet, prefix the same commands with `cargo run --`.

## Configuration

Wallet secrets are stored in macOS Keychain. Config can be local or centralized in Bitwarden.

| Storage | Location | Contents |
|---------|----------|----------|
| Keychain | `den-wallet` service | Wallet private keys |
| Config file | `~/.config/den/config.toml` (or `~/Library/Application Support/den/config.toml` on macOS) | Local config backend |
| Bitwarden item | `DEN_BW_CONFIG_ITEM_ID` | Centralized config backend |
| Env vars | Shell environment | `HELIUS_API_KEY`, `DEN_CONFIG_BACKEND`, `DEN_BW_CONFIG_ITEM_ID` |

### Onboarding (TUI)
- On first launch, the app opens a setup wizard in the TUI and blocks normal actions until setup is complete.
- Choose `This Mac` for local config, or `Bitwarden` for synced config.
- Bitwarden mode includes guided actions: check status, API key login, unlock, then enter config item ID.
- The app initializes default config in the selected Bitwarden item when notes are empty/invalid.
- Press `o` on the Settings tab to run the setup wizard again.

### CLI Commands
```
Wallet Management:
--add-wallet NAME       Import key from DEN_SECRET_KEY with name
--add-watch NAME ADDR   Add a watch-only wallet
--list-wallets          List all wallets
--switch-wallet NAME    Set active wallet by name or ID
--rename-wallet NAME NEW  Rename a wallet
--remove-wallet NAME    Remove a wallet
--import                Import key from DEN_SECRET_KEY (legacy)
--clear [NAME]          Remove private key (active or named)

Contacts:
--list-contacts         List all contacts
--export-contacts [FILE] Export contacts as JSON (stdout or file)
--import-contacts FILE  Import contacts from JSON, skip duplicates

Configuration:
--set-api-key KEY       Store Helius API key in config
--clear-api-key         Remove API key
--set-network NET       Set default network (mainnet|devnet)
--migrate-config-to-bitwarden [--force]  Copy local config to Bitwarden
--config-path           Show active config location
--status                Show full status
```

## Feature Status

### Key Management

| Feature | Status | Notes |
|---------|--------|-------|
| Import keypair (base58) | Done | Via TUI modal or `--import` CLI |
| Import keypair (JSON byte array) | Done | Auto-detected from `[...]` format |
| Secure key storage (macOS Keychain) | Done | Via `keyring` crate |
| Delete stored key | Done | `--clear` CLI flag |
| Derive address from stored key | Done | Falls back when `WALLET_ADDRESS` unset |
| Generate new keypair | Not started | |
| Mnemonic / seed phrase (BIP39) | Not started | |
| HD derivation (BIP44 m/44'/501') | Not started | |
| Multiple accounts / wallets | Done | Full and watch-only wallets supported |
| Export / backup key | Not started | |
| Password / PIN protection | Not started | |
| Session auto-lock | Not started | |
| Hardware wallet (Ledger) | Not started | |

### Balances & Assets

| Feature | Status | Notes |
|---------|--------|-------|
| SOL balance | Done | Via Helius DAS API |
| SPL token balances | Done | Via `getAssetsByOwner` with `showFungible` |
| Token account discovery | Done | DAS returns all fungible assets |
| Token metadata resolution | Done | Symbols and names from DAS metadata |
| Token prices (USD) | Done | Via DAS `price_info.price_per_token` |
| Portfolio value (USD) | Done | SOL + token values displayed |
| Token2022 support | Not started | DAS may already return these; untested |
| NFT display | Not started | |
| Real-time price charts | Not started | Chart uses seeded fake data |

### Transactions

| Feature | Status | Notes |
|---------|--------|-------|
| Transaction history list | Done | Via `getSignaturesForAddress`, last 10 |
| Send SOL | Not started | Send tab is a static mockup |
| Send SPL tokens | Not started | |
| Transaction detail view | Not started | Shows signature + slot only |
| Transaction confirmation / review | Not started | |
| Transaction simulation | Not started | |
| Priority fees | Not started | |
| Versioned transactions (v0) | Not started | |
| Devnet airdrop | Not started | |

### Signing

| Feature | Status | Notes |
|---------|--------|-------|
| Sign arbitrary message | Done | From stored Keychain key |
| Sign transaction | Not started | No send flow yet |

### Network

| Feature | Status | Notes |
|---------|--------|-------|
| Mainnet | Done | Via Helius RPC |
| Devnet | Done | Via Helius devnet RPC |
| Network toggle | Done | `n` keybinding |
| Custom RPC endpoint | Not started | Hardcoded to Helius |

### Address Book

| Feature | Status | Notes |
|---------|--------|-------|
| View contacts | Done | CLI list and TUI address book |
| Add / edit / delete contacts | Partial | Import/export supported, interactive editing still limited |
| Persistent storage | Done | Contacts saved to local JSON config |

### TUI / UX

| Feature | Status | Notes |
|---------|--------|-------|
| Tab navigation (1-8) | Done | |
| Sidebar nav | Done | Hidden below 70 col width |
| Responsive layout | Done | Breakpoints at 60, 70, 90 col |
| Keyboard shortcuts | Done | |
| Input modals | Done | Import key, sign message |
| Status bar with context colors | Done | Error/success/info coloring |
| Color theme | Done | Earthy palette (bark, fawn, ash, moss, ember) |
| Copy address to clipboard | Not started | |
| QR code display | Not started | Receive tab shows placeholder |
| Async data loading | Not started | Blocking reqwest freezes UI |
| Loading / spinner indicators | Not started | |
| Auto-refresh on interval | Not started | Manual `r` only |

### CLI

| Feature | Status | Notes |
|---------|--------|-------|
| `--add-wallet` | Done | Imports a named wallet from `DEN_SECRET_KEY` |
| `--add-watch` | Done | Adds a watch-only wallet |
| `--list-wallets` | Done | Lists configured wallets |
| `--switch-wallet` | Done | Sets active wallet |
| `--rename-wallet` | Done | Renames a wallet |
| `--remove-wallet` | Done | Removes a wallet |
| `--import` | Done | Legacy import from `DEN_SECRET_KEY` |
| `--clear` | Done | Removes private key from active or named wallet |
| `--list-contacts` | Done | Lists contacts |
| `--export-contacts` | Done | Exports contacts as JSON |
| `--import-contacts` | Done | Imports contacts from JSON |
| `--set-api-key` | Done | Stores Helius API key in config |
| `--clear-api-key` | Done | Removes API key from config |
| `--set-network` | Done | Persists default network to config |
| `--config-path` | Done | Shows active config location |
| `--status` | Done | Shows full wallet/config summary |
| `--help` | Done | |
| `--balance` (headless query) | Not started | |

### Advanced / Future

| Feature | Status | Notes |
|---------|--------|-------|
| Staking / delegation | Not started | |
| Swap / DEX integration | Not started | |
| dApp connection | Not started | |
| Multi-sig support | Not started | |
| Token account creation (ATA) | Not started | |

## Notes
- If `WALLET_ADDRESS` is not set, the app attempts to derive it from Keychain.

## Release

Git tags that start with `v` trigger the release workflow in [release.yml](/Users/josh/Projects/_nothingdao/den/.github/workflows/release.yml). That workflow builds `den` tarballs for macOS targets and uploads the archives plus SHA256 checksum files directly to the GitHub release.

The Homebrew formula itself should live in a separate tap repository, for example `nothingdao/homebrew-tap`. A starter formula is included at [den.rb.template](/Users/josh/Projects/_nothingdao/den/packaging/homebrew/den.rb.template).

### Homebrew Release Checklist

1. Push the main branch to GitHub.
2. Create and push a tag such as `v0.1.2`.
3. Wait for [release.yml](/Users/josh/Projects/_nothingdao/den/.github/workflows/release.yml) to publish the macOS tarballs and `.sha256` files.
4. Copy [den.rb.template](/Users/josh/Projects/_nothingdao/den/packaging/homebrew/den.rb.template) into `nothingdao/homebrew-tap` as `Formula/den.rb`.
5. Replace `version` and both `sha256` placeholders in the formula with the values from the GitHub release assets.
6. Test locally with `brew install nothingdao/tap/den`.
