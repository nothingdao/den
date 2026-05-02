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
- Done: Generate new random keypair
- Done: Create/restore 12-word English seed phrase wallets
- Done: HD derivation at `m/44'/501'/0'/0'` (account index 0)
- Done: Confirmation-gated secret reveal/copy for private keys or seed phrases
- Pending: Password / PIN protection
- Pending: Session auto-lock
- Pending: Hardware wallet support

## Balances & Assets

- Done: SOL balance via Helius
- Done: SPL token balances and metadata
- Done: Token prices and portfolio total
- Partial: Token2022 assets are marked as unsupported/unknown when DAS reports non-SPL Token programs; sends remain blocked
- Done: NFT display summary/list from DAS assets
- Pending: Real-time charts (unavailable; fake seeded charts removed)

## Transactions

- Done: Recent transaction history list
- Done: Send SOL
- Done: Send SPL tokens (SPL Token only; Token2022 sends blocked until validation)
- Done: Transaction detail view
- Done: Confirmation/review flow
- Done: Simulation before broadcast (failures block sending)
- Pending: Priority fees
- Pending: Versioned transactions
- Pending: Devnet airdrop

## Signing

- Done: Sign arbitrary message
- Done: Sign transaction for reviewed send flow

## Network

- Done: Mainnet and devnet
- Done: Network toggle
- Done: Custom RPC endpoint for standard RPC balance/history and send operations

## Contacts

- Done: List contacts
- Done: Persistent contact storage
- Done: Import contacts from JSON
- Done: Export contacts as JSON
- Done: Add/edit/delete contact flows with address validation and duplicate checks

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
- Done: Configurable color theme (loaded from `~/.config/den/theme.toml`, hot-reloads on file change ~250ms)
- Done: Clipboard copy for wallet/contact receive addresses
- Done: QR display for receive address
- Done: Async loading/spinners for wallet refreshes
- Pending: Auto-refresh

## Packaging

- Done: GitHub release workflow for macOS assets
- Done: Homebrew tap distribution
- Pending: crates.io distribution

## Known Gaps

- Token2022 sends remain disabled; non-SPL Token programs are visible but unsupported for sending
- Custom RPC endpoints do not provide Helius DAS token/NFT metadata unless they implement those methods
- Chart data is unavailable rather than real historical charting
