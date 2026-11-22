//! Where the numbers come from.
//!
//! Every provider reads a credential that the vendor's own CLI already left on
//! this machine, asks the vendor's own usage endpoint, and maps the answer to a
//! list of `Quota`s. Copia never writes credentials, never refreshes them and
//! never sends them anywhere except the vendor they belong to.
//!
//! None of these endpoints is a documented public API. They are what the
//! vendors' CLIs call, as catalogued by the MIT-licensed CodexBar project
//! (https://github.com/steipete/CodexBar/tree/main/docs). Expect them to change.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Serialize, Clone, Debug)]
pub struct Quota {
    /// Short name of the window: "5h", "week", "pro"…
    pub label: String,
    /// Fraction of the quota already used, 0.0 – 1.0.
    pub used: f64,
    /// When it resets: epoch seconds (number) or an ISO-8601 string.
    pub resets_at: Option<Value>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Provider {
    pub id: String,
    pub name: String,
    /// False when no credential for this provider was found on this machine.
    pub configured: bool,
    pub quotas: Vec<Quota>,
    pub error: Option<String>,
    /// True when this provider's request or reply shape is a guess that has
    /// not been confirmed against a live account; the popover says so.
    pub experimental: bool,
}

#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Config {
    pub refresh_minutes: Option<u64>,
    /// Provider ids to skip: "claude", "codex", "gemini", "opencode-go".
    pub disabled: Vec<String>,
    pub custom: Vec<Custom>,
}

#[derive(Deserialize, Clone)]
pub struct Custom {
    pub name: String,
    /// Program and arguments; must print quota JSON on stdout (see README).
    pub command: Vec<String>,
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        Some(dirs::config_dir()?.join("copia").join("config.json"))
    }

    pub fn load() -> Config {
        Self::path()
            .and_then(read_json)
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default()
    }
}

/// Providers whose endpoint details are educated guesses.
const EXPERIMENTAL: [&str; 1] = ["opencode-go"];

/// Ok(None) = not set up on this machine; Err = set up but the fetch failed.
type Fetched = Result<Option<Vec<Quota>>, String>;

pub fn fetch_all(config: &Config) -> Vec<Provider> {
    let builtin: [(&str, &str, fn() -> Fetched); 4] = [
        ("claude", "Claude", claude),
        ("codex", "Codex", codex),
        ("gemini", "Gemini", gemini),
        ("opencode-go", "OpenCode Go", opencode_go),
    ];

    std::thread::scope(|scope| {
        let mut jobs = Vec::new();
        for (id, name, fetch) in builtin {
            if config.disabled.iter().any(|d| d == id) {
                continue;
            }
            jobs.push((id.to_string(), name.to_string(), scope.spawn(fetch)));
        }
        for custom in &config.custom {
            let id = format!("custom:{}", custom.name);
            jobs.push((id, custom.name.clone(), scope.spawn(|| run_custom(custom))));
        }
        jobs.into_iter()
            .map(|(id, name, job)| {
                let fetched = job.join().unwrap_or_else(|_| Err("crashed".into()));
                let (configured, quotas, error) = match fetched {
                    Ok(Some(quotas)) if quotas.is_empty() => {
                        (true, quotas, Some("no quota windows in the reply".to_string()))
                    }
                    Ok(Some(quotas)) => (true, quotas, None),
                    Ok(None) => (false, Vec::new(), None),
                    Err(error) => (true, Vec::new(), Some(error)),
                };
                let experimental = EXPERIMENTAL.contains(&id.as_str());
                Provider { id, name, configured, quotas, error, experimental }
            })
            .collect()
    })
}

/// Made-up numbers for screenshots and for trying the UI without any account.
pub fn demo() -> Vec<Provider> {
    let now = now_secs();
    let quota = |label: &str, used: f64, secs: u64| Quota {
        label: label.into(),
        used,
        resets_at: Some(json!(now + secs)),
    };
    let provider = |id: &str, name: &str, quotas: Vec<Quota>| Provider {
        id: id.into(),
        name: name.into(),
        configured: true,
        quotas,
        error: None,
        experimental: EXPERIMENTAL.contains(&id),
    };
    vec![
        provider(
            "claude",
            "Claude",
            vec![quota("5h", 0.38, 2 * 3600 + 700), quota("week", 0.61, 3 * 86400)],
        ),
        provider(
            "codex",
            "Codex",
            vec![quota("5h", 0.12, 4 * 3600), quota("week", 0.87, 5 * 86400)],
        ),
        provider(
            "gemini",
            "Gemini",
            vec![quota("pro", 0.95, 9 * 3600), quota("flash", 0.20, 9 * 3600)],
        ),
        provider(
            "opencode-go",
            "OpenCode Go",
            vec![
                quota("5h", 0.05, 3600),
                quota("week", 0.44, 2 * 86400),
                quota("month", 0.70, 12 * 86400),
            ],
        ),
    ]
}

// ---------------------------------------------------------------- Claude ---

fn claude() -> Fetched {
    let creds = claude_keychain().or_else(|| read_json(dirs::home_dir()?.join(".claude/.credentials.json")));
    let token = match creds
        .as_ref()
        .and_then(|c| c.pointer("/claudeAiOauth/accessToken"))
        .and_then(Value::as_str)
    {
        Some(token) => token,
        None => return Ok(None),
    };

    let reply = send(
        agent()
            .get("https://api.anthropic.com/api/oauth/usage")
            .set("Authorization", &format!("Bearer {token}"))
            .set("anthropic-beta", "oauth-2025-04-20"),
        None,
    )
    .map_err(|e| stale(e, "claude"))?;

    let windows = [
        ("five_hour", "5h"),
        ("seven_day", "week"),
        ("seven_day_opus", "opus"),
        ("seven_day_sonnet", "sonnet"),
    ];
    let mut quotas = Vec::new();
    for (key, label) in windows {
        let window = &reply[key];
        if let Some(percent) = window["utilization"].as_f64() {
            quotas.push(Quota {
                label: label.into(),
                used: fraction(percent / 100.0),
                resets_at: some(&window["resets_at"]),
            });
        }
    }
    Ok(Some(quotas))
}

/// On macOS Claude Code keeps its login in the Keychain. Asking the system
/// `security` tool means macOS itself prompts the user for permission.
#[cfg(target_os = "macos")]
fn claude_keychain() -> Option<Value> {
    let out = Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

#[cfg(not(target_os = "macos"))]
fn claude_keychain() -> Option<Value> {
    None
}

// ----------------------------------------------------------------- Codex ---

fn codex() -> Fetched {
    let home = match std::env::var_os("CODEX_HOME") {
        Some(dir) => PathBuf::from(dir),
        None => match dirs::home_dir() {
            Some(home) => home.join(".codex"),
            None => return Ok(None),
        },
    };
    let Some(auth) = read_json(home.join("auth.json")) else {
        return Ok(None);
    };
    let Some(token) = auth.pointer("/tokens/access_token").and_then(Value::as_str) else {
        return Ok(None);
    };

    let mut request = agent()
        .get("https://chatgpt.com/backend-api/wham/usage")
        .set("Authorization", &format!("Bearer {token}"));
    if let Some(account) = auth.pointer("/tokens/account_id").and_then(Value::as_str) {
        request = request.set("ChatGPT-Account-Id", account);
    }
    let reply = send(request, None).map_err(|e| stale(e, "codex"))?;

    let mut quotas = Vec::new();
    for key in ["primary_window", "secondary_window"] {
        let window = &reply["rate_limit"][key];
        if let Some(percent) = window["used_percent"].as_f64() {
            let resets_at = some(&window["reset_at"]).or_else(|| {
                let secs = window["reset_after_seconds"].as_u64()?;
                Some(json!(now_secs() + secs))
            });
            quotas.push(Quota {
                label: window["limit_window_seconds"]
                    .as_u64()
                    .map(window_label)
                    .unwrap_or_else(|| key.trim_end_matches("_window").to_string()),
                used: fraction(percent / 100.0),
                resets_at,
            });
        }
    }
    Ok(Some(quotas))
}

// ---------------------------------------------------------------- Gemini ---

fn gemini() -> Fetched {
    let Some(creds) = dirs::home_dir().and_then(|h| read_json(h.join(".gemini/oauth_creds.json"))) else {
        return Ok(None);
    };
    let Some(token) = creds["access_token"].as_str() else {
        return Ok(None);
    };
    // Copia does not refresh tokens (that needs the Gemini CLI's own OAuth
    // client secret), so an expired one is reported instead of used.
    if let Some(expiry_ms) = creds["expiry_date"].as_f64() {
        if expiry_ms < now_secs() as f64 * 1000.0 {
            return Err("sign-in expired — run `gemini` once to renew it".into());
        }
    }

    let reply = send(
        agent()
            .post("https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota")
            .set("Authorization", &format!("Bearer {token}")),
        Some(json!({})),
    )
    .map_err(|e| stale(e, "gemini"))?;

    // One bucket per model; show the tightest bucket of each model tier.
    let mut quotas: Vec<Quota> = Vec::new();
    for bucket in reply["buckets"].as_array().into_iter().flatten() {
        let Some(remaining) = bucket["remainingFraction"].as_f64() else {
            continue;
        };
        let model = bucket["modelId"].as_str().unwrap_or("quota");
        let label = if model.contains("flash-lite") {
            "lite"
        } else if model.contains("flash") {
            "flash"
        } else if model.contains("pro") {
            "pro"
        } else {
            model
        };
        let quota = Quota {
            label: label.into(),
            used: fraction(1.0 - remaining),
            resets_at: some(&bucket["resetTime"]),
        };
        match quotas.iter_mut().find(|q| q.label == label) {
            Some(existing) if existing.used < quota.used => *existing = quota,
            Some(_) => {}
            None => quotas.push(quota),
        }
    }
    Ok(Some(quotas))
}

// ----------------------------------------------------------- OpenCode Go ---

/// Experimental: the usage endpoint and reply shape are taken from CodexBar's
/// notes and have not been confirmed against a live subscription.
fn opencode_go() -> Fetched {
    let key = std::env::var("OPENCODE_API_KEY").ok().filter(|k| !k.is_empty()).or_else(|| {
        let auth = read_json(dirs::home_dir()?.join(".local/share/opencode/auth.json"))?;
        Some(auth.pointer("/opencode-go/key")?.as_str()?.to_string())
    });
    let Some(key) = key else {
        return Ok(None);
    };

    let reply = send(
        agent()
            .get("https://opencode.ai/zen/go/v1/usage")
            .set("Authorization", &format!("Bearer {key}")),
        None,
    )?;

    let windows = [
        ("rolling", "rollingUsage", "5h"),
        ("weekly", "weeklyUsage", "week"),
        ("monthly", "monthlyUsage", "month"),
    ];
    let mut quotas = Vec::new();
    for (nested, flat, label) in windows {
        let window = match &reply["usage"][nested] {
            Value::Null => &reply[flat],
            found => found,
        };
        let percent = window["percent"].as_f64().or_else(|| window["usagePercent"].as_f64());
        if let Some(percent) = percent {
            quotas.push(Quota {
                label: label.into(),
                used: fraction(percent / 100.0),
                resets_at: window["resetInSec"].as_u64().map(|s| json!(now_secs() + s)),
            });
        }
    }
    Ok(Some(quotas))
}

// ---------------------------------------------------------------- Custom ---

/// Anything else: run a command the user configured and read quota JSON from
/// its stdout — either `[{label, used | remaining, resets_at?}]` or
/// `{"quotas": [...]}`, with `used` / `remaining` as 0–1 fractions.
fn run_custom(custom: &Custom) -> Fetched {
    let (program, args) = custom.command.split_first().ok_or("empty command")?;
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{program} exited with {}", out.status));
    }
    let reply: Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("output is not JSON: {e}"))?;
    let list = match &reply["quotas"] {
        Value::Null => &reply,
        found => found,
    };
    let mut quotas = Vec::new();
    for item in list.as_array().ok_or("expected a list of quotas")? {
        let used = match (item["used"].as_f64(), item["remaining"].as_f64()) {
            (Some(used), _) => used,
            (None, Some(remaining)) => 1.0 - remaining,
            _ => return Err("each quota needs `used` or `remaining`".into()),
        };
        quotas.push(Quota {
            label: item["label"].as_str().unwrap_or("quota").to_string(),
            used: fraction(used),
            resets_at: some(&item["resets_at"]),
        });
    }
    Ok(Some(quotas))
}

// --------------------------------------------------------------- Helpers ---

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("copia/", env!("CARGO_PKG_VERSION")))
        .build()
}

fn send(request: ureq::Request, body: Option<Value>) -> Result<Value, String> {
    let result = match body {
        Some(body) => request.send_json(body),
        None => request.call(),
    };
    match result {
        Ok(response) => response
            .into_json()
            .map_err(|e| format!("unreadable reply: {e}")),
        Err(ureq::Error::Status(code, response)) => {
            let text = response.into_string().unwrap_or_default();
            let detail: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let detail: String = detail.chars().take(100).collect();
            if detail.is_empty() || detail.starts_with('<') {
                Err(format!("HTTP {code}"))
            } else {
                Err(format!("HTTP {code}: {detail}"))
            }
        }
        Err(e) => Err(format!("network: {e}")),
    }
}

/// A 401 almost always means the CLI's token aged out; say how to fix it.
fn stale(error: String, cli: &str) -> String {
    if error.starts_with("HTTP 401") {
        format!("sign-in expired — run `{cli}` once to renew it")
    } else {
        error
    }
}

fn read_json(path: PathBuf) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn some(value: &Value) -> Option<Value> {
    match value {
        Value::String(_) | Value::Number(_) => Some(value.clone()),
        _ => None,
    }
}

fn fraction(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn window_label(seconds: u64) -> String {
    match seconds {
        604_800 => "week".into(),
        s if s >= 86_400 && s % 86_400 == 0 => format!("{}d", s / 86_400),
        s if s >= 3_600 => format!("{}h", s / 3_600),
        s => format!("{}m", (s / 60).max(1)),
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
