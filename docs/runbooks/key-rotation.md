# Key Rotation

## Provider upstream keys

Rotate without downtime via pool overlap:

1. Add the new credential as a second provider entry pointing at the same upstream, or update the inactive entry first.
2. Confirm traffic succeeds through the new credential (check logs / spend).
3. Remove or disable the old entry.

Never edit the active entry in place if you cannot tolerate a failed request during propagation.

## Virtual keys

Revoke immediately: delete or zero the budget on the compromised key in the admin API, then issue a replacement. There is no grace period; revocation is effective on next request (≤5s cache TTL).

## FERROGATE_ENCRYPTION_KEY

This key seals upstream provider credentials. Changing it makes already-stored credentials unreadable — there is no re-decrypt path. To rotate it, re-enter every provider credential after the change.
