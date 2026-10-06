#!/usr/bin/env python3
"""Validate self-contained USD with official OpenUSD; never execute asset scripts."""
import json
import math
from pathlib import Path, PurePosixPath
import stat
import struct
import sys
import tempfile
import zipfile

from pxr import Sdf, Usd, UsdGeom, UsdUtils

MAX_FILES = 4096
MAX_EXPANDED = 512 * 1024 * 1024


def safe_path(value):
    p = PurePosixPath(value)
    if not value or '\\' in value or ':' in value or p.is_absolute():
        raise ValueError('asset paths must be relative package paths')
    if any(part in ('', '.', '..') for part in value.split('/')):
        raise ValueError('asset paths must not contain empty or traversal components')
    return p


def validate_layers(root_file, package_root, packaged):
    # Inspect every layer before composition, including unused layers/variants.
    paths = list(package_root.rglob('*')) if packaged else [root_file]
    for path in paths:
        if not path.is_file() or path.suffix.lower() not in ('.usd', '.usda', '.usdc'):
            continue
        layer = Sdf.Layer.FindOrOpen(str(path))
        if layer is None:
            raise ValueError('could not parse a USD layer')
        dependencies = []
        UsdUtils.ModifyAssetPaths(layer, lambda s: dependencies.append(s) or s)
        resolved_paths = {}
        for dependency in dependencies:
            if not packaged:
                raise ValueError('external assets require a self-contained USDZ upload')
            safe_path(dependency)
            resolved = (path.parent / dependency).resolve()
            if not resolved.is_file():
                # The USDZ resolver also searches from the package root.
                resolved = (package_root / dependency).resolve()
            if not resolved.is_relative_to(package_root.resolve()) or not resolved.is_file():
                raise ValueError('missing or external package dependency')
            resolved_paths[dependency] = str(resolved)
        if packaged:
            # Mirror package resolution while composing the temporary extracted stage.
            UsdUtils.ModifyAssetPaths(layer, lambda s: resolved_paths.get(s, s))
    stage = Usd.Stage.Open(str(root_file), Usd.Stage.LoadAll)
    if stage is None or stage.GetCompositionErrors():
        raise ValueError('invalid USD composition')
    if not stage.GetDefaultPrim():
        raise ValueError('construct requires a defaultPrim')
    prims = list(stage.Traverse())
    if not prims:
        raise ValueError('construct has no active objects')
    for prim in prims:
        if prim.IsA(UsdGeom.Mesh):
            mesh = UsdGeom.Mesh(prim)
            samples = {Usd.TimeCode.Default()}
            samples.update(Usd.TimeCode(t) for attr in
                           (mesh.GetPointsAttr(), mesh.GetFaceVertexCountsAttr(), mesh.GetFaceVertexIndicesAttr())
                           for t in attr.GetTimeSamples())
            for sample in samples:
                points = mesh.GetPointsAttr().Get(sample) or []
                counts = mesh.GetFaceVertexCountsAttr().Get(sample) or []
                indices = mesh.GetFaceVertexIndicesAttr().Get(sample) or []
                valid, _ = UsdGeom.Mesh.ValidateTopology(indices, counts, len(points))
                if not valid or any(not math.isfinite(v) for p in points for v in p):
                    raise ValueError('invalid mesh topology or non-finite point')
    return {'prim_count': len(prims), 'default_prim': stage.GetDefaultPrim().GetPath().pathString}


def validate(path):
    path = Path(path).resolve()
    if path.suffix.lower() != '.usdz':
        result = validate_layers(path, path.parent, False)
        layer = Sdf.Layer.FindOrOpen(str(path))
        fmt = layer.GetFileFormat().formatId
        if fmt == 'usd':
            with path.open('rb') as source:
                magic = source.read(8)
            fmt = 'usdc' if magic == b'PXR-USDC' else 'usda' if magic.startswith(b'#usda') else 'unknown'
        if fmt not in ('usda', 'usdc'):
            raise ValueError('unsupported USD encoding')
        return dict(result, format=fmt)
    with zipfile.ZipFile(path) as archive, tempfile.TemporaryDirectory() as tmp:
        entries = archive.infolist()
        if not entries or len(entries) > MAX_FILES:
            raise ValueError('invalid USDZ entry count')
        if sum(e.file_size for e in entries) > MAX_EXPANDED:
            raise ValueError('USDZ exceeds expanded size limit')
        seen = set()
        with path.open('rb') as raw:
            for entry in entries:
                safe_path(entry.filename)
                key = entry.filename.casefold()
                if key in seen or entry.is_dir():
                    raise ValueError('duplicate or directory USDZ entry')
                seen.add(key)
                if entry.compress_type != zipfile.ZIP_STORED or entry.flag_bits & 1:
                    raise ValueError('USDZ entries must be uncompressed and unencrypted')
                if stat.S_ISLNK(entry.external_attr >> 16):
                    raise ValueError('USDZ symlinks are forbidden')
                if Path(entry.filename).suffix.lower() == '.usdz':
                    raise ValueError('nested USDZ packages are unsupported')
                raw.seek(entry.header_offset)
                header = raw.read(30)
                if len(header) != 30 or header[:4] != b'PK\x03\x04':
                    raise ValueError('invalid USDZ ZIP header')
                name_length, extra_length = struct.unpack_from('<HH', header, 26)
                if (entry.header_offset + 30 + name_length + extra_length) % 64:
                    raise ValueError('USDZ data must be aligned to 64 bytes')
        if Path(entries[0].filename).suffix.lower() not in ('.usd', '.usda', '.usdc'):
            raise ValueError('first USDZ entry must be its root USD layer')
        archive.extractall(tmp)
        result = validate_layers(Path(tmp) / entries[0].filename, Path(tmp), True)
        # Also verify the actual package can be opened by OpenUSD.
        packaged_stage = Usd.Stage.Open(str(path), Usd.Stage.LoadAll)
        if packaged_stage is None or packaged_stage.GetCompositionErrors():
            raise ValueError('USDZ cannot be composed')
        _, _, unresolved = UsdUtils.ComputeAllDependencies(str(path))
        if unresolved:
            raise ValueError('USDZ contains unresolved dependencies')
        return dict(result, format='usdz')


if __name__ == '__main__':
    try:
        print(json.dumps(validate(sys.argv[1])))
    except Exception as error:
        print(json.dumps({'error': str(error)[:1000]}))
        sys.exit(1)
