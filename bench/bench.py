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


def verify_extraction(src_dir: Path, extract_dir: Path) -> None:
    """Verify extracted files match source files."""
    src_files = {f.relative_to(src_dir): f for f in src_dir.rglob("*") if f.is_file()}
    extract_files = {f.relative_to(extract_dir): f for f in extract_dir.rglob("*") if f.is_file()}

    # Check file count
    if len(src_files) != len(extract_files):
        raise ValueError(f"File count mismatch: {len(src_files)} source, {len(extract_files)} extracted")

    # Check each file exists and has correct size
    for rel_path, src_file in src_files.items():
        if rel_path not in extract_files:
            raise ValueError(f"Missing file in extraction: {rel_path}")

        extract_file = extract_files[rel_path]
        src_size = src_file.stat().st_size
        extract_size = extract_file.stat().st_size

        if src_size != extract_size:
            raise ValueError(f"Size mismatch for {rel_path}: {src_size} vs {extract_size}")

        # Verify content matches
        if src_file.read_bytes() != extract_file.read_bytes():
            raise ValueError(f"Content mismatch for {rel_path}")


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


def benchmark_bindle_uncompressed(bindle_bin: Path, src_dir: Path, archive_path: Path) -> tuple[float, int, float]:
    """Benchmark bindle without compression."""
    # Pack
    start = time.perf_counter()
    subprocess.run(
        [str(bindle_bin), "pack", str(archive_path), str(src_dir)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=True,
    )
    pack_time = time.perf_counter() - start

    size = archive_path.stat().st_size

    # Unpack
    extract_dir = archive_path.parent / f"extract_{archive_path.stem}"
    extract_dir.mkdir(exist_ok=True)
    start = time.perf_counter()
    subprocess.run(
        [str(bindle_bin), "unpack", str(archive_path), str(extract_dir)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=True,
    )
    unpack_time = time.perf_counter() - start

    # Verify extraction (not timed)
    verify_extraction(src_dir, extract_dir)

    return pack_time, size, unpack_time


def benchmark_bindle_compressed(bindle_bin: Path, src_dir: Path, archive_path: Path) -> tuple[float, int, float]:
    """Benchmark bindle with zstd compression."""
    # Pack
    start = time.perf_counter()
    subprocess.run(
        [str(bindle_bin), "pack", str(archive_path), str(src_dir), "--compress"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=True,
    )
    pack_time = time.perf_counter() - start

    size = archive_path.stat().st_size

    # Unpack
    extract_dir = archive_path.parent / f"extract_{archive_path.stem}"
    extract_dir.mkdir(exist_ok=True)
    start = time.perf_counter()
    subprocess.run(
        [str(bindle_bin), "unpack", str(archive_path), str(extract_dir)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=True,
    )
    unpack_time = time.perf_counter() - start

    # Verify extraction (not timed)
    verify_extraction(src_dir, extract_dir)

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
    extract_dir = archive_path.parent / f"extract_{archive_path.stem}"
    extract_dir.mkdir(exist_ok=True)
    start = time.perf_counter()
    subprocess.run(
        ["tar", "-xf", str(archive_path), "-C", str(extract_dir)],
        capture_output=True,
        check=True,
    )
    unpack_time = time.perf_counter() - start

    # Verify extraction (not timed)
    verify_extraction(src_dir, extract_dir)

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
    extract_dir = archive_path.parent / f"extract_{archive_path.stem}"
    extract_dir.mkdir(exist_ok=True)
    start = time.perf_counter()
    subprocess.run(
        ["tar", "-xzf", str(archive_path), "-C", str(extract_dir)],
        capture_output=True,
        check=True,
    )
    unpack_time = time.perf_counter() - start

    # Verify extraction (not timed)
    verify_extraction(src_dir, extract_dir)

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
    extract_dir = archive_path.parent / f"extract_{archive_path.stem}"
    extract_dir.mkdir(exist_ok=True)
    start = time.perf_counter()
    subprocess.run(
        ["unzip", "-o", "-q", str(archive_path), "-d", str(extract_dir)],
        capture_output=True,
        check=True,
    )
    unpack_time = time.perf_counter() - start

    # Verify extraction (not timed)
    verify_extraction(src_dir, extract_dir)

    return pack_time, size, unpack_time


def main():
    project_root = Path(__file__).parent.parent

    print("Building bindle in release mode...")
    subprocess.run(
        ["cargo", "build", "--release", "--features", "cli"],
        cwd=project_root,
        capture_output=True,
        check=True,
    )

    # Get the built binary path
    bindle_bin = project_root / "target" / "release" / "bindle"
    if not bindle_bin.exists():
        raise FileNotFoundError(f"Built binary not found at {bindle_bin}")

    with tempfile.TemporaryDirectory() as tmpdir:
        tmpdir = Path(tmpdir)

        # Ensure directories exist and warm up filesystem
        test_data = tmpdir / "test_data"
        test_data.mkdir(parents=True, exist_ok=True)

        # Warm up: write and delete a small file to initialize filesystem
        warmup_file = tmpdir / "warmup"
        warmup_file.write_bytes(b"warmup" * 1000)
        warmup_file.unlink()

        # Create test data
        print("Creating test dataset...")
        create_test_data(test_data)

        # Warm up: read all test files to initialize filesystem caches
        for f in test_data.rglob("*"):
            if f.is_file():
                _ = f.read_bytes()

        # Calculate total size
        total_size = sum(f.stat().st_size for f in test_data.rglob("*") if f.is_file())
        file_count = len(list(test_data.rglob("*")))

        print(f"Test dataset: {file_count} files, {format_size(total_size)}\n")

        benchmarks = [
            ("bindle (uncompressed)", lambda run: benchmark_bindle_uncompressed(
                bindle_bin, test_data, tmpdir / f"test_{run}.bndl"
            )),
            ("bindle (zstd)", lambda run: benchmark_bindle_compressed(
                bindle_bin, test_data, tmpdir / f"test_zstd_{run}.bndl"
            )),
            ("tar", lambda run: benchmark_tar(
                test_data, tmpdir / f"test_{run}.tar"
            )),
            ("tar.gz", lambda run: benchmark_tar_gz(
                test_data, tmpdir / f"test_{run}.tar.gz"
            )),
            ("zip", lambda run: benchmark_zip(
                test_data, tmpdir / f"test_{run}.zip"
            )),
        ]

        results = []
        num_runs = 4  # Run each test 4 times, discard first, average remaining 3

        for name, bench_fn in benchmarks:
            print(f"Benchmarking {name}...", flush=True)
            try:
                pack_times = []
                unpack_times = []
                size = 0

                for run in range(num_runs):
                    pack_time, run_size, unpack_time = bench_fn(run)
                    pack_times.append(pack_time)
                    unpack_times.append(unpack_time)
                    size = run_size

                # Discard first run, average the rest
                avg_pack = sum(pack_times[1:]) / (num_runs - 1)
                avg_unpack = sum(unpack_times[1:]) / (num_runs - 1)

                results.append((name, avg_pack, size, avg_unpack))
            except subprocess.CalledProcessError as e:
                print(f"  ERROR: Command failed with exit code {e.returncode}")
                if e.stderr:
                    print(f"  stderr: {e.stderr.decode()}")
                results.append((name, 0, 0, 0))
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
