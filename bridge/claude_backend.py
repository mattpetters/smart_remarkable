"""Isolated Claude Code vision turns using the Mac's existing CLI login."""
import base64
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

from agent_process import run_agent, read_only_events


def claude_executable():
    return shutil.which("claude") or str(Path.home() / ".local/bin/claude")


def run_claude(prompt, images, *, mode, model, timeout, executable=None):
    from codex_bridge import RequestError, answer_schema, answer_instructions, validate_answer, format_answer_sources
    content = [{"type": "text", "text": answer_instructions(prompt, images, mode)}]
    content += [{"type": "image", "source": {"type": "base64", "media_type": "image/png",
                 "data": base64.b64encode(png).decode()}} for png in images]
    message = {"type": "user", "message": {"role": "user", "content": content}}
    command = [executable or claude_executable(), "-p", "--safe-mode", "--no-session-persistence",
               "--setting-sources", "", "--strict-mcp-config", "--mcp-config", '{"mcpServers":{}}',
               "--permission-mode", "bypassPermissions", "--permission-prompts", "none",
               "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
               "--json-schema", json.dumps(answer_schema(mode)), "--model", model or "sonnet"]
    with tempfile.TemporaryDirectory(prefix="remarkable-claude-") as directory:
        try:
            result = run_agent(command, input=json.dumps(message) + "\n", cwd=directory, timeout=timeout)
        except subprocess.TimeoutExpired as error:
            raise RequestError(504, "Claude timed out", safe_to_fallback=read_only_events(error.stdout, "claude")) from None
        except OSError:
            raise RequestError(503, "Claude executable is unavailable", safe_to_fallback=True) from None
        safe = read_only_events(result.stdout, "claude")
        try:
            events = [json.loads(line) for line in (result.stdout or "").splitlines()]
            final = next(event for event in reversed(events) if event.get("type") == "result")
            if result.returncode or final.get("is_error") or final.get("subtype") != "success":
                raise ValueError("Incomplete result")
            answer = validate_answer(format_answer_sources(final["structured_output"], mode, prompt), mode)
        except (ValueError, TypeError, KeyError, StopIteration, RequestError):
            raise RequestError(502, "Claude did not complete a valid answer", safe_to_fallback=safe) from None
        return answer
