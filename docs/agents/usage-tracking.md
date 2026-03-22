# Usage & Limit Tracking per AI CLI

How mtt tracks real usage limits for each supported AI CLI.

## Claude Code

**Method:** Direct OAuth API call to Anthropic.

### API
```
GET https://api.anthropic.com/api/oauth/usage
Authorization: Bearer <access_token>
```

### Auth Flow
1. Read refresh token from macOS Keychain (`Claude Code-credentials`) or `~/.claude/.credentials.json`
2. Refresh: `POST https://platform.claude.com/v1/oauth/token`
   ```json
   { "grant_type": "refresh_token", "refresh_token": "...", "client_id": "9d1c250a-e61b-44d9-88ed-5944d1962f5e" }
   ```
3. Call usage API with fresh access token

### Response
```json
{
  "five_hour": { "utilization": 0.16, "resets_at": "2026-03-22T21:00:00Z" },
  "seven_day": { "utilization": 0.42, "resets_at": "2026-03-27T04:59:59Z" },
  "seven_day_sonnet": { "utilization": 0.03 },
  "seven_day_opus": { "utilization": 0.12 }
}
```
- `utilization`: 0.0–1.0 (multiply by 100 for %)
- Cache 60s+ to avoid rate limits (429s)

### Credential Locations
- macOS Keychain: `security find-generic-password -s "Claude Code-credentials" -w`
- File fallback: `~/.claude/.credentials.json`

---

## OpenAI Codex CLI

**Method:** Connect to local app-server IPC.

### Architecture
Codex runs a local app-server process (`codex app-server`). The CLI communicates via JSON-RPC.

### Rate Limit Schema
From `codex-rs/app-server-protocol/schema/json/v2/GetAccountRateLimitsResponse.json`:
```json
{
  "rateLimits": {
    "limitId": "codex",
    "planType": "pro",
    "primary": {
      "usedPercent": 16,
      "resetsAt": 1742680800,
      "windowDurationMins": 300
    },
    "secondary": {
      "usedPercent": 42,
      "resetsAt": 1742947200,
      "windowDurationMins": 10080
    }
  }
}
```
- `primary` = 5-hour rolling window (300 mins)
- `secondary` = weekly cap (10080 mins)
- `usedPercent` = 0–100 integer

### Credential Location
`~/.codex/auth.json`:
```json
{ "tokens": { "access_token": "...", "refresh_token": "...", "account_id": "..." } }
```

### Notifications
The server pushes `AccountRateLimitsUpdatedNotification` with same schema when limits change.

### Process Discovery
```bash
ps aux | grep "codex app-server"
# Running as: codex app-server --analytics-default-enabled
```

---

## Google Gemini CLI

**Method:** Google Cloud Code Assist API.

### API
```
POST https://cloudcode-pa.googleapis.com/v1beta5:retrieveUserQuota
Authorization: Bearer <access_token>
Content-Type: application/json

{ "project": "<project_id>" }
```

### Response (`RetrieveUserQuotaResponse`)
```json
{
  "buckets": [
    {
      "modelId": "gemini-2.5-pro",
      "remainingAmount": "850",
      "remainingFraction": 0.85,
      "resetTime": "2026-03-23T03:16:00Z",
      "tokenType": "REQUEST"
    },
    {
      "modelId": "gemini-3.1-pro-preview",
      "remainingAmount": "1000",
      "remainingFraction": 1.0,
      "resetTime": "2026-03-23T03:16:00Z"
    }
  ]
}
```
- `remainingFraction`: 0.0–1.0 (fraction remaining, NOT used)
- `remainingAmount`: absolute count remaining
- `limit = remaining / remainingFraction` (calculated)
- Per-model quotas with individual reset times

### Auth
`~/.gemini/oauth_creds.json`:
```json
{ "access_token": "...", "refresh_token": "...", "expiry_date": ..., "scope": "..." }
```

Token refresh via Google OAuth2.

### Tier Info
- Free (Personal Google): 60 req/min, 1000 req/day
- Google One AI Pro: higher limits
- Workspace/Enterprise: subscription-based

---

## Antigravity IDE (via OpenCode routing)

**Method:** Internal gRPC to Antigravity Language Server.

### Process Discovery
Scan for running `language_server_macos_arm` (macOS ARM) or platform equivalent.
Extract CSRF token and port from process arguments.

### API
```
POST https://localhost:<port>/exa.language_server_pb.LanguageServerService/GetUserStatus
```

### Response
Per-model info with `remainingFraction` and `resetTime`.

### Model Groups (shared quota pools)
| Group | Models |
|-------|--------|
| Claude | Claude 4.5 Sonnet, Sonnet Thinking, Opus |
| Gemini Pro | Gemini Pro High/Low |
| Gemini Flash | Gemini Flash variants |
| Gemini Image | Gemini Pro Image |

Source: [vscode-antigravity-cockpit](https://github.com/jlcodes99/vscode-antigravity-cockpit)

---

## Implementation Status in mtt

| CLI | Method | Status |
|-----|--------|--------|
| Claude | OAuth API → `api.anthropic.com/api/oauth/usage` | **Implemented** |
| Codex | Local app-server IPC → `GetAccountRateLimits` | **Planned** — need to discover server port |
| Gemini | Google API → `cloudcode-pa.googleapis.com:retrieveUserQuota` | **Planned** — need OAuth token refresh |
| Antigravity | gRPC to Language Server → `GetUserStatus` | **Planned** — via opencode routing |

### Implementation: `src/usage/mod.rs`
- `read_claude_usage()` — OAuth refresh + API call + caching
- `read_codex_usage()` — TODO: connect to local app-server
- `read_gemini_usage()` — TODO: Google OAuth + Cloud API call
