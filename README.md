# Construct registry

A REST service for self-contained USD constructs. The Rust API runs on EC2, stores files in private S3, invokes an OpenUSD Lambda validator, and records metadata in RDS PostgreSQL. See [DEPLOY.md](DEPLOY.md) for this project's deployment steps.

## API

| Method | Path | Authentication | Response |
|---|---|---|---|
| GET | `/health` | None | Process liveness |
| POST | `/constructs?filename=crate.usda` | Upload bearer key | 201 for new content, 200 for duplicate |
| GET | `/constructs?limit=50&offset=0` | Read bearer key | Paginated metadata |
| GET | `/constructs/{address}/metadata` | Read bearer key | Construct record |
| GET | `/constructs/{address}` | Read bearer key | Original bytes streamed from S3 |

Uploads use `Content-Type: application/octet-stream`. Addresses are `sha256:<64 lowercase hexadecimal digits>`, computed from the original file bytes. Different encodings of the same scene have different addresses. Downloads return those original bytes; engine import is the client's responsibility. The upload key does not grant read access, and the read key does not grant upload access. There are no usage tracking or engine integration features.

## Features and lifecycle

- `config`: non-secret environment settings and Secrets Manager JSON loading.
- `app`: composition root, credentials, database pool, AWS clients.
- `features/api`: routes, bearer authorization, request handling and HTTP errors.
- `features/constructs`: receive → stage → validate → publish → record → cleanup.
- `features/storage`: bounded local temporary receive, S3 staging, publication and streamed reads.
- `features/validation`: synchronous Lambda contract and hash/size verification.
- `features/database`: PostgreSQL migrations, records, collection and deduplication.
- `server`: startup dependency checks and graceful shutdown.
- `lambda_handler.py`: bounded S3 download, hash verification and isolated validator subprocess.
- `validate_usd.py`: OpenUSD parsing, defaultPrim, composition, mesh and package checks.

## Limits and behavior

Uploads support `.usd`, `.usda`, `.usdc`, `.usdz`, up to 128 MiB; at most two uploads run per API instance with a 120-second request deadline. Validation runs in a subprocess with a 20-second deadline and never executes Lua. Raw layers must have no external dependencies; USDZ packages must be self-contained. USDZ limits are 4096 entries and 512 MiB expanded data. This validates USD structure, not whether a particular engine implements every schema or custom behavior.

Database transaction advisory locks serialize publication for the same hash across instances. Metadata is committed only after S3 publication succeeds. S3 and PostgreSQL do not share a transaction: a process crash or database failure after publication can leave a validated object without a record. Reuploading the same bytes repairs that case; no automatic permanent-object deletion is performed. Temporary S3 objects are deleted on ordinary completion; the bucket lifecycle covers cancellation, crashes and noncurrent versions. Ensure the incoming cleanup rules exist.

The service loads secrets at startup; restart after application password/key rotation. The RDS master secret is never used. Database connections require verified TLS using the RDS CA bundle. The API image contains no Python or OpenUSD. Runtime credentials come from the EC2 instance role. `/health` is a liveness endpoint, not ongoing AWS dependency monitoring.

## Configuration

`compose.yaml` contains this project's bucket, Lambda name, database endpoint, and application-secret ARNs. Secret values remain in Secrets Manager. The database secret needs `host`, `port`, `dbname`, `username`, `password`; the authentication secret needs distinct `upload_api_key` and `read_api_key` strings, each at least 32 characters. The service requires `construct_app`, database `construct_registry`, and schema `registry` owned by that user. Migrations run on startup.

## Verification

```sh
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
python3 -m unittest discover -s tests -p 'test_*.py' -v
```

Python tests require `usd-core==26.5` and boto3. Rust units check authentication separation, upload guards, streamed size enforcement, hashes, cleanup and Lambda responses. Python tests exercise actual OpenUSD formats/packages and the Lambda boundary with mocked S3. See deployment smoke tests for live AWS verification. Docker image builds and deployment require your local Docker and AWS access; they are not performed by these unit tests.
