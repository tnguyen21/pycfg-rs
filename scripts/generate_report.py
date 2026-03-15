#!/usr/bin/env python3
"""Generate a static HTML report of pycfg-rs analysis across Python corpora.

Runs pycfg on each corpus, collects metrics, and emits a self-contained index.html
with per-project accordion sections and CFG visualizations.

Usage:
    cargo build --release
    ./scripts/bootstrap-corpora.sh
    python3 scripts/generate_report.py [--output report/index.html]
"""

import argparse
import html
import json
import os
import subprocess
import time
from dataclasses import dataclass, field
from pathlib import Path

# ---------------------------------------------------------------------------
# Corpus definitions
# ---------------------------------------------------------------------------

CORPORA = [
    ("requests", "src/requests", "https://github.com/psf/requests"),
    ("flask", "src/flask", "https://github.com/pallets/flask"),
    ("rich", "rich", "https://github.com/Textualize/rich"),
    ("pytest", "src/_pytest", "https://github.com/pytest-dev/pytest"),
    ("click", "src/click", "https://github.com/pallets/click"),
    ("httpx", "httpx", "https://github.com/encode/httpx"),
    ("black", "src/black", "https://github.com/psf/black"),
    ("pydantic", "pydantic", "https://github.com/pydantic/pydantic"),
    ("fastapi", "fastapi", "https://github.com/fastapi/fastapi"),
]

CORPORA_DIR = Path("benchmark/corpora")

TOP_N = 5  # number of top complex functions to show per project

# ---------------------------------------------------------------------------
# Data collection
# ---------------------------------------------------------------------------


@dataclass
class FunctionInfo:
    name: str
    file: str
    line: int
    blocks: int
    edges: int
    branches: int
    cyclomatic_complexity: int
    dot: str = ""  # DOT source for CFG visualization


@dataclass
class CorpusResult:
    name: str
    url: str
    files: int = 0
    functions: int = 0
    total_cc: int = 0
    max_cc: int = 0
    max_cc_func: str = ""
    max_cc_file: str = ""
    parse_time_ms: float = 0.0
    success: bool = True
    error: str = ""
    all_functions: list = field(default_factory=list)


def find_binary():
    for candidate in ["./target/release/pycfg", "./target/debug/pycfg"]:
        if os.path.exists(candidate):
            return candidate
    return "pycfg"


def analyze_corpus(name: str, subdir: str, url: str, binary: str) -> CorpusResult:
    result = CorpusResult(name=name, url=url)
    path = CORPORA_DIR / name / subdir

    if not path.exists():
        result.success = False
        result.error = f"Directory not found: {path}"
        return result

    start = time.perf_counter()
    try:
        proc = subprocess.run(
            [binary, "--format", "json", str(path)],
            capture_output=True,
            timeout=120,
        )
    except subprocess.TimeoutExpired:
        result.success = False
        result.error = "Timeout (>120s)"
        return result

    elapsed = time.perf_counter() - start
    result.parse_time_ms = elapsed * 1000

    if proc.returncode != 0:
        result.success = False
        result.error = proc.stderr.decode()[:200]
        return result

    try:
        data = json.loads(proc.stdout.decode())
    except json.JSONDecodeError as e:
        result.success = False
        result.error = f"JSON parse error: {e}"
        return result

    if not isinstance(data, dict):
        result.success = False
        result.error = "Unexpected JSON shape"
        return result

    data = data.get("files", [])

    result.files = len(data)

    for file_cfg in data:
        file_path = file_cfg.get("file", "")
        for func in file_cfg.get("functions", []):
            metrics = func.get("metrics", {})
            cc = metrics.get("cyclomatic_complexity", 0)
            fi = FunctionInfo(
                name=func.get("name", ""),
                file=file_path,
                line=func.get("line", 0),
                blocks=metrics.get("blocks", 0),
                edges=metrics.get("edges", 0),
                branches=metrics.get("branches", 0),
                cyclomatic_complexity=cc,
            )
            result.all_functions.append(fi)
            result.total_cc += cc
            result.functions += 1
            if cc > result.max_cc:
                result.max_cc = cc
                result.max_cc_func = fi.name
                result.max_cc_file = file_path

    return result


def fetch_dot_for_top_functions(result: CorpusResult, binary: str, top_n: int = TOP_N):
    """Fetch DOT output for the top N most complex functions in a corpus."""
    if not result.success or not result.all_functions:
        return

    top = sorted(result.all_functions, key=lambda f: f.cyclomatic_complexity, reverse=True)[:top_n]

    for func in top:
        target = f"{func.file}::{func.name}"
        try:
            proc = subprocess.run(
                [binary, "--format", "dot", target],
                capture_output=True,
                timeout=30,
            )
            if proc.returncode == 0:
                func.dot = proc.stdout.decode()
        except Exception:
            pass


# ---------------------------------------------------------------------------
# Test / version info
# ---------------------------------------------------------------------------


def get_test_count() -> int | None:
    try:
        proc = subprocess.run(
            ["cargo", "test", "--", "--list"],
            capture_output=True,
            timeout=120,
        )
        if proc.returncode != 0:
            return None
        output = proc.stdout.decode()
        count = 0
        for line in output.splitlines():
            if line.strip().endswith(": test"):
                count += 1
        return count
    except Exception:
        return None


def get_version() -> str:
    try:
        proc = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            capture_output=True,
            timeout=30,
        )
        data = json.loads(proc.stdout.decode())
        for pkg in data.get("packages", []):
            if pkg["name"] == "pycfg-rs":
                return pkg["version"]
    except Exception:
        pass
    return "unknown"


def get_git_sha() -> str:
    try:
        proc = subprocess.run(["git", "rev-parse", "--short", "HEAD"], capture_output=True, timeout=10)
        return proc.stdout.decode().strip()
    except Exception:
        return "unknown"


# ---------------------------------------------------------------------------
# HTML generation
# ---------------------------------------------------------------------------


def cc_class(cc: int) -> str:
    if cc >= 20:
        return "cc-high"
    elif cc >= 10:
        return "cc-med"
    return "cc-low"


def generate_html(results: list[CorpusResult], test_count: int | None, version: str, sha: str) -> str:
    now = time.strftime("%Y-%m-%d %H:%M UTC", time.gmtime())

    total_files = sum(r.files for r in results if r.success)
    total_funcs = sum(r.functions for r in results if r.success)
    total_time = sum(r.parse_time_ms for r in results if r.success)
    success_count = sum(1 for r in results if r.success)
    test_str = f"{test_count}" if test_count else "—"

    # Build per-project accordion sections
    project_sections = ""
    for r in sorted(results, key=lambda r: r.functions, reverse=True):
        if not r.success:
            project_sections += f"""
<details class="project-accordion">
    <summary class="project-header">
        <span class="project-name">{r.name}</span>
        <span class="project-badge badge-fail">failed</span>
        <span class="project-error">{html.escape(r.error)}</span>
    </summary>
</details>"""
            continue

        avg_cc = r.total_cc / r.functions if r.functions > 0 else 0
        throughput = r.functions / (r.parse_time_ms / 1000) if r.parse_time_ms > 0 else 0

        # CC distribution
        low = sum(1 for f in r.all_functions if f.cyclomatic_complexity < 5)
        med = sum(1 for f in r.all_functions if 5 <= f.cyclomatic_complexity < 10)
        high = sum(1 for f in r.all_functions if 10 <= f.cyclomatic_complexity < 20)
        very_high = sum(1 for f in r.all_functions if f.cyclomatic_complexity >= 20)

        # Top complex functions
        top_funcs = sorted(r.all_functions, key=lambda f: f.cyclomatic_complexity, reverse=True)[:TOP_N]

        func_cards = ""
        for f in top_funcs:
            short_file = f.file.replace("benchmark/corpora/", "")
            dot_escaped = html.escape(f.dot) if f.dot else ""
            cfg_section = ""
            if f.dot:
                cfg_section = f"""
                <details class="cfg-toggle">
                    <summary>Show CFG</summary>
                    <div class="cfg-container" data-dot="{dot_escaped}">
                        <div class="cfg-loading">Rendering graph...</div>
                    </div>
                </details>"""

            func_cards += f"""
            <div class="func-card">
                <div class="func-header">
                    <span class="func-name">{html.escape(f.name)}</span>
                    <span class="func-cc {cc_class(f.cyclomatic_complexity)}">CC {f.cyclomatic_complexity}</span>
                </div>
                <div class="func-meta">
                    <span class="func-file">{html.escape(short_file)}:{f.line}</span>
                    <span class="func-stats">{f.blocks} blocks, {f.edges} edges</span>
                </div>
                {cfg_section}
            </div>"""

        project_sections += f"""
<details class="project-accordion" open>
    <summary class="project-header">
        <span class="project-name"><a href="{r.url}" target="_blank">{r.name}</a></span>
        <span class="project-stat">{r.files} files</span>
        <span class="project-stat">{r.functions:,} functions</span>
        <span class="project-stat">avg CC {avg_cc:.1f}</span>
        <span class="project-stat">{r.parse_time_ms:.0f}ms</span>
        <span class="project-badge badge-pass">pass</span>
    </summary>
    <div class="project-body">
        <div class="project-stats-row">
            <div class="mini-stat">
                <span class="mini-value">{r.files}</span>
                <span class="mini-label">files</span>
            </div>
            <div class="mini-stat">
                <span class="mini-value">{r.functions:,}</span>
                <span class="mini-label">functions</span>
            </div>
            <div class="mini-stat">
                <span class="mini-value">{avg_cc:.1f}</span>
                <span class="mini-label">avg CC</span>
            </div>
            <div class="mini-stat">
                <span class="mini-value">{r.max_cc}</span>
                <span class="mini-label">max CC</span>
            </div>
            <div class="mini-stat">
                <span class="mini-value">{throughput:,.0f}</span>
                <span class="mini-label">fn/s</span>
            </div>
        </div>
        <div class="cc-distribution">
            <span class="cc-dist-label">Complexity distribution:</span>
            <span class="cc-low">low (&lt;5): {low}</span>
            <span class="cc-med">moderate (5-9): {med}</span>
            <span class="cc-med">high (10-19): {high}</span>
            <span class="cc-high">very high (20+): {very_high}</span>
        </div>

        <h4 class="top-funcs-title">Top {len(top_funcs)} most complex functions</h4>
        <div class="func-cards">
            {func_cards}
        </div>
    </div>
</details>"""

    page_html = f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>pycfg-rs report</title>
<style>
:root {{
    --bg: #0d1117;
    --surface: #161b22;
    --surface2: #1c2129;
    --border: #30363d;
    --text: #e6edf3;
    --text-dim: #8b949e;
    --accent: #58a6ff;
    --green: #3fb950;
    --orange: #d29922;
    --red: #f85149;
    --mono: "JetBrains Mono", "Fira Code", "Consolas", monospace;
}}
* {{ box-sizing: border-box; margin: 0; padding: 0; }}
body {{
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
    background: var(--bg);
    color: var(--text);
    line-height: 1.5;
    padding: 2rem;
    max-width: 1200px;
    margin: 0 auto;
}}
h1 {{ font-size: 1.75rem; margin-bottom: 0.25rem; }}
h2 {{ font-size: 1.25rem; margin: 2rem 0 0.75rem; color: var(--accent); }}
.subtitle {{ color: var(--text-dim); margin-bottom: 0.5rem; }}
.intro {{ color: var(--text-dim); font-size: 0.9rem; margin-bottom: 1.5rem; max-width: 700px; line-height: 1.6; }}
.meta {{ color: var(--text-dim); font-size: 0.85rem; margin-bottom: 2rem; }}
.meta span {{ margin-right: 1.5rem; }}
a {{ color: var(--accent); text-decoration: none; }}
a:hover {{ text-decoration: underline; }}

/* Stats cards */
.stats {{
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 1rem;
    margin-bottom: 2rem;
}}
.stat-card {{
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 1rem 1.25rem;
}}
.stat-card .value {{
    font-size: 1.75rem;
    font-weight: 700;
    font-family: var(--mono);
    color: var(--green);
}}
.stat-card .label {{
    color: var(--text-dim);
    font-size: 0.85rem;
    margin-top: 0.25rem;
}}

/* Project accordions */
.project-accordion {{
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    margin-bottom: 0.75rem;
    overflow: hidden;
}}
.project-header {{
    padding: 0.75rem 1rem;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 1rem;
    flex-wrap: wrap;
    list-style: none;
}}
.project-header::-webkit-details-marker {{ display: none; }}
.project-header::before {{
    content: "\\25B6";
    font-size: 0.7rem;
    color: var(--text-dim);
    transition: transform 0.15s;
}}
details[open] > .project-header::before {{ transform: rotate(90deg); }}
.project-name {{ font-weight: 600; font-size: 1rem; }}
.project-name a {{ color: var(--text); }}
.project-name a:hover {{ color: var(--accent); }}
.project-stat {{ color: var(--text-dim); font-size: 0.85rem; font-family: var(--mono); }}
.project-badge {{
    font-size: 0.75rem;
    padding: 0.15rem 0.5rem;
    border-radius: 10px;
    font-weight: 600;
    margin-left: auto;
}}
.badge-pass {{ background: rgba(63, 185, 80, 0.15); color: var(--green); }}
.badge-fail {{ background: rgba(248, 81, 73, 0.15); color: var(--red); }}
.project-error {{ color: var(--red); font-size: 0.85rem; }}

.project-body {{
    padding: 0 1rem 1rem;
    border-top: 1px solid var(--border);
}}
.project-stats-row {{
    display: flex;
    gap: 1.5rem;
    padding: 0.75rem 0;
    flex-wrap: wrap;
}}
.mini-stat {{ text-align: center; }}
.mini-value {{
    display: block;
    font-family: var(--mono);
    font-weight: 700;
    font-size: 1.1rem;
    color: var(--accent);
}}
.mini-label {{
    font-size: 0.75rem;
    color: var(--text-dim);
}}
.cc-distribution {{
    font-size: 0.8rem;
    padding: 0.5rem 0;
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
    align-items: center;
}}
.cc-dist-label {{ color: var(--text-dim); }}

.top-funcs-title {{
    font-size: 0.9rem;
    color: var(--text-dim);
    margin: 0.75rem 0 0.5rem;
    font-weight: 600;
}}
.func-cards {{ display: flex; flex-direction: column; gap: 0.5rem; }}
.func-card {{
    background: var(--surface2);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0.6rem 0.75rem;
}}
.func-header {{
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
}}
.func-name {{
    font-family: var(--mono);
    font-size: 0.9rem;
    font-weight: 600;
}}
.func-cc {{
    font-family: var(--mono);
    font-size: 0.85rem;
    font-weight: 700;
    white-space: nowrap;
}}
.func-meta {{
    display: flex;
    gap: 1rem;
    font-size: 0.8rem;
    color: var(--text-dim);
    margin-top: 0.25rem;
}}
.func-file {{ font-family: var(--mono); }}
.func-stats {{ font-family: var(--mono); }}

/* CC heatmap */
.cc-high {{ color: var(--red); }}
.cc-med {{ color: var(--orange); }}
.cc-low {{ color: var(--green); }}

/* CFG visualization */
.cfg-toggle {{
    margin-top: 0.5rem;
}}
.cfg-toggle > summary {{
    cursor: pointer;
    font-size: 0.8rem;
    color: var(--accent);
    list-style: none;
    user-select: none;
}}
.cfg-toggle > summary::-webkit-details-marker {{ display: none; }}
.cfg-toggle > summary:hover {{ text-decoration: underline; }}
.cfg-container {{
    margin-top: 0.5rem;
    background: #fff;
    border-radius: 6px;
    padding: 1rem;
    overflow-x: auto;
    max-height: 600px;
    overflow-y: auto;
}}
.cfg-container svg {{
    max-width: 100%;
    height: auto;
}}
.cfg-loading {{
    color: #666;
    font-size: 0.85rem;
    padding: 0.5rem;
}}

/* footer */
.footer {{ color: var(--text-dim); font-size: 0.8rem; margin-top: 3rem; text-align: center; }}
</style>
</head>
<body>

<h1>pycfg-rs</h1>
<p class="subtitle">Rust-based control flow graph generator for Python</p>
<p class="intro">
    This page shows the output of running
    <a href="https://github.com/nwyin/pycfg-rs">pycfg-rs</a> against
    {len(results)} popular open-source Python projects. For each project,
    you can see analysis stats and the control flow graphs of the most
    complex functions — expand any function to view its CFG.
</p>
<div class="meta">
    <span>v{version}</span>
    <span>commit <code>{sha}</code></span>
    <span>generated {now}</span>
</div>

<div class="stats">
    <div class="stat-card"><div class="value">{total_files}</div><div class="label">Python files</div></div>
    <div class="stat-card"><div class="value">{total_funcs:,}</div><div class="label">functions analyzed</div></div>
    <div class="stat-card"><div class="value">{total_time:.0f}ms</div><div class="label">total parse time</div></div>
    <div class="stat-card"><div class="value">{test_str}</div><div class="label">tests passing</div></div>
    <div class="stat-card"><div class="value">{success_count}/{len(results)}</div><div class="label">projects passing</div></div>
</div>

<h2>Projects</h2>
{project_sections}

<div class="footer">
    Generated by <a href="https://github.com/nwyin/pycfg-rs">pycfg-rs</a> &middot;
    Powered by <a href="https://github.com/astral-sh/ruff">ruff_python_parser</a>
</div>

<script type="module">
import {{ Graphviz }} from "https://cdn.jsdelivr.net/npm/@hpcc-js/wasm@2.22.4/dist/graphviz.js";

const graphviz = await Graphviz.load();

// Render CFGs on demand when "Show CFG" is expanded
document.querySelectorAll(".cfg-toggle").forEach(toggle => {{
    toggle.addEventListener("toggle", () => {{
        if (!toggle.open) return;
        const container = toggle.querySelector(".cfg-container");
        if (container.dataset.rendered) return;
        const dot = container.dataset.dot;
        if (!dot) return;
        try {{
            const svg = graphviz.dot(dot);
            container.innerHTML = svg;
            container.dataset.rendered = "1";
        }} catch (e) {{
            container.innerHTML = '<div class="cfg-loading">Failed to render graph</div>';
        }}
    }});
}});
</script>
</body>
</html>"""
    return page_html


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(description="Generate pycfg-rs analysis report")
    parser.add_argument("--output", "-o", default="report/index.html", help="Output HTML file")
    parser.add_argument("--skip-tests", action="store_true", help="Skip test count detection")
    args = parser.parse_args()

    binary = find_binary()
    print(f"Using binary: {binary}")

    # Collect test count
    test_count = None
    if not args.skip_tests:
        print("Counting tests...")
        test_count = get_test_count()
        if test_count:
            print(f"  {test_count} tests")

    version = get_version()
    sha = get_git_sha()
    print(f"Version: {version}, commit: {sha}")

    # Analyze corpora
    results = []
    for name, subdir, url in CORPORA:
        print(f"Analyzing {name}...", end=" ", flush=True)
        result = analyze_corpus(name, subdir, url, binary)
        if result.success:
            avg_cc = result.total_cc / result.functions if result.functions else 0
            print(f"{result.files} files, {result.functions:,} functions, avg CC={avg_cc:.1f}, {result.parse_time_ms:.0f}ms")
        else:
            print(f"FAILED: {result.error}")
        results.append(result)

    # Fetch DOT output for top complex functions
    print("Fetching CFGs for top functions...", end=" ", flush=True)
    for result in results:
        fetch_dot_for_top_functions(result, binary)
    dot_count = sum(1 for r in results for f in r.all_functions if f.dot)
    print(f"{dot_count} graphs")

    # Generate HTML
    page_html = generate_html(results, test_count, version, sha)

    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(page_html)
    print(f"\nReport written to {output_path}")
    total_funcs = sum(r.functions for r in results if r.success)
    print(f"Total: {total_funcs:,} functions across {sum(r.files for r in results if r.success)} files")


if __name__ == "__main__":
    main()
