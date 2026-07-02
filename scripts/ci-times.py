#!/usr/bin/env python3
"""Overview of recent CI runs and their job timings, via the GitHub CLI.

Usage:
  scripts/ci-times.py                 # last 10 runs of ci.yaml
  scripts/ci-times.py -n 5            # last 5 runs
  scripts/ci-times.py -b font-fixups  # only runs for a branch
  scripts/ci-times.py -w nightly.yaml # another workflow
  scripts/ci-times.py --no-jobs       # one line per run, no job breakdown

Requires an authenticated `gh` (https://cli.github.com).
"""

import argparse
import json
import subprocess
import sys
from datetime import datetime, timezone

GREEN, YELLOW, RED, DIM, BOLD, RESET = (
    "\033[32m", "\033[33m", "\033[31m", "\033[2m", "\033[1m", "\033[0m",
)


def gh(*args):
    result = subprocess.run(["gh", *args], capture_output=True, text=True)
    if result.returncode != 0:
        sys.exit(f"gh {' '.join(args)} failed:\n{result.stderr.strip()}")
    return result.stdout


def parse_ts(ts):
    return datetime.fromisoformat(ts.replace("Z", "+00:00")) if ts else None


def fmt_duration(seconds):
    if seconds is None:
        return "?"
    seconds = int(seconds)
    if seconds < 60:
        return f"{seconds}s"
    return f"{seconds // 60}m{seconds % 60:02d}s"


def duration_color(seconds, running=False):
    if seconds is None:
        return DIM
    if seconds >= 600:
        return RED
    if seconds >= 300:
        return YELLOW
    return DIM if running else GREEN


def conclusion_mark(status, conclusion):
    if status != "completed":
        return f"{YELLOW}●{RESET}"
    return {
        "success": f"{GREEN}✓{RESET}",
        "failure": f"{RED}✗{RESET}",
        "cancelled": f"{DIM}∅{RESET}",
        "skipped": f"{DIM}-{RESET}",
    }.get(conclusion, f"{YELLOW}?{RESET}")


def job_duration(job, now):
    start = parse_ts(job.get("started_at"))
    if start is None:
        return None, False
    end = parse_ts(job.get("completed_at"))
    if end is None:
        return (now - start).total_seconds(), True
    return (end - start).total_seconds(), False


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("-n", "--count", type=int, default=10, help="number of runs (default 10)")
    parser.add_argument("-w", "--workflow", default="ci.yaml", help="workflow file (default ci.yaml)")
    parser.add_argument("-b", "--branch", help="only show runs for this branch")
    parser.add_argument("--no-jobs", action="store_true", help="skip the per-job breakdown")
    args = parser.parse_args()

    list_args = [
        "run", "list", "--workflow", args.workflow, "--limit", str(args.count),
        "--json", "databaseId,headBranch,event,status,conclusion,createdAt,updatedAt,displayTitle",
    ]
    if args.branch:
        list_args += ["--branch", args.branch]
    runs = json.loads(gh(*list_args))
    if not runs:
        sys.exit("no runs found")

    now = datetime.now(timezone.utc)
    for run in runs:
        start = parse_ts(run["createdAt"])
        running = run["status"] != "completed"
        end = now if running else parse_ts(run["updatedAt"])
        total = (end - start).total_seconds()
        mark = conclusion_mark(run["status"], run["conclusion"])
        color = duration_color(total, running)
        suffix = " (running)" if running else ""
        print(
            f"{mark} {BOLD}{run['headBranch']}{RESET}"
            f" {DIM}{run['event']} · {start:%Y-%m-%d %H:%M} · #{run['databaseId']}{RESET}"
            f"  {color}{fmt_duration(total)}{suffix}{RESET}"
            f"  {DIM}{run['displayTitle'][:60]}{RESET}"
        )

        if args.no_jobs:
            continue
        jobs = json.loads(gh(
            "api", f"repos/{{owner}}/{{repo}}/actions/runs/{run['databaseId']}/jobs",
            "--paginate", "--jq", '{jobs: [.jobs[]]}',
        ))["jobs"]
        for job in sorted(jobs, key=lambda j: j["name"]):
            secs, job_running = job_duration(job, now)
            jmark = conclusion_mark(job["status"], job["conclusion"])
            jcolor = duration_color(secs, job_running)
            jsuffix = " (running)" if job_running else ""
            print(f"    {jmark} {job['name']:<24} {jcolor}{fmt_duration(secs)}{jsuffix}{RESET}")
        print()


if __name__ == "__main__":
    main()
