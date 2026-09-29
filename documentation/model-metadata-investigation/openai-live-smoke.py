#!/usr/bin/env python3
"""Opt-in, billed OpenAI Responses smoke tests. Never prints credentials or prompts."""
import json
import os
import sys
import urllib.error
import urllib.request

key = os.environ.get("OPENAI_API_KEY")
if not key:
    sys.exit("Set OPENAI_API_KEY in the environment; no requests were sent.")
model = os.environ.get("OPENAI_SMOKE_MODEL", "gpt-5.6-terra")
url = os.environ.get("OPENAI_SMOKE_BASE_URL", "https://api.openai.com/v1").rstrip("/")
if not url.startswith("https://"):
    sys.exit("OPENAI_SMOKE_BASE_URL must be HTTPS")
# The default tests inexpensive levels supported by the default model. Override
# for other models or to opt into potentially expensive high/max requests.
levels = [level.strip() for level in os.environ.get("OPENAI_SMOKE_EFFORTS", "none,low,medium").split(",")]
valid_levels = {"none", "minimal", "low", "medium", "high", "xhigh", "max"}
if not levels or any(level not in valid_levels for level in levels):
    sys.exit(f"OPENAI_SMOKE_EFFORTS must be comma-separated values from {sorted(valid_levels)}")


def request(label, payload):
    req = urllib.request.Request(
        url + "/responses",
        json.dumps(payload).encode(),
        {"Authorization": "Bearer " + key, "Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=90) as response:
            data = json.load(response)
    except urllib.error.HTTPError as error:
        body = error.read(2048).decode("utf-8", "replace")
        sys.exit(f"{label}: HTTP {error.code}: {body}")
    print(json.dumps({"case": label, "model": data.get("model"), "status": data.get("status"),
                      "output_types": [item.get("type") for item in data.get("output", [])],
                      "usage": data.get("usage")}, indent=2))
    if data.get("status") != "completed":
        sys.exit(f"{label}: expected completed status")
    return data


print(f"Running {2 + len(levels)} billed requests against {model}. Efforts: {levels}")
request("text", {"model": model, "input": "Reply with OK.", "max_output_tokens": 64, "store": False})
response = request("tool-call", {
    "model": model, "input": "Call the ping function with value test.", "store": False,
    "max_output_tokens": 128,
    "tools": [{"type": "function", "name": "ping", "description": "Echo a value",
               "parameters": {"type": "object", "properties": {"value": {"type": "string"}},
                              "required": ["value"]}}],
    "tool_choice": "required",
})
if not any(item.get("type") == "function_call" for item in response.get("output", [])):
    sys.exit("Expected a function_call output")
print("Tool call verified. No tool was executed.")

for level in levels:
    response = request(f"effort={level}", {
        "model": model, "input": "Reply with OK.", "max_output_tokens": 128,
        "store": False, "reasoning": {"effort": level},
    })
    if not response.get("output"):
        sys.exit(f"effort={level}: expected nonempty output")
print("Requested effort levels were accepted; API usage does not prove how much reasoning occurred.")
