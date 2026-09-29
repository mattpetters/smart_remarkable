#!/usr/bin/env python3
"""One isolated Hermes turn. Run with the Python from a Hermes installation."""
import json
import os
from pathlib import Path
import sys


def final_text(result):
    response = result.get("final_response")
    if (result.get("error") or result.get("failed") or result.get("partial")
            or result.get("interrupted") or result.get("completed") is False
            or not isinstance(response, str) or not response.strip()):
        raise RuntimeError("Hermes did not complete an answer")
    return response


def main():
    request_path, output_path, agent_path = map(Path, sys.argv[1:4])
    request = json.loads(request_path.read_text())
    sys.path.insert(0, str(agent_path))
    # The parent supplies a temporary HERMES_HOME, so personal memories, plugins,
    # gateways and background jobs are not loaded into a notebook conversation.
    from run_agent import AIAgent
    key = "local"
    if request.get("omlx_settings"):
        data = json.loads(Path(request["omlx_settings"]).expanduser().read_text())
        key = data.get("auth", {}).get("api_key") or key
    audit = Path(request["audit_path"])
    audit.write_text("read-only")
    def tool_start(call_id, name, args):
        if name not in ("web_search", "web_extract"):
            try:
                with audit.open("w") as handle:
                    handle.write("actions-possible")
                    handle.flush()
                    os.fsync(handle.fileno())
            except OSError:
                # Hermes swallows callback exceptions. Exit before the tool
                # executes if we cannot persist the conservative action guard.
                os._exit(70)
    agent = AIAgent(
        model=request["model"], base_url=request["base_url"], api_key=key,
        provider="custom", api_mode="chat_completions",
        max_iterations=12, max_tokens=4096, run_budget_seconds=request["timeout"] - 10,
        tool_start_callback=tool_start,
        enabled_toolsets=["web", "terminal", "file"],
        skip_context_files=True, skip_memory=True, skip_background_review=True,
        save_trajectories=False, quiet_mode=True,
        reasoning_config={"enabled": False},
        request_overrides={"extra_body": {"chat_template_kwargs": {"enable_thinking": False}}},
    )
    try:
        result = agent.run_conversation(request["content"], system_message=request["instructions"])
        response = final_text(result)
        output_path.write_text(json.dumps({"text": response}))
    finally:
        agent.close()


if __name__ == "__main__":
    main()
