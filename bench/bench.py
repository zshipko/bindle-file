#!/usr/bin/env -S uv run
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""
Benchmark comparing bindle vs tar/tar.gz/zip for archive operations.

Measures:
- Archive creation time
- Archive size
- Extraction/read time
"""

import subprocess
import tempfile
import time
from pathlib import Path


def format_size(bytes: int) -> str:
    """Format bytes as human readable string."""
    for unit in ["B", "KB", "MB", "GB"]:
        if bytes < 1024:
            return f"{bytes:.1f} {unit}"
        bytes //= 1024
    return f"{bytes:.1f} TB"


def format_time(seconds: float) -> str:
    """Format seconds as human readable string."""
    if seconds < 0.001:
        return f"{seconds * 1_000_000:.1f} µs"
    elif seconds < 1:
        return f"{seconds * 1000:.1f} ms"
    else:
        return f"{seconds:.3f} s"


def create_test_data(base_dir: Path) -> None:
    """Create a variety of test files."""
    base_dir.mkdir(parents=True, exist_ok=True)

    # Small text files (highly compressible)
    for i in range(100):
        (base_dir / f"text_{i}.txt").write_text(
            f"This is test file {i}\n" * 100
        )

    # Medium files with repetitive data
    for i in range(10):
        (base_dir / f"medium_{i}.dat").write_bytes(
            bytes([i % 256] * 100_000)
        )

    # Large file with repetitive data
    (base_dir / "large.dat").write_bytes(b"X" * 10_000_000)

    # Binary-like data (less compressible)
    import random
    random.seed(42)
    (base_dir / "random.bin").write_bytes(
        bytes(random.randint(0, 255) for _ in range(1_000_000))
    )


def benchmark_bindle_uncompressed(src_dir: Path, archive_path: Path) -> tuple[float, int, float]:
    """Benchmark bindle without compression."""
    # Pack
    start = time.perf_counter()
    subprocess.run(
        ["cargo", "run", "--release", "--", "pack", str(archive_path), str(src_dir)],
        cwd=Path(__file__).parent.parent,
        capture_output=True,
        check=True,
    )
    pack_time = time.perf_counter() - start

    size = archive_path.stat().st_size

    # Unpack
    extract_dir = archive_path.parent / "extract_bindle_none"
    extract_dir.mkdir(exist_ok=True)
    start = time.perf_counter()
    subprocess.run(
        ["cargo", "run", "--release", "--", "unpack", str(archive_path), str(extract_dir)],
        cwd=Path(__file__).parent.parent,
        capture_output=True,
        check=True,
    )
    unpack_time = time.perf_counter() - start

    return pack_time, size, unpack_time


def benchmark_bindle_compressed(src_dir: Path, archive_path: Path) -> tuple[float, int, float]:
    """Benchmark bindle with zstd compression."""
    # Pack
    start = time.perf_counter()
    subprocess.run(
        ["cargo", "run", "--release", "--", "pack", str(archive_path), str(src_dir), "--compress"],
        cwd=Path(__file__).parent.parent,
        capture_output=True,
        check=True,
    )
    pack_time = time.perf_counter() - start

    size = archive_path.stat().st_size

    # Unpack
    extract_dir = archive_path.parent / "extract_bindle_zstd"
    extract_dir.mkdir(exist_ok=True)
    start = time.perf_counter()
    subprocess.run(
        ["cargo", "run", "--release", "--", "unpack", str(archive_path), str(extract_dir)],
        cwd=Path(__file__).parent.parent,
        capture_output=True,
        check=True,
    )
    unpack_time = time.perf_counter() - start

    return pack_time, size, unpack_time


def benchmark_tar(src_dir: Path, archive_path: Path) -> tuple[float, int, float]:
    """Benchmark tar (uncompressed) using CLI."""
    # Create
    start = time.perf_counter()
    subprocess.run(
        ["tar", "-cf", str(archive_path), "-C", str(src_dir), "."],
        capture_output=True,
        check=True,
    )
    pack_time = time.perf_counter() - start

    size = archive_path.stat().st_size

    # Extract
    extract_dir = archive_path.parent / "extract_tar"
    extract_dir.mkdir(exist_ok=True)
    start = time.perf_counter()
    subprocess.run(
        ["tar", "-xf", str(archive_path), "-C", str(extract_dir)],
        capture_output=True,
        check=True,
    )
    unpack_time = time.perf_counter() - start

    return pack_time, size, unpack_time


def benchmark_tar_gz(src_dir: Path, archive_path: Path) -> tuple[float, int, float]:
    """Benchmark tar.gz using CLI."""
    # Create
    start = time.perf_counter()
    subprocess.run(
        ["tar", "-czf", str(archive_path), "-C", str(src_dir), "."],
        capture_output=True,
        check=True,
    )
    pack_time = time.perf_counter() - start

    size = archive_path.stat().st_size

    # Extract
    extract_dir = archive_path.parent / "extract_tar_gz"
    extract_dir.mkdir(exist_ok=True)
    start = time.perf_counter()
    subprocess.run(
        ["tar", "-xzf", str(archive_path), "-C", str(extract_dir)],
        capture_output=True,
        check=True,
    )
    unpack_time = time.perf_counter() - start

    return pack_time, size, unpack_time


def benchmark_zip(src_dir: Path, archive_path: Path) -> tuple[float, int, float]:
    """Benchmark zip using CLI."""
    # Create - zip requires being in the directory or using find
    start = time.perf_counter()
    subprocess.run(
        ["sh", "-c", f"cd {src_dir} && zip -r -q {archive_path} ."],
        capture_output=True,
        check=True,
    )
    pack_time = time.perf_counter() - start

    size = archive_path.stat().st_size

    # Extract
    extract_dir = archive_path.parent / "extract_zip"
    extract_dir.mkdir(exist_ok=True)
    start = time.perf_counter()
    subprocess.run(
        ["unzip", "-q", str(archive_path), "-d", str(extract_dir)],
        capture_output=True,
        check=True,
    )
    unpack_time = time.perf_counter() - start

    return pack_time, size, unpack_time


def main():
    print("Building bindle in release mode...")
    subprocess.run(
        ["cargo", "build", "--release", "--features", "cli"],
        cwd=Path(__file__).parent.parent,
        capture_output=True,
        check=True,
    )

    with tempfile.TemporaryDirectory() as tmpdir:
        tmpdir = Path(tmpdir)

        # Create test data
        print("Creating test dataset...")
        test_data = tmpdir / "test_data"
        create_test_data(test_data)

        # Calculate total size
        total_size = sum(f.stat().st_size for f in test_data.rglob("*") if f.is_file())
        file_count = len(list(test_data.rglob("*")))

        print(f"Test dataset: {file_count} files, {format_size(total_size)}\n")

        benchmarks = [
            ("bindle (uncompressed)", lambda: benchmark_bindle_uncompressed(
                test_data, tmpdir / "test.bndl"
            )),
            ("bindle (zstd)", lambda: benchmark_bindle_compressed(
                test_data, tmpdir / "test_zstd.bndl"
            )),
            ("tar", lambda: benchmark_tar(
                test_data, tmpdir / "test.tar"
            )),
            ("tar.gz", lambda: benchmark_tar_gz(
                test_data, tmpdir / "test.tar.gz"
            )),
            ("zip", lambda: benchmark_zip(
                test_data, tmpdir / "test.zip"
            )),
        ]

        results = []
        for name, bench_fn in benchmarks:
            print(f"Benchmarking {name}...", flush=True)
            try:
                pack_time, size, unpack_time = bench_fn()
                results.append((name, pack_time, size, unpack_time))
            except Exception as e:
                print(f"  ERROR: {e}")
                results.append((name, 0, 0, 0))

        # Print results
        print("\n" + "=" * 90)
        print(f"{'Format':<22} {'Pack Time':<15} {'Size':<15} {'Unpack Time':<15} {'Ratio':>10}")
        print("=" * 90)

        for name, pack_time, size, unpack_time in results:
            if size > 0:
                ratio = (size / total_size) * 100
                print(
                    f"{name:<22} {format_time(pack_time):<15} "
                    f"{format_size(size):<15} {format_time(unpack_time):<15} "
                    f"{ratio:>9.1f}%"
                )
            else:
                print(f"{name:<22} {'FAILED'}")

        print("=" * 90)


if __name__ == "__main__":
    main()
