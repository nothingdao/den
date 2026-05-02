# Den — Architecture Overview

## Structure

Den is currently implemented primarily in `src/main.rs`. Application state, TUI rendering, CLI handling, config logic, wallet management, contacts, and Helius fetches all live there.

## Major Subsystems

- TUI: Ratatui/Crossterm rendering, keyboard handling, tabs, modals
- CLI: headless wallet/config/contact commands exposed through `den --help`
- Wallets: full wallets and watch-only wallets, with active-wallet selection
- Config: local file backend plus optional Bitwarden-backed sync
- Contacts: persisted contact list with JSON import/export
- Network data: Helius requests for balances, tokens, and history run on a background refresh worker
- Transactions: send flows build legacy SOL/SPL Token transactions, simulate before review, then sign/broadcast only after typed confirmation
- Release: GitHub Actions builds macOS binaries and publishes release assets used by Homebrew

## State Model

Core app state includes:

- active tab and selection state
- wallet list and active wallet
- token/history/account data
- onboarding/setup state
- status messaging
- current network
- contacts

## Data Flow

Typical refresh flow:

```text
user action
-> resolve active wallet + config
-> build RPC/DAS requests
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

Watch-only wallets are blocked before send entry. Token2022 sends are intentionally blocked until asset support is validated.

## Storage

### Secrets

- Private keys live in macOS Keychain via `keyring`

### Config

- Local backend uses files under the Den config directory
- Bitwarden backend uses a configured Bitwarden item and local bootstrap/cache state

### Contacts

- Contacts are persisted locally as JSON
- CLI import/export reads and writes JSON files

## CLI Surface

The shipped CLI includes:

- wallet management commands
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
