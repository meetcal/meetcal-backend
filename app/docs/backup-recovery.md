# Backup and Recovery

Nightly backups are created by `app/scripts/backup_db.sh`.

The script:

- runs `pg_dump` inside the `meetcal` Docker Postgres container
- writes a compressed dump to `app/backups/`
- uploads the dump to Cloudflare R2 under `s3://meetcal-backups/postgres/`
- removes local and R2 backups older than `KEEP_DAYS`
- optionally pings Uptime Kuma when `UPTIME_KUMA_PUSH_URL` is set

## Cron

The installed user crontab should include:

```cron
0 3 * * * /home/maddisen/dev/meetcal-llc/meetcal-backend/app/scripts/backup_db.sh >> /home/maddisen/dev/meetcal-llc/meetcal-backend/app/backups/meetcal-backup.log 2>&1
```

Check it with:

```bash
crontab -l
```

Run a backup manually with:

```bash
cd /home/maddisen/dev/meetcal-llc/meetcal-backend/app
scripts/backup_db.sh
```

## Uptime Kuma

Create a Push monitor in Uptime Kuma and copy its Push URL into the repo root `.env`:

```bash
UPTIME_KUMA_PUSH_URL="http://your-kuma-host:3001/api/push/your-monitor-token"
```

If Kuma gives you a longer generated URL with `?status=up&msg=OK&ping=`, either form works. The backup script strips the query string and sends its own success or failure status.

## Restore Test

Download a backup from R2:

```bash
set -a
source /home/maddisen/dev/meetcal-llc/meetcal-backend/.env
set +a

docker run --rm \
  -e AWS_ACCESS_KEY_ID \
  -e AWS_SECRET_ACCESS_KEY \
  -e AWS_DEFAULT_REGION=auto \
  -v /tmp:/tmp \
  amazon/aws-cli \
  s3 cp "s3://${R2_BUCKET}/postgres/meetcal_YYYYMMDD_HHMMSS.sql.gz" \
  /tmp/meetcal-restore.sql.gz \
  --endpoint-url "${R2_ENDPOINT_URL}"
```

Restore into a temporary database:

```bash
docker rm -f meetcal-restore-test >/dev/null 2>&1 || true

docker run -d \
  --name meetcal-restore-test \
  -e POSTGRES_USER=postgres \
  -e POSTGRES_PASSWORD="${POSTGRES_PASSWORD}" \
  -e POSTGRES_DB=meetcal_restore \
  postgres:16

until docker exec meetcal-restore-test pg_isready -U postgres -d meetcal_restore; do
  sleep 1
done

docker exec meetcal-restore-test \
  psql -v ON_ERROR_STOP=1 -U postgres -d meetcal_restore \
  -c "create role meetcal_api;"

gzip -dc /tmp/meetcal-restore.sql.gz |
  docker exec -i meetcal-restore-test \
    psql -v ON_ERROR_STOP=1 -U postgres -d meetcal_restore
```

Sanity-check the restored database:

```bash
docker exec meetcal-restore-test \
  psql -U postgres -d meetcal_restore \
  -c "select count(*) from information_schema.tables where table_schema = 'public' and table_type = 'BASE TABLE';"

docker exec meetcal-restore-test \
  psql -U postgres -d meetcal_restore \
  -c "select count(*) from meets;"
```

Clean up the temporary restore container:

```bash
docker rm -f meetcal-restore-test
```

## Production Checks

The production Postgres container should use a named volume for `/var/lib/postgresql/data`.

Check with:

```bash
docker container inspect meetcal \
  --format '{{range .Mounts}}{{.Destination}}={{.Name}}{{end}}'
```

Postgres should not be publicly exposed. If it only needs local access, bind it to `127.0.0.1:5432` instead of `0.0.0.0:5432`, or block external access with the host firewall.
