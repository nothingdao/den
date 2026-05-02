# Linux Support Plan

Status: design target, not yet implemented/tested.

Den should be portable beyond macOS. The daemon, key-storage provider API, and XDG path support should make Linux a first-class platform over time.

## Goals

- Run the Den TUI on Linux terminals.
- Support Linux desktop secret storage.
- Support headless/server Linux with an encrypted-file key provider.
- Use XDG config/data/runtime paths.
- Use Unix sockets for native local daemon clients.
- Keep browser clients as Den clients, not key stores.

## Platform Model

| Platform | TUI | Secret backend | Native IPC | Browser client |
|---|---:|---|---|---|
| macOS | yes | `macos-keychain` | Unix socket + localhost HTTP | Safari extension |
| Linux desktop | planned | `linux-secret-service` | Unix socket + localhost HTTP | Firefox/Chrome extension later |
| Linux headless | planned | `encrypted-file` | Unix socket | optional |
| Windows | later | `windows-credential-manager` | named pipe + localhost HTTP | Chrome/Edge later |

## Key Storage

Linux should use the provider model described in `key-storage-api.md`.

Preferred desktop backend:

```text
linux-secret-service
```

This maps to Freedesktop Secret Service implementations such as GNOME Keyring or KWallet, likely through the Rust `keyring` crate.

Headless fallback:

```text
encrypted-file
```

This provider should use strong encryption and KDF defaults, explicit passphrase/unlock UX, and clear backup warnings.

The domain model should use provider-neutral terms:

```toml
key_provider = "linux-secret-service"
key_ref = "den-wallet:wallet_..."
```

Avoid naming wallet metadata around Apple Keychain concepts.

## XDG Paths

Den should follow XDG conventions on Linux:

```text
config:  $XDG_CONFIG_HOME/den/config.toml
         fallback ~/.config/den/config.toml

data:    $XDG_DATA_HOME/den/
         fallback ~/.local/share/den/

runtime: $XDG_RUNTIME_DIR/den/den.sock
         fallback TBD when XDG_RUNTIME_DIR is unavailable
```

The runtime directory and socket should be user-private.

Recommended permissions:

```text
$XDG_RUNTIME_DIR/den      0700
$XDG_RUNTIME_DIR/den.sock 0600-equivalent socket permissions
```

## Daemon Transport

Linux native clients should prefer Unix sockets:

```text
$XDG_RUNTIME_DIR/den/den.sock
```

Browser extensions cannot generally use Unix sockets directly, so the daemon may also expose localhost HTTP for browser clients:

```text
http://127.0.0.1:<port>/v1/health
```

Both transports should call the same daemon core handlers and enforce the same session/approval policy.

## Packaging Targets

Initial Linux distribution options:

- `cargo install`
- `.deb`
- `.rpm`
- AUR
- Nix flake
- Homebrew-on-Linux if useful

For early Linux support, `cargo install` plus documented prerequisites may be enough. Packaged releases can follow once the key provider and daemon behavior are stable.

## Browser Extension on Linux

Future Firefox/Chrome extensions should follow the same product rule as Safari:

- extension is a Den client
- no private keys in browser storage
- daemon must be running
- user authorizes extension sessions through Den
- signing goes through Den daemon/key provider

## Open Questions

- Which Linux Secret Service crate/backend is reliable enough across GNOME/KDE?
- What is the encrypted-file format and unlock UX?
- Where should daemon port discovery live for browser extensions?
- Should Linux daemon startup integrate with systemd user services?
- Which packaging format should be first beyond `cargo install`?
