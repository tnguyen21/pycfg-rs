#!/usr/bin/env python3
"""Generate a static HTML product page + corpus report for pycfg-rs.

Three sections:
  1. Hero: pitch, install, example, comparison table, links
  2. Corpus report: per-project accordion stats with CFG visualizations
  3. Essay: why intra-procedural CFGs matter for LLM workflows

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
from html import escape
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
# Static content
# ---------------------------------------------------------------------------

COMPARISON_ROWS = [
    ("Speed (rich, 100 files)", "65 ms", "84 ms", "7 ms*"),
    ("Per-function speed", "0.066 ms", "0.35 ms", "0.19 ms"),
    ("File coverage", "100%", "74%", "13%"),
    ("Output formats", "text / JSON / DOT", "DOT only", "Python object"),
    ("Python 3.10+ (match/case)", "yes", "no", "no"),
    ("Runtime required", "no", "no", "yes (import)"),
    ("Language", "Rust", "Python", "Python"),
    ("Maintained", "active", "unmaintained", "archived"),
]

EXAMPLE_PYTHON = """\
def process(request):
    try:
        user = authenticate(request)
        if user.is_admin:
            return admin_dashboard(user)
        return user_dashboard(user)
    except AuthError:
        return redirect("/login")
    finally:
        log_access(request)"""

EXAMPLE_OUTPUT = """\
$ pycfg src/app/service.py::process

Function: process (line 1)
  Block 0 (entry):
    [L2] try:
    -> 1 (try-body)
    -> 3 (exception: AuthError)
  Block 1 (try-body):
    [L3] user = authenticate(request)
    [L4] if user.is_admin:
    -> 2 (True)
    -> 5 (False)
  Block 2 (if-true):
    [L5] return admin_dashboard(user)
    -> 4 (finally)
  Block 3 (except: AuthError):
    [L7] return redirect("/login")
    -> 4 (finally)
  Block 4 (finally):
    [L9] log_access(request)
    -> 6 (exit)
  Block 5 (if-false):
    [L6] return user_dashboard(user)
    -> 4 (finally)
  Block 6 (exit):

  Metrics: 7 blocks, 8 edges, 2 branches, CC=4"""

WHY_ESSAY = """\
<p>
LLMs struggle with control flow reasoning. Given a function with nested
branches, exception handlers, and early returns, models mis-predict which
paths are reachable, confuse exception routing with normal flow, and miss
edge cases in <code>try</code>/<code>finally</code> blocks. The
<a href="https://arxiv.org/abs/2501.16456">CoCoNUT benchmark</a> (2025)
found that even the best model traced only 47% of control flow paths
correctly, with accuracy dropping sharply as complexity increased. This is
a representation problem &mdash; source code buries control flow in
indentation and keywords. CFGs make it explicit.
</p>

<p>
A control flow graph reduces a function to its structural skeleton: basic
blocks connected by labeled edges. Instead of asking "what happens if
<code>authenticate</code> raises?", an agent reads the graph and sees the
edge directly: <em>Block 0 &rarr; Block 3 (exception: AuthError)</em>.
Branches, loops, <code>break</code>/<code>continue</code>, and exception
routing all become first-class, queryable structure.
</p>

<p>
<strong>Cyclomatic complexity</strong> is the useful byproduct. CC = E &minus; N + 2
counts the number of linearly independent paths through a function.
A function with CC &gt; 15 has too many paths for a model to reason about
in one pass. An agent that checks CC first can decide to decompose the
function, or to focus its attention on the high-complexity branches,
before attempting a change.
</p>

<p>
The key design choice in pycfg-rs is <strong>machine-consumable output</strong>.
The JSON format preserves block IDs, edge labels, statement text, and
line numbers &mdash; everything an agent needs to cross-reference back to
source. The text format is designed to be readable in a prompt without
extra parsing. DOT output feeds into Graphviz for visual inspection.
Three formats, one structural truth.
</p>

<p>
Static analysis is not omniscient. <code>exec</code>, dynamic dispatch,
and metaprogramming will always create blind spots. But for the
overwhelmingly common case &mdash; regular Python functions with
branches, loops, and exception handling &mdash; the CFG is exact,
and it tells the model which code to reason about carefully.
</p>"""


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
# Version / git info
# ---------------------------------------------------------------------------


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
# HTML helpers
# ---------------------------------------------------------------------------


def cc_class(cc: int) -> str:
    if cc >= 20:
        return "cc-high"
    elif cc >= 10:
        return "cc-med"
    return "cc-low"


# ---------------------------------------------------------------------------
# Section 1: Hero
# ---------------------------------------------------------------------------


def _hero_html() -> str:
    comparison_rows = ""
    for label, pycfg, staticfg, pgraphs in COMPARISON_ROWS:
        comparison_rows += f"""<tr>
  <td>{escape(label)}</td>
  <td class="highlight">{escape(pycfg)}</td>
  <td>{escape(staticfg)}</td>
  <td>{escape(pgraphs)}</td>
</tr>"""

    return f"""
<header class="hero">
  <h1>pycfg-rs</h1>
  <p class="tagline">Fast intra-procedural control flow graph generation for Python.</p>
  <p class="tagline-sub">No runtime required. Text, JSON, and DOT output for humans, machines, and agents.</p>

  <nav class="page-nav">
    <a href="#get-started">Get started</a>
    <a href="#comparison">Comparison</a>
    <a href="#corpus">Corpus results</a>
    <a href="#why">Why CFGs?</a>
  </nav>

  <div class="section" id="get-started">
    <h2>Get started</h2>
    <pre class="code-block"><span class="prompt">$</span> git clone https://github.com/nwyin/pycfg-rs && cd pycfg-rs
<span class="prompt">$</span> cargo build --release
<span class="prompt">$</span> target/release/pycfg src/app/service.py::UserService.get_profile</pre>
  </div>

  <div class="example-block">
    <div class="example-side">
      <p class="example-label">Python source</p>
      <pre class="code-block">{escape(EXAMPLE_PYTHON)}</pre>
    </div>
    <div class="example-side">
      <p class="example-label">What pycfg-rs produces</p>
      <pre class="code-block">{escape(EXAMPLE_OUTPUT)}</pre>
    </div>
  </div>

  <div class="section" id="comparison">
    <h2>How it compares</h2>
    <p class="section-desc">
      Speed measured on the <a href="https://github.com/Textualize/rich">rich</a> corpus (100 files, 984 functions).
      * python-graphs only analyzed 13% of files due to import requirements.
      <a href="https://github.com/nwyin/pycfg-rs/blob/main/docs/roadmap.md">Roadmap and limitations.</a>
    </p>
    <table class="comparison-table">
      <thead>
        <tr>
          <th></th>
          <th class="highlight">pycfg-rs</th>
          <th><a href="https://github.com/coetaur0/staticfg">staticfg</a></th>
          <th><a href="https://github.com/google-research/python-graphs">python-graphs</a></th>
        </tr>
      </thead>
      <tbody>
        {comparison_rows}
      </tbody>
    </table>
  </div>

  <p class="sibling-link">
    For inter-procedural call graphs, see
    <a href="https://github.com/nwyin/pycg-rs">pycg-rs</a> &mdash;
    together they cover call graphs and control flow for Python static analysis.
  </p>
</header>"""


# ---------------------------------------------------------------------------
# Section 2: Corpus report
# ---------------------------------------------------------------------------


def _corpus_html(results: list[CorpusResult]) -> str:
    ok = [r for r in results if r.success]
    total_funcs = sum(r.functions for r in ok)
    total_time = sum(r.parse_time_ms for r in ok)
    throughput = total_funcs / (total_time / 1000) if total_time > 0 else 0
    total_files = sum(r.files for r in ok)
    success_count = len(ok)

    # Per-project accordion sections
    project_sections = ""
    for r in sorted(results, key=lambda r: r.functions, reverse=True):
        if not r.success:
            project_sections += f"""
<details class="project-accordion">
    <summary class="project-header">
        <span class="project-name">{r.name}</span>
        <span class="project-badge badge-fail">failed</span>
        <span class="project-error">{escape(r.error)}</span>
    </summary>
</details>"""
            continue

        avg_cc = r.total_cc / r.functions if r.functions > 0 else 0
        proj_throughput = r.functions / (r.parse_time_ms / 1000) if r.parse_time_ms > 0 else 0

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
                    <span class="func-name">{escape(f.name)}</span>
                    <span class="func-cc {cc_class(f.cyclomatic_complexity)}">CC {f.cyclomatic_complexity}</span>
                </div>
                <div class="func-meta">
                    <span class="func-file">{escape(short_file)}:{f.line}</span>
                    <span class="func-stats">{f.blocks} blocks, {f.edges} edges</span>
                </div>
                {cfg_section}
            </div>"""

        project_sections += f"""
<details class="project-accordion">
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
                <span class="mini-value">{proj_throughput:,.0f}</span>
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

    return f"""
<section class="section" id="corpus">
  <h2>Corpus results</h2>
  <p class="section-desc">
    Analysis of {len(results)} popular open-source Python projects, run on every push.
    Expand a project to see complexity distribution and CFG visualizations.
  </p>
  <div class="report-summary">
    <span class="stat">Throughput: <strong>{throughput:,.0f} functions/sec</strong> across {len(ok)} projects</span>
    <span class="stat">Total: <strong>{total_funcs:,}</strong> functions in <strong>{total_files}</strong> files</span>
    <span class="stat">Time: <strong>{total_time:.0f} ms</strong></span>
    <span class="stat">Coverage: <strong>{success_count}/{len(results)}</strong> projects passing</span>
  </div>
  {project_sections}
</section>"""


# ---------------------------------------------------------------------------
# Section 3: Essay
# ---------------------------------------------------------------------------


def _essay_html() -> str:
    return f"""
<section class="section why-essay" id="why">
  <h2>Why machine-readable CFGs matter for LLM workflows</h2>
  {WHY_ESSAY}
</section>"""


# ---------------------------------------------------------------------------
# Full page assembly
# ---------------------------------------------------------------------------


def generate_html(results: list[CorpusResult], version: str, sha: str) -> str:
    now = time.strftime("%Y-%m-%d %H:%M UTC", time.gmtime())

    hero = _hero_html()
    corpus = _corpus_html(results)
    essay = _essay_html()

    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>pycfg-rs &mdash; Fast Python CFG generator</title>
<style>
:root {{
    --bg: #0d1117;
    --surface: #161b22;
    --surface2: #1c2129;
    --border: #30363d;
    --text: #e6edf3;
    --text-muted: #8b949e;
    --accent: #58a6ff;
    --green: #3fb950;
    --orange: #d29922;
    --red: #f85149;
    --font: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
    --mono: "SFMono-Regular", Consolas, "Liberation Mono", Menlo, monospace;
}}
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{
    font-family: var(--font);
    background: var(--bg);
    color: var(--text);
    line-height: 1.6;
    padding: 2rem;
    max-width: 1100px;
    margin: 0 auto;
}}
a {{ color: var(--accent); text-decoration: none; }}
a:hover {{ text-decoration: underline; }}

/* Hero */
.hero {{ margin-bottom: 3rem; }}
h1 {{ font-size: 2rem; margin-bottom: 0.25rem; }}
.tagline {{ font-size: 1.125rem; color: var(--text); margin-bottom: 0.125rem; }}
.tagline-sub {{ font-size: 0.9375rem; color: var(--text-muted); margin-bottom: 1.5rem; }}

/* Nav */
.page-nav {{ margin-bottom: 2rem; display: flex; gap: 1.5rem; font-size: 0.875rem; }}
.page-nav a {{ color: var(--text-muted); }}
.page-nav a:hover {{ color: var(--accent); }}

/* Code blocks */
.code-block {{
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 1rem;
    font-family: var(--mono);
    font-size: 0.8125rem;
    line-height: 1.6;
    overflow-x: auto;
    white-space: pre;
}}
.code-block .prompt {{ color: var(--text-muted); }}

/* Example side-by-side */
.example-block {{
    display: flex;
    gap: 1rem;
    margin: 1.5rem 0 2rem;
}}
.example-side {{ flex: 1; min-width: 0; }}
.example-label {{
    color: var(--text-muted);
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    margin-bottom: 0.5rem;
}}
@media (max-width: 768px) {{
    .example-block {{ flex-direction: column; }}
}}

/* Sections */
.section {{ margin-top: 2.5rem; }}
.section h2 {{
    font-size: 1.125rem;
    margin-bottom: 0.5rem;
    padding-bottom: 0.5rem;
    border-bottom: 1px solid var(--border);
}}
.section-desc {{ color: var(--text-muted); font-size: 0.8125rem; margin-bottom: 1rem; line-height: 1.6; }}
.section-desc a {{ color: var(--accent); }}

/* Report summary stats */
.report-summary {{
    display: flex;
    gap: 2rem;
    margin-bottom: 1.5rem;
    font-size: 0.875rem;
    flex-wrap: wrap;
}}
.report-summary .stat {{ color: var(--text-muted); }}
.report-summary .stat strong {{ color: var(--text); font-family: var(--mono); }}

/* Tables */
table {{
    width: 100%;
    border-collapse: collapse;
    font-size: 0.875rem;
    margin-bottom: 1rem;
}}
th, td {{
    padding: 0.5rem 0.75rem;
    text-align: left;
    border-bottom: 1px solid var(--border);
}}
th {{
    color: var(--text-muted);
    font-weight: 500;
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    white-space: nowrap;
}}
td {{ font-family: var(--mono); font-size: 0.8125rem; }}
tr:hover {{ background: rgba(88,166,255,0.04); }}

/* Comparison table */
.comparison-table th.highlight,
.comparison-table td.highlight {{
    color: var(--green);
    font-weight: 600;
}}
.comparison-table th a {{ color: var(--text-muted); }}
.comparison-table th a:hover {{ color: var(--accent); }}

/* Sibling link */
.sibling-link {{
    color: var(--text-muted);
    font-size: 0.875rem;
    margin-top: 2rem;
    padding: 0.75rem 1rem;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 6px;
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
    color: var(--text-muted);
    transition: transform 0.15s;
}}
details[open] > .project-header::before {{ transform: rotate(90deg); }}
.project-name {{ font-weight: 600; font-size: 1rem; }}
.project-name a {{ color: var(--text); }}
.project-name a:hover {{ color: var(--accent); }}
.project-stat {{ color: var(--text-muted); font-size: 0.85rem; font-family: var(--mono); }}
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
.mini-label {{ font-size: 0.75rem; color: var(--text-muted); }}
.cc-distribution {{
    font-size: 0.8rem;
    padding: 0.5rem 0;
    display: flex;
    gap: 1rem;
    flex-wrap: wrap;
    align-items: center;
}}
.cc-dist-label {{ color: var(--text-muted); }}

.top-funcs-title {{
    font-size: 0.9rem;
    color: var(--text-muted);
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
.func-name {{ font-family: var(--mono); font-size: 0.9rem; font-weight: 600; }}
.func-cc {{ font-family: var(--mono); font-size: 0.85rem; font-weight: 700; white-space: nowrap; }}
.func-meta {{
    display: flex;
    gap: 1rem;
    font-size: 0.8rem;
    color: var(--text-muted);
    margin-top: 0.25rem;
}}
.func-file {{ font-family: var(--mono); }}
.func-stats {{ font-family: var(--mono); }}

/* CC heatmap */
.cc-high {{ color: var(--red); }}
.cc-med {{ color: var(--orange); }}
.cc-low {{ color: var(--green); }}

/* CFG visualization */
.cfg-toggle {{ margin-top: 0.5rem; }}
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
.cfg-container svg {{ max-width: 100%; height: auto; }}
.cfg-loading {{ color: #666; font-size: 0.85rem; padding: 0.5rem; }}

/* Why essay */
.why-essay p {{
    color: var(--text-muted);
    font-size: 0.9375rem;
    line-height: 1.7;
    max-width: 65ch;
    margin-bottom: 1rem;
}}
.why-essay strong {{ color: var(--text); }}
.why-essay em {{ color: var(--text); font-style: italic; }}
.why-essay code {{
    font-family: var(--mono);
    font-size: 0.85em;
    background: var(--surface);
    padding: 0.1em 0.35em;
    border-radius: 3px;
}}

/* Footer */
footer {{
    margin-top: 3rem;
    padding-top: 1rem;
    border-top: 1px solid var(--border);
    color: var(--text-muted);
    font-size: 0.75rem;
}}
footer a {{ color: var(--accent); }}
</style>
</head>
<body>

{hero}
{corpus}
{essay}

<footer>
  <a href="https://github.com/nwyin/pycfg-rs">pycfg-rs</a> v{version}
  &middot; commit <code>{sha}</code>
  &middot; generated {now}
  &middot; powered by <a href="https://github.com/astral-sh/ruff">ruff_python_parser</a>
</footer>

<script type="module">
import {{ Graphviz }} from "https://cdn.jsdelivr.net/npm/@hpcc-js/wasm@2.22.4/dist/graphviz.js";

const graphviz = await Graphviz.load();

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


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def main():
    parser = argparse.ArgumentParser(description="Generate pycfg-rs product page + corpus report")
    parser.add_argument("--output", "-o", default="report/index.html", help="Output HTML file")
    args = parser.parse_args()

    binary = find_binary()
    print(f"Using binary: {binary}")

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
    page_html = generate_html(results, version, sha)

    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(page_html)
    print(f"\nReport written to {output_path}")
    total_funcs = sum(r.functions for r in results if r.success)
    print(f"Total: {total_funcs:,} functions across {sum(r.files for r in results if r.success)} files")


if __name__ == "__main__":
    main()
