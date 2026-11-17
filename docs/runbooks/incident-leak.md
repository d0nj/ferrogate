# Incident: Leaked Credential

## Leaked virtual key (`sk-fg-*`)

1. Revoke in admin API: delete the key or set its budget to zero.
2. Confirm revocation: a request with the old key must return 401.
3. Issue a replacement key to the owner.
4. Review request logs for abuse window (usage, spend, upstream calls).

## Leaked session token

1. Force logout for the affected user (admin API session revoke) and have them re-authenticate.
2. If theft vector is unknown, rotate any shared secrets on the path (reverse proxy auth, TLS certs).
3. Check admin audit trail for actions taken during the window.

Treat session cookies as bearer tokens: `Secure`, `HttpOnly`, `SameSite=Lax` limit exposure but do not survive XSS or log leakage.
