# Callback — Build Spec

> **Historical document:** This file preserves the original product and implementation proposal. It is not the source of truth for the current schema, code, release status, or roadmap. Read [`docs/current-state.md`](docs/current-state.md), [`docs/architecture.md`](docs/architecture.md), and [`docs/roadmap.md`](docs/roadmap.md) for implementation-backed documentation.

**Local-first reminders that fire on context, not clock time.**

---

## 1. The idea

You make roughly ten micro-promises a day. "I'll send that over." "Let me look into it." "I'll get back to you by Friday." You keep about six. The other four don't fail because you're careless — they fail because the moment you *could* act on them and the moment you *remember* them never overlap.

Every reminder tool asks you for a time. But a promise isn't a time, it's a **context**. "Send Priya the invoice" doesn't want 9:00 AM Tuesday when you're on a train. It wants the next time your cursor lands in Slack.

Callback is the loop closed:

> **Capture** the promise silently, at the instant you make it, from your own outgoing messages.
> **Resurface** it at the exact moment you have the ability to keep it.

You type "I'll send you the invoice tomorrow" into a Slack DM. Nothing visible happens. Two days later you focus Slack and a small card slides in: *You told Priya you'd send the invoice. 2 days ago.* One click marks it done.

You never wrote a to-do. You never set a reminder. The list wrote itself, and it read itself back to you at the only moment it mattered.

### Why this is one product and not two stapled together

A promise is a callback registered against a context that hasn't fired yet. The capture half is meaningless without the trigger half — you'd just have another guilt-inducing list. The trigger half without capture is a power-user toy nobody bothers to configure. Each half is the reason the other one gets used. That's the test for whether a combination is real, and this one passes it.

### Positioning

| | |
|---|---|
| **Stance** | Local-only runtime, MIT licensed, free forever |
| **Against** | Every cloud AI notetaker that uploads your messages to a server |
| **Audience** | Developers and knowledge workers — the people who star repos and write the blog posts |
| **One-line pitch** | Show HN: Callback — local-first reminders that fire on context, not clock time |

The privacy architecture *is* the marketing. Callback's runtime opens no TCP or UDP listener and transmits no message content. Chrome Store and package-manager traffic is outside this runtime guarantee. The repo remains open so these constraints can be verified.

### Explicit non-goals

Scope discipline is what gets this shipped. Callback does **not** do:

- Mobile (see §2 — the core mechanic is impossible on iOS)
- Cloud sync, accounts, login, or any server
- Calendar integration, email sending, task assignment, team features
- Meeting transcription or voice
- Full task management — this is a capture-and-resurface layer, not a Todoist replacement

---

## 2. Feasibility verdict

Brutal version: **desktop is 100% doable, mobile is not.** Here's exactly where each platform lands.

### iOS — impossible, cut it

No API exists for a third-party app to detect "the user just opened Slack." The nearest thing is Apple's Screen Time stack (`FamilyControls` + `DeviceActivity`), which fails three separate ways:

- It fires on **usage thresholds** ("15 minutes elapsed"), not on app launch
- The `DeviceActivityMonitor` extension is sandboxed so aggressively it cannot make network calls or run arbitrary logic
- The entitlement requires **manual Apple approval**, granted almost exclusively to parental-control vendors

Reading outgoing iMessages: no API at any privilege level. iOS is architecturally closed to this product. Do not spend a day on it.

### Android — possible, but not worth it in v1

| Mechanism | Verdict |
|---|---|
| `UsageStatsManager` + Usage Access permission | Works. ~1s poll granularity. Permission granted in system Settings, not a dialog — high drop-off. |
| `AccessibilityService` (instant, elegant) | **Will get you removed from Play.** Google restricts this API to genuine accessibility tools and actively enforces it. |
| Background service survival | Xiaomi, Oppo, Vivo, Samsung kill it regardless of spec compliance. See dontkillmyapp.com. |
| Outgoing message capture | `NotificationListenerService` only sees messages *you receive*. SMS reading is restricted to default-SMS-handler apps. |

So Android gives you a degraded version of one half, at the cost of a month of OEM-specific debugging, producing a feature that silently stops working on a large share of devices. Silently-broken is worse than absent. **Cut it.**

### Windows first; macOS / Linux are compile-safe extension points

- **Windows:** `SetWinEventHook` with `EVENT_SYSTEM_FOREGROUND` — event-driven, no polling, no permission prompt
- **macOS / Linux:** preserve the interfaces and compile-safe no-op adapters, but do not claim v1 functionality

Windows foreground identity alone is insufficient for browser context. Callback treats Gmail or Slack web as focused only when Chrome is foreground and the extension reports a matching visible active tab and context. Gmail web and Slack web are the only capture targets in v1; Slack desktop capture is not covered by the extension.

### Firewalls — a non-issue if you make one right choice

Windows Defender Firewall prompts only when a process **binds a listening socket**. Callback never does.

The critical decision: **do not use a localhost WebSocket to connect the browser extension to the desktop app.** Use **Chrome Native Messaging**, which pipes over stdin/stdout and never touches the network stack. This single choice eliminates the entire firewall category — no prompts, no antivirus heuristics about a background process listening on a port, no corporate-network breakage.

### The costs nobody warns you about

| Item | Reality |
|---|---|
| Windows code signing | $200–400/yr. Without it: SmartScreen "Windows protected your PC" wall. |
| macOS notarization | $99/yr Apple Developer Program. Without it: Gatekeeper block. |
| Chrome Web Store | **$5 one-time.** The only unavoidable cost, and avoidable entirely (see below). |
| Gmail API `gmail.readonly` | **Restricted scope** — public production apps require a third-party CASA security assessment costing thousands. No free path. |

Two of these dictate architecture:

**Gmail's restricted scope is why the extension reads the DOM instead of calling the API.** No OAuth, no verification, no cost, and the data genuinely never leaves the machine. The constraint pushed you toward the better design.

**Signing costs shape distribution, but package managers do not eliminate SmartScreen risk.** Target **winget and Scoop** for the Windows-first release and record unsigned-binary warnings as a launch risk. Revisit signing when the warning burden or audience justifies it.

---

## 3. Architecture

```
┌─────────────────────────────┐         ┌──────────────────────────────┐
│  Chrome Extension (MV3)     │         │  Callback (Tauri + Rust)     │
│                             │         │                              │
│  content script             │         │  extraction engine           │
│   ├─ Gmail web DOM hook     │         │  SQLite single writer        │
│   └─ Slack web DOM hook     │         │  surfacing engine            │
│         │                   │         │  review/settings UI          │
│         ▼                   │         └──────────────▲───────────────┘
│  service worker + outbox    │                        │
│   └─ connectNative()        │                        │
└──────────────┬──────────────┘                        │
               │ stdio                                │ current-user ACL
               ▼                                      │ named pipe
┌─────────────────────────────┐                        │
│ callback-native-host.exe    │────────────────────────┘
│ separate Chrome-launched    │
│ process                     │
└─────────────────────────────┘

┌─────────────────────────────┐
│ Windows focus watcher       │────────────────────────► Tauri Rust core
│ SetWinEventHook             │
└─────────────────────────────┘

        NO CALLBACK-OWNED LISTENER. NO SERVER. NO MESSAGE TRANSMISSION.
```

Chrome launches `callback-native-host.exe` as a separate native-messaging process; the stdio loop must not run inside the Tauri GUI process. The host validates a versioned envelope and extension origin, then forwards it over a current-user-only named pipe. The Tauri core owns the single logical SQLite writer.

**Stack:** Tauri v2, Rust, React, TypeScript, bundled SQLite, and native Windows adapters behind platform interfaces.

---

## 4. Data model

```sql
CREATE TABLE promises (
  id            INTEGER PRIMARY KEY,
  text          TEXT NOT NULL,        -- the extracted clause
  raw_message   TEXT NOT NULL,        -- full source message, for context
  source_app    TEXT NOT NULL,        -- 'slack' | 'gmail'
  source_ctx    TEXT,                 -- slack channel id / gmail thread id
  recipient     TEXT,                 -- display name if detected
  deadline      INTEGER,              -- unix ts, nullable
  confidence    REAL NOT NULL,        -- 0.0–1.0 from scorer
  status        TEXT NOT NULL,        -- 'open'|'done'|'dismissed'|'archived'|'review'
  created_at    INTEGER NOT NULL,
  resolved_at   INTEGER
);

CREATE TABLE triggers (
  id          INTEGER PRIMARY KEY,
  promise_id  INTEGER NOT NULL REFERENCES promises(id) ON DELETE CASCADE,
  kind        TEXT NOT NULL,   -- 'app_focus'|'app_ctx_focus'|'deadline'|'manual'
  match_value TEXT NOT NULL,   -- 'slack.exe' | 'slack:D0123ABC' | ''
  priority    INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE surface_events (
  id          INTEGER PRIMARY KEY,
  promise_id  INTEGER NOT NULL REFERENCES promises(id) ON DELETE CASCADE,
  shown_at    INTEGER NOT NULL,
  action      TEXT             -- 'done'|'snooze'|'not_a_promise'|'ignored'
);

CREATE TABLE blocklist (      -- learned false positives
  id       INTEGER PRIMARY KEY,
  pattern  TEXT NOT NULL,     -- normalized clause skeleton
  hits     INTEGER DEFAULT 1
);

CREATE TABLE settings (k TEXT PRIMARY KEY, v TEXT NOT NULL);

CREATE INDEX idx_promises_status ON promises(status);
CREATE INDEX idx_triggers_match  ON triggers(kind, match_value);
```

---

## 5. The extraction engine

**This is where the product lives or dies.** Below ~70% precision, users uninstall within a day. Precision beats recall by a wide margin — a missed promise costs nothing, a false one costs trust.

### Start with heuristics, not an LLM

A regex-plus-scoring approach runs in microseconds, is fully debuggable, and requires no model download. A 1B-parameter local model is a black box you cannot tune when it misfires. Build the scorer first; only reach for a model if you can *prove* the ceiling is the bottleneck.

### Pipeline

1. **Segment** the message into clauses on `.`, `;`, `,`, ` and `, newlines
2. **Normalize** — expand contractions (`I'll` → `I will`), lowercase a copy, keep the original for display
3. **Score** each clause independently
4. **Route** by score

### Scoring

**Positive signals (additive):**

| Signal | Points | Examples |
|---|---|---|
| First-person commissive modal | +3 | `I will`, `I'll`, `I'm going to`, `I'm gonna`, `let me`, `I can` |
| Deliverable action verb | +2 | send, share, ship, push, draft, review, check, fix, update, email, ping, call, book, schedule, follow up, get back, look into, take care of, circle back |
| Temporal anchor | +2 | today, tonight, tomorrow, EOD, EOW, by Friday, this week, next week, in an hour, later, shortly, ASAP, by the 3rd |
| Concrete object noun | +1 | invoice, doc, deck, link, file, PR, notes, draft, quote, contract, numbers, report |

**Hard kills (reject regardless of score):**

- Clause ends in `?` — it's a question
- Second-person request: `can you`, `could you`, `would you mind`, `please <verb>` — that's a promise *from* someone else
- Opinion frames: `I think`, `I'd say`, `I believe`, `I feel like`
- Past/completed: `I sent`, `I already`, `I've`, `I did`
- Line begins with `>` or sits inside a quoted/forwarded block
- Clause skeleton matches an entry in `blocklist`

**Soft penalties:**

- Conditional wrapper (`if`, `unless`, `in case`, `assuming`): −2
- Pure attendance (`I'll be there`, `I'll be late`, `I'm in`): −3 — not a deliverable
- Clause > 25 words: −1 — likely narrative, not commitment

### Routing

| Score | Destination |
|---|---|
| **≥ 6** | Captured, surfaceable |
| **4–5** | `status='review'` — visible in the app window, **never** surfaced as a notification |
| **≤ 3** | Discarded silently |

The review bucket is important: it lets you tune the threshold against real data without ever annoying the user with a low-confidence guess.

### Deadline parsing

Use `two_timer` or `chrono-english` (Rust crates) on the temporal anchor. Resolve relative expressions against message timestamp. Store `NULL` on failure — a promise with no deadline is still valid, it just relies on the context trigger.

### Learning from rejection

"Not a promise" writes a **normalized skeleton** of the clause to `blocklist` — POS-ish shape rather than the literal string, so `I'll be at the standup` also blocks `I'll be at the retro`. After 3 blocklist hits on a similar shape, raise the effective threshold for that pattern class. This is the entire personalization system, and it needs no model.

### Tuning protocol

Before showing anyone: export 300 of your own sent messages, hand-label them, and run the scorer. Iterate weights until precision ≥ 80% on your own data. Do not ship at 60% and hope.

---

## 6. The trigger engine

### Focus detection

Prefer **event-driven** over polling on every platform. Most Rust active-window crates poll at 500ms–1s; writing thin FFI gets you zero-cost event hooks and no battery complaints in your HN thread.

```rust
// Windows: no polling, no permissions
SetWinEventHook(
    EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND,
    null_mut(), Some(callback), 0, 0,
    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS
);
// in callback: GetWindowThreadProcessId → OpenProcess → QueryFullProcessImageNameW
```

```swift
// macOS: no polling, no permissions (bundle ID only, no window title)
NSWorkspace.shared.notificationCenter.addObserver(
    forName: NSWorkspace.didActivateApplicationNotification, ...
)
```

**Debounce:** require **5 seconds of continuous focus** before a trigger is eligible. This kills alt-tab flicker and — more importantly — avoids interrupting the user in the first instant of switching apps, when they already have an intent loaded.

### Auto-linking promises to triggers

The user should never configure a trigger manually. Assign on capture, in priority order:

1. **Context-specific** (best) — promise made in Slack DM `D0123` → trigger fires on focusing *that DM*. The extension reports the channel/thread ID, so this is free precision. Requires an in-page hook from the extension reporting current context on navigation.
2. **App-level** (default) — promise made in Slack → fires on Slack focus. Always created as a fallback with lower priority.
3. **Keyword→app map** (bonus) — "I'll push the fix" → also register a trigger on VS Code focus. Ship a small default map, make it user-editable JSON.
4. **Deadline escalation** — if a deadline passes with no trigger fired, allow exactly **one** time-based surface. This is the only clock-based behavior in the product, and it exists so nothing gets silently lost.

---

## 7. The surfacing engine

**Notification fatigue kills these apps more reliably than bugs do.** Every rule below is a v1 requirement, not v2 polish.

| Rule | Value |
|---|---|
| Global surfaces per day | **3 maximum** |
| Minimum gap between surfaces | 90 minutes |
| Focus dwell before eligible | 5 seconds |
| Same promise shown twice in a day | Never |
| Shown 3× with no action | Auto-archive, ask "still relevant?" in-app |
| Callback quiet hours | Persist and enforce locally |
| Windows Do Not Disturb | Let Windows suppress native notifications |
| First 30 minutes after app install | Silent — let it collect before it speaks |

**Selection:** when multiple promises match a fired trigger, surface exactly **one** — highest priority by (deadline proximity, then confidence, then age). Never stack cards.

**The card:** promise text, recipient, relative age, and three actions — **Done** / **Snooze** / **Not a promise**. Nothing else. It should be dismissible with Escape and readable in under two seconds.

Callback never bypasses Windows Do Not Disturb with an always-on-top card. A suppressed backlog is not burst later; eligibility is reconsidered one candidate at a time after a new focus transition.

---

## 8. Browser extension — implementation notes

### Manifest V3, minimal permissions

```json
{
  "manifest_version": 3,
  "name": "Callback Capture",
  "version": "0.1.0",
  "key": "<base64-public-key>",
  "permissions": ["nativeMessaging", "storage"],
  "host_permissions": ["https://mail.google.com/*", "https://app.slack.com/*"],
  "background": { "service_worker": "sw.js" },
  "content_scripts": [{
    "matches": ["https://mail.google.com/*", "https://app.slack.com/*"],
    "js": ["capture.js"],
    "run_at": "document_idle"
  }]
}
```

The `key` field is not mandatory for every Chrome extension. Callback keeps it operationally required during development because native-messaging host manifests whitelist a specific extension ID via `allowed_origins`; pinning the development ID keeps that allowlist stable across machines.

### Hooking send

- **Gmail web and Slack web only:** capture intent before teardown, but persist it only after the site confirms the send succeeded. Click and keyboard paths must deduplicate.
- Treat both sites' DOM structures as unsupported, changeable contracts. Keep selectors external from day one and use content-free health probes. Never log message bodies while probing selector health.
- Slack desktop is outside v1 capture scope.

### The fragility problem, and the mitigation

DOM hooks are the fragile spine of this product. Gmail and Slack change markup without notice, and when they do, Callback silently stops capturing — the worst possible failure mode, because the user thinks it's working.

Three-part mitigation:

1. **Externalize selectors** into a versioned `selectors.json` with ordered fallback chains per site. Users can patch a break without waiting on you, and fixes arrive as one-line PRs.
2. **Ship a self-test.** If the user has been active on Gmail for 7 days with zero captures, show a banner: *"Gmail capture may be broken — check for a selector update."* Never fail silently.
3. **Add a global hotkey** (e.g. `Ctrl+Shift+K`) that opens a quick-capture window. Do not claim it can silently read arbitrary selected text.

### Native messaging wiring

Protocol: 32-bit length prefix in **native byte order**, then UTF-8 JSON. Chrome-to-host messages are limited to 64 MiB; host-to-Chrome messages are limited to 1 MiB. On Windows the host must put stdin/stdout in binary mode and emit nothing except framed messages on stdout.

Host manifest install locations:

| OS | Location |
|---|---|
| Windows | Registry: `HKCU\Software\Google\Chrome\NativeMessagingHosts\com.callback.host` → path to manifest JSON |
| macOS | `~/Library/Application Support/Google/Chrome/NativeMessagingHosts/com.callback.host.json` |
| Linux | `~/.config/google-chrome/NativeMessagingHosts/com.callback.host.json` |

The Tauri app writes these on first run. Ship an in-app "Reconnect extension" button that rewrites them — this will be your most common support request.

---

## 9. Phased plan with kill gates

### Phase 0 — Prove the trigger *(one weekend)*

Tauri shell, SQLite, native focus watcher, and **hardcoded rules from a dropdown**: "when I focus [app] → show [text]." No extension. No extraction. No AI.

> **KILL GATE:** Use it yourself for five days. If context-triggered reminders don't feel meaningfully better than time-based ones, **stop here**. Everything downstream is built on this assumption, and it costs you two days to test instead of two months.

### Phase 1 — Capture *(one week)*

Chrome extension for Slack and Gmail. Native messaging pipe. Heuristic extractor. Everything lands in the review queue — **no surfacing yet**. Run it for a week and read the queue daily.

> **KILL GATE:** precision ≥ 70% on your own real messages. Below that, tune weights, don't proceed.

### Phase 2 — Close the loop *(one week)*

Auto-linking, surfacing engine with all rate limits, notification card, Done/Snooze/Not-a-promise, blocklist learning, tray icon, autostart.

> **KILL GATE:** two weeks of personal daily use. Track your own **acceptance rate** — surfaces where you clicked Done or acted. Below 40%, the trigger logic or the extraction is wrong. Fix before launch.

### Phase 3 — Polish and launch *(one week)*

- Onboarding: one screen, install extension, done
- Settings: daily cap, quiet hours, per-app enable, selector update button
- Cross-platform builds via GitHub Actions
- winget manifest PR, Scoop bucket, Homebrew tap
- README with a **15-second GIF above the fold** — type promise → open app → card appears. That GIF is 80% of your launch. Make it before you make the README.
- Show HN post leading with the local-only architecture

### Phase 4 — Optional, only if justified

Local LLM extraction (Qwen2.5-1.5B via `llama.cpp`) behind a **default-off** toggle. Build only if Phase 2 proves heuristic precision is the ceiling. More sites (Linear, GitHub, Discord, Teams) via community selector packs. Read-only PWA that syncs through the user's own storage — never your server.

---

## 10. Risks, ranked

| # | Risk | Severity | Mitigation |
|---|---|---|---|
| 1 | Extraction precision too low → uninstall in day one | **Critical** | Review-queue gate in Phase 1; precision-over-recall thresholds; hand-tune on 300 labeled messages before anyone sees it |
| 2 | Notification fatigue → app gets muted, then removed | **Critical** | Hard 3/day cap, 90-min gap, 5s dwell, auto-archive after 3 ignores — all v1 |
| 3 | DOM selectors break silently | High | External selector packs, 7-day self-test banner, global-hotkey fallback |
| 4 | Native messaging setup friction | Medium | Pinned extension key, auto-written host manifests, in-app reconnect button |
| 5 | Unsigned binary warnings | Medium | Package managers instead of direct download |
| 6 | "Reading my messages" privacy optics | Medium | Zero sockets is a verifiable claim in an open repo — lead with it rather than defend it |
| 7 | Linux Wayland degraded | Low | Document honestly as best-effort |

### What should make you abandon this

Be willing to say these out loud:

- Phase 0 doesn't feel better than a normal reminder → the core premise is wrong
- You can't get precision past ~65% on your own messages after a week of tuning → the capture is unusable and an LLM probably won't save it
- Your own acceptance rate sits under 40% after two weeks → you built a guilt machine, not a tool

Any one of these is a real signal. Two days spent testing the premise is cheaper than two months spent building on it.

---

## 11. First three commits

1. Tauri v2 scaffold + `rusqlite` with the schema above + tray icon
2. Platform focus watcher behind a `trait FocusWatcher` with a `focus_changed(app_id, ctx)` event — Windows and macOS impls, X11 stub
3. Hardcoded-rule matcher + native notification — **that's Phase 0 complete and your kill gate ready to run**
