# Copia

[![build](https://github.com/potpiemuncher/copia/actions/workflows/build.yml/badge.svg)](https://github.com/potpiemuncher/copia/actions/workflows/build.yml)
[![license: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

*cōpia* — Latin for plenty, supply, what you have on hand.

A small menubar / system-tray app that shows how much of your AI coding
subscriptions you have left. The tray icon is itself a tiny chart, one bar per
tool. Click it and you get the same thing bigger: labelled vertical bars, one
per quota window, filled to what remains.

```
 COPIA  as of 4:51 PM                 [Refresh]

  62%   39%  │   88%   13%  │
  ▓▓    ░░   │   ▓▓    ░░   │
  ▓▓    ░░   │   ▓▓    ░░   │
  ▓▓    ▓▓   │   ▓▓    ░░   │
  ▓▓    ▓▓   │   ▓▓    ▓▓   │
  5h   week  │   5h   week  │
   Claude    │     Codex    │   …
```

Built with [Tauri 2](https://tauri.app) (Rust + one HTML file), MIT licensed.

## Status

Early, and honest about it:

- **Built and run on macOS only** (Apple silicon), with demo data. The tray
  icon, the popover and its layout work there.
- **Claude and Codex have been fetched from live accounts** (22 Nov 2026).
  Claude returned its 5-hour and weekly windows; Codex returned one weekly
  window. **Gemini and OpenCode Go have not been tried against a real
  account**; their code follows the endpoints listed below, so expect to fix
  field names.
- **macOS fullscreen:** the popover did not come on screen while another app
  was in a fullscreen Space. It likely needs to become an `NSPanel` (see
  `tauri-nspanel`). Outside fullscreen it shows normally.
- **Windows and Linux compile in CI but have not been run by anyone yet.**

## What it reads

Copia does not log in to anything. It reuses the sign-in each tool's own CLI
already left on your computer, asks that vendor's own usage endpoint, and shows
the answer. Credentials are read only, never written or refreshed, and each one
is sent only to the vendor it belongs to.

| Tool | Credential it reads | Asks |
| --- | --- | --- |
| Claude | macOS Keychain item `Claude Code-credentials`, else `~/.claude/.credentials.json` | `api.anthropic.com/api/oauth/usage` |
| Codex / ChatGPT | `~/.codex/auth.json` (or `$CODEX_HOME`) | `chatgpt.com/backend-api/wham/usage` |
| Gemini CLI | `~/.gemini/oauth_creds.json` | `cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota` |
| OpenCode Go | `$OPENCODE_API_KEY`, else `~/.local/share/opencode/auth.json` | `opencode.ai/inference/go/v1/usage` |

Things to know:

- On macOS the first Claude fetch makes the system ask whether Copia may read
  the Keychain item. Choose *Always Allow* or it asks every refresh. The
  approval is for Copia's own signed app only, so a rebuilt development binary
  asks again. (Versions before this one asked on behalf of `/usr/bin/security`,
  which let any program read the token. If you allowed that, remove `security`
  from the item's *Access Control* tab in Keychain Access.)
- None of these are documented public APIs. They are what the vendors' own
  tools call and can change without notice.
- Copia never refreshes tokens. When one ages out the bar is replaced by
  "sign-in expired — run `claude` once to renew it" (or `codex`, `gemini`).
- Google stopped serving Gemini CLI sign-ins for individual, AI Pro and Ultra
  accounts in June 2026, so Gemini will show an error for those accounts.
- OpenCode Go is the one provider with a vendor-documented usage endpoint, and
  Copia follows that documentation, but it has not been run with a real Go
  key. OpenCode v2 keeps credentials in a SQLite database Copia does not read,
  so if `auth.json` is missing or stale, set `OPENCODE_API_KEY` to your Go key.
- **Not built yet:** Antigravity, OpenCode Zen balance, Cursor, Copilot. Use a
  custom command (below) or send a patch to `src/providers.rs`; a provider is
  one function.

A tool you are not signed in to is simply left out.

## Run it

You need [Rust](https://rustup.rs). On Linux also the Tauri system libraries:

```bash
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev
```

Then:

```bash
cargo run --release
```

To look at it with made-up numbers and without touching any credential:

```bash
cargo run --release -- --demo
```

To check what Copia can read without opening the tray, print the quotas once
as JSON (no tokens are printed):

```bash
cargo run --release -- --print
```

To make an installable app (`.app`/`.dmg`, `.msi`, `.deb`/`.AppImage`):

```bash
cargo install tauri-cli --version "^2" --locked
```

```bash
cargo tauri build
```

## Using it

- **Left click** the tray icon: show or hide the bars. Clicking anywhere else
  hides them, as does Esc. Ctrl+R / ⌘R refreshes.
- **Right click**: Show quotas / Refresh now / Quit. On Linux, where trays only
  give apps a menu, this menu is the way in.
- **The icon**: one bar per signed-in tool, filled to what is left of that
  tool's *tightest* quota, so a nearly empty bar means "one of this tool's
  limits is close". On macOS it is a template image that follows the menubar
  colour; elsewhere bars are purple, amber below 50% and red below 20%.
- Hover the icon for the numbers as text.
- A provider marked **unverified** in the popover is one whose request or reply
  code has not been run against a live account (currently OpenCode Go).
- The popover follows the system light / dark theme, redraws its bars in
  system colours under Windows High Contrast, and exposes each bar to screen
  readers as a meter ("week: 39% left, resets in 3d").

## Settings

Optional. `config.json` in `~/Library/Application Support/copia/` (macOS),
`~/.config/copia/` (Linux) or `%APPDATA%\copia\` (Windows). It is re-read on
every refresh.

```json
{
  "refresh_minutes": 5,
  "disabled": ["gemini"],
  "custom": [
    { "name": "My tool", "command": ["/path/to/script"] }
  ]
}
```

Provider ids for `disabled`: `claude`, `codex`, `gemini`, `opencode-go`.

A **custom** provider is any program that prints quotas as JSON, with `used` or
`remaining` as a fraction from 0 to 1 and `resets_at` as epoch seconds or an
ISO-8601 time:

```json
[
  { "label": "day", "remaining": 0.4, "resets_at": "2026-11-23T00:00:00Z" },
  { "label": "month", "used": 0.15 }
]
```

A custom command gets 15 seconds and 1 MB of output; past either it is stopped
and its bar shows why. Because these commands run on every refresh, on macOS
and Linux Copia runs them only when `config.json` and its folder belong to you
and cannot be written by group or others (`chmod go-w`); otherwise it skips
them and says so in the popover.

## Layout

```
src/main.rs        tray icon, popover window, refresh loop
src/providers.rs   one function per tool: read credential, fetch, map to quotas
src/icon.rs        draws the tray icon's bars
ui/index.html      the popover (plain HTML, CSS and JS, no build step)
ui/style.css       its styles
ui/app.js          its script (kept out of the HTML for the strict CSP)
ui/preview.js      demo stand-in so index.html can be opened in a browser
scripts/make_icons.py   regenerates icons/
```

## Credits

- The idea came from [@zip.mu.social](https://rainshare.dev/seed/zip.mu.social/3n2v6joikno2l)
  on Rainshare: <https://rainshare.dev/seed/zip.mu.social/3n2v6joikno2l>.
- The work was done by Claude Fable 5.2 (`claude-fable-5-2`) in Claude Code.
- Which endpoint each vendor's CLI calls was learned from the provider notes in
  [CodexBar](https://github.com/steipete/CodexBar) (MIT), a far more complete
  macOS app. If you are on a Mac and want 50+ providers today, use that.
  [Win-CodexBar](https://github.com/nesszer/Win-CodexBar) and
  [AI Usage Widget](https://odrasile.github.io/ai-usage-widget/) are also worth
  a look.

## License

[MIT](LICENSE)
