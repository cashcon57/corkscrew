#!/usr/bin/env python3
"""
Corkscrew LLM Chat — End-to-End Test Suite

Tests the MLX (or Ollama) LLM server directly to verify:
1. Server is running and responsive
2. Simple prompts get content responses
3. Tool-calling prompts produce <tool_call> blocks
4. Streaming works correctly
5. Context window is large enough for system prompt + tools

Usage:
    python3 /tmp/corkscrew-llm-e2e-test.py [--port 8080] [--ollama]
"""

import argparse
import json
import sys
import time
import urllib.request
import urllib.error

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

MODEL_ID = "mlx-community/Qwen3.5-27B-4bit"  # auto-detected in main()

PASS = "\033[92m✓ PASS\033[0m"
FAIL = "\033[91m✗ FAIL\033[0m"
WARN = "\033[93m⚠ WARN\033[0m"
INFO = "\033[94mℹ INFO\033[0m"

# Minimal system prompt matching Corkscrew's actual prompt
SYSTEM_PROMPT = """You are an expert Bethesda game modder built into Corkscrew, a mod manager for Wine/CrossOver on macOS and Linux. You have deep knowledge of Skyrim Special Edition modding and direct tool access to the mod manager.

Game: Skyrim Special Edition | Platform: macOS (Wine) | Mods installed: 42 | Page: Mods

## Rules
- You have web_search for general research and search_nexus for searching NexusMods directly.
- NEVER fabricate mod names, Nexus IDs, or URLs. Use search_nexus or web_search to verify.
- Use tools proactively — look things up rather than guessing.
- Max 5 tool calls per response. Give a final answer, don't loop.
- Be concise.

## Available tools
You can call tools by writing a <tool_call> block. Format:
<tool_call>
{"name": "tool_name", "arguments": {"arg": "value"}}
</tool_call>

You may call multiple tools. Available tools:

### search_nexus
Search NexusMods for mods matching a query.
Parameters: {"type":"object","properties":{"query":{"type":"string","description":"Search query"},"game":{"type":"string","description":"Game domain, default skyrimspecialedition"}},"required":["query"]}

### web_search
Search the web for information.
Parameters: {"type":"object","properties":{"query":{"type":"string","description":"The search query"}},"required":["query"]}

### list_mods
List all installed mods with enabled/disabled status.
Parameters: {"type":"object","properties":{"filter":{"type":"string","description":"Optional search filter"}}}

### get_mod_info
Get detailed info about a mod.
Parameters: {"type":"object","properties":{"mod_name":{"type":"string","description":"The mod name"}},"required":["mod_name"]}

### enable_mod
Enable a mod by name.
Parameters: {"type":"object","properties":{"mod_name":{"type":"string","description":"The name of the mod to enable"}},"required":["mod_name"]}

### disable_mod
Disable a mod by name.
Parameters: {"type":"object","properties":{"mod_name":{"type":"string","description":"The name of the mod to disable"}},"required":["mod_name"]}
"""

TOOL_DEFINITIONS = [
    {
        "type": "function",
        "function": {
            "name": "search_nexus",
            "description": "Search NexusMods for mods.",
            "parameters": {
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"]
            }
        }
    },
    {
        "type": "function",
        "function": {
            "name": "web_search",
            "description": "Search the web.",
            "parameters": {
                "type": "object",
                "properties": {"query": {"type": "string"}},
                "required": ["query"]
            }
        }
    },
    {
        "type": "function",
        "function": {
            "name": "list_mods",
            "description": "List installed mods.",
            "parameters": {
                "type": "object",
                "properties": {"filter": {"type": "string"}}
            }
        }
    },
    {
        "type": "function",
        "function": {
            "name": "enable_mod",
            "description": "Enable a mod by name.",
            "parameters": {
                "type": "object",
                "properties": {"mod_name": {"type": "string"}},
                "required": ["mod_name"]
            }
        }
    },
]

# ---------------------------------------------------------------------------
# Test infrastructure
# ---------------------------------------------------------------------------

results = []

def test(name, passed, detail=""):
    status = PASS if passed else FAIL
    results.append((name, passed, detail))
    print(f"  {status} {name}")
    if detail and not passed:
        print(f"        {detail[:200]}")

def send_request(base_url, messages, stream=False, tools=None, model=None):
    """Send a chat completion request and return the response."""
    body = {
        "model": model or MODEL_ID,
        "messages": messages,
        "temperature": 0.7,
        "max_tokens": 2048,
        "stream": stream,
        "chat_template_kwargs": {"enable_thinking": False},
    }
    if tools:
        body["tools"] = tools

    data = json.dumps(body).encode()
    req = urllib.request.Request(
        f"{base_url}/v1/chat/completions",
        data=data,
        headers={"Content-Type": "application/json"},
    )

    try:
        with urllib.request.urlopen(req, timeout=120) as resp:
            if stream:
                return resp.read().decode()
            else:
                return json.loads(resp.read().decode())
    except urllib.error.HTTPError as e:
        error_body = e.read().decode() if e.fp else ""
        return {"error": f"HTTP {e.code}: {error_body[:500]}"}
    except Exception as e:
        return {"error": str(e)}

def parse_sse_content(raw):
    """Parse SSE stream into accumulated content."""
    content = []
    reasoning = []
    tool_call_chunks = {}

    for line in raw.split("\n"):
        line = line.strip()
        if line.startswith(":"):
            continue  # keepalive comment
        if not line.startswith("data: "):
            continue
        payload = line[6:]
        if payload == "[DONE]":
            break
        try:
            obj = json.loads(payload)
            delta = obj.get("choices", [{}])[0].get("delta", {})

            if "content" in delta and delta["content"]:
                content.append(delta["content"])
            if "reasoning" in delta and delta["reasoning"]:
                reasoning.append(delta["reasoning"])
            if "tool_calls" in delta:
                for tc in delta["tool_calls"]:
                    idx = tc.get("index", 0)
                    if idx not in tool_call_chunks:
                        tool_call_chunks[idx] = {"name": "", "arguments": ""}
                    fn = tc.get("function", {})
                    if "name" in fn:
                        tool_call_chunks[idx]["name"] += fn["name"]
                    if "arguments" in fn:
                        tool_call_chunks[idx]["arguments"] += fn["arguments"]
        except json.JSONDecodeError:
            continue

    return {
        "content": "".join(content),
        "reasoning": "".join(reasoning),
        "tool_calls": list(tool_call_chunks.values()) if tool_call_chunks else None,
    }

def parse_tool_calls_from_text(text):
    """Parse <tool_call> blocks from text (MLX text-based tool calling)."""
    calls = []
    while "<tool_call>" in text:
        start = text.find("<tool_call>")
        after = start + len("<tool_call>")
        end = text.find("</tool_call>", after)
        if end == -1:
            break
        json_str = text[after:end].strip()
        try:
            obj = json.loads(json_str)
            calls.append({
                "name": obj.get("name", ""),
                "arguments": obj.get("arguments", {}),
            })
        except json.JSONDecodeError:
            pass
        text = text[:start] + text[end + len("</tool_call>"):]
    return calls, text.strip()

# ---------------------------------------------------------------------------
# Test cases
# ---------------------------------------------------------------------------

def test_server_health(base_url):
    """Test 1: Server is running and responsive."""
    global MODEL_ID
    print("\n━━━ Test 1: Server Health ━━━")
    try:
        req = urllib.request.Request(f"{base_url}/v1/models")
        with urllib.request.urlopen(req, timeout=5) as resp:
            body = json.loads(resp.read().decode())
            models = [m["id"] for m in body.get("data", [])]
            test("Server responds to /v1/models", True)
            test("At least one model loaded", len(models) > 0, f"Models: {models}")
            if models:
                MODEL_ID = models[0]
                print(f"  {INFO} Using model: {MODEL_ID}")
            print(f"  {INFO} Models: {', '.join(models)}")
            return True
    except Exception as e:
        test("Server responds", False, str(e))
        return False

def test_simple_prompt(base_url):
    """Test 2: Simple prompt gets a content response."""
    print("\n━━━ Test 2: Simple Prompt (non-streaming) ━━━")
    messages = [
        {"role": "system", "content": "You are a helpful assistant. Reply in one sentence."},
        {"role": "user", "content": "What is SKSE?"},
    ]
    t0 = time.time()
    resp = send_request(base_url, messages, stream=False)
    elapsed = time.time() - t0

    if "error" in resp:
        test("Request succeeds", False, resp["error"])
        return

    test("Request succeeds", True)

    content = ""
    choices = resp.get("choices", [])
    if choices:
        msg = choices[0].get("message", {})
        content = msg.get("content", "") or ""

    test("Response has content", len(content.strip()) > 0, f"Got: '{content[:100]}'")
    test("Response mentions SKSE/Skyrim", any(w in content.lower() for w in ["skse", "skyrim", "script"]),
         f"Content: '{content[:100]}'")
    print(f"  {INFO} Response ({elapsed:.1f}s): {content[:150]}")

def test_system_prompt_fits(base_url):
    """Test 3: Full system prompt + tools fit in context."""
    print("\n━━━ Test 3: System Prompt + Tools Fit ━━━")
    messages = [
        {"role": "system", "content": SYSTEM_PROMPT},
        {"role": "user", "content": "Hello, what can you help me with?"},
    ]
    t0 = time.time()
    resp = send_request(base_url, messages, stream=False, tools=TOOL_DEFINITIONS)
    elapsed = time.time() - t0

    if "error" in resp:
        test("Full system prompt fits in context", False, resp["error"])
        return

    content = ""
    choices = resp.get("choices", [])
    if choices:
        content = choices[0].get("message", {}).get("content", "") or ""

    test("Full system prompt fits in context", len(content.strip()) > 0,
         f"Content length: {len(content)}")
    test("Response time < 60s", elapsed < 60, f"Took {elapsed:.1f}s")
    print(f"  {INFO} Response ({elapsed:.1f}s): {content[:150]}")

def test_tool_calling_nonstream(base_url):
    """Test 4: Tool-calling prompt produces tool calls (non-streaming)."""
    print("\n━━━ Test 4: Tool Calling (non-streaming) ━━━")
    messages = [
        {"role": "system", "content": SYSTEM_PROMPT},
        {"role": "user", "content": "Find me a mod that lets me summon a horse I'm immediately riding"},
    ]
    t0 = time.time()
    resp = send_request(base_url, messages, stream=False, tools=TOOL_DEFINITIONS)
    elapsed = time.time() - t0

    if "error" in resp:
        test("Tool-calling request succeeds", False, resp["error"])
        return

    test("Tool-calling request succeeds", True)

    content = ""
    native_tool_calls = None
    choices = resp.get("choices", [])
    if choices:
        msg = choices[0].get("message", {})
        content = msg.get("content", "") or ""
        native_tool_calls = msg.get("tool_calls")

    # Check for native tool calls (OpenAI format)
    has_native = native_tool_calls and len(native_tool_calls) > 0

    # Check for text-based tool calls (<tool_call> blocks)
    text_calls, clean_text = parse_tool_calls_from_text(content)
    has_text = len(text_calls) > 0

    has_any_tool_call = has_native or has_text

    test("Response contains tool call(s)", has_any_tool_call,
         f"Native: {has_native}, Text-based: {has_text}")

    if has_native:
        for tc in native_tool_calls:
            fn = tc.get("function", {})
            print(f"  {INFO} Native tool call: {fn.get('name')}({fn.get('arguments')})")
            test(f"Tool '{fn.get('name')}' is a known tool",
                 fn.get("name") in ["search_nexus", "web_search", "list_mods", "get_mod_info"],
                 f"Got: {fn.get('name')}")

    if has_text:
        for tc in text_calls:
            print(f"  {INFO} Text tool call: {tc['name']}({json.dumps(tc['arguments'])})")
            test(f"Tool '{tc['name']}' is a known tool",
                 tc["name"] in ["search_nexus", "web_search", "list_mods", "get_mod_info"],
                 f"Got: {tc['name']}")

    if not has_any_tool_call:
        print(f"  {INFO} Full response: {content[:300]}")
        # Check if model tried but format was wrong
        if "search" in content.lower() or "nexus" in content.lower():
            print(f"  {WARN} Model mentioned searching but didn't use tool format")

    print(f"  {INFO} Elapsed: {elapsed:.1f}s")

def test_streaming(base_url):
    """Test 5: Streaming works and produces content."""
    print("\n━━━ Test 5: Streaming ━━━")
    messages = [
        {"role": "system", "content": "Reply in one sentence."},
        {"role": "user", "content": "What is a mod manager?"},
    ]
    t0 = time.time()
    raw = send_request(base_url, messages, stream=True)
    elapsed = time.time() - t0

    if isinstance(raw, dict) and "error" in raw:
        test("Streaming request succeeds", False, raw["error"])
        return

    test("Streaming request succeeds", True)

    parsed = parse_sse_content(raw)
    content = parsed["content"]
    reasoning = parsed["reasoning"]

    test("Stream produces content tokens", len(content) > 0,
         f"Content: {len(content)} chars, Reasoning: {len(reasoning)} chars")

    if not content and reasoning:
        test("Content in reasoning field (thinking mode leak)", False,
             "Model outputting to reasoning instead of content — thinking mode not disabled")

    has_data_lines = raw.count("data: ") > 1
    test("Multiple SSE data lines (real streaming)", has_data_lines,
         f"Found {raw.count('data: ')} data lines")

    print(f"  {INFO} Response ({elapsed:.1f}s): {content[:150]}")

def test_streaming_tool_call(base_url):
    """Test 6: Streaming with tool-calling prompt."""
    print("\n━━━ Test 6: Streaming + Tool Calling ━━━")
    messages = [
        {"role": "system", "content": SYSTEM_PROMPT},
        {"role": "user", "content": "Search nexus for a lighting overhaul mod"},
    ]
    t0 = time.time()
    raw = send_request(base_url, messages, stream=True, tools=TOOL_DEFINITIONS)
    elapsed = time.time() - t0

    if isinstance(raw, dict) and "error" in raw:
        test("Streaming tool request succeeds", False, raw["error"])
        return

    test("Streaming tool request succeeds", True)

    parsed = parse_sse_content(raw)
    content = parsed["content"]
    reasoning = parsed["reasoning"]
    native_calls = parsed["tool_calls"]

    # Check text-based tool calls in content
    text_calls, clean = parse_tool_calls_from_text(content)

    has_native = native_calls and any(c["name"] for c in native_calls)
    has_text = len(text_calls) > 0

    test("Streaming response has tool call(s)", has_native or has_text,
         f"Native: {native_calls}, Text: {text_calls}")

    if has_native:
        for tc in native_calls:
            print(f"  {INFO} Native: {tc['name']}({tc['arguments']})")
    if has_text:
        for tc in text_calls:
            print(f"  {INFO} Text: {tc['name']}({json.dumps(tc['arguments'])})")

    if not has_native and not has_text:
        print(f"  {INFO} Content: {content[:300]}")
        if reasoning:
            print(f"  {WARN} Reasoning: {reasoning[:200]}")

    print(f"  {INFO} Elapsed: {elapsed:.1f}s")

def test_context_not_empty_response(base_url):
    """Test 7: Verify no empty responses on complex prompts."""
    print("\n━━━ Test 7: No Empty Responses ━━━")
    messages = [
        {"role": "system", "content": SYSTEM_PROMPT},
        {"role": "user", "content": "I want to make Skyrim look beautiful with ENB and weather mods. What do you recommend?"},
    ]
    t0 = time.time()
    resp = send_request(base_url, messages, stream=False, tools=TOOL_DEFINITIONS)
    elapsed = time.time() - t0

    if "error" in resp:
        test("Complex prompt doesn't error", False, resp["error"])
        return

    content = ""
    choices = resp.get("choices", [])
    if choices:
        msg = choices[0].get("message", {})
        content = msg.get("content", "") or ""
        tool_calls = msg.get("tool_calls")
        has_tool = tool_calls and len(tool_calls) > 0
    else:
        has_tool = False

    has_something = len(content.strip()) > 0 or has_tool
    test("Response is not empty", has_something,
         f"Content: {len(content)} chars, Tools: {has_tool}")

    # Check for the specific "not enough context" pattern
    text_calls, _ = parse_tool_calls_from_text(content)
    has_any = len(content.strip()) > 0 or has_tool or len(text_calls) > 0
    test("Response has meaningful content or tool calls", has_any)

    print(f"  {INFO} Response ({elapsed:.1f}s): {content[:200]}")

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    parser = argparse.ArgumentParser(description="Corkscrew LLM E2E Tests")
    parser.add_argument("--port", type=int, default=8080)
    parser.add_argument("--ollama", action="store_true", help="Test Ollama instead of MLX")
    args = parser.parse_args()

    if args.ollama:
        base_url = f"http://localhost:11434/v1"
    else:
        base_url = f"http://localhost:{args.port}"

    backend = "Ollama" if args.ollama else "MLX"
    print(f"\n{'='*60}")
    print(f"  Corkscrew LLM E2E Test Suite — {backend} @ {base_url}")
    print(f"{'='*60}")

    if not test_server_health(base_url):
        print(f"\n{FAIL} Server not reachable. Aborting.")
        sys.exit(1)

    test_simple_prompt(base_url)
    test_system_prompt_fits(base_url)
    test_tool_calling_nonstream(base_url)
    test_streaming(base_url)
    test_streaming_tool_call(base_url)
    test_context_not_empty_response(base_url)

    # Summary
    passed = sum(1 for _, p, _ in results if p)
    failed = sum(1 for _, p, _ in results if not p)
    total = len(results)

    print(f"\n{'='*60}")
    print(f"  Results: {passed}/{total} passed, {failed} failed")
    print(f"{'='*60}")

    if failed > 0:
        print(f"\nFailed tests:")
        for name, p, detail in results:
            if not p:
                print(f"  {FAIL} {name}: {detail}")

    sys.exit(0 if failed == 0 else 1)

if __name__ == "__main__":
    main()
