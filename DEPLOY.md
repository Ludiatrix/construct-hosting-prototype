# Deploy construct-portfolio-project

Region: **us-east-2 (Ohio)**. Run local commands in **Fish**. Run server commands in **EC2 Session Manager's shell**. Complete HTTPS/DNS and the existing infrastructure setup first. Deploy the real validator before the API. Do not enable uploads against the placeholder image.

## 1. Check roles and credentials

In IAM, compare the EC2 role's custom application policy with `deploy/ec2-runtime-policy.json`. Its S3 resources must use the actual bucket name ending in `-an`. Keep `AmazonSSMManagedInstanceCore` attached.

Compare the validator's S3 policy with `deploy/validator-storage-policy.json`, and keep `AWSLambdaBasicExecutionRole` attached. Both roles should use their original EC2/Lambda service trust relationships. Your local administrative identity handles image pushes and Lambda updates; don't grant those permissions to the EC2 runtime role.

Confirm the DB secret contains the application's matching PostgreSQL password, `construct_app`, `construct_registry`, and the endpoint in compose.yaml. Confirm `registry` is owned by `construct_app`. Confirm the API secret has two distinct strong keys. Save their values in your password manager for smoke testing, not in the project files.

On your PC, enter the extracted project directory containing Cargo.toml, Dockerfile and compose.yaml. Then:

```fish
set -gx AWS_PROFILE construct-portfolio-project
set -gx AWS_REGION us-east-2
set PROJECT_REGISTRY 111859959018.dkr.ecr.us-east-2.amazonaws.com
aws sts get-caller-identity
aws ecr get-login-password --region "$AWS_REGION" | sudo docker login --username AWS --password-stdin "$PROJECT_REGISTRY"
```

Use the same AWS profile you used successfully earlier if its name differs. Confirm the account is 111859959018. All local Docker commands use sudo consistently with your current installation.

## 2. Build and push the validator

```fish
sudo docker buildx build --platform linux/amd64 --provenance=false --load -f Dockerfile.validator -t "$PROJECT_REGISTRY/construct-portfolio-project-validator:validator-v1" .
sudo docker push "$PROJECT_REGISTRY/construct-portfolio-project-validator:validator-v1"
```

Immutable tags cannot be reused with a different image. For future builds, choose new revision tags and update the commands accordingly.

In Lambda → construct-portfolio-project-validator → Configuration:

- Memory: 2048 MB.
- Timeout: 60 seconds.
- Ephemeral storage: 1024 MB.
- Environment variables: add `CONSTRUCT_BUCKET` = `construct-portfolio-project-111859959018-us-east-2-an`.
- Keep VPC unset and architecture x86_64.

Save the configuration before updating the image. Under Code → Deploy new image, choose the validator repository's `validator-v1` image. Clear any image configuration override for the command, entrypoint or working directory left from the placeholder. The image defines `lambda_handler.handler`. Wait for the function to become Active and the update to finish. Merely pushing a new image does not update Lambda.

Create a Lambda test event:

```json
{"action":"health"}
```

Expected:

```json
{"status":"ready","protocol_version":1}
```

You can also rerun the previous probe event for `incoming/probe.txt` if that temporary object still exists. S3 lifecycle may already have removed it.

## 3. Build and push the API

The build compiles the AWS Rust SDK and can take several minutes on the first run.

```fish
sudo docker buildx build --platform linux/amd64 --provenance=false --load -t "$PROJECT_REGISTRY/construct-portfolio-project-api:api-v1" .
sudo docker push "$PROJECT_REGISTRY/construct-portfolio-project-api:api-v1"
```

The image includes a downloaded RDS CA bundle. Rebuild it when AWS requires a CA-bundle update. Retain Cargo.lock; the Docker build uses --locked.

## 4. Install the compose file on EC2

Open EC2 → your instance → Connect → Session Manager. The instance needs IMDSv2 with response hop limit 2 for Docker credential access. Do not copy local AWS credentials to EC2.

Run:

```sh
sudo mkdir -p /opt/construct-portfolio-project
cd /opt/construct-portfolio-project
sudo nano compose.yaml
```

Paste the full contents of the supplied `compose.yaml` and save. It contains identifiers, not passwords. If nano isn't installed, run `sudo apt install -y nano`.

Then:

```sh
export AWS_REGION=us-east-2
aws sts get-caller-identity
aws ecr get-login-password --region us-east-2 | sudo docker login --username AWS --password-stdin 111859959018.dkr.ecr.us-east-2.amazonaws.com
sudo docker compose pull
sudo docker compose up -d
sudo docker compose logs --tail=80 registry
curl -i http://127.0.0.1:8080/health
```

Expected health status: HTTP 200 and `{"status":"ok"}`. Startup retrieves secrets, connects with verified RDS TLS, applies migrations, and verifies the real validator contract. If startup fails, keep Caddy's placeholder and fix the logged dependency error. Common causes: incorrect S3 prefix policy, secret password mismatch, an unowned registry schema, missing Lambda bucket environment, placeholder/command override, and Docker metadata hop limit.

## 5. Connect Caddy to the API

Only after local health works, back up the placeholder and replace its configuration:

```sh
sudo cp /etc/caddy/Caddyfile /etc/caddy/Caddyfile.placeholder
sudo tee /etc/caddy/Caddyfile >/dev/null <<'EOF'
constructs.elvendynamics.com {
    reverse_proxy 127.0.0.1:8080
}
EOF
sudo caddy validate --config /etc/caddy/Caddyfile
sudo systemctl reload caddy
```

From your PC:

```fish
curl -i https://constructs.elvendynamics.com/health
```

Expected: HTTP 200. The root path `/` will return 404; the application has no website.

## 6. Run live acceptance checks

On your PC, read keys interactively so their values are not literal commands in shell history:

```fish
read --silent --prompt-str 'Upload API key: ' UPLOAD_KEY
read --silent --prompt-str 'Read API key: ' READ_KEY
set API_URL https://constructs.elvendynamics.com
```

Upload the sample. Save the metadata response:

```fish
curl --fail-with-body -D /tmp/construct-upload-headers.txt -o /tmp/construct-record.json -X POST "$API_URL/constructs?filename=crate.usda" -H "Authorization: Bearer $UPLOAD_KEY" -H 'Content-Type: application/octet-stream' --data-binary @samples/crate.usda
cat /tmp/construct-upload-headers.txt
cat /tmp/construct-record.json
set ADDRESS (python3 -c 'import json; print(json.load(open("/tmp/construct-record.json"))["address"])')
```

A new file returns 201. Repeating the same upload returns 200 and the same address. The first upload's filename is retained.

Download and compare:

```fish
curl --fail-with-body -o /tmp/downloaded-crate.usda "$API_URL/constructs/$ADDRESS" -H "Authorization: Bearer $READ_KEY"
cmp samples/crate.usda /tmp/downloaded-crate.usda
curl --fail-with-body "$API_URL/constructs/$ADDRESS/metadata" -H "Authorization: Bearer $READ_KEY"
curl --fail-with-body "$API_URL/constructs?limit=10&offset=0" -H "Authorization: Bearer $READ_KEY"
```

`cmp` should produce no output. Unauthenticated requests should return 401:

```fish
curl -i "$API_URL/constructs"
```

Invalid USD should return 422:

```fish
printf 'not USD' | curl -i -X POST "$API_URL/constructs?filename=bad.usda" -H "Authorization: Bearer $UPLOAD_KEY" -H 'Content-Type: application/octet-stream' --data-binary @-
```

In S3, confirm `constructs/<hash>.usda` exists, and no published object was created for bad.usda. In PostgreSQL, check the metadata as construct_app using verified TLS:

```sql
SELECT address, format, size_bytes FROM registry.constructs ORDER BY created_at DESC;
```

Restart the container on EC2 using `sudo docker compose restart`, then repeat the download and list checks. Persistent files and records live in S3/RDS, not on the EC2 root disk.

After checks, clear the keys from your Fish session:

```fish
set -e UPLOAD_KEY
set -e READ_KEY
```

## 7. Logs and rollback

Lambda uses its existing CloudWatch log group and error alarm. API logs are bounded local Docker logs, viewed with `sudo docker compose logs --tail=100 registry`; this release does not ship them to CloudWatch. Your EC2 status alarm monitors the instance, not REST availability.

If deployment fails, restore the Caddy placeholder:

```sh
sudo cp /etc/caddy/Caddyfile.placeholder /etc/caddy/Caddyfile
sudo systemctl reload caddy
```

Keep the validator-v1 and api-v1 image digests for rollback. For a future API revision, change the compose image tag, run `compose pull` and `compose up -d`. Ensure the new API and validator contracts are compatible before switching. Secrets are loaded at startup; force-recreate the API after rotating app credentials. Do not delete the RDS or S3 resources when updating containers.
