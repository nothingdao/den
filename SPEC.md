# Den — Spec

Feature/source-of-truth status for the current app.

## Key Management

- Done: Import keypair from base58 or JSON byte array
- Done: Secure storage in macOS Keychain
- Done: Remove stored key with `--clear`
- Done: Derive wallet address from stored key
- Done: Multiple wallets
- Done: Watch-only wallets
- Done: Switch, rename, and remove wallets
- Pending: Generate new keypair
- Pending: Mnemonic / seed phrase
- Pending: HD derivation
- Pending: Export / backup key
- Pending: Password / PIN protection
- Pending: Session auto-lock
- Pending: Hardware wallet support

## Balances & Assets

- Done: SOL balance via Helius
- Done: SPL token balances and metadata
- Done: Token prices and portfolio total
- Pending: Token2022 support validation
- Pending: NFT display
- Pending: Real-time charts

## Transactions

- Done: Recent transaction history list
- Pending: Send SOL
- Pending: Send SPL tokens
- Pending: Transaction detail view
- Pending: Confirmation/review flow
- Pending: Simulation
- Pending: Priority fees
- Pending: Versioned transactions
- Pending: Devnet airdrop

## Signing

- Done: Sign arbitrary message
- Pending: Sign transaction

## Network

- Done: Mainnet and devnet
- Done: Network toggle
- Pending: Custom RPC endpoint

## Contacts

- Done: List contacts
- Done: Persistent contact storage
- Done: Import contacts from JSON
- Done: Export contacts as JSON
- Partial: Add/edit/delete contact flows

## Configuration

- Done: Local config backend
- Done: Bitwarden config backend
- Done: First-launch onboarding wizard
- Done: API key stored in config
- Done: CLI status/config inspection
- Done: Config migration to Bitwarden

## TUI / UX

- Done: Tab navigation and responsive layout
- Done: Sidebar navigation
- Done: Status bar and modal flows
- Done: Earthy color theme
- Pending: Clipboard copy
- Pending: QR display
- Pending: Async loading/spinners
- Pending: Auto-refresh

## Packaging

- Done: GitHub release workflow for macOS assets
- Done: Homebrew tap distribution
- Pending: crates.io distribution

## Known Gaps

- Blocking HTTP still freezes the UI during refresh
- Send flow remains mostly static
- Chart data is still seeded/fake
