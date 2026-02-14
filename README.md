# Den Wallet TUI

Terminal wallet dashboard for Solana, built with Ratatui. It can show live balances and recent activity via Helius, or run in a placeholder mode when no credentials are set.

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
cargo run
```

## Setup

Store your Helius API key (required for live data):
```bash
cargo run -- --set-api-key YOUR_HELIUS_KEY
```

Import a wallet key (base58 from Phantom, or JSON byte array from Solana CLI):
```bash
cargo run -- --import  # reads DEN_SECRET_KEY env var
```

Or import interactively: launch the TUI and press `i` to paste a key.

Check current status:
```bash
cargo run -- --status
```

## Configuration

Secrets are stored in macOS Keychain. Preferences are stored in a config file created automatically on first run:

| Storage | Location | Contents |
|---------|----------|----------|
| Keychain | `den-wallet` service | Private key, Helius API key |
| Config file | `~/.config/den/config.toml` (or `~/Library/Application Support/den/config.toml` on macOS) | Default network, wallet address, theme |
| Env vars | Shell environment | `HELIUS_API_KEY`, `WALLET_ADDRESS` (override keychain/config) |

### CLI Commands
```
--import           Store key from DEN_SECRET_KEY in Keychain
--clear            Remove private key from Keychain
--set-api-key KEY  Store Helius API key in Keychain
--clear-api-key    Remove API key from Keychain
--set-network NET  Set default network (mainnet|devnet)
--config-path      Show config file location
--status           Show current configuration status
--help             Show all commands
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
| Multiple accounts / wallets | Not started | Single account only |
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
| View contacts | Done | Hardcoded placeholder entries |
| Add / edit / delete contacts | Not started | |
| Persistent storage | Not started | Contacts reset on restart |

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
| `--import` | Done | Reads `DEN_SECRET_KEY` env var |
| `--clear` | Done | Removes private key from Keychain |
| `--set-api-key` | Done | Stores Helius API key in Keychain |
| `--clear-api-key` | Done | Removes API key from Keychain |
| `--set-network` | Done | Persists default network to config |
| `--config-path` | Done | Shows config file location |
| `--status` | Done | Shows config/keychain state summary |
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
