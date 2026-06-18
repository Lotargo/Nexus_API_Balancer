# Rate Limiting

Per-key limits enforced in `ApiKey::try_use()` (`core.rs`).

## Limit Types

| Limit | Scope | Reset | Field |
|-------|-------|-------|-------|
| RPS | Requests per second | Every second | `rps_limit` |
| RPD | Requests per day | 00:00 UTC | `rpd_limit` |
| TPM | Tokens per minute | Every 60s | `tpm_limit` |
| TPD | Tokens per day | 00:00 UTC | `tpd_limit` |

## Cooldown

When `cooldown_on_limit: true` and a limit is hit:
- RPS → 1s cooldown
- RPD → 3600s cooldown
- TPM → 60s cooldown
- TPD → 3600s cooldown

During cooldown, the key returns `"Key cooling down"` error.

## Max Request Tokens

`max_request_tokens` limits the estimated input token count per request (checked before `try_use`). Returns 413 Payload Too Large.

## Key Expiration

Keys can have an `expires_at` field — expired keys return `"Key expired"` error.

## Concurrency

Each key can have multiple slots defined by `concurrency`. The pool (`KeyPool`) uses an `async-channel` bounded queue — keys are acquired/released, allowing concurrent requests up to `concurrency * number_of_secrets`.
