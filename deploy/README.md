# Optional Linux service installation

Nothing in this directory is installed automatically. The service template does not edit Klipper, Moonraker, printer configuration, macros, firmware or existing services.

Build the correct executable for the host first. A Raspberry Pi must be running a 64-bit GNU/Linux system for the ARM64 binary. Native `cargo build --release -j 2` on that machine avoids assuming compatibility with a different distribution's system libraries.

The template expects:

| Item | Location / requirement |
|---|---|
| Executable | `/usr/local/bin/stella-scope`, executable by the service account. |
| Account | Existing unprivileged system user and group named `stella-scope`. Create these using the host's normal account-management tools before enabling the unit. |
| Configuration | `/etc/stella-scope.json`, readable by the service account. Start from `stella-scope.example.json`. |
| Recording directory | `/var/lib/stella-scope`; systemd creates it through `StateDirectory`. |
| Database setting | `stella-scope.sqlite3` relative to that directory, or an absolute path within it. The unit intentionally cannot write elsewhere. |
| Optional credentials | `/etc/stella-scope.env`, root-owned and mode `0600`, containing `STELLA_MOONRAKER_API_KEY=your-key` only when needed. Never commit this file. |
| Optional Klipper log | Set the real local `klippy_log_path`; grant the service account narrowly scoped read/traverse permissions. Do not make the entire printer home directory world-readable. |

After those prerequisites have been deliberately prepared, copy `stella-scope.service` to `/etc/systemd/system/stella-scope.service`, reload systemd, and enable/start that unit. Inspect `journalctl -u stella-scope` and `/api/meta` for configuration, connection or permission errors. The JSON listener remains loopback-only unless you explicitly change it.

`ProtectHome=read-only` permits filesystem visibility, not a bypass of ordinary file permissions. A log in a private printer account directory may still require a suitable ACL/group arrangement. Missing access is shown as a collection limitation. Log rotation can also require maintaining permissions for newly created log files.

The service retries process-level failures; ordinary printer disconnections are handled inside the still-running recorder. To change configuration or replace the binary, stop this service, make the change, then restart it. Preserve the recording and any SQLite sidecars when making a coherent backup. Do not run a second writer against the same database.

Rollback consists of stopping/disabling this newly added service and restoring the previous executable/configuration. No rollback of printer firmware or printer settings is needed because this service does not change them.

This unit is a supplied deployment template, not a claim that it has been installed or tested on any particular Raspberry Pi. Physical printer acceptance and deployment performance remain separate checks described in the root README.
