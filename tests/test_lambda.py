import hashlib
import importlib.util
import io
import os
from pathlib import Path
import unittest
from unittest.mock import patch

os.environ.setdefault("AWS_ACCESS_KEY_ID", "test")
os.environ.setdefault("AWS_SECRET_ACCESS_KEY", "test")
os.environ.setdefault("AWS_DEFAULT_REGION", "us-east-2")
os.environ["CONSTRUCT_BUCKET"] = "test-bucket"
ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("lambda_handler", ROOT / "lambda_handler.py")
handler = importlib.util.module_from_spec(spec)
spec.loader.exec_module(handler)

class LambdaTests(unittest.TestCase):
    def event(self, data):
        return {"action": "validate", "bucket": "test-bucket",
            "key": "incoming/12345678-1234-1234-1234-123456789012.usda",
            "sha256": hashlib.sha256(data).hexdigest(), "size_bytes": len(data)}

    def invoke(self, data, event=None):
        with patch.object(handler.S3, "get_object", return_value={
                "Body": io.BytesIO(data), "ContentLength": len(data)}):
            return handler.handler(event or self.event(data), None)

    def test_health_and_valid_construct(self):
        self.assertEqual(handler.handler({"action":"health"}, None)["status"], "ready")
        data = (ROOT / "samples/crate.usda").read_bytes()
        result = self.invoke(data)
        self.assertEqual(result["status"], "valid")
        self.assertEqual(result["validation"]["default_prim"], "/Crate")
        self.assertEqual(result["sha256"], hashlib.sha256(data).hexdigest())

    def test_invalid_construct(self):
        self.assertEqual(self.invoke(b"not USD")["status"], "invalid")

    def test_wrong_hash_size_bucket_and_key(self):
        data = (ROOT / "samples/crate.usda").read_bytes()
        for change in [{"sha256":"0" * 64}, {"size_bytes":1},
                {"bucket":"another-bucket"}, {"key":"incoming/../asset.usda"}]:
            event = self.event(data) | change
            with self.assertRaises(ValueError):
                self.invoke(data, event)

    def test_timeout_rejects_construct(self):
        data = b"#usda 1.0"
        with patch.object(handler.subprocess, "run", side_effect=handler.subprocess.TimeoutExpired("validator", 20)):
            self.assertEqual(self.invoke(data)["status"], "invalid")

if __name__ == "__main__":
    unittest.main()
