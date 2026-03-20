# MTT v2 — AI CLI Control Plane

## Vision

MTT is not a terminal multiplexer. It is an **AI CLI control plane** that uses tmux as its rendering engine. Zellij manages panes. MTT manages AI agents.

## Architecture

```
mtt (orchestration layer)
  │
  ├── tmux (rendering engine — panes, windows, native terminal)
  │     ├── Window 0: mtt dashboard (auto-refresh, no TUI capture)
  │     ├── Window 1: claude session (native terminal, full colors)
  │     ├── Window 2: codex session (native terminal, arrow keys work)
  │     └── Window N: ...
  │
  ├── AI awareness layer
  │     ├── CLI detection (claude, codex, gemini, opencode)
  │     ├── Usage window tracking (5hr limits, scheduled activation)
  │     ├── Stall detection + auto-resume
  │     ├── Conversation history + search
  │     └── Cost/token tracking
  │
  └── Orchestration layer
        ├── Broadcast (same prompt → multiple AIs)
        ├── AI-to-AI piping (Claude output → Codex input)
        ├── Session templates (preconfigured multi-AI setups)
        └── Remote management (Tailscale SSH + tmux)
```

## Key Difference from tmux/Zellij

tmux/Zellij are general-purpose terminal multiplexers. They manage panes and windows. They know nothing about what's running inside them.

MTT knows:
- What CLI is running (Claude, Codex, Gemini, OpenCode)
- How much usage time is left (5hr window countdown)
- When a session has stalled (output monitoring)
- The full conversation history (recorded + searchable)
- How to send the same prompt to multiple AIs at once
- How to pipe one AI's output into another AI's input

## Interface: tmux-native

No bubbletea TUI. No terminal nesting. Instead:

### Dashboard Window (Window 0)

A Go program that prints and auto-refreshes. Not interactive — just a status display.

```
MTT — AI CLI Control Plane                          12:34:05

  #  CLI      PROJECT              STATUS    WINDOW
  1  claude   ~/ind/duct           ● active  3h22m left
  2  codex    ~/ind/omni-base      ● active  4h01m left
  3  claude   ~/corp/saerons-qmd   ◐ stalled 1h45m left
  4  gemini   ~/ind/mother-term    ● active  —

  [stalled] #3 claude saerons-qmd — auto-resume sent 30s ago

  Ctrl-B 1..4 → switch to session | mtt help → commands
```

### Session Windows (Window 1-N)

Native tmux windows. Full terminal. Colors work. Arrow keys work. No interference from mtt. Each window runs the AI CLI directly.

### CLI Commands (from any terminal)

```bash
# Session management
mtt ls                              # list all sessions
mtt spawn claude ~/project          # start Claude in new tmux window
mtt spawn codex ~/project           # start Codex in new tmux window
mtt kill 3                          # kill session #3
mtt attach 2                        # switch to session #2's window

# AI-specific
mtt broadcast "explain this code"   # send to ALL active sessions
mtt send 1 "refactor this function" # send to session #1
mtt resume 3                        # manually resume stalled session
mtt history 1                       # show session #1's history
mtt history search "auth"           # search across all sessions

# Usage tracking
mtt status                          # show usage windows for all models
mtt ping 7am claude                 # schedule ping to activate at 7am
mtt window                          # show remaining time per model

# Templates
mtt template review                 # spawn Claude + Codex + Gemini for code review
mtt template debug                  # spawn Claude with debug-focused prompt

# Remote
mtt connect user@host               # register remote machine
mtt spawn claude ~/proj --remote h  # spawn on remote
mtt discover myserver               # find AI CLIs on remote

# Piping
mtt pipe 1 2                        # pipe session 1's output → session 2's input
```

## Core Features

### 1. Dashboard (Window 0)

Auto-refreshing status display. Not interactive — runs as a simple Go program that prints to stdout every second.

Shows:
- All sessions with CLI type, project, status, usage window remaining
- Stall alerts with time since stall and auto-resume status
- Key hint for switching windows

### 2. Session Spawning

`mtt spawn claude ~/project` creates a new tmux window, runs `claude` in it, starts monitoring.

- Each session gets its own tmux window (full native terminal)
- mtt records the window index, PID, CLI type, CWD
- Monitoring starts automatically (stall detection, history recording)

### 3. Usage Window Tracking

AI CLIs have rate limits (e.g., 5hr windows). mtt tracks:
- When the window was activated (first interaction)
- Remaining time in the window
- Scheduled pings to activate windows at specific times
- Dashboard shows countdown per session

### 4. Stall Detection + Auto-Resume

Monitors tmux pane output via `capture-pane`. If output stops changing for longer than the configured timeout:
- Mark session as stalled
- If policy is auto-resume: send "continue" via `send-keys`
- Show alert in dashboard

### 5. Conversation History

Periodically captures tmux pane output and appends to log files.
- `~/.mtt/history/<session>/output.log`
- `mtt history <id>` shows recent history
- `mtt history search <query>` searches across all sessions
- History persists across mtt restarts

### 6. Broadcast

Send the same prompt to multiple AI CLIs simultaneously.
- `mtt broadcast "explain this code"` → sends to ALL active sessions
- `mtt broadcast --to 1,3 "fix this bug"` → sends to specific sessions
- Dashboard shows which sessions received the broadcast

### 7. AI-to-AI Piping

Route output from one AI session as input to another.
- `mtt pipe 1 2` → session 1's new output is sent to session 2
- Useful for: "Claude writes code, Codex reviews it"
- Pipeline templates for common workflows

### 8. Session Templates

Preconfigured multi-session setups for common workflows.

```toml
[templates.review]
description = "Code review with three AIs"
[[templates.review.sessions]]
cli = "claude"
prompt = "Review this code for correctness"
[[templates.review.sessions]]
cli = "codex"
prompt = "Review this code for performance"
[[templates.review.sessions]]
cli = "gemini"
prompt = "Review this code for security"

[templates.debug]
description = "Debug session"
[[templates.debug.sessions]]
cli = "claude"
args = "--dangerously-skip-permissions"
```

### 9. Remote Management

Manage AI CLIs on remote machines via SSH (Tailscale-friendly).
- `mtt connect user@host` — register host
- `mtt spawn claude --remote host` — spawn on remote in tmux
- `mtt discover host` — find AI CLIs running on remote
- Remote sessions appear in the dashboard alongside local ones

### 10. Cost/Token Tracking (future)

Parse AI CLI output to estimate token usage and costs.
- Per-session token count
- Per-model daily/weekly cost estimate
- Alert when approaching limits

## Config

```toml
# ~/.mtt/config.toml

[settings]
state_dir = "~/.mtt"
history_interval = "5s"
default_stall_timeout = "120s"
default_resume_message = "continue"

[limits]
claude = "5h"
codex = "5h"
gemini = "4h"
opencode = "0"

[[schedules]]
session = "claude"
time = "07:00"
repeat = "daily"

[templates.review]
description = "Multi-AI code review"
[[templates.review.sessions]]
cli = "claude"
[[templates.review.sessions]]
cli = "codex"

[[hosts]]
name = "devbox"
address = "user@devbox.tail1234.ts.net"
```

## Project Structure

```
mother-terminal/
├── cmd/
│   └── mtt/              CLI entrypoint (cobra or plain flag)
├── internal/
│   ├── tmux/             tmux session/window/pane management
│   ├── dashboard/        auto-refresh status display
│   ├── monitor/          stall detection + policy engine
│   ├── history/          output recording + search
│   ├── scheduler/        ping scheduling + usage windows
│   ├── broadcast/        multi-session message fan-out
│   ├── pipe/             AI-to-AI output routing
│   ├── template/         session template loading + spawning
│   ├── remote/           SSH + tmux remote management
│   └── config/           TOML config loading
├── pkg/
│   ├── types.go          shared types
│   └── errors.go         domain errors
└── config.example.toml
```

## Migration from v1

The bubbletea TUI code (`internal/tui/`) is removed. The backend layer (`internal/backend/`) is simplified — only tmux backend matters now since all sessions run in tmux.

Core packages that carry over:
- `internal/history/` — history recording (works as-is)
- `internal/remote/` — remote management (works as-is)
- `internal/scheduler/` — ping scheduling (works as-is)
- `internal/config/` — config loading (extend for templates)
- `pkg/` — shared types (works as-is)

New packages:
- `internal/tmux/` — replaces the generic backend layer
- `internal/dashboard/` — simple print-and-refresh (replaces bubbletea TUI)
- `internal/broadcast/` — multi-session fan-out
- `internal/pipe/` — AI-to-AI routing
- `internal/template/` — session templates

## Design Decisions

1. **tmux as rendering engine** — proven terminal emulation, no color/key issues, users already know the shortcuts
2. **CLI commands, not persistent TUI** — each command does one thing and exits, except the dashboard which is a long-running display
3. **Dashboard is a tmux window** — not an interactive TUI, just auto-refreshing stdout
4. **Everything is tmux send-keys/capture-pane** — one reliable mechanism for all session interaction
5. **AI awareness is the differentiator** — stall detection, usage tracking, broadcast, piping, templates are features no multiplexer provides
