# Security

## Sensitive files

The following files may contain active authentication tokens:

- `~/.codex/auth.json`
- `~/.codex/account_backup/<profile>/auth.json`

Treat them as secrets.

## Recommended protections

1. Keep backup directory permission restricted:
   - `chmod 700 ~/.codex/account_backup`
2. Restrict each `auth.json` file:
   - `chmod 600 ~/.codex/account_backup/*/auth.json`
3. On Windows, restrict access to `%CODEX_HOME%\account_backup` and `%CODEX_HOME%\bin` with NTFS permissions:
   - `icacls %CODEX_HOME%\account_backup /inheritance:r /grant:r %USERNAME%:F`
4. Never push token files to Git repositories.
5. Avoid syncing backup folders to public cloud storage.

## Threat model summary

This project stores and switches tokens locally, but login and quota refresh do contact official OpenAI / ChatGPT endpoints through the Codex CLI, ChatGPT Web API, or `codex app-server`. It should not send tokens to third-party services by design.

Main risks are accidental token exposure through Git, screenshots, shared terminals, insecure backups, or loose Windows ACLs.

The local Windows Edge browser-login patch intentionally reuses the user's normal Edge state only for profiles whose `profile.json` explicitly sets `login_browser` to `edge`, rather than creating dedicated browser profiles. That removes the old browser-profile cache surface, but it means Edge's currently signed-in ChatGPT account is the identity that OAuth will return. The app must keep the pre-write identity check so a known profile is not silently overwritten by a different browser account.

Profile switching has a similar identity boundary: `.current_profile` and `.active_profile` are local hints, not proof of which ChatGPT account is currently in root `auth.json`. Before writing root account state back to a profile, the app must compare the live `auth.json` identity with known profile identities and refuse unmanaged drift instead of overwriting a stale slot.
