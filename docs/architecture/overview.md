# Den — Architecture Overview

## Structure

Den is currently implemented primarily in `src/main.rs`, with theme loading isolated in `src/theme.rs`. Application state, TUI rendering, CLI handling, config logic, wallet management, contacts, and network fetches still mostly live in the main module.

## Major Subsystems

- TUI: Ratatui/Crossterm rendering, keyboard handling, tabs, modals
- CLI: headless wallet/config/contact commands exposed through `den --help`
- Wallets: full wallets and watch-only wallets, with active-wallet selection, random key generation, and 12-word seed phrase create/restore
- Config: local file backend plus optional Bitwarden-backed sync
- Contacts: persisted contact list with JSON import/export
- Network data: Helius requests for balances, tokens, NFTs, and history run on a background refresh worker; custom RPC supports standard RPC balance/history/sends without DAS metadata
- Transactions: send flows build legacy SOL/SPL Token transactions, simulate before review, then sign/broadcast only after typed confirmation
- Release: GitHub Actions builds macOS binaries and publishes release assets used by Homebrew

## State Model

Core app state includes:

- active tab and selection state
- wallet list and active wallet
- token/NFT/history/account data
- onboarding/setup state
- status messaging
- current network
- contacts

## Data Flow

Typical refresh flow:

```text
user action
-> resolve active wallet + config
-> build RPC/DAS requests, or standard RPC-only requests for custom endpoints
-> enqueue background refresh worker
-> fetch balances/tokens/history via blocking reqwest off the UI thread
-> send refreshed snapshot back over a channel
-> map responses into app state
-> redraw TUI
```

Requests still use blocking reqwest internally, but refresh work runs off the UI thread so keyboard input and redraws continue while loading.

Send flow:

```text
Send tab
-> choose SOL/SPL Token asset
-> enter recipient + amount
-> build legacy transaction instructions
-> simulate unsigned transaction with sigVerify=false
-> require review screen + typed SEND confirmation
-> load signing key from Keychain
-> sign and broadcast with preflight enabled
```

Watch-only wallets are blocked before send entry. Token2022/non-SPL Token sends are intentionally blocked; assets from unsupported programs are marked in the asset view.

## Storage

### Secrets

- Private keys live in macOS Keychain via `keyring`
- Seed phrases for mnemonic-created/restored wallets are also stored in Keychain and only revealed after typing `REVEAL`

### Config

- Local backend uses files under the Den config directory
- Bitwarden backend uses a configured Bitwarden item and local bootstrap/cache state

### Contacts

- Contacts are persisted locally as JSON
- CLI import/export reads and writes JSON files

## CLI Surface

The shipped CLI includes:

- wallet management commands, including random key generation and seed phrase restore
- contact import/export/list commands
- config/network status commands
- legacy `--import`

The CLI is now broader than the original “just launch the TUI” shape and should be treated as part of the product surface.

## Packaging

Current public distribution is:

```text
GitHub release assets
-> Homebrew tap formula
-> `brew install nothingdao/tap/den`
```

The binary users run is `den`.
