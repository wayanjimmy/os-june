# ADR 0038: The Windows routine Gateway is app-supervised

Date: 2026-07-24
Status: accepted

## Context

Hermes runs its cron scheduler inside a separate Gateway process. June must
keep that Gateway alive while the app is hidden, but stop it before June's
app-owned provider proxy disappears on explicit quit or update relaunch.

On macOS, `hermes gateway start` installs and starts a launchd LaunchAgent.
June unloads that service before stopping the provider proxy. On Windows, the
same command expects an independently installed Scheduled Task or Startup
entry. When neither exists, the pinned CLI prompts to install one. June invokes
lifecycle commands non-interactively with no stdin, so Routine creation failed
instead of starting a scheduler.

Installing persistent Windows service state would also give the Gateway a
broader lifetime than its provider proxy. A login task could relaunch against
stale process-local proxy coordinates after June quits or updates.

Hermes also exposes `gateway run --replace`. It runs the scheduler in the
foreground, writes its own PID into Gateway status, and replaces a stale prior
Gateway for the same Hermes home.

## Decision

On Windows, June starts the routine Gateway as an owned foreground child using
the runtime's real Python interpreter directly:

```text
python.exe -m hermes_cli.main gateway run --replace
```

For the relocatable bundle this is the embedded interpreter. For the managed
virtual environment, June reads `pyvenv.cfg`, starts the base interpreter to
avoid retaining only the Windows virtual-environment redirector, and supplies
the managed install and site-packages paths explicitly.

June retains the child handle and accepts readiness only when the loopback
dashboard reports both `gateway_running: true` and a `gateway_pid` equal to the
spawned Python process. Starting Python directly avoids tracking the temporary
`hermes.exe` launcher instead of the actual Gateway.

An inherited or unowned Windows Gateway is replaced before June marks the
scheduler reconciled. Runtime reapply uses the same replacement path. Explicit
quit first invokes Hermes's graceful `gateway stop` while the provider proxy is
alive, then force-terminates the tracked Windows process tree if it remains.

macOS keeps the existing launchd lifecycle. Routine mutations continue to
require scheduler readiness before they are saved.

## Consequences

- Windows Routines work without a Scheduled Task, Startup entry, elevation, or
  an interactive installation prompt.
- The Gateway remains alive when June's window is hidden because the app process
  remains alive, but it does not intentionally survive explicit app quit.
- A stale Gateway left by a crash can survive until the next launch; the next
  `--replace` reconciles it. A Windows Job Object or parent-death monitor is a
  potential follow-up, not part of this first lifecycle correction.
- June does not pass `--external-supervisor` yet. Supporting Hermes's supervised
  restart handoff requires a generation-aware exit monitor and bounded respawn
  policy first.
- Before starting, June probes Hermes's Windows Scheduled Task and Startup
  entry state. A manually installed persistent Gateway is rejected with an
  actionable error rather than silently replaced; June also does not use
  `--force`.
- Windows release validation must cover create, scheduled firing while hidden,
  explicit quit, relaunch, and update handoff.

## Alternatives considered

- **Skip scheduler readiness on Windows.** Rejected because jobs could be saved
  successfully but never fire.
- **Install and use Hermes's Windows Scheduled Task.** Rejected because its
  login-persistent lifetime exceeds June's process-local provider proxy and
  complicates updates, cleanup, policy restrictions, and stale paths.
- **Mark Routines unsupported on Windows.** Safe as a release fallback, but
  unnecessary because the pinned foreground Gateway and scheduler are
  Windows-compatible.
- **Use `--external-supervisor` immediately.** Deferred until June owns the
  corresponding exit-code monitor, restart generations, and retry policy.
