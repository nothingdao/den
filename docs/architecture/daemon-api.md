# Den Daemon API Design

Status: design target, not yet implemented.

The Den daemon API is the stable local interface for browser extensions, native apps, scripts, games, Solana tooling, and future clients. Publicly, this should be framed as the **Den Local Wallet API**: a documented, versioned way for local applications to connect to wallets and request signatures without reading Den config files, Keychain entries, key-provider internals, or Rust implementation details directly.

## Transport

The daemon core should be transport-agnostic. Multiple local transports can expose the same API semantics.

Browser-compatible transport:

```text
http://127.0.0.1:<ephemeral-or-configured-port>
```

Native/local transport:

```text
macOS/Linux Unix socket, e.g. $XDG_RUNTIME_DIR/den/den.sock
```

Unix sockets should be preferred for native apps, CLIs, scripts, and local Solana tooling. Browser extensions generally cannot open Unix sockets directly, so they use localhost HTTP or a future native-messaging bridge.

Requirements:

- local-only binding
- no remote network exposure
- explicit session authorization before privileged endpoints
- version endpoint for compatibility checks

## Native App Use Case

A native app can add external Solana wallet signing by integrating with Den instead of embedding wallet/key logic:

```text
Native app
  -> Den Unix socket API
  -> Den daemon approval/session layer
  -> Den key-storage/signing provider
```

The app does not need to know whether a key is backed by macOS Keychain, Linux Secret Service, an encrypted file, Bitwarden, Ledger, or another provider. Den owns storage, signing policy, and approval UX.

## API Versioning

All routes should be versioned:

```text
/v1/health
/v1/session/request
/v1/wallets
```

Breaking changes require a new version path.

## Authentication Model

### Session request

A client starts unauthenticated:

```http
POST /v1/session/request
Content-Type: application/json

{
  "clientName": "Den Browser Extension",
  "clientKind": "browser-extension",
  "origin": "safari-extension://...",
  "capabilities": ["wallets:read", "wallets:write-public", "sign"]
}
```

Daemon returns a pending request and human-verification code:

```json
{
  "requestId": "req_...",
  "code": "482913",
  "status": "pending"
}
```

The TUI displays the request and code. User approves in terminal. Client then polls:

```http
GET /v1/session/request/req_...
```

Approved response:

```json
{
  "status": "approved",
  "sessionToken": "den_sess_...",
  "expiresAt": "2026-05-02T04:00:00Z"
}
```

Authenticated calls use:

```http
Authorization: Bearer den_sess_...
```

## Core Endpoints

### Health

```http
GET /v1/health
```

```json
{
  "ok": true,
  "version": "0.1.0",
  "apiVersion": "v1",
  "network": "devnet",
  "locked": false
}
```

### Wallets

```http
GET /v1/wallets
```

```json
{
  "wallets": [
    {
      "id": "wallet_...",
      "name": "Primary",
      "address": "...",
      "kind": "full",
      "origin": "generated",
      "active": true,
      "canSign": true
    },
    {
      "id": "wallet_...",
      "name": "Vault",
      "address": "...",
      "kind": "watch-only",
      "active": false,
      "canSign": false
    }
  ]
}
```

Public CRUD operations:

```http
PATCH /v1/wallets/:id          # rename non-secret metadata
POST /v1/wallets/watch-only    # add watch-only address
DELETE /v1/wallets/:id         # delete watch-only, or create approval request for full wallet
POST /v1/wallets/:id/activate  # switch active wallet
```

Sensitive wallet operations create approval requests:

```http
POST /v1/wallets/generate
POST /v1/wallets/import
POST /v1/wallets/restore-mnemonic
POST /v1/wallets/:id/reveal-secret
```

The browser extension should not call reveal-secret. That endpoint is for trusted local clients only and should require terminal confirmation.

### dApp Connect

```http
POST /v1/dapp/connect
```

```json
{
  "origin": "http://localhost:3000",
  "walletId": "wallet_...",
  "silent": false
}
```

Response:

```json
{
  "address": "...",
  "walletId": "wallet_...",
  "permissions": ["connect"]
}
```

### Sign Message

```http
POST /v1/sign/message
```

```json
{
  "origin": "http://localhost:3000",
  "walletId": "wallet_...",
  "messageBase64": "...",
  "display": "human-readable preview if available"
}
```

Response after approval:

```json
{
  "signatureBase64": "...",
  "address": "..."
}
```

### Sign Transaction

```http
POST /v1/sign/transaction
```

```json
{
  "origin": "http://localhost:3000",
  "walletId": "wallet_...",
  "transactionBase64": "...",
  "transactionVersion": "legacy"
}
```

Daemon should decode and preview:

- fee payer
- signers
- instructions/programs
- SOL/token balance changes where possible
- warnings/errors

Response:

```json
{
  "signedTransactionBase64": "..."
}
```

### Sign All Transactions

```http
POST /v1/sign/transactions
```

Same as sign transaction, but with an ordered array. Approval UI must clearly show batch size and previews.

## Approval Requests

Clients may need to observe pending requests:

```http
GET /v1/requests
POST /v1/requests/:id/approve
POST /v1/requests/:id/reject
```

The daemon owns final approval state. Browser approval UI can request approval, but Den should support terminal approval as the higher-trust surface.

## Errors

Error responses should be structured:

```json
{
  "error": {
    "code": "DEN_LOCKED",
    "message": "Den is locked. Unlock in the terminal.",
    "retryable": true
  }
}
```

Suggested codes:

- `DEN_NOT_AUTHORIZED`
- `DEN_SESSION_EXPIRED`
- `DEN_LOCKED`
- `DEN_WALLET_NOT_FOUND`
- `DEN_WATCH_ONLY_WALLET`
- `DEN_USER_REJECTED`
- `DEN_UNSUPPORTED_TRANSACTION`
- `DEN_SIMULATION_FAILED`
- `DEN_KEY_PROVIDER_UNAVAILABLE`

## Client SDKs and Examples

Once the API stabilizes, Den should provide small clients/examples:

- Rust client for native apps and CLIs
- TypeScript client for Node/Electron/local tooling
- browser-extension integration example over localhost HTTP
- Unix socket JSON-RPC examples

SDKs should be thin wrappers around the documented API, not privileged internal integrations.

## Documentation Requirement

The daemon API must be documented as a public local API before external clients depend on it. Documentation should include:

- route reference
- request/response schemas
- auth/session lifecycle
- error codes
- example browser-extension flow
- example native app Unix socket flow
- example script/client flow
- compatibility/version policy
