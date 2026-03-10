"""
Benchmark pycfg-rs vs staticfg vs python-graphs.

Apple-to-apple comparison on two axes:
  1. File-level: all tools process the same source files (staticfg, pycfg-rs)
  2. Function-level: tools that can target individual functions (python-graphs, pycfg-rs)

python-graphs requires importing modules at runtime, so it can only analyze
files with no unresolved imports. We report success rates alongside timings.

Usage:
    source benchmark/.venv/bin/activate
    python benchmark/bench.py                               # test fixtures
    python benchmark/bench.py --corpus benchmark/corpora/flask/src     # real codebase
    python benchmark/bench.py --corpus benchmark/corpora/rich/rich     # large codebase
"""

import argparse
import ast
import importlib.util
import inspect
import json
import os
import subprocess
import sys
import time
from pathlib import Path


def collect_python_files(directory: str) -> list[str]:
    files = []
    for root, _, filenames in os.walk(directory):
        if "__pycache__" in root:
            continue
        for f in filenames:
            if f.endswith(".py"):
                files.append(os.path.join(root, f))
    files.sort()
    return files


def count_functions_in_file(filepath: str) -> int:
    """Count function/method defs using stdlib ast (no import needed)."""
    try:
        with open(filepath) as f:
            tree = ast.parse(f.read())
    except Exception:
        return 0
    count = 0
    for node in ast.walk(tree):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            count += 1
    return count


def extract_functions_from_file(filepath: str) -> list[tuple[str, object]]:
    """Import a Python file and return all function/method objects."""
    module_name = Path(filepath).stem
    try:
        spec = importlib.util.spec_from_file_location(module_name, filepath)
        if spec is None or spec.loader is None:
            return []
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
    except Exception:
        return []

    functions = []
    for name, obj in inspect.getmembers(mod):
        if inspect.isfunction(obj) and obj.__module__ == module_name:
            functions.append((name, obj))
        elif inspect.isclass(obj) and obj.__module__ == module_name:
            for mname, method in inspect.getmembers(obj, predicate=inspect.isfunction):
                if not mname.startswith("_") or mname in ("__init__", "__call__"):
                    functions.append((f"{name}.{mname}", method))
    return functions


# ---------------------------------------------------------------------------
# Benchmark runners
# ---------------------------------------------------------------------------


def bench_pycfg_rs(files: list[str], iterations: int) -> dict:
    """Benchmark pycfg-rs (Rust binary via subprocess)."""
    binary = None
    for candidate in ["./target/release/pycfg", "./target/debug/pycfg"]:
        if os.path.exists(candidate):
            binary = candidate
            break
    if binary is None:
        try:
            subprocess.run(["pycfg", "--version"], capture_output=True, check=True)
            binary = "pycfg"
        except (FileNotFoundError, subprocess.CalledProcessError):
            return {"error": "pycfg binary not found. Run: cargo build --release"}

    # Warm up
    subprocess.run([binary, "--format", "json"] + files, capture_output=True)

    times = []
    last_result = None
    for _ in range(iterations):
        start = time.perf_counter()
        result = subprocess.run([binary, "--format", "json"] + files, capture_output=True)
        elapsed = time.perf_counter() - start
        if result.returncode != 0:
            return {"error": f"pycfg failed: {result.stderr.decode()[:200]}"}
        times.append(elapsed)
        last_result = result

    # Count from output
    output = last_result.stdout.decode()
    try:
        data = json.loads(output)
        file_cfgs = data.get("files", []) if isinstance(data, dict) else []
        num_functions = sum(len(f.get("functions", [])) for f in file_cfgs)
        num_files = len(file_cfgs)
    except json.JSONDecodeError:
        num_functions = 0
        num_files = 0

    return {
        "times": times,
        "median_ms": sorted(times)[len(times) // 2] * 1000,
        "min_ms": min(times) * 1000,
        "files_ok": num_files,
        "files_total": len(files),
        "functions": num_functions,
        "note": f"binary: {binary}",
    }


def bench_staticfg(files: list[str], iterations: int) -> dict:
    """Benchmark staticfg (Python, file-level CFGs)."""
    try:
        from staticfg import CFGBuilder
    except ImportError:
        return {"error": "staticfg not installed. pip install staticfg"}

    builder = CFGBuilder()

    # Warm up + count successes
    successes = []
    failures = 0
    for filepath in files:
        try:
            builder.build_from_file(Path(filepath).stem, filepath)
            successes.append(filepath)
        except Exception:
            failures += 1

    # Count functions in successful files (staticfg builds one CFG per file, not per function)
    total_funcs = sum(count_functions_in_file(f) for f in successes)

    times = []
    for _ in range(iterations):
        start = time.perf_counter()
        for filepath in successes:
            builder.build_from_file(Path(filepath).stem, filepath)
        elapsed = time.perf_counter() - start
        times.append(elapsed)

    return {
        "times": times,
        "median_ms": sorted(times)[len(times) // 2] * 1000,
        "min_ms": min(times) * 1000,
        "files_ok": len(successes),
        "files_total": len(files),
        "functions": total_funcs,
        "note": f"{failures} files failed to parse",
    }


def bench_python_graphs(files: list[str], iterations: int) -> dict:
    """Benchmark python-graphs (Google, requires runtime import)."""
    try:
        from python_graphs import control_flow
    except ImportError:
        return {"error": "python-graphs not installed. pip install python-graphs"}

    # Pre-extract function objects (not timed — this is setup)
    all_functions = []
    importable_files = 0
    for filepath in files:
        funcs = extract_functions_from_file(filepath)
        if funcs:
            importable_files += 1
        all_functions.extend(funcs)

    # Filter to functions that python-graphs can actually analyze
    analyzable = []
    for name, func in all_functions:
        try:
            control_flow.get_control_flow_graph(func)
            analyzable.append((name, func))
        except Exception:
            pass

    if not analyzable:
        return {
            "error": f"no functions analyzable (0/{len(all_functions)} extracted, {importable_files}/{len(files)} files importable)"
        }

    times = []
    for _ in range(iterations):
        start = time.perf_counter()
        for _, func in analyzable:
            control_flow.get_control_flow_graph(func)
        elapsed = time.perf_counter() - start
        times.append(elapsed)

    return {
        "times": times,
        "median_ms": sorted(times)[len(times) // 2] * 1000,
        "min_ms": min(times) * 1000,
        "files_ok": importable_files,
        "files_total": len(files),
        "functions": len(analyzable),
        "note": f"{len(all_functions) - len(analyzable)} functions failed analysis",
    }


# ---------------------------------------------------------------------------
# Reporting
# ---------------------------------------------------------------------------


def format_result(name: str, result: dict) -> str:
    if "error" in result:
        return f"  {name:20s}  ERROR: {result['error']}"
    files_str = f"{result['files_ok']}/{result['files_total']} files"
    return (
        f"  {name:20s}  "
        f"median={result['median_ms']:8.1f}ms  "
        f"min={result['min_ms']:8.1f}ms  "
        f"{result['functions']:4d} functions  "
        f"{files_str}"
    )


def format_throughput(name: str, result: dict) -> str:
    if "error" in result:
        return ""
    funcs = result["functions"]
    ms = result["min_ms"]
    if funcs > 0 and ms > 0:
        per_func = ms / funcs
        funcs_per_sec = funcs / (ms / 1000)
        return f"  {name:20s}  {per_func:6.3f} ms/function  ({funcs_per_sec:,.0f} functions/sec)"
    return f"  {name:20s}  N/A"


def main():
    parser = argparse.ArgumentParser(
        description="Benchmark pycfg-rs vs staticfg vs python-graphs",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  python bench/bench.py                                  # test fixtures (tiny)
  python benchmark/bench.py --corpus benchmark/corpora/requests/src    # requests (~18 files)
  python benchmark/bench.py --corpus benchmark/corpora/flask/src/flask # flask (~20 files)
  python benchmark/bench.py --corpus benchmark/corpora/rich/rich       # rich (~70 files)
""",
    )
    parser.add_argument("--corpus", default=None, help="Directory of Python files to analyze")
    parser.add_argument("--iterations", "-n", type=int, default=10, help="Iterations per tool (default: 10)")
    args = parser.parse_args()

    if args.corpus:
        target_dir = args.corpus
    else:
        target_dir = os.path.join(os.path.dirname(__file__), "..", "tests", "test_code")

    files = collect_python_files(target_dir)
    if not files:
        print(f"No Python files found in {target_dir}")
        sys.exit(1)

    total_funcs = sum(count_functions_in_file(f) for f in files)
    print(f"Corpus: {target_dir}")
    print(f"Files: {len(files)}, Functions (ast count): {total_funcs}")
    print(f"Iterations: {args.iterations}")
    print()

    tools = {}

    print("Running pycfg-rs (Rust, subprocess)...")
    tools["pycfg-rs"] = bench_pycfg_rs(files, args.iterations)
    print(format_result("pycfg-rs", tools["pycfg-rs"]))
    print()

    print("Running staticfg (Python, in-process)...")
    tools["staticfg"] = bench_staticfg(files, args.iterations)
    print(format_result("staticfg", tools["staticfg"]))
    print()

    print("Running python-graphs (Python, in-process)...")
    tools["python-graphs"] = bench_python_graphs(files, args.iterations)
    print(format_result("python-graphs", tools["python-graphs"]))
    print()

    # Summary
    print("=" * 78)
    print("RESULTS (lower is better)")
    print("=" * 78)
    print()
    for name, result in tools.items():
        print(format_result(name, result))
    print()

    # Throughput (per-function)
    print("-" * 78)
    print("THROUGHPUT")
    print("-" * 78)
    print()
    for name, result in tools.items():
        line = format_throughput(name, result)
        if line:
            print(line)
    print()

    # Notes
    print("-" * 78)
    print("NOTES")
    print("-" * 78)
    valid = {k: v for k, v in tools.items() if "error" not in v}
    for name, result in valid.items():
        if "note" in result:
            print(f"  {name}: {result['note']}")
    print()
    print("  pycfg-rs timing includes subprocess spawn (~1-2ms overhead).")
    print("  staticfg builds one CFG per file (not per function).")
    print("  python-graphs requires runtime import; files with unresolved deps are skipped.")

    # Pairwise speedup vs pycfg-rs (only for tools that analyzed similar counts)
    if "pycfg-rs" in valid:
        pycfg_min = valid["pycfg-rs"]["min_ms"]
        print()
        print("-" * 78)
        print("SPEEDUP (pycfg-rs vs others, using min time)")
        print("-" * 78)
        for name, result in valid.items():
            if name == "pycfg-rs":
                continue
            other_min = result["min_ms"]
            if other_min > 0:
                ratio = other_min / pycfg_min
                if ratio > 1:
                    print(f"  pycfg-rs is {ratio:.1f}x faster than {name}")
                else:
                    print(f"  pycfg-rs is {1/ratio:.1f}x slower than {name}")
                # Normalize by function count for fairer comparison
                pycfg_funcs = valid["pycfg-rs"]["functions"]
                other_funcs = result["functions"]
                if pycfg_funcs > 0 and other_funcs > 0:
                    pycfg_per_func = pycfg_min / pycfg_funcs
                    other_per_func = other_min / other_funcs
                    norm_ratio = other_per_func / pycfg_per_func
                    if norm_ratio > 1:
                        print(f"    (per function: pycfg-rs is {norm_ratio:.1f}x faster)")
                    else:
                        print(f"    (per function: pycfg-rs is {1/norm_ratio:.1f}x slower)")


if __name__ == "__main__":
    main()
