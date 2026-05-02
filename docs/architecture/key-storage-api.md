# Den Key Storage Provider API

Status: design target, not yet implemented.

Den currently stores private keys and mnemonic phrases in macOS Keychain through the `keyring` crate. That is a good default for macOS, but Den's wallet core should not be permanently tied to Apple Keychain.

Den should define a key-storage provider API so users and downstream clients can choose an appropriate secret backend.

## Goals

- Keep browser clients away from private keys and mnemonics.
- Let Den run on platforms without macOS Keychain.
- Support alternative storage backends without changing wallet/signing logic.
- Make backend capabilities explicit and inspectable.
- Preserve safe reveal/backup flows regardless of backend.

## Non-Goals

- Remote custody by default.
- Browser-extension key storage.
- Exposing raw secrets through the daemon API.
- Replacing hardware-wallet signing APIs; hardware wallets may be providers, but they are not just secret stores.

## Provider Interface

Conceptual Rust trait:

```rust
pub trait KeyStorageProvider {
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    fn capabilities(&self) -> KeyStorageCapabilities;

    fn store_private_key(&self, wallet_id: &str, secret: SecretBytes) -> Result<()>;
    fn load_private_key(&self, wallet_id: &str) -> Result<SecretBytes>;
    fn delete_private_key(&self, wallet_id: &str) -> Result<()>;

    fn store_mnemonic(&self, wallet_id: &str, phrase: SecretString) -> Result<()>;
    fn load_mnemonic(&self, wallet_id: &str) -> Result<SecretString>;
    fn delete_mnemonic(&self, wallet_id: &str) -> Result<()>;

    fn sign_message(&self, wallet_id: &str, message: &[u8]) -> Result<Signature>;
    fn sign_transaction(&self, wallet_id: &str, transaction: &[u8]) -> Result<Vec<u8>>;
}
```

The exact trait may differ, but the API boundary should distinguish:

- providers that can export/load raw keys
- providers that can sign without exporting keys
- providers that can store mnemonics
- providers that require interactive unlock
- providers that are unavailable on a platform

## Capability Model

```rust
pub struct KeyStorageCapabilities {
    pub stores_private_keys: bool,
    pub stores_mnemonics: bool,
    pub can_export_private_keys: bool,
    pub can_export_mnemonics: bool,
    pub signs_without_export: bool,
    pub requires_unlock: bool,
    pub supports_hardware_confirmation: bool,
}
```

Clients should use capabilities to decide which UI actions are available.

## Candidate Providers

### `keychain`

Current macOS default.

- Backend: macOS Keychain via `keyring`
- Stores: private keys, mnemonic phrases
- Export/reveal: allowed only after Den confirmation gates
- Platforms: macOS

### `secret-service`

Linux desktop secret storage.

- Backend: Freedesktop Secret Service / GNOME Keyring / KWallet via `keyring`
- Platforms: Linux

### `windows-credential-manager`

Windows secure credential storage.

- Backend: Windows Credential Manager via `keyring`
- Platforms: Windows

### `encrypted-file`

Portable encrypted local file provider.

- Backend: encrypted file under Den config directory
- Requires passphrase or OS-protected key wrapping
- Useful for platforms without a native secret service
- Must have strong KDF defaults and explicit backup guidance

### `bitwarden`

Optional synced secret provider.

- Backend: Bitwarden item/CLI/API
- Requires careful unlock/session handling
- Should be opt-in and explicit

### `hardware-ledger`

Hardware signer provider.

- Does not export private keys
- Signs via device confirmation
- Stores public wallet metadata only in Den config

## Config Shape

Potential config:

```toml
[key_storage]
provider = "keychain"

[key_storage.encrypted_file]
path = "~/.config/den/keys.enc"
kdf = "argon2id"
```

Wallet metadata should record which provider owns the key:

```toml
[[wallets]]
id = "wallet_..."
name = "Primary"
address = "..."
kind = "full"
key_provider = "keychain"
key_ref = "den-wallet:wallet_..."
```

## Daemon Interaction

The daemon should expose provider metadata, not secrets:

```http
GET /v1/key-storage/providers
```

```json
{
  "active": "keychain",
  "providers": [
    {
      "id": "keychain",
      "label": "macOS Keychain",
      "available": true,
      "capabilities": {
        "storesPrivateKeys": true,
        "storesMnemonics": true,
        "canExportPrivateKeys": true,
        "canExportMnemonics": true,
        "signsWithoutExport": false,
        "requiresUnlock": false
      }
    }
  ]
}
```

Signing APIs call the active provider internally. Browser clients never select or access raw provider secrets directly.

## Security Requirements

- Secret values must be zeroized where practical.
- Logs must never include secret material.
- Reveal/export requires explicit user confirmation.
- Browser clients cannot call secret reveal endpoints.
- Provider migration requires confirmation and verification.
- Provider availability failures must not corrupt wallet metadata.

## Migration Path

1. Wrap current Keychain functions behind a provider abstraction.
2. Add provider ID/ref fields to wallet metadata.
3. Keep `keychain` as default on macOS.
4. Add provider introspection to CLI/TUI.
5. Add daemon provider metadata endpoint.
6. Add additional providers incrementally.
