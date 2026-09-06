# burrow.rules

Declarative, agent-readable per-app cleaning rules (`burrow.rules/v1`).

**License: Apache-2.0** (deliberately permissive — the rule *format* and *data* are the
community ecosystem; only the conductor code is FSL). Contributions are accepted under
Apache-2.0 with the `provenance` block filled in.

## Format (one JSON file per app)

```jsonc
{
  "schema": "burrow.rules/v1",
  "app": { "bundle_ids": ["com.vendor.App"], "name": "App" },
  "rules": [{
    "id": "app.cache",
    "category": "cache",            // cache | log | crash-report | history | dev-artifact | …
    "risk": "safe",                 // safe | caution | risky  (risky is never auto-selected)
    "recommend": true,              // preselected in quick-clean (safe only)
    "explain": "Why this is safe to remove.",
    "targets": [{ "path": "~/Library/Caches/com.vendor.App", "search": "walk_all" }],
    "action": { "type": "delete", "method": "trash" }   // closed enum — never shell
  }],
  "provenance": {                   // REQUIRED — the agent-native differentiator
    "source": "builtin",            // builtin | community | agent | user
    "evidence": ["vendor doc URL / why the path is safe"],
    "license": "Apache-2.0"
  }
}
```

Every rule is statically auditable and dry-runnable (no arbitrary shell), and carries its
own evidence — so an agent can justify each deletion. Validate with `burrow rules validate`.

Path facts here are derived from vendor documentation and observed behavior; permissive
sources (kondo MIT, mac-cleanup-py Apache-2.0) are credited in each file's `provenance`.
No GPL rule data is copied.
