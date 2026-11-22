// Browser-only stand-in for the Tauri backend, so the popover can be designed
// by opening index.html from any static server. Never loaded inside the app.
(() => {
  const now = Math.floor(Date.now() / 1000);
  const q = (label, used, secs) => ({ label, used, resets_at: now + secs });
  const p = (name, quotas, extra = {}) => ({ id: name, name, configured: true, quotas, error: null, ...extra });
  const snapshot = {
    demo: true, loading: false, updated_at: now,
    providers: [
      p("Claude", [q("5h", 0.38, 8900), q("week", 0.61, 259200)]),
      p("Codex", [q("5h", 0.12, 14400), q("week", 0.87, 432000)]),
      p("Gemini", [], { error: "sign-in expired — run `gemini` once to renew it" }),
      p("OpenCode Go", [], { configured: false }),
    ],
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd) => (cmd === "get_snapshot" ? snapshot : null) },
    event: { listen: async () => () => {} },
  };
})();
