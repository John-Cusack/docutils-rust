#!/usr/bin/env python3
"""Two-track bench: CPython `docutils` vs `docutils-rust`, measured only.

Runs `publish_string` (report_level 1, halt_level 4 — identical settings)
over the same inputs on both sides and reports wall time + peak RSS.

Usage: venv/bin/python benches/two_track.py [--iters N] [--big-kb N]

Each side runs in a fresh process per input (honest peak RSS). The Rust
side loops internally; the Python side loops in-process. Outputs a
Markdown table on stdout. No projections.
"""

import argparse
import io
import os
import statistics
import subprocess
import sys
import tempfile
import time

MIRROR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CORPUS = os.path.join(MIRROR, "tests", "corpus")
BENCH_BIN = os.path.join(MIRROR, "target", "release", "examples", "bench")


def python_worker(path, iters):
    import resource

    import docutils.core

    with open(path) as f:
        src = f.read()
    over = {"halt_level": 4, "report_level": 1, "warning_stream": io.StringIO()}
    out = docutils.core.publish_string(src, writer_name="html5", settings_overrides=dict(over))
    assert len(out) > 0
    t0 = time.perf_counter()
    for _ in range(iters):
        out = docutils.core.publish_string(
            src, writer_name="html5", settings_overrides=dict(over)
        )
    wall = (time.perf_counter() - t0) / iters
    assert len(out) > 0
    print(f"wall_ms={wall * 1000:.2f} peak_kb={resource.getrusage(resource.RUSAGE_SELF).ru_maxrss}")


def run_worker(python, path, iters, reps=3):
    walls, peaks = [], []
    for _ in range(reps):
        p = subprocess.run(
            [python, __file__, "--worker", path, str(iters)],
            capture_output=True, text=True, timeout=900,
        )
        assert p.returncode == 0, p.stderr[-500:]
        kv = dict(t.split("=") for t in p.stdout.split())
        walls.append(float(kv["wall_ms"]))
        peaks.append(int(kv["peak_kb"]))
    return statistics.median(walls), max(peaks)


def run_rust(path, iters, reps=3):
    walls, peaks = [], []
    for _ in range(reps):
        t0 = time.perf_counter()
        p = subprocess.run(
            ["/usr/bin/time", "-v", BENCH_BIN, path, str(iters)],
            capture_output=True, text=True, timeout=900,
        )
        wall = (time.perf_counter() - t0) * 1000 / iters
        assert p.returncode == 0, p.stderr[-500:]
        for line in p.stdout.splitlines():
            if line.startswith("iters="):
                kv = dict(t.split("=") for t in line.split())
                wall = float(kv["elapsed_ms"]) / iters
        peak = 0
        for line in p.stderr.splitlines():
            if "Maximum resident set size" in line:
                peak = int(line.split(":")[1].split()[0])
        walls.append(wall)
        peaks.append(peak)
    return statistics.median(walls), max(peaks)


def make_big(path, target_kb):
    with open(os.path.join(CORPUS, "smoke.rst")) as f:
        body = f.read()
    # Drop the title block (first 3 lines) so tiling stays one document.
    body = "\n".join(body.splitlines()[3:])
    rep = max(1, (target_kb * 1024) // max(1, len(body)))
    with open(path, "w") as f:
        f.write("Big Bench\n=========\n\n")
        for _ in range(rep):
            f.write(body)
            f.write("\n\n")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--worker", nargs=2, metavar=("PATH", "ITERS"))
    ap.add_argument("--iters", type=int, default=20)
    ap.add_argument("--big-kb", type=int, default=1024)
    args = ap.parse_args()
    if args.worker:
        python_worker(args.worker[0], int(args.worker[1]))
        return 0

    subprocess.run(
        ["cargo", "build", "--release", "--example", "bench", "-p", "docutils-core"],
        cwd=MIRROR, check=True, capture_output=True,
    )
    python = os.environ.get("BENCH_PYTHON", "/home/john/firstmate/data/rs-port-docutils/venv/bin/python")

    files = sorted(
        os.path.join(CORPUS, fn) for fn in os.listdir(CORPUS) if fn.endswith(".rst")
    )
    with tempfile.TemporaryDirectory() as tmp:
        big = os.path.join(tmp, "big.rst")
        make_big(big, args.big_kb)
        files.append(big)
        print("| input | size | python wall | rust wall | python peak RSS | rust peak RSS |")
        print("| --- | --- | --- | --- | --- | --- |")
        for path in files:
            name = os.path.basename(path)
            kb = os.path.getsize(path) / 1024
            py_wall, py_rss = run_worker(python, path, args.iters)
            rs_wall, rs_rss = run_rust(path, args.iters)
            print(
                f"| {name} | {kb:.1f} KiB | {py_wall:.2f} ms"
                f" | {rs_wall:.2f} ms | {py_rss} KiB | {rs_rss} KiB |"
            )
    return 0


if __name__ == "__main__":
    sys.exit(main())
