"""Bound CLI lifetimes, including child processes, without logging private IO."""
import os
import signal
import subprocess


def run_agent(command, *, timeout, input=None, text=True, **kwargs):
    with subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=text, start_new_session=True,
                          **kwargs) as process:
        try:
            stdout, stderr = process.communicate(input, timeout=timeout)
        except subprocess.TimeoutExpired:
            # Kill the entire request process group before considering fallback.
            # CLI tools can otherwise keep acting after their parent times out.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            stdout, stderr = process.communicate()
            raise subprocess.TimeoutExpired(command, timeout, output=stdout, stderr=stderr) from None
        return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)


def read_only_events(output, provider):
    """Fail closed when a failed CLI may already have taken external actions."""
    import json
    seen = False
    for line in (output or "").splitlines():
        try:
            event = json.loads(line)
            kind = event["type"]
            if provider == "codex":
                if kind.startswith("item."):
                    if event["item"]["type"] not in ("web_search", "agent_message", "reasoning"):
                        return False
                elif kind not in ("thread.started", "turn.started", "turn.completed", "turn.failed", "error"):
                    return False
            else:
                if kind == "assistant":
                    for block in event["message"]["content"]:
                        if block.get("type") == "tool_use" and block.get("name") not in ("WebSearch", "WebFetch", "StructuredOutput"):
                            return False
                        if block.get("type") not in ("text", "thinking", "redacted_thinking", "tool_use"):
                            return False
                elif kind not in ("system", "user", "result", "rate_limit_event"):
                    return False
            seen = True
        except (ValueError, KeyError, TypeError, AttributeError):
            return False
    return seen
