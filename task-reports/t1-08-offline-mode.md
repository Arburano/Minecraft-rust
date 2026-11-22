# T1-08: offline mode for client and server

## What was done

- **`auth`** (previously a skeleton; now depends only on `sha2`, with no network dependency):
  - `PlayerName`: 3–16 characters `A-Z a-z 0-9 _`.
  - `PlayerId::offline`: SHA-256 of `"offline-player:" + lowercase name`, first 16 bytes, UUID version 8 with the RFC 4122 variant, case-insensitive.
  - `Identity` (`#[non_exhaustive]`, currently `Offline { name, id }`; the Microsoft identity is added after T0-10).
  - `AdmissionPolicy` (offline admitted only when `allow_offline` is set) and `AuthError`.
- **`server`:**
  - The command line uses clap: `--offline`, `--bind`, `--port` (default 47777).
  - Online mode listens on `0.0.0.0` by default. Offline mode listens on `127.0.0.1` by default and accepts `--bind` only for loopback or private LAN addresses (RFC 1918, link-local, IPv6 ULA/link-local). It refuses `0.0.0.0`, `::` and public addresses with an explanatory error and prints a warning on start.
- **`client`:** `--offline-name <NAME>`. Without it the client explains that Microsoft login is not available yet (T0-10) instead of silently going offline.
- **Docs:** `auth-and-assets.md` ("Offline mode") updated with the exact rules.

**Tests:**
- `auth`: the offline ID is deterministic and case-insensitive; a golden UUID for "Steve" (`7cae3b2c-69eb-8a03-8712-d41254abbd5a`) was computed independently with Python `hashlib`; name validation; admission policy.
- `server`: online is the default and refuses offline players; offline binds localhost; LAN addresses are accepted, public and unspecified ones refused.
- `client`: offline is opt-in and invalid names are rejected.
- Manual runs: `server --offline --bind 0.0.0.0` exits with an error; `server --offline` warns and binds `127.0.0.1:47777`.

## What went wrong

- The first "golden" UUID test compared the function's output with itself. It was replaced by a value computed independently in Python.

## Issues encountered

- **No outbound network calls in offline mode** is guaranteed structurally (the `auth` crate has no networking dependency) rather than by a network-blocking test. The client and server do not open sockets yet; when stage 2 adds networking, a test with a mock or blocked network should be added to the login path.
- **Settings persistence** for the offline name is T1-13 (client settings); for now it is a command-line flag.
- **The server now depends on `auth`:** a downward dependency, binary on library, consistent with `overview.md`.
