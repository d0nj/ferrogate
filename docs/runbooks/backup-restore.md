# Backup / Restore (SQLite)

## Backup

```bash
sqlite3 /data/ferrogate.db ".backup '/backup/ferrogate-$(date +%F).db'"
```

Copy the resulting file off-host. Copying the live `.db` file directly is unsafe while WAL (`-wal`, `-shm`) files exist; `.backup` produces a consistent snapshot. Verify restores periodically on a scratch host.

## Restore (rollback)

1. Stop the gateway.
2. Replace `/data/ferrogate.db` with the backup file.
3. Remove stale `/data/ferrogate.db-wal` and `/data/ferrogate.db-shm` if present.
4. Start the gateway and check `/health`.

Rollback is restore: there are no down-migration steps beyond restoring the pre-upgrade file. Migration `000013` backfills sealed credentials; a backup taken before it lacks that data, so re-upgrade re-runs the backfill on start.
