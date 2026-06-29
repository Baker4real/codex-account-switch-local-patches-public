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

Main risk is accidental token exposure through Git, screenshots, shared terminals, browser profile directories used during OAuth login, insecure backups, or loose Windows ACLs.
