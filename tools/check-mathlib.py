#!/usr/bin/env python3
"""Read-only Mathlib sample check. Requires Lean/Lake and built import dependencies."""

import argparse
import concurrent.futures
import difflib
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--mathlib-dir", required=True, type=pathlib.Path)
parser.add_argument("--binary", default="leanwisp")
parser.add_argument("--output-dir", required=True, type=pathlib.Path)
parser.add_argument(
    "--lean-path", help="Override inherited LEAN_PATH with built dependency directories"
)
args = parser.parse_args()
ROOT = args.mathlib_dir.resolve()
OUT = args.output_dir.resolve()
if OUT == ROOT or ROOT in OUT.parents:
    parser.error("output directory must be outside Mathlib")
OUT.mkdir(parents=True, exist_ok=True)
BIN = shutil.which(args.binary)
if BIN is None:
    parser.error("formatter binary not found")
BIN = str(pathlib.Path(BIN).resolve())
PROJECT = OUT / "project"
PROJECT.mkdir(exist_ok=True)
(PROJECT / "lakefile.toml").write_text('name = "leanwisp_mathlib_eval"\n')
if args.lean_path is not None:
    os.environ["LEAN_PATH"] = args.lean_path
files = sorted(ROOT.glob("Mathlib/**/*.lean"))
if not files:
    parser.error("no Mathlib Lean sources found")
# Fixed coverage across foundational modules, algebra, analysis, topology and tactics.
wanted = [
    "Mathlib/Basic/Logic/Basic.lean",
    "Mathlib/Data/Nat/Basic.lean",
    "Mathlib/Algebra/Group/Basic.lean",
    "Mathlib/Data/List/Basic.lean",
    "Mathlib/Data/Set/Basic.lean",
    "Mathlib/Topology/Basic.lean",
    "Mathlib/Analysis/SpecificLimits/Basic.lean",
    "Mathlib/LinearAlgebra/Basic.lean",
    "Mathlib/NumberTheory/Divisors.lean",
    "Mathlib/Tactic/GCongr.lean",
    "Mathlib/Order/Basic.lean",
    "Mathlib/CategoryTheory/Category/Basic.lean",
]
selected = [ROOT / p for p in wanted if (ROOT / p).exists()]
for area in [
    "Basic",
    "Data",
    "Algebra",
    "Analysis",
    "Topology",
    "LinearAlgebra",
    "NumberTheory",
    "Tactic",
    "Order",
    "CategoryTheory",
]:
    candidates = sorted(
        (ROOT / "Mathlib" / area).rglob("*.lean"),
        key=lambda p: (p.stat().st_size, str(p)),
    )
    if candidates:
        for p in [candidates[0], candidates[len(candidates) // 2]]:
            if p not in selected:
                selected.append(p)
meta = {
    "MathlibCommit": subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
    ).strip(),
    "Formatter": BIN,
    "TotalLeanFiles": len(files),
    "Selected": len(selected),
    "Paths": [str(p.relative_to(ROOT)) for p in selected],
}
(OUT / "manifest.json").write_text(json.dumps(meta, indent=2))


def call(args, timeout=120):
    start = time.monotonic()
    try:
        p = subprocess.run(
            args,
            cwd=PROJECT,
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
        return {
            "Code": p.returncode,
            "Seconds": round(time.monotonic() - start, 3),
            "Stdout": p.stdout,
            "Stderr": p.stderr,
        }
    except subprocess.TimeoutExpired as e:
        return {
            "Code": "timeout",
            "Seconds": timeout,
            "Stdout": str(e.stdout or ""),
            "Stderr": str(e.stderr or ""),
        }


def work(path):
    rel = path.relative_to(ROOT)
    log = OUT / "logs" / rel.with_suffix("")
    log.parent.mkdir(parents=True, exist_ok=True)
    before = path.read_bytes()
    sha = hashlib.sha256(before).hexdigest()
    result = {"Path": str(rel), "Bytes": len(before), "OriginalSHA256": sha}
    baseline = call(["lake", "env", "lean", "--root=" + str(ROOT), str(path)])
    result["Baseline"] = baseline
    proc = call(
        [
            BIN,
            "source",
            "format-source",
            "--file",
            str(path),
            "--project-dir",
            str(PROJECT),
        ],
        90,
    )
    result["Format"] = proc
    if proc["Code"] == 0:
        data = json.loads(proc["Stdout"])
        formatted = data["Source"]
        proc["Stdout"] = "saved in formatted/"
        dest = OUT / "formatted" / rel
        dest.parent.mkdir(parents=True, exist_ok=True)
        dest.write_text(formatted)
        result["Changed"] = data["Changed"]
        result["Kept"] = data["Kept"]
        result["ChangedLines"] = sum(
            line[:1] in "+-" and line[:3] not in ("+++", "---")
            for line in difflib.unified_diff(
                before.decode().splitlines(), formatted.splitlines(), lineterm="", n=0
            )
            if not line.startswith("@@")
        )
        result["Compile"] = call(
            ["lake", "env", "lean", "--root=" + str(OUT / "formatted"), str(dest)]
        )
        again = call(
            [
                BIN,
                "--output",
                "text",
                "source",
                "format-source",
                "--file",
                str(dest),
                "--project-dir",
                str(PROJECT),
            ],
            90,
        )
        result["Idempotent"] = again["Code"] == 0 and again["Stdout"] == formatted
        result["SecondPassCode"] = again["Code"]
        result["LinesBefore"] = len(before.splitlines())
        result["LinesAfter"] = len(formatted.splitlines())
        result["Over100Before"] = sum(
            len(s) > 100 for s in before.decode().splitlines()
        )
        result["Over100After"] = sum(len(s) > 100 for s in formatted.splitlines())
    result["OriginalUnchanged"] = hashlib.sha256(path.read_bytes()).hexdigest() == sha
    log.with_suffix(".json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2)
    )
    print(
        json.dumps(
            {k: result[k] for k in ["Path", "Bytes", "OriginalUnchanged"]}
            | {
                "Baseline": baseline["Code"],
                "Format": proc["Code"],
                "Compile": result.get("Compile", {}).get("Code"),
                "Idempotent": result.get("Idempotent"),
                "ChangedLines": result.get("ChangedLines"),
                "Kept": len(result.get("Kept", [])),
            },
            ensure_ascii=False,
        ),
        flush=True,
    )
    return result


with concurrent.futures.ThreadPoolExecutor(max_workers=2) as ex:
    results = list(ex.map(work, selected))
(OUT / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2))
print(
    json.dumps(
        {
            "Count": len(results),
            "BaselineOK": sum(x["Baseline"]["Code"] == 0 for x in results),
            "FormatOK": sum(x["Format"]["Code"] == 0 for x in results),
            "CompileOK": sum(x.get("Compile", {}).get("Code") == 0 for x in results),
            "Unchanged": all(x["OriginalUnchanged"] for x in results),
            "ChangedLines": sum(x.get("ChangedLines", 0) for x in results),
            "SourceLines": sum(x.get("LinesBefore", 0) for x in results),
            "KeptCommands": sum(len(x.get("Kept", [])) for x in results),
        }
    ),
    flush=True,
)

raise SystemExit(
    0
    if all(
        x["Baseline"]["Code"] == 0
        and x["Format"]["Code"] == 0
        and x.get("Compile", {}).get("Code") == 0
        and x.get("Idempotent")
        and not x.get("Kept")
        and x["OriginalUnchanged"]
        for x in results
    )
    else 1
)
