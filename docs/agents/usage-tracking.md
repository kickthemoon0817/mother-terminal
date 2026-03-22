# Usage & Limit Tracking per AI CLI

How mtt tracks usage limits for each supported AI CLI.

## Claude Code

**Method:** Direct OAuth API call to Anthropic.

### API Endpoint
```
GET https://api.anthropic.com/api/oauth/usage
Authorization: Bearer <access_token>
```

### Authentication
1. Read refresh token from macOS Keychain (`Claude Code-credentials`) or `~/.claude/.credentials.json`
2. Refresh access token via `POST https://platform.claude.com/v1/oauth/token`
   ```json
   {
     "grant_type": "refresh_token",
     "refresh_token": "<refresh_token>",
     "client_id": "9d1c250a-e61b-44d9-88ed-5944d1962f5e"
   }
   ```
3. Use fresh access token for the usage API call

### Response
```json
{
  "five_hour": { "utilization": 0.16, "resets_at": "2026-03-22T21:00:00.810Z" },
  "seven_day": { "utilization": 0.42, "resets_at": "2026-03-27T04:59:59.810Z" },
  "seven_day_sonnet": { "utilization": 0.03, "resets_at": "..." },
  "seven_day_opus": { "utilization": 0.12, "resets_at": "..." }
}
```

- `utilization` is 0.0–1.0 (fraction, multiply by 100 for percentage)
- `five_hour` = 5-hour rolling window
- `seven_day` = weekly cap

### Rate Limits
Cache results for 60+ seconds. The API returns 429 if called too frequently.

### Implementation
`src/usage/mod.rs` — `read_claude_usage()`, `refresh_access_token()`, `get_refresh_token()`

---

## OpenAI Codex CLI

**Method:** No public REST API available. Usage tracked internally via app-server protocol.

### How Codex Tracks Usage Internally
Codex CLI communicates with a local app server via gRPC-like protocol:
- `GetAccountRateLimits` returns a `RateLimitSnapshot`
- Schema: `codex-rs/app-server-protocol/schema/json/v2/GetAccountRateLimitsResponse.json`

### RateLimitSnapshot Structure
```json
{
  "rateLimits": {
    "limitId": "codex",
    "limitName": "Codex",
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
    },
    "credits": { "hasCredits": false, "unlimited": false }
  }
}
```

- `primary` = 5-hour rolling window
- `secondary` = weekly cap
- `usedPercent` = 0–100 integer

### Credential Location
`~/.codex/auth.json`
```json
{
  "tokens": {
    "id_token": "...",
    "access_token": "...",
    "refresh_token": "...",
    "account_id": "..."
  }
}
```

### Current Status in mtt
No public API to call. Options for future:
1. Parse Codex CLI `/status` command output from screen
2. Connect to the local app-server directly (complex, undocumented)
3. Show session time as fallback

---

## Google Gemini CLI

**Method:** No simple usage API. Google uses Cloud quotas.

### Quota Structure
- Personal Google Account: 60 req/min, 1000 req/day (free)
- Gemini API Key: 100 req/day free tier
- Workspace/Enterprise: subscription-based

### Monitoring
- Google Cloud Console: IAM & Admin > Quotas
- Google AI Studio dashboards
- No OAuth-style usage percentage API like Claude

### Current Status in mtt
Show session time as fallback. No direct usage API available.

---

## Antigravity IDE (Reference)

The Antigravity Cockpit extension tracks usage by connecting to Antigravity's Language Server:

### How It Works
1. Scans system processes for `language_server_macos_arm` (or platform equivalent)
2. Extracts CSRF token and connection info from process arguments
3. Calls internal gRPC API: `/exa.language_server_pb.LanguageServerService/GetUserStatus`
4. Response includes per-model `remainingFraction` and `resetTime`

### Model Groups
Models are grouped by quota pool:
- **Claude group**: Claude 4.5 Sonnet, Sonnet Thinking, Opus variants
- **Gemini Pro group**: Gemini Pro High/Low variants
- **Gemini Flash group**: Gemini Flash variants
- **Gemini Image group**: Gemini Pro Image

Source: [vscode-antigravity-cockpit](https://github.com/jlcodes99/vscode-antigravity-cockpit)

This approach is Antigravity-specific and cannot be reused by mtt.

---

## Summary

| CLI | Usage API | Auth | Status in mtt |
|-----|-----------|------|---------------|
| Claude | `api.anthropic.com/api/oauth/usage` | Keychain OAuth | Implemented |
| Codex | Internal app-server IPC only | `~/.codex/auth.json` | Session time fallback |
| Gemini | Google Cloud Quotas (complex) | Google account | Session time fallback |
| Antigravity | Internal gRPC to Language Server | Process scanning | N/A (IDE-specific) |
