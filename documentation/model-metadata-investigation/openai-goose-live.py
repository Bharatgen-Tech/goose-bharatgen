#!/usr/bin/env python3
"""Run billed end-to-end OpenAI smoke cases through the goose CLI."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
GOOSE = Path(os.environ.get("GOOSE_SMOKE_BINARY", str(ROOT / "target/debug/goose"))).resolve()
MODEL = os.environ.get("OPENAI_SMOKE_MODEL", "gpt-5.6-terra")
LEVELS = [level.strip() for level in os.environ.get("OPENAI_SMOKE_EFFORTS", "low,medium").split(",")]
VALID = {"off", "low", "medium", "high", "max"}
if not os.environ.get("OPENAI_API_KEY"):
    sys.exit("Export OPENAI_API_KEY first; no requests sent.")
if not GOOSE.is_file():
    sys.exit(f"No goose binary at {GOOSE}. Build with cargo build -p goose-cli, or set GOOSE_SMOKE_BINARY.")
if not LEVELS or any(level not in VALID for level in LEVELS):
    sys.exit(f"OPENAI_SMOKE_EFFORTS must be comma-separated goose levels from {sorted(VALID)}")


def run_case(label, prompt, env, expect_tool=False):
    print(f"Running {label} (billed)...", flush=True)
    command = [str(GOOSE), "run", "--provider", "openai", "--model", MODEL,
               "--no-profile", "--no-session", "--output-format", "stream-json",
               "--max-turns", "3", "--text", prompt]
    if expect_tool:
        command += ["--with-builtin", "developer"]
    try:
        result = subprocess.run(command, cwd=env["GOOSE_PATH_ROOT"], env=env,
                                capture_output=True, text=True, timeout=180, check=False)
    except subprocess.TimeoutExpired:
        sys.exit(f"{label}: goose timed out after 180s")
    if result.returncode:
        sys.exit(f"{label}: goose exited {result.returncode}\n{result.stderr[-3000:]}\n{result.stdout[-3000:]}")
    try:
        events = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
    except json.JSONDecodeError:
        sys.exit(f"{label}: goose did not produce stream-json\n{result.stderr[-1500:]}\n{result.stdout[-1500:]}")
    errors = [event.get("error") for event in events if event.get("type") == "error"]
    completions = [event for event in events if event.get("type") == "complete"]
    if errors or not completions:
        sys.exit(f"{label}: missing completion or reported error: {errors}\n{result.stderr[-1500:]}")
    content = [block for event in events if event.get("type") == "message"
               for block in event["message"].get("content", [])]
    requests = [block for block in content if block.get("type") == "toolRequest"]
    responses = [block for block in content if block.get("type") == "toolResponse"]
    if expect_tool and (not requests or not responses):
        sys.exit(f"{label}: missing tool request or response in goose stream (requests={len(requests)}, responses={len(responses)})")
    if expect_tool and not any("GOOSE_OPENAI_TOOL_OK" in json.dumps(block) for block in responses):
        sys.exit(f"{label}: shell output marker was not present in the tool response")
    if expect_tool and not any("GOOSE_OPENAI_TOOL_OK" in json.dumps(block)
                               for event in events if event.get("type") == "message"
                               and event["message"].get("role") == "assistant"
                               for block in event["message"].get("content", []) if block.get("type") == "text"):
        sys.exit(f"{label}: assistant did not report the tool result after replay")
    usage = completions[-1]
    print(f"{label}: completed; {len(requests)} tool request(s), {len(responses)} tool response(s); "
          f"tokens={usage.get('total_tokens')}; cost_usd={usage.get('cost_usd')}")


with tempfile.TemporaryDirectory(prefix="goose-openai-smoke-") as directory:
    env = os.environ.copy()
    env.update({"GOOSE_PATH_ROOT": directory, "GOOSE_MODE": "auto", "OPENAI_HOST": "https://api.openai.com",
                "OPENAI_STORE": "false", "GOOSE_DISABLE_KEYRING": "1"})
    env.pop("GOOSE_ADDITIONAL_CONFIG_FILES", None)
    # The test requires the implicit native Responses path, not a persisted Chat override.
    for setting in ("OPENAI_BASE_URL", "OPENAI_BASE_PATH", "GOOSE_MAX_TOKENS"):
        env.pop(setting, None)
    run_case("streamed-text", "Reply exactly: GOOSE_OPENAI_SMOKE_OK", env)
    for level in LEVELS:
        env["GOOSE_THINKING_EFFORT"] = level
        run_case(f"thinking-{level}", "Reply briefly with the word OK.", env)
    env.pop("GOOSE_THINKING_EFFORT", None)
    run_case("tool-replay", "Use the developer shell tool to execute exactly `printf GOOSE_OPENAI_TOOL_OK` "
             "and then report its output. Do not use any other commands or tools.", env, expect_tool=True)
print("All goose OpenAI smoke cases completed. Inspect request logs separately to confirm wire parameters.")
