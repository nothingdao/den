# Den Product Ecosystem

Den is the local wallet authority for the NothingDAO wallet ecosystem. The terminal app, local daemon, browser extension, and future clients should share one wallet model and one secret-storage policy.

## Product Roles

```text
Den TUI / CLI
  - primary user interface for wallet management
  - owns onboarding, wallet CRUD, contacts, network config, backup/reveal UX
  - starts or connects to the local Den daemon

Den daemon / local wallet core
  - local-only API for wallet clients
  - mediates sessions, permissions, approvals, and signing
  - signs with configured key-storage providers
  - never exposes private keys or seed phrases over client APIs

Den browser extension
  - browser client and dApp bridge
  - injects Wallet Standard / window.solana
  - displays wallet state and approval UI
  - talks to the Den daemon for wallets, connect, and signing
  - stores only non-secret client/session state

Future clients
  - desktop UI, scripts, local services, or game clients
  - use the documented daemon API instead of reading Den internals
```

## Core Principle

The browser extension is not a wallet. It is a Den client.

Private keys, seed phrases, signing policy, and backup/reveal flows belong to Den core. Browser clients should never store or receive secret material.

## Target UX

### Den is running

1. User runs `den` in the terminal.
2. Den starts the local daemon/session.
3. Browser extension detects the daemon.
4. Terminal asks the user to authorize the extension session.
5. Extension shows Den wallets and lets dApps connect/sign through Den.

### Den is not running

The extension should show a clear recovery message:

```text
Den is not running.
Run `den` in your terminal to start the daemon.
```

No fallback key store should be used in the extension.

## Extension CRUD Scope

The extension may provide a wallet-management UI, but every operation goes through the daemon.

Safe extension operations:

- list wallets
- switch active wallet
- rename wallet
- add watch-only wallet
- remove watch-only wallet
- copy public addresses
- view origin/session permissions

Sensitive operations require Den approval, preferably in the terminal/TUI:

- generate a full wallet
- import a private key
- restore a mnemonic
- delete a full wallet
- reveal/export secrets
- sign a message
- sign a transaction

## Security Baseline

- Daemon binds only to localhost or local IPC.
- Extension sessions require terminal authorization.
- Session tokens are random, short-lived, and scoped.
- Signing requests include origin, wallet ID, and preview data.
- Transaction signing requires explicit approval initially.
- Watch-only wallets never sign.
- Secret reveal/export is never exposed to browser clients.
- The daemon API is stable and documented so third-party clients do not depend on private files or implementation details.

## Implementation Phases

1. Add `den daemon` or daemon mode started by the TUI.
2. Add health/session endpoints and extension detection.
3. Add terminal-approved client sessions.
4. Add wallet CRUD APIs for non-secret operations.
5. Route extension connect/sign-message/sign-transaction through the daemon.
6. Remove hardcoded keys from `den-browser-extension`.
7. Add origin permissions, daemon lifecycle management, and richer TUI controls.
8. Refactor Den internals so the TUI and CLI become clients of Den core/daemon APIs where appropriate.
