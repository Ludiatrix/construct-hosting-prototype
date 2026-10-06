# Construct Registry

A small Rust REST service for storing and retrieving USD constructs.

1. Upload a USD file or USDZ package.
2. Validate it with official OpenUSD.
3. Store the original bytes under their SHA-256 content address.
4. Record metadata in SQLite.
5. Retrieve the original file by its address from any game engine or HTTP client.

## Run locally

Install Rust and Python 3.12. From this directory:

```sh
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
export CONSTRUCT_PYTHON="$PWD/.venv/bin/python"
export CONSTRUCT_API_KEY="$(python3 -c 'import secrets; print(secrets.token_urlsafe(32))')"
cargo run --locked --release
```

The server defaults to `127.0.0.1:8080`. Keep your API key available to your client.
All construct endpoints require `Authorization: Bearer <key>`. `/health` is public.
This is a single-owner service with one shared key, not a user-account system.

Alternatively, copy `.env.example` to `.env`, set a strong key, then:

```sh
docker compose up --build -d
```

Docker stores the database and files in the `construct-data` volume. The example
publishes only on localhost. Put the service behind your HTTPS reverse proxy when
hosting it remotely. Docker configuration is supplied but was not built here.

## API

| Method | Endpoint | Result |
|---|---|---|
| POST | `/constructs?filename=crate.usdz` | Validate and store raw request bytes; return metadata |
| GET | `/constructs/{address}` | Download the original bytes |
| GET | `/constructs/{address}/metadata` | Read one database record |
| GET | `/constructs?limit=50&offset=0` | List records, newest first; maximum limit 200 |
| GET | `/health` | Liveness response |

Upload uses `Content-Type: application/octet-stream`, not multipart or JSON.

```sh
curl --fail-with-body \
  -H "Authorization: Bearer $CONSTRUCT_API_KEY" \
  -H 'Content-Type: application/octet-stream' \
  --data-binary @fixtures/crate.usda \
  'http://127.0.0.1:8080/constructs?filename=crate.usda'
```

The response contains `address`, `filename`, `format`, `size_bytes`, `created_at`,
`default_prim`, and `prim_count`. A new upload returns HTTP 201; identical bytes
return HTTP 200 and the existing record. The first filename is retained.

Copy the returned address, including its `sha256:` prefix:

```sh
ADDRESS='sha256:PASTE_THE_RETURNED_HASH_HERE'
curl --fail-with-body \
  -H "Authorization: Bearer $CONSTRUCT_API_KEY" \
  "http://127.0.0.1:8080/constructs/$ADDRESS" \
  --output downloaded.usda

curl --fail-with-body \
  -H "Authorization: Bearer $CONSTRUCT_API_KEY" \
  'http://127.0.0.1:8080/constructs?limit=50&offset=0'
```

Use the response's `format` for your engine's local file extension. Generic `.usd`
files are detected as `usda` or `usdc`. A downloaded USDZ remains a USDZ package.
The HTTP API returns files; your engine's existing USD importer handles import.
Custom attributes and scripts remain opaque stored data and are never executed.

## Validation

Accepts `.usd`, `.usda`, `.usdc`, and `.usdz`, up to 128 MiB per upload.
OpenUSD must parse and compose the construct successfully, with a valid
`defaultPrim` and at least one active prim. Mesh point values and topology are
checked at authored sample times. This is file/composition validation, not a
promise of engine compatibility or complete physics/material semantics.

Standalone USD files must have no asset dependencies. Use USDZ for textures,
referenced layers, and other asset files. USDZ validation checks paths, duplicate
entries, symlinks, compression/encryption, 64-byte alignment, dependencies,
composition, and a 512 MiB expanded-size limit. Nested USDZ packages and asset
paths containing `..` are intentionally unsupported.

Invalid USD returns HTTP 422. Invalid parameters return 400, oversized uploads
413, incorrect media types 415, missing records 404, and invalid credentials 401.
Two uploads may run at once; additional uploads return 503 and can be retried.
Uploads time out after 120 seconds; OpenUSD validation has a 20-second timeout.

Validation runs in a separate Python process using `usd-core`, because Rust does
not parse USD natively here. This is not a hardened public upload sandbox; use the
service for your authenticated authoring workflow.

## Storage and configuration

| Variable | Default | Purpose |
|---|---|---|
| `CONSTRUCT_API_KEY` | Required | Shared bearer key |
| `CONSTRUCT_BIND` | `127.0.0.1:8080` | Listening address |
| `CONSTRUCT_DATA_DIR` | `data` | Persistent database and object directory |
| `CONSTRUCT_PYTHON` | `python3` | Python executable with OpenUSD installed |
| `CONSTRUCT_VALIDATOR` | `validate_usd.py` | Validator script path |

`data/registry.sqlite3` contains the `constructs` table. `data/objects/` holds
immutable files named by hash and format. `data/incoming/` is temporary staging.
Back up the database and object directory together while the service is stopped.

Content addresses hash original file bytes, not normalized scene semantics.
Equivalent scenes with different encoding or bytes get different addresses.
Files are published before SQLite records; an interrupted commit can leave an
unlisted object that a later identical upload can reuse. There is no delete API,
cloud bucket dependency, usage collection, or cleanup daemon. Storage uses the
server's persistent disk or a mounted storage volume. Run one instance per data
directory. Do not modify its object files manually.

## Verify

With OpenUSD installed in `CONSTRUCT_PYTHON`:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
"$CONSTRUCT_PYTHON" -m unittest discover -s tests -p 'test_*.py' -v
```

Verified here: formatting and strict Clippy checks, all four Rust API tests, all five
Python validation tests, and live HTTP upload/download checks for ASCII USD,
binary USD, and USDZ with independently checked SHA-256 addresses.

Tests cover upload/download byte preservation, content deduplication, database
reopening, concurrent duplicate uploads, authentication, size/type/path handling,
invalid USD, missing dependencies, topology, and ASCII/binary/USDZ validation.

## Files

- `src/lib.rs`: REST endpoints, SQLite records, hashing, storage, validation calls.
- `src/main.rs`: configuration and server startup.
- `validate_usd.py`: OpenUSD validation.
- `fixtures/crate.usda`: a self-contained sample construct.
- `tests/`: API and USD validation tests.
- `Dockerfile`, `compose.yaml`: container deployment with persistent storage.
