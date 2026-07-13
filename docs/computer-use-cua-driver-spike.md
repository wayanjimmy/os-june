# Computer use cua-driver sandbox spike (JUN-288)

A timeboxed spike answering one phase-2 question for **Computer use** (see
[browser-computer-use-prd.md](browser-computer-use-prd.md) "Computer use (phase
2)" and [ADR-0017](adr/0017-browser-use-via-june-extension.md)): when June
bundles a pinned, signed **cua-driver** as an app resource, do the driver's
private-interface lookups work under the app's Seatbelt profile, or must the
Rust broker run it outside the write jail, as the **dictation helper**
effectively is? This note records the answer, the evidence, and the recommended
spawn topology. It is a findings note, not a build.

## Answer

Run the driver outside the write jail. The private-interface lookups are not the
blocker: under June's `(allow default)` profile the window-server and
accessibility connection paths survive the jail unchanged (proven below). The
blocker is the write jail plus TCC identity. As shipped, cua-driver is a
single-instance daemon that must create a control socket and pid file under
`~/Library/Caches/cua-driver/` and persist config under `~/.cua-driver/`, none
of which are (or should be) June write roots, so a driver launched as a child of
the in-jail runtime cannot start. Conveniently, cua-driver already solves the
identity half of this for us: its `cua-driver mcp` process is a thin client that
proxies to a `CuaDriver.app` daemon launched through LaunchServices "so TCC
grants attach to the bundle". The recommended topology therefore matches the
dictation-helper precedent almost exactly: the Rust broker owns a June-bundled,
separately-signed driver daemon that runs outside the jail with its own stable
code identity, and the in-jail runtime's stdio client proxies to it over a
June-controlled socket placed inside a write root. Version pinning is a bundled
app resource plus binary-path and version env overrides; the upstream network
installer never runs because nothing in June's spawn path invokes it.

## What the driver is (provenance and pin)

The pinned Hermes runtime (`HERMES_AGENT_INSTALL_COMMIT`
`2bd1977d8fad185c9b4be47884f7e87f1add0ce3`, tag v2026.6.19) drives computer use
through the upstream `cua-driver` from `trycua/cua`. The Hermes backend pins the
driver at `0.5.0` by default (`PINNED_CUA_DRIVER_VERSION` in
`tools/computer_use/cua_backend.py`, overridable via `HERMES_CUA_DRIVER_VERSION`).

Provenance of the artifact read and inspected for this spike (fetched directly,
not through any installer):

- Hermes source tarball:
  `https://github.com/NousResearch/hermes-agent/archive/2bd1977d8fad185c9b4be47884f7e87f1add0ce3.tar.gz`,
  sha256 `7a9bd367066183898831c2760f269368ab54b458a1d1b51d14ef1f484dd490cc`
  (matches `HERMES_SOURCE_TARBALL_SHA256`).
- Driver release artifact:
  `https://github.com/trycua/cua/releases/download/cua-driver-rs-v0.5.0/cua-driver-rs-0.5.0-darwin-universal.tar.gz`,
  sha256 `dc9ad275b712ab6893d88dec4399c0294b12714730c4fdba47109eef004fbbc7`.
  Tag `cua-driver-rs-v0.5.0` confirmed present on the release repo.

The artifact is a universal `CuaDriver.app` bundle:

- `codesign` identity `com.trycua.driver`, authority "Developer ID Application:
  Cua AI, Inc. (YCK386LBJ7)", hardened runtime, both `x86_64` and `arm64`.
- `LSUIElement` true (background agent), `LSMinimumSystemVersion` 13.0, no TCC
  usage strings in the Info.plist (Accessibility and screen recording do not
  require them).
- `otool -L` links `ApplicationServices`, `CoreGraphics`, `ScreenCaptureKit`,
  `CoreMedia`, `CoreVideo`, `IOSurface`, `Metal`, `AppKit`.
- The private interfaces are resolved at runtime, not statically linked. Strings
  in the binary include `SLEventPostToPid` and `_SLPSGetFrontProcess` from
  `/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight`, plus
  `CGEventPostToPid` ("delivered to the target pid ... No focus steal") and the
  accessibility calls `AXUIElementPerformAction` and
  `AXUIElementSetAttributeValue`. Capture is through ScreenCaptureKit. These are
  the background-control primitives ADR-0017 relies on (events posted to the
  target process, no cursor, focus, or Space theft).

## How the runtime launches the driver

`tools/computer_use/cua_backend.py` is the default backend. It speaks MCP over
stdio to a child process it spawns itself:

- Command: `HERMES_CUA_DRIVER_CMD` (default `cua-driver`), resolved with
  `shutil.which`, which also accepts an absolute path.
- Args: `["mcp"]` (stdio MCP transport). Backend selection is
  `HERMES_COMPUTER_USE_BACKEND` (default `cua`).
- The stdio child is launched through the Python MCP SDK's `stdio_client`, so it
  is a descendant of the runtime process and inherits whatever spawn context the
  runtime has, including the Seatbelt profile.

At the pin, the backend has no attach-to-a-running-driver transport: it always
spawns `cua-driver mcp` as a stdio child. That matters for topology (below): the
runtime, not June's broker, owns the stdio client process.

But the driver's own design splits work across two processes. Per the binary's
`--help` and strings, `cua-driver mcp` on macOS is a thin client that
auto-launches and proxies to a persistent `CuaDriver.app` daemon (the proxy path
is "the path you actually run"), and the daemon is what holds recording state,
element-index caches, and the privileged system connections. The relevant
subcommands and knobs:

- `serve` / `stop` / `status`: explicit daemon lifecycle.
- `mcp --socket <path>` (also the daemon side): override the daemon Unix-domain
  socket path used by the proxy.
- `mcp --no-daemon-relaunch` (`CUA_DRIVER_RS_MCP_NO_RELAUNCH=1`): keep the stdio
  client from auto-launching the daemon, so June can own daemon lifecycle.
- `CUA_DRIVER_RS_MCP_FORCE_PROXY=1`: force proxying to the daemon.
- `CUA_DRIVER_RS_HOME`: relocate `~/.cua-driver`.
- `CUA_DRIVER_RS_TELEMETRY_ENABLED=0`, `CUA_DRIVER_RS_UPDATE_CHECK=0`: disable
  the driver's telemetry (posts to a third-party analytics host) and its GitHub
  update check.
- `permissions grant`: "Launch CuaDriver via LaunchServices so the permission
  dialog attributes to com.trycua.driver (not your terminal), wait for the
  grant, then confirm the driver's own status." `permissions status` is
  read-only and answers via the running daemon so the result carries the
  bundle identity, or `unknown` when no daemon is running.

## Version pinning and keeping the installer from running

Confirmed. June bundles `CuaDriver.app` 0.5.0 as an app resource and points the
runtime at the bundled inner executable with `HERMES_CUA_DRIVER_CMD` (absolute
path) plus `HERMES_CUA_DRIVER_VERSION=0.5.0`. With that set,
`cua_driver_binary_available()` (which is `shutil.which(HERMES_CUA_DRIVER_CMD)`)
returns the bundled path and `check_computer_use_requirements()` returns true, so
the runtime never reaches an install path.

There is no network fetch on the runtime path. When the driver is missing, the
backend only raises a hint string; it does not install. The upstream network
installer (`/bin/bash -c "$(curl -fsSL .../libs/cua-driver/scripts/install.sh)"`,
which delegates to a Rust installer that downloads a release into
`~/.local/bin` and `~/.cua-driver`) is reachable only through the Hermes CLI:
`hermes tools` (the interactive toolset-enable wizard),
`hermes computer-use install`, and `hermes update`
(`_run_cua_driver_installer` in `hermes_cli/tools_config.py`). June never invokes
those. June enables toolsets by writing config.yaml directly (`sync_hermes_config`
/ `render_hermes_config`), not through the wizard. The driver binary itself also
has an `update --apply` subcommand that runs "the canonical installer"; June
must never call it, and should set `CUA_DRIVER_RS_UPDATE_CHECK=0` so the driver
does not even phone home to check.

One gap worth flagging: the Hermes-layer `HERMES_CUA_DRIVER_VERSION` pin does not
flow into the upstream installer (the installer keys on `CUA_DRIVER_RS_VERSION`
and otherwise pulls latest). This is another reason June must bundle a fixed
artifact rather than lean on any install step to honor the pin.

## Does it work under the write jail

June wraps sandboxed spawns in `/usr/bin/sandbox-exec -f <profile>`, where the
profile from `build_sandbox_profile()` is `(allow default)` plus a
`(deny file-write*)` write jail re-granting only `sandbox_write_roots()`
(HERMES_HOME, the managed runtime dir, `/private/tmp`, `/private/var/tmp`,
`$TMPDIR`) plus a secret-read denylist. The profile does not deny mach lookups
or any window-server or accessibility interface.

### The private-interface lookups survive the jail

A small Swift probe was compiled and run bare, then under a faithful replica of
the generated profile (same shape and write roots as `build_sandbox_profile()`,
validated by the kernel via `sandbox-exec`). The probe calls `CGMainDisplayID`
and `CGDisplayPixelsWide/High` (needs a window-server connection),
`CGWindowListCopyWindowInfo` (needs a window-server connection), and the
prompt-free TCC status calls `AXIsProcessTrusted` and
`CGPreflightScreenCaptureAccess`.

Bare and jailed produced identical output:

```
DISPLAY did=24 3008x1612
WINDOWLIST count=40
AX_TRUSTED=false
SCREEN_PREFLIGHT=false
PROBE_DONE
```

Two conclusions. First, the write jail does not block the window-server or
accessibility connection mechanism the driver's private SPIs ride on; those
paths work in-jail. Second, Seatbelt and TCC are orthogonal: the jail neither
grants nor removes an Accessibility or screen-recording grant, and a fresh
unbundled binary starts with neither (hence the `false` values), which is exactly
why the driver needs a stable, separately-signed code identity plus a human
grant.

### The write jail blocks the driver's runtime state

Under the same profile, writes inside a write root succeed and the driver's real
runtime paths are denied:

```
# inside HERMES_HOME (a write root)
WROTE_HERMES_HOME_OK

# ~/Library/Caches/cua-driver/ (the daemon's socket + pid dir)
mkdir: /Users/<user>/Library/Caches/cua-driver: Operation not permitted
DENIED_CACHE

# ~/.cua-driver/config.json (the driver's config)
mkdir: /Users/<user>/.cua-driver: Operation not permitted
DENIED_DOTHOME
```

Strings in the driver confirm these are load-bearing: the daemon binds a
single-instance Unix-domain socket at `~/Library/Caches/cua-driver/cua-driver.sock`
with a `cua-driver.pid` lock, and persists configuration to
`~/.cua-driver/config.json` (plus `.telemetry_id`, `.installation_recorded`, and
an update-check cache under `~/.cua-driver-rs/`). A driver launched as a child of
the in-jail runtime cannot create any of these, so it cannot start. This is the
concrete reason to run it outside the write jail, matching the PRD's hypothesis.

## Recommended spawn topology

The broker spawns a June-bundled, separately-signed driver bundle directly,
dictation-helper style, and keeps it out of the write jail; the in-jail runtime
talks to it as a proxy client. Concretely:

1. Bundle `CuaDriver.app` 0.5.0 as an app resource and re-sign it under June's
   Developer ID and a June bundle id (for example `co.opensoftware.june.cua-driver`),
   exactly as `build.rs` builds and `sign_helper_app()` signs the dictation
   helper (`co.opensoftware.june.dictation-helper`). This gives the daemon a
   stable, auditable designated requirement so its Accessibility and screen
   recording grants survive app updates and attribute to a June-controlled
   identity, not to the runtime or a third-party signature.
2. The Rust broker (the unsandboxed host, the same layer that already spawns the
   dictation helper via `spawn_helper()` and the gateway daemon via
   `build_hermes_gateway_start_command`, both deliberately outside the Seatbelt
   wrapper) starts the driver daemon directly, for example
   `cua-driver serve --socket <june-controlled path>`, or through LaunchServices.
   Because the daemon is a child of the broker (launchd), not of the
   `sandbox-exec` runtime, it runs outside the write jail with full access to
   its own state dirs, and holds the privileged window-server, accessibility, and
   screen-capture connections under its own bundle identity.
3. The in-jail runtime keeps spawning `cua-driver mcp` as its stdio child (as
   upstream does), pointed at the bundled binary via `HERMES_CUA_DRIVER_CMD`. June
   forces it onto the proxy path and off self-management with
   `CUA_DRIVER_RS_MCP_FORCE_PROXY=1`, `CUA_DRIVER_RS_MCP_NO_RELAUNCH=1`, and
   `--socket` / matching env pointing at the same June-controlled socket, which
   must live inside a write root so the jailed client can connect to it. The
   jailed client does no privileged work; it relays to the daemon.
4. Injection point: extend `apply_isolated_hermes_env()` (and the isolated-env
   allowlist) to set `HERMES_CUA_DRIVER_CMD`, `HERMES_CUA_DRIVER_VERSION`,
   `CUA_DRIVER_RS_HOME` (a write-root path), `CUA_DRIVER_RS_MCP_FORCE_PROXY`,
   `CUA_DRIVER_RS_MCP_NO_RELAUNCH`, `CUA_DRIVER_RS_TELEMETRY_ENABLED=0`, and
   `CUA_DRIVER_RS_UPDATE_CHECK=0` for computer-use-enabled spawns, and add a
   `computer_use` block to `render_hermes_config()`.

This keeps the agent runtime itself under the write jail. The one privileged
process (the daemon) is out of the jail by construction, with a bounded,
auditable trust surface, which is the same trade already accepted for the
dictation helper and the gateway daemon.

### Alternative if the daemon is ever kept in-jail

If a future design instead runs the daemon inside the jail (not recommended), the
profile needs write grants for the daemon's state. From the evidence that is at
least: the socket and pid dir `~/Library/Caches/cua-driver/` and the config home
`~/.cua-driver/` (the latter partly relocatable with `CUA_DRIVER_RS_HOME`). The
socket path is relocatable with `--socket`, so placing it under an existing write
root avoids widening the profile for that piece. This path re-widens the jail
toward `$HOME` and tracks driver-internal paths across versions, so the
out-of-jail daemon is preferred.

## TCC (Accessibility and screen recording)

TCC is orthogonal to Seatbelt (proven above) and is the real gate on computer
use. The driver needs two grants: Accessibility (for AX posting and the private
SkyLight event SPIs) and screen recording (for ScreenCaptureKit capture). These
attach to the daemon's bundle identity, which is why the separately-signed
bundle matters. Onboarding follows the dictation-helper pattern the PRD already
names: a bundle-scoped, prompting grant with polled re-checks. The driver's own
`permissions grant` (LaunchServices launch so the dialog attributes to the
bundle) and `permissions status` (read-only, daemon-attributed, `unknown` when no
daemon) map directly onto that flow.

## Blocked

- End-to-end capture and click against a scratch target was not exercised.
  Driving the daemon requires live Accessibility and screen-recording grants for
  the driver bundle, which can only be granted by a human in System Settings (or
  via `cua-driver permissions grant`, which opens a GUI dialog). Per the spike's
  safety rules, no TCC database edit or bypass was attempted. The mechanism was
  verified up to that boundary: the window-server and accessibility connection
  paths work in-jail, and TCC status is readable without prompting. Human steps
  to unblock a future end-to-end run: launch the June-signed driver bundle, then
  in System Settings under Privacy and Security enable the bundle under
  Accessibility and under Screen Recording (or run `cua-driver permissions grant`
  and approve the dialog), then re-check with `cua-driver permissions status`.

## Open questions for phase-2 planning

- Socket reachability from inside the jail: confirm the jailed `cua-driver mcp`
  client can `connect()` to a daemon socket placed under a write root, and settle
  whether any residual client-side writes (config, telemetry id) need
  `CUA_DRIVER_RS_HOME` pointed at a write root or can be fully suppressed.
- Daemon lifecycle and reuse: one daemon per machine (single-instance socket) or
  one per session, and how the broker supervises it (start, health, stop on
  revoke), including whether `serve` or a LaunchServices launch is the cleaner
  owner. The runtime's mode split (sandboxed vs unrestricted, one gateway per
  mode) suggests computer use should also be a distinct, consent-gated spawn.
- Re-signing and notarization: whether re-signing the third-party bundle under
  June's identity (changing its designated requirement) is enough for stable TCC,
  and how the release self-test the PRD calls for starts the bundled daemon and
  probes `permissions status` and a capture on a macOS bump.
- Provider vision path: the toolset hard-requires a vision-capable model; wire
  the availability gate and the switch-model notice to the same capability source
  as the rest of the app.
- Approval surface: route the toolset's mutating actions
  (`click`, `type`, `scroll`, `drag`, `key`, `set_value`, `focus_app`) through
  June's existing tool-approval infra as chat approval cards, since the backend
  defaults to allow when no CLI approval callback is wired.
- Pin hygiene: the Hermes `HERMES_CUA_DRIVER_VERSION` does not drive the upstream
  installer, so the pin lives entirely in the bundled artifact; fold a driver
  sha256 check into the bundling step and the Hermes upgrade checklist.
