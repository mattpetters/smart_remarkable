"""Explicit provider order, bounded attempts, and no replay of possible actions."""
import json
from pathlib import Path
import re
import time

PROVIDERS = ("codex", "hermes", "claude")


def valid_models(models):
    return (isinstance(models, dict) and not set(models) - set(PROVIDERS)
            and all(isinstance(v, str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._/+:-]{0,95}", v)
                    for v in models.values()))


def valid_order(order):
    return (isinstance(order, list) and 1 <= len(order) <= 3
            and all(isinstance(p, str) and p in PROVIDERS for p in order)
            and len(set(order)) == len(order))


def configuration(server):
    try:
        data = json.loads(Path(server.backend_config).read_text())
        return {key: value for key, value in data.items() if key in PROVIDERS and isinstance(value, dict)} if isinstance(data, dict) else {}
    except (TypeError, OSError, ValueError):
        return {}


def catalog(server):
    config = configuration(server)
    defaults = {"codex": server.model or "default", "claude": config.get("claude", {}).get("model", "sonnet"),
                "hermes": config.get("hermes", {}).get("model", "unconfigured")}
    result = {}
    for provider in PROVIDERS:
        default = defaults[provider]
        if not valid_models({provider: default}):
            default = "unconfigured"
        choices = config.get(provider, {}).get("models", [])
        if not isinstance(choices, list):
            choices = []
        result[provider] = {"model": default, "models": list(dict.fromkeys(
            [default] + [model for model in choices if valid_models({provider: model})]))}
    return result


def route(server, prompt, images, backend, preferences, receipt):
    from codex_bridge import RequestError, validate_answer
    from claude_backend import run_claude
    from hermes_backend import run_hermes
    config = configuration(server)
    models = {p: c["model"] for p, c in catalog(server).items()}
    models.update(preferences.get("models", {}))
    # Legacy clients remain single-provider. Cross-provider disclosure requires
    # an explicit order from device settings, including Hermes -> cloud.
    order = [backend]
    if preferences.get("auto_fallback", False):
        order += [p for p in preferences.get("backend_order", []) if p != backend]
    deadline = time.monotonic() + server.timeout_seconds
    attempts = []
    for index, provider in enumerate(order):
        remaining = deadline - time.monotonic()
        if remaining < 1:
            break
        # Keep a useful slice for every remaining provider. Single-provider
        # requests can use the whole budget; an attempt never exceeds 150s.
        budget = min(150, remaining / max(1, len(order) - index)) if len(order) > 1 else remaining
        receipt["progress"] = {"provider": provider, "model": models[provider], "attempt": index + 1,
                               "total": len(order), "phase": "thinking"}
        try:
            if provider == "codex":
                answer = server.runner(prompt, images, mode=server.mode, model=None if models[provider] == "default" else models[provider],
                                       timeout=budget, executable=server.executable)
            elif provider == "claude":
                answer = run_claude(prompt, images, mode=server.mode, model=models[provider], timeout=budget)
            else:
                local = {**config.get("hermes", {}), "model": models[provider]}
                answer = run_hermes(prompt, images, mode=server.mode, timeout=budget, config=local)
            answer = validate_answer(answer, server.mode)
            receipt["progress"]["phase"] = "complete"
            return answer, {"provider": provider, "model": models[provider], "attempts": attempts}
        except RequestError as error:
            attempts.append({"provider": provider, "status": error.status})
            print(f"Backend attempt {index + 1}: {provider}, HTTP {error.status}, safe fallback={error.safe_to_fallback}", flush=True)
            if len(order) == 1:
                raise
            if not error.safe_to_fallback:
                raise RequestError(502, "Backend failed after possible tool actions; automatic replay stopped") from None
    raise RequestError(502, "All configured backends failed or timed out")
