# Native App Integration

Status: design target, not yet implemented.

Den's local wallet core and Unix socket transport can let native apps add external Solana wallet signing without embedding wallet logic or storing keys.

## Product Framing

Den is a local wallet authority and signing service for Solana apps.

The TUI is the first client and control panel. The browser extension is a browser bridge. Native apps can integrate through the Den Local Wallet API over Unix sockets.

```text
Native app
  -> Den Unix socket API
  -> Den daemon session/approval layer
  -> Den key-storage/signing provider
```

A native app should not know or care whether the signing key lives in macOS Keychain, Linux Secret Service, an encrypted file, Bitwarden, Ledger, or another future provider. It asks Den to connect/sign; Den handles storage, policy, and approval.

## Use Cases

### Native games

A native game can use Den for:

- connect wallet
- identify player wallet
- sign gameplay settlement transactions
- sign claim/mint transactions
- sign messages for session authentication

### Desktop apps

A desktop app can support:

- wallet connect
- active wallet lookup
- wallet list/switching
- sign message
- sign transaction

### CLI tools and scripts

Tools can use a small client such as `denctl`:

```bash
denctl wallets list
denctl connect --app astrds-native
denctl sign-message --wallet primary --message "hello"
denctl sign-transaction --file tx.bin
```

### Solana development tools

Local Solana/Anchor tooling could request signatures from Den instead of directly loading keypair files.

## Transport

Native clients should prefer Unix sockets:

```text
macOS: ~/.config/den/den.sock or platform runtime dir
Linux: $XDG_RUNTIME_DIR/den/den.sock
```

Browser extensions use localhost HTTP or native messaging because browsers generally cannot open Unix sockets directly.

Both transports should share the same daemon core and API semantics.

## API Style

The Unix socket transport should use a stable, documented protocol. JSON-RPC is a good fit:

```json
{
  "jsonrpc": "2.0",
  "id": "1",
  "method": "wallets.list",
  "params": {}
}
```

Response:

```json
{
  "jsonrpc": "2.0",
  "id": "1",
  "result": {
    "wallets": []
  }
}
```

HTTP routes can map to the same internal request handlers. The public API should be called the **Den Local Wallet API**, not an extension-specific API.

## Session Authorization

Native apps should request sessions just like browser clients:

```json
{
  "method": "session.request",
  "params": {
    "clientName": "ASTRDS Native",
    "clientKind": "native-app",
    "capabilities": ["wallets:read", "sign"]
  }
}
```

Den TUI displays:

```text
Authorize native app?
Client: ASTRDS Native
Code: 482913
[y] allow for this session  [n] deny
```

Approved clients receive a scoped session token.

## Signing Flow

```text
native app submits signing request
-> daemon validates session/capabilities
-> daemon creates approval request
-> Den TUI/approval surface displays origin/client + transaction preview
-> user approves
-> daemon signs using configured provider
-> native app receives signature/signed transaction
```

Transaction previews should include, where possible:

- requested wallet
- fee payer
- program IDs/instruction labels
- SOL/token balance changes
- warnings for unknown programs
- simulation result when available

## Client SDKs

Once the daemon API stabilizes, Den should provide small client libraries or examples:

- Rust client for native apps and CLIs
- TypeScript client for Node/Electron/local tooling
- JSON-RPC examples using `socat`/`curl` equivalents

SDKs should be thin wrappers around the documented API, not privileged internal integrations.

## Security Guidance for App Developers

Native apps integrating Den should:

- request the minimum capabilities needed
- display which wallet/account is connected
- expect user rejection
- handle session expiration
- never ask users to paste private keys
- never attempt to read Den config or key-storage internals
- treat Den as the signing authority

## Relationship to Browser Extension

The browser extension is just one Den client. Native apps should use the same conceptual API with a native transport.

```text
Browser extension -> localhost HTTP/native messaging -> Den daemon
Native app        -> Unix socket JSON-RPC          -> Den daemon
```

Same sessions. Same approvals. Same key-storage providers. Same signing policy.
