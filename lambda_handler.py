"""S3-to-OpenUSD validation boundary. Never execute construct scripts."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

import boto3

MAX_UPLOAD = 128 * 1024 * 1024
KEY = re.compile(r"incoming/[0-9a-f-]{36}\.(usd|usda|usdc|usdz)\Z")
HASH = re.compile(r"[0-9a-f]{64}\Z")
S3 = boto3.client("s3")


def handler(event, context):
    if event.get("action") == "health":
        from pxr import Sdf, Usd, UsdGeom, UsdUtils
        if not os.environ.get("CONSTRUCT_BUCKET"):
            raise RuntimeError("set CONSTRUCT_BUCKET on the Lambda function")
        return {"status": "ready", "protocol_version": 1}
    bucket = os.environ["CONSTRUCT_BUCKET"]
    if event.get("bucket") != bucket:
        raise ValueError("unexpected storage bucket")
    if event.get("action") == "probe_s3" and event.get("key") == "incoming/probe.txt":
        S3.head_object(Bucket=bucket, Key="incoming/probe.txt")
        return {"status": "s3_access_verified", "validation_implemented": True}
    if event.get("action") != "validate":
        raise ValueError("unsupported validator action")
    key, expected_hash, expected_size = event.get("key", ""), event.get("sha256", ""), event.get("size_bytes")
    match = KEY.fullmatch(key)
    if not match or not HASH.fullmatch(expected_hash) or type(expected_size) is not int or not 1 <= expected_size <= MAX_UPLOAD:
        raise ValueError("invalid validation request")
    response = S3.get_object(Bucket=bucket, Key=key)
    body = response["Body"]
    try:
        if response["ContentLength"] != expected_size:
            raise ValueError("uploaded object size mismatch")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / ("asset." + match.group(1))
            sha, size = hashlib.sha256(), 0
            with path.open("wb") as target:
                while chunk := body.read(1024 * 1024):
                    size += len(chunk)
                    if size > MAX_UPLOAD:
                        raise ValueError("uploaded object exceeds size limit")
                    sha.update(chunk)
                    target.write(chunk)
            if size != expected_size or sha.hexdigest() != expected_hash:
                raise ValueError("uploaded object content mismatch")
            script = Path(__file__).with_name("validate_usd.py")
            try:
                output = subprocess.run([sys.executable, str(script), str(path)],
                    stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                    timeout=20, check=False, cwd=directory,
                    env=dict(os.environ, OMP_NUM_THREADS="1"))
            except subprocess.TimeoutExpired:
                return {"status": "invalid", "error": "USD validation timed out"}
            result = json.loads(output.stdout)
            if output.returncode:
                if not isinstance(result.get("error"), str):
                    raise RuntimeError("validator failed without an error response")
                return {"status": "invalid", "error": result["error"][:1000]}
            return {"status": "valid", "sha256": expected_hash, "size_bytes": size, "validation": result}
    finally:
        body.close()
