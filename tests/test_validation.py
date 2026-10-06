import importlib.util
from pathlib import Path
import tempfile
import unittest
from pxr import Sdf, UsdUtils

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('validator', ROOT / 'validate_usd.py')
validator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validator)


class ValidationTests(unittest.TestCase):
    def test_ascii_binary_generic_and_usdz(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = ROOT / 'fixtures/crate.usda'
            layer = Sdf.Layer.FindOrOpen(str(source))
            self.assertTrue(layer.Export(str(root / 'crate.usdc')))
            (root / 'crate.usd').write_bytes(source.read_bytes())
            self.assertTrue(UsdUtils.CreateNewUsdzPackage(Sdf.AssetPath(str(source)), str(root / 'crate.usdz')))
            for path, fmt in [(source, 'usda'), (root / 'crate.usdc', 'usdc'), (root / 'crate.usd', 'usda'), (root / 'crate.usdz', 'usdz')]:
                self.assertEqual(validator.validate(path)['format'], fmt)

    def test_missing_default_prim_and_invalid_topology(self):
        for text in [
            '#usda 1.0\ndef Xform "Root" {}\n',
            '#usda 1.0\n(defaultPrim="Root")\ndef Mesh "Root" {\npoint3f[] points=[(0,0,0)]\nint[] faceVertexCounts=[3]\nint[] faceVertexIndices=[0,1,2]\n}\n',
        ]:
            with tempfile.TemporaryDirectory() as tmp:
                path = Path(tmp) / 'bad.usda'
                path.write_text(text)
                with self.assertRaises(ValueError):
                    validator.validate(path)

    def test_external_assets_sublayers_and_variant_references(self):
        for text in [
            '#usda 1.0\n(defaultPrim="Root")\ndef Xform "Root" {\ncustom asset a=@/etc/passwd@\n}\n',
            '#usda 1.0\n(defaultPrim="Root"; subLayers=[@missing.usda@])\ndef Xform "Root" {}\n',
            '#usda 1.0\n(defaultPrim="Root")\ndef Xform "Root" (prepend references=@missing.usda@) {}\n',
            '#usda 1.0\n(defaultPrim="Root")\ndef Xform "Root" (prepend variantSets="choice") {\nvariantSet "choice" = {\n"hidden" (prepend references=@missing.usda@) {}\n}\n}\n',
        ]:
            with tempfile.TemporaryDirectory() as tmp:
                path = Path(tmp) / 'bad.usda'
                path.write_text(text)
                with self.assertRaises(ValueError):
                    validator.validate(path)

    def test_usdz_with_texture_and_referenced_layer(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'texture.png').write_bytes(b'asset bytes')
            (root / 'child.usda').write_text('#usda 1.0\n(defaultPrim="Child")\ndef Xform "Child" {\ncustom asset texture=@texture.png@\n}\n')
            (root / 'root.usda').write_text('#usda 1.0\n(defaultPrim="Root")\ndef Xform "Root" (prepend references=@child.usda@) {}\n')
            self.assertTrue(UsdUtils.CreateNewUsdzPackage(Sdf.AssetPath(str(root / 'root.usda')), str(root / 'bundle.usdz')))
            self.assertEqual(validator.validate(root / 'bundle.usdz')['default_prim'], '/Root')

    def test_invalid_zip_and_traversal(self):
        import zipfile
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'bad.usdz'
            with zipfile.ZipFile(path, 'w') as z:
                z.writestr('../root.usda', b'#usda 1.0')
            with self.assertRaises(ValueError):
                validator.validate(path)
        for path in ['/tmp/file.usda', '../file.usda', 'https://host/file.usda', 'x\\file.usda', 'a//b.usda']:
            with self.assertRaises(ValueError):
                validator.safe_path(path)


if __name__ == '__main__':
    unittest.main()
