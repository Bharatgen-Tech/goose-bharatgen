#!/usr/bin/env python3
"""Opt-in, low-output OpenAI Responses smoke tests. Never prints credentials."""
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


def request(payload):
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
        sys.exit(f"HTTP {error.code}: {body}")
    print(json.dumps({"model": data.get("model"), "status": data.get("status"),
                      "output_types": [item.get("type") for item in data.get("output", [])],
                      "usage": data.get("usage")}, indent=2))
    return data


print("Text request (billed):")
request({"model": model, "input": "Reply with OK.", "max_output_tokens": 64, "store": False})
print("Tool-call request (billed):")
response = request({
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
