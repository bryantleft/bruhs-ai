"""Trunk post-build hook: gzip smaller release assets once, before publication."""

import gzip
import io
import os
from pathlib import Path
import stat
import tempfile


COMPRESSIBLE = {".html", ".js", ".mjs", ".css", ".json", ".svg", ".wasm"}


def precompress(directory: Path) -> dict[str, int]:
    if not directory.is_dir():
        raise NotADirectoryError(f"Trunk staging directory is missing: {directory}")
    saved = {"files": 0, "identity_bytes": 0, "gzip_bytes": 0}
    for source in sorted(directory.rglob("*")):
        if source.suffix not in COMPRESSIBLE or not source.is_file():
            continue
        metadata = source.stat()
        content = source.read_bytes()
        buffer = io.BytesIO()
        # Empty filename and zero timestamp keep output independent of build
        # path and time. Level 9 trades build CPU for the smallest measured gzip.
        with gzip.GzipFile(filename="", mode="wb", fileobj=buffer,
                           compresslevel=9, mtime=0) as stream:
            stream.write(content)
        encoded = buffer.getvalue()
        sidecar = source.with_name(source.name + ".gz")
        if len(encoded) >= len(content):
            # A rerun may replace an older, compressible source with a small one.
            sidecar.unlink(missing_ok=True)
            continue
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(dir=source.parent, delete=False) as output:
                temporary = Path(output.name)
                output.write(encoded)
            temporary.chmod(stat.S_IMODE(metadata.st_mode))
            # Last-Modified validators for both representations describe the
            # same asset, including reproducible builds with fixed source times.
            os.utime(temporary, ns=(metadata.st_atime_ns, metadata.st_mtime_ns))
            temporary.replace(sidecar)
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
        saved["files"] += 1
        saved["identity_bytes"] += len(content)
        saved["gzip_bytes"] += len(encoded)
    return saved


def main() -> None:
    profile = os.environ["TRUNK_PROFILE"]
    if profile == "debug":
        return
    if profile != "release":
        raise ValueError(f"Unsupported Trunk build profile: {profile!r}")
    result = precompress(Path(os.environ["TRUNK_STAGING_DIR"]))
    print(f"Precompressed {result['files']} release assets: "
          f"{result['identity_bytes']} identity bytes, {result['gzip_bytes']} gzip bytes")


if __name__ == "__main__":
    main()
