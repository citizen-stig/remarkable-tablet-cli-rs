# SSH connection latency: what the tablet costs

Every CLI command opens a new SSH connection. Over USB that costs ~215 ms before the first command can run, and almost all of it is spent waiting on the tablet, not on the client. This document records where the time goes, which server-side changes would remove part of it, and why we are not making them. We avoid changing the device.

Measured on 2026-11-10: reMarkable 2, firmware 20261009, `dropbear_2025.89`, BusyBox 1.37, USB (10.11.99.1), macOS client. Ping RTT is 0.5–1.5 ms.

## Where the time goes

A timestamped OpenSSH session (`ssh -vvv root@10.11.99.1 true`, each debug line stamped on arrival) shows three server pauses of ~60–70 ms each. Every network round-trip costs about 1 ms.

| Step | Arrives at (ms) | What the tablet is doing |
|---|---|---|
| TCP connected | 0.1 | |
| Server id line (`SSH-2.0-dropbear…`) | ~60 | systemd starts a new dropbear process for this connection |
| Key exchange reply | ~130 | x25519 key exchange and ed25519 host-key signature |
| Public key accepted (no signature yet) | ~143 | |
| Authenticated | ~210 | verifying the client's ed25519 signature |
| `true` executed, exit status | ~222 | |

The CLI's `SshConnection::connect` (`crates/remarkable-tablet/src/connection/ssh.rs`) shows the same profile: ~215 ms until authentication, which matches OpenSSH. Client-side knobs that we measured and that did nothing:

- `TCP_NODELAY`
- curve25519 kex instead of mlkem768
- a 16 MiB channel window

## How dropbear runs on the tablet

dropbear is socket-activated per connection. It is not a long-running daemon.

- `dropbear-usb0.socket`, `dropbear-usb1.socket`, `dropbear-wlan.socket` (`/usr/lib/systemd/system/`): `ListenStream=22`, `Accept=yes`, `BindToDevice=usb0`/`usb1`/`wlan0`.
- Every connection starts a `dropbear@<n>-<local>-<remote>.service` instance. It runs `/usr/sbin/dropbear -G root -i -r /etc/dropbear/dropbear_ed25519_host_key -B`, where `-B` comes from `/etc/default/dropbear`'s `DROPBEAR_EXTRA_ARGS`.
- The only host key is ed25519 (`/etc/dropbear/dropbear_ed25519_host_key`). `/home/root/.ssh/authorized_keys` holds one ed25519 key.
- The CPU governor is `ondemand` (792 / 996 / 1200 MHz, `sampling_rate` 20000, `up_threshold` 95). The tablet has 2 cores.

## Server-side measurements

All tests were temporary and changed nothing on disk:

- The test server was a second dropbear on `10.11.99.1:2222`, started in the foreground under `timeout 300` and killed afterwards.
- It used `authorized_keys` and throwaway host keys in `/run/rmtest` (RAM only), passed with `-D`.
- Port 22 was never touched. The governor test wrote sysfs only and restored `ondemand`.

Times are OpenSSH `ssh true` wall times over 5–6 runs, with `-F /dev/null` so `~/.ssh/config` cannot pick the identity.

| Server setup | `ssh true` (ms) | Banner (ms) | Auth step (ms) |
|---|---|---|---|
| Stock: port 22, socket-activated, ed25519 client key | 257–268 | ~60 | ~70 |
| Standalone dropbear (forks per connection), ed25519 client key | 209–215 | ~0 | ~70 |
| Standalone dropbear, RSA client key | 140–167 | ~0 | ~10–15 |

What these numbers show:

- **Socket activation costs ~55 ms per connection.** It is the time for systemd to create a service instance and exec dropbear. A standalone dropbear forks and answers in ~1 ms.
- **ed25519 client auth costs ~55 ms more than RSA.** Verifying an ed25519 signature on the tablet takes ~55 ms; RSA verification is cheap.
- **The CPU governor does not matter.** With `performance` instead of `ondemand`, connect stayed at 225 vs 234 ms and auth at 65 ms. The actual clock under `performance` was not read, so the slow-math explanation is not confirmed.

Host key type and kex, on the standalone server with the RSA client key:

| Host key ↓ / kex → | curve25519-sha256 | ecdh-sha2-nistp256 | mlkem768x25519-sha256 |
|---|---|---|---|
| ssh-ed25519 (stock) | 138–158 | 139–164 | 137–171 |
| ecdsa-sha2-nistp256 | 118–156 | 139–156 | 140–146 |
| rsa-sha2-256 | 237–264 | 259–272 | 222–231 |

The ~65–80 ms key exchange is a fixed cost. An ECDSA host key saves ~6 ms, which is within noise. An RSA host key adds ~100 ms.

## Options that need device changes (not pursued)

We avoid changing the device. These are recorded for reference:

1. **Standalone dropbear next to the stock socket, about −55 ms per command.** Add a unit in `/etc/systemd/system/` that runs `dropbear -F -p 10.11.99.1:2222 -r /etc/dropbear/dropbear_ed25519_host_key -G root`. Leave port 22 as it is, so access never depends on the new unit. The CLI would try 2222 first and fall back to 22; a refused connection over USB costs ~1 ms. Firmware updates rewrite the root filesystem, so the unit would probably disappear after an update, and the CLI would fall back silently.
2. **RSA client key, about −55 ms per command.** Add a dedicated RSA key to `/home/root/.ssh/authorized_keys` next to the existing ed25519 line, and make the client try it first. Today the client tries agent identities in agent order, and the first key, ed25519, is accepted. `/home` survives firmware updates.
3. **ECDSA host key.** ~6 ms. Not worth it.

Options 1 and 2 together take OpenSSH `ssh true` from ~265 to ~145 ms.

## Does specifying the exact key on the client help?

Not with this setup. The tablet accepts the agent's first identity, so the client is never rejected before the right key. The ~65 ms auth step is the server verifying an ed25519 signature. OpenSSH spends the same ~66 ms with an explicit `IdentityFile`. Naming the exact key only helps when the agent offers rejected keys first; we did not measure the cost per rejected key. It does not help ed25519 auth itself. Only switching the key type to RSA does, and that needs `authorized_keys` changed on the device.

## Client-only option left: reuse one connection

Without device changes, the remaining large win is to stop paying the ~215 ms per command at all. That means a short-lived background process holding one SSH session that later commands reuse, like OpenSSH's `ControlMaster`. It needs design work and was out of scope here:

- lifecycle and idle timeout
- what happens when the tablet sleeps
- concurrent commands
- a socket path short enough for macOS's 104-byte limit

## Metadata transfer (for reference)

Tree loads stream every `.metadata` and `.content` file in one `exec` (`read_metadata_files` in `ssh.rs`), about 3.8 MB for a 456-item library. The transfer runs at ~20 MB/s and this limit is shared. Two parallel SSH connections, each streaming half, were no faster, so the bottleneck is likely the USB link or the tablet's network stack, not one CPU core. Shrinking the bytes on the tablet costs more CPU time than it saves:

| Option | Result |
|---|---|
| SSH zlib compression | +260 ms |
| `gzip -1` | 489 ms of CPU for an 8x reduction |
| Stripping JSON indentation with `sed` (−42% bytes) | ~570 ms of CPU |
| zstd, lz4, lzop | not installed |
| AES-GCM, `hmac-sha1` | not offered by the server |
| chacha20-poly1305 | already the fastest cipher on offer |

## Caveats

- The tablet drops its USB network after sleep. Every connect then times out after 5 s until it is woken.
- The tablet's host key was regenerated on 2026-11-09. A stale `known_hosts` entry breaks OpenSSH comparisons, so use `-o UserKnownHostsFile=/dev/null`. The CLI accepts any host key.
- Interleaved A/B runs over USB are needed for any comparison. The metadata phase alternates between ~325 and ~390 ms regimes that last minutes, and the CPU governor does not cause them.
