# Mother Terminal — Design Spec

## Overview

Mother Terminal is a universal AI CLI orchestrator. It controls multiple AI CLI tools (Claude Code, Codex, Gemini CLI, OpenCode, and others) by injecting real string keystrokes into their running terminal sessions and pressing Enter — exactly as a human would type.

It can discover running sessions, send pings, schedule usage window activations, detect stalled sessions, and monitor query completion.

## Core Requirements

- **Language:** Go
- **TUI framework:** bubbletea (primary interface)
- **CLI interface:** Stubbed, low priority
- **Config format:** TOML
- **Architecture:** Modular monolith with interface boundaries
- **GitHub:** kickthemoon0817/mother-terminal

## Injection Backends

Mother Terminal supports 4 keystroke injection backends via a plugin interface. Each implements the same `Injector` interface. Build tags ensure only platform-relevant code compiles.

| Backend | Mechanism | Platform |
|---------|-----------|----------|
| tmux | `tmux send-keys` / `tmux capture-pane` | All |
| macOS native | AppleScript / `osascript` | darwin |
| X11 | `xdotool` | linux (X11) |
| Wayland | `ydotool` | linux (Wayland) |

Additionally, a PTY backend (`creack/pty`) handles sessions spawned by Mother Terminal itself.

## Injector Interface

```go
type Injector interface {
    Discover() ([]Session, error)
    SendKeys(session Session, text string) error
    ReadOutput(session Session, lines int) (string, error)
    Ping(session Session) (PingResult, error)
    IsAvailable() bool
}
```

A backend registry auto-detects available backends at startup by calling `IsAvailable()` on each.

## Session Model

```go
type Session struct {
    ID        string
    Name      string          // user-friendly name
    CLI       CLIType         // claude, codex, gemini, opencode
    Backend   BackendType     // tmux, macos, x11, wayland, pty
    Target    string          // backend-specific identifier
    Status    SessionStatus   // discovered, active, stalled, dead
    Policy    StallPolicy     // notify, auto_resume, custom
}
```

### Supported CLIs

- Claude Code (`claude`)
- Codex (`codex`)
- Gemini CLI (`gemini`)
- OpenCode (`opencode`)

### Session State Machine

```
discovered -> active -> stalled -> active (resumed)
                   \-> dead
```

## Session Discovery

### Auto-discovery (process-based)

1. Scan running processes for known CLI binary names
2. Trace each match to its controlling terminal
3. Ask each active backend if it owns that terminal
4. First backend to claim ownership creates the Session

### Manual Registration (TOML config)

```toml
[[sessions]]
name = "claude-main"
cli = "claude"
backend = "tmux"
target = "dev:0.1"
```

Auto-discovery is default. Manual registration takes priority for conflicts.

## Ping System

Three levels:

1. **Liveness check** — Verify process is running and terminal is responsive
2. **Active probe** — Send a lightweight prompt, verify response within timeout
3. **Scheduled activation** — Send a probe at a configured time to start the usage window

### PingResult

```go
type PingResult struct {
    Alive       bool
    Responsive  bool
    Latency     time.Duration
}
```

## Usage Window Tracking

AI CLIs have usage limits (e.g., 5-hour windows). Mother Terminal tracks these:

1. User schedules a ping time (e.g., 07:00)
2. At that time, Mother sends an active probe
3. On success, a usage window starts tracking
4. TUI shows countdown: `claude-main: 4h23m remaining`
5. On expiry, TUI marks the window and optionally schedules next ping

### Config

```toml
[[schedules]]
session = "claude-main"
time = "07:00"
repeat = "daily"
probe = "active"

[limits]
claude = "5h"
codex = "5h"
gemini = "4h"
opencode = "0"       # 0 = no limit tracking
```

Window state is persisted to survive restarts.

## Stall Detection & Recovery

After sending a query:

1. Start a timer
2. Periodically capture output and compare with previous
3. If output stops changing before completion signal detected, mark as `stalled`
4. Apply session's stall policy:
   - `notify` — Highlight in TUI, optional system notification
   - `auto_resume` — Send configurable follow-up prompt (default: `"continue"`)
   - Per-session configurable timeout and resume message

## Project Structure

```
mother-terminal/
├── cmd/
│   ├── mother/          main entrypoint (TUI mode)
│   └── motherctl/       CLI commands (low priority, stubbed)
├── internal/
│   ├── tui/             bubbletea app
│   │   ├── app.go       root model, key bindings, layout
│   │   ├── dashboard.go session grid, status indicators
│   │   ├── detail.go    single-session expanded view
│   │   ├── input.go     query input bar
│   │   └── timer.go     5hr window countdown display
│   ├── backend/
│   │   ├── registry.go  Injector interface + backend registry
│   │   ├── tmux/        tmux send-keys / capture-pane
│   │   ├── macos/       osascript / AppleScript
│   │   ├── x11/         xdotool
│   │   └── wayland/     ydotool
│   ├── discovery/
│   │   ├── scanner.go   process-tree scanning for known CLIs
│   │   └── registry.go  manual session registration
│   ├── session/
│   │   ├── manager.go   session lifecycle
│   │   ├── state.go     per-session state machine
│   │   └── monitor.go   stall detection + policy engine
│   ├── scheduler/
│   │   ├── ping.go      scheduled ping execution
│   │   └── window.go    5hr usage window tracking
│   ├── pty/
│   │   └── spawner.go   creack/pty for Mother-spawned sessions
│   └── config/
│       └── config.go    TOML parsing + validation
├── pkg/
│   ├── types.go         shared types
│   └── errors.go        domain errors
├── config.example.toml
└── go.mod
```

## TUI Dashboard

The bubbletea TUI is the primary interface, showing:

- **Session grid** — All managed sessions with status indicators (active/stalled/dead)
- **Per-model usage limits** — Remaining time in usage windows, rate limit status
- **Timer countdowns** — 5hr window remaining per session
- **Input bar** — Select target session and type query
- **Detail view** — Expand a single session to see recent output

## Broadcast Mode (Low Priority)

Send the same query to multiple AI CLIs simultaneously and compare responses side-by-side. Deferred to post-MVP.

## CLI Interface (Low Priority)

Stubbed commands for scripting:
- `mother send <target> "query"`
- `mother status`
- `mother ping <target>`

Full CLI parity is deferred.

## Design Decisions

1. **Go over Node.js** — Better concurrency (goroutines), superior TUI framework (bubbletea), single binary distribution, natural fit for systems-level terminal management.
2. **Native PTY + universal injection** — Mother can both spawn new sessions (PTY) and inject into existing ones (4 backends). The tmux layer is not required.
3. **Modular monolith over plugins** — Go's plugin system is fragile. Interface boundaries + build tags give the same extensibility without runtime pain.
4. **TOML config** — Clean, readable, well-supported in Go.
5. **Process-based discovery + manual override** — Auto-discovery for convenience, manual registration for edge cases.
