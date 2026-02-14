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

## Configuration
Set environment variables before running:

- `HELIUS_API_KEY` (required for live data)
- `WALLET_ADDRESS` (optional if a key is stored in Keychain)

Example:
```bash
export HELIUS_API_KEY="your_key"
export WALLET_ADDRESS="your_wallet_address"
cargo run
```

## Keychain Commands (macOS)
Import a secret key into Keychain:
```bash
export DEN_SECRET_KEY="[..."  # JSON array or base58 key
cargo run -- --import
```

Clear the stored key:
```bash
cargo run -- --clear
```

Show help:
```bash
cargo run -- --help
```

## Notes
- If `WALLET_ADDRESS` is not set, the app attempts to derive it from Keychain.
