# Den — Architecture Overview

## Structure

Den is currently implemented primarily in `src/main.rs`. Application state, TUI rendering, CLI handling, config logic, wallet management, contacts, and Helius fetches all live there.

## Major Subsystems

- TUI: Ratatui/Crossterm rendering, keyboard handling, tabs, modals
- CLI: headless wallet/config/contact commands exposed through `den --help`
- Wallets: full wallets and watch-only wallets, with active-wallet selection
- Config: local file backend plus optional Bitwarden-backed sync
- Contacts: persisted contact list with JSON import/export
- Network data: blocking Helius requests for balances, tokens, and history
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
-> fetch balances/tokens/history via blocking reqwest
-> map responses into app state
-> redraw TUI
```

Because requests are blocking, the UI can stall during refresh.

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
