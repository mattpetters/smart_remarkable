import base64
import json
from pathlib import Path
import subprocess
import threading
import tempfile
import unittest
from unittest.mock import patch
import urllib.error
import urllib.request

from codex_bridge import Bridge, PNG_PREFIX, RequestError, cli_failure_reason, format_answer_sources, run_codex, selection_input, validate_answer, web_lookup_count

PNG = b"\x89PNG\r\n\x1a\nfixture"
PAGE = b"\x89PNG\r\n\x1a\npage-context"


def request_body():
    return {"model": "codex", "messages": [{"role": "user", "content": [
        {"type": "text", "text": "Answer the handwriting"},
        {"type": "image_url", "image_url": {"url": PNG_PREFIX + base64.b64encode(PNG).decode()}},
    ]}], "tools": [{"type": "function", "function": {"name": "draw_text"}}]}


class IllustrationTests(unittest.TestCase):
    def test_drawings_survive_validation_and_reflow_without_executable_fields(self):
        from illustrations import validate_illustrations
        drawing = {"title": "An illustrative plot", "strokes": [[{"x": 40, "y": 30}, {"x": 40, "y": 300}]],
                   "labels": [{"x": 80, "y": 330, "text": "Time (s)"}]}
        answer = {"lines": ["A complete explanation."], "illustrations": [drawing]}
        self.assertEqual(validate_answer(format_answer_sources(answer, "ink", ""), "ink"), answer)
        for point in ({"x": -1, "y": 20}, {"x": float("nan"), "y": 20}, {"x": True, "y": 20}):
            invalid = json.loads(json.dumps(drawing)); invalid["strokes"][0][0] = point
            with self.assertRaises(ValueError): validate_illustrations([invalid])
        for change in ({"svg": "<script/>"}, {"labels": [{"x": 580, "y": 330, "text": "Too wide"}]}):
            with self.assertRaises(ValueError): validate_illustrations([{**drawing, **change}])
        with self.assertRaises(ValueError): validate_illustrations([drawing]*3)


class BridgeTests(unittest.TestCase):
    def setUp(self):
        self.calls = []
        def runner(prompt, png, **kwargs):
            self.calls.append((prompt, png, kwargs))
            return {"text": "Four."}
        self.server = Bridge(0, "t" * 48, runner=runner)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.url = f"http://127.0.0.1:{self.server.server_port}"

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    def post(self, body, token="t" * 48, key=None, retry=False):
        data = body if isinstance(body, bytes) else json.dumps(body).encode()
        headers = {"Authorization": "Bearer " + token, "Content-Type": "application/json"}
        if key: headers["Idempotency-Key"] = key
        if retry: headers["X-Remarkable-Retry"] = "1"
        req = urllib.request.Request(self.url + "/v1/chat/completions", data=data, headers=headers)
        try:
            response = urllib.request.urlopen(req)
        except urllib.error.HTTPError as e:
            response = e
        with response:
            return response.status, json.load(response)

    def test_backend_preferences_route_locally_without_codex_fallback(self):
        body = request_body()
        body["remarkable_settings"] = {"backend": "hermes"}
        self.assertEqual(self.post(body)[0], 503)
        self.assertFalse(self.calls)
        body["remarkable_settings"] = {"backend": "unknown"}
        self.assertEqual(self.post(body)[0], 400)
        self.assertFalse(self.calls)

    def test_hermes_result_is_cached_under_the_same_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "backends.json"
            config.write_text(json.dumps({"hermes": {"model": "local-vision"}}))
            self.server.backend_config = config
            body = request_body()
            body["remarkable_settings"] = {"backend": "hermes"}
            with patch("hermes_backend.run_hermes", return_value={"text": "Local answer."}) as runner:
                first = self.post(body, key="hermes-request")
                second = self.post(body, key="hermes-request", retry=True)
            self.assertEqual(first, second)
            self.assertEqual(first[0], 200)
            runner.assert_called_once()
            self.assertFalse(self.calls)

    def test_context_preference_removes_page_before_runner(self):
        body = request_body()
        body["messages"][0]["content"].append({"type": "image_url", "image_url": {"url": PNG_PREFIX + base64.b64encode(PAGE).decode()}})
        body["remarkable_settings"] = {"page_context": False, "reply_length": "detailed"}
        self.assertEqual(self.post(body)[0], 200)
        self.assertEqual(self.calls[-1][1], [PNG])
        self.assertIn("Include useful explanation and examples", self.calls[-1][0])

    def test_selection_round_trip(self):
        status, body = self.post(request_body())
        self.assertEqual(status, 200)
        call = body["choices"][0]["message"]["tool_calls"][0]["function"]
        self.assertEqual(call["name"], "draw_text")
        self.assertEqual(json.loads(call["arguments"]), {"text": "Four."})
        self.assertEqual(self.calls[0][1], [PNG])

    def test_selection_and_page_keep_their_order_without_carrying_old_context(self):
        body = request_body()
        body["messages"][0]["content"].extend([
            {"type": "text", "text": "Image 2: visible page context"},
            {"type": "image_url", "image_url": {"url": PNG_PREFIX + base64.b64encode(PAGE).decode()}},
        ])
        self.assertEqual(self.post(body)[0], 200)
        self.assertEqual(self.calls[-1][1], [PNG, PAGE])
        self.assertEqual(self.post(request_body())[0], 200)
        self.assertEqual(self.calls[-1][1], [PNG])

    def test_rejects_extra_or_remote_context_images(self):
        body = request_body()
        image = body["messages"][0]["content"][1]
        body["messages"][0]["content"].extend([image, image])
        self.assertEqual(self.post(body)[0], 400)
        body = request_body()
        body["messages"][0]["content"].append({"type": "image_url", "image_url": {"url": "https://example.com/page.png"}})
        self.assertEqual(self.post(body)[0], 400)
        self.assertFalse(self.calls)

    def test_rejects_bad_auth_before_inference(self):
        self.assertEqual(self.post(request_body(), "wrong")[0], 401)
        self.assertFalse(self.calls)

    def test_rejects_remote_image_urls(self):
        body = request_body()
        body["messages"][0]["content"][1]["image_url"]["url"] = "http://127.0.0.1/private"
        self.assertEqual(self.post(body)[0], 400)
        self.assertFalse(self.calls)

    def test_rejects_malformed_json_and_shapes(self):
        for body in (b"{", [], {"messages": [None]}, {"messages": [{"content": [None]}]}):
            self.assertEqual(self.post(body)[0], 400)

    def test_rejects_bad_base64_and_missing_images(self):
        body = request_body()
        body["messages"][0]["content"][1]["image_url"]["url"] = PNG_PREFIX + "invalid!!"
        self.assertEqual(self.post(body)[0], 400)
        body["messages"][0]["content"].pop()
        self.assertEqual(self.post(body)[0], 400)

    def test_rejects_unregistered_output_tool(self):
        body = request_body()
        body["tools"] = []
        self.assertEqual(self.post(body)[0], 400)

    def test_serializes_device_inference(self):
        self.server.inference_lock.acquire()
        try:
            self.assertEqual(self.post(request_body())[0], 429)
            self.assertFalse(self.calls)
        finally:
            self.server.inference_lock.release()

    def test_backend_failure_releases_lock(self):
        def fail(*args, **kwargs):
            raise RequestError(504, "Timed out")
        self.server.runner = fail
        self.assertEqual(self.post(request_body())[0], 504)
        self.assertFalse(self.server.inference_lock.locked())

    def test_rejects_oversize_answer(self):
        self.server.runner = lambda *a, **kw: {"text": "x" * 1601}
        self.assertEqual(self.post(request_body())[0], 502)

    def test_retry_retrieves_same_answer_without_repeating_tools(self):
        first = self.post(request_body(), key="request-0001")
        second = self.post(request_body(), key="request-0001", retry=True)
        self.assertEqual(first, second)
        self.assertEqual(len(self.calls), 1)
        altered = request_body(); altered["model"] = "changed"
        self.assertEqual(self.post(altered, key="request-0001", retry=True)[0], 409)
        self.assertEqual(len(self.calls), 1)

    def test_unknown_or_expired_retry_never_restarts_an_action(self):
        self.assertEqual(self.post(request_body(), key="request-unknown", retry=True)[0], 409)
        self.assertFalse(self.calls)
        self.post(request_body(), key="request-expired")
        self.server.receipts["request-expired"]["created"] -= 1801
        self.assertEqual(self.post(request_body(), key="request-expired", retry=True)[0], 409)
        self.assertEqual(len(self.calls), 1)

    def test_concurrent_retry_joins_the_running_invocation(self):
        entered, release = threading.Event(), threading.Event()
        def runner(*args, **kwargs):
            self.calls.append(1); entered.set()
            self.assertTrue(release.wait(3))
            return {"text": "Complete answer."}
        self.server.runner = runner
        results = []
        first = threading.Thread(target=lambda: results.append(self.post(request_body(), key="request-running")))
        first.start(); self.assertTrue(entered.wait(2))
        timer = threading.Timer(0.1, release.set); timer.start()
        second = self.post(request_body(), key="request-running", retry=True)
        first.join(3); timer.join()
        self.assertEqual(results, [second])
        self.assertEqual(second[0], 200)
        self.assertEqual(len(self.calls), 1)

    def test_failed_invocation_is_not_reexecuted_by_transport_retry(self):
        def runner(*args, **kwargs):
            self.calls.append(1)
            raise RequestError(504, "Timed out")
        self.server.runner = runner
        self.assertEqual(self.post(request_body(), key="request-failed")[0], 504)
        self.assertEqual(self.post(request_body(), key="request-failed", retry=True)[0], 504)
        self.assertEqual(len(self.calls), 1)


class CodexTests(unittest.TestCase):
    def test_failure_diagnostics_do_not_echo_cli_output(self):
        self.assertEqual(cli_failure_reason('401 unauthorized: PRIVATE_PROMPT'), 'authentication')
        self.assertEqual(cli_failure_reason('rate limit: PRIVATE_ACCOUNT'), 'usage_limit')
        self.assertEqual(cli_failure_reason('unexpected PRIVATE_CONTENT'), 'unclassified')
        self.assertEqual(cli_failure_reason(None), 'unclassified')

    def test_page_context_reaches_codex_and_temporary_images_are_removed(self):
        images = []
        def process(command, **kwargs):
            images.extend(Path(command[i + 1]) for i, arg in enumerate(command) if arg == "--image")
            self.assertEqual([path.read_bytes() for path in images], [PNG, PAGE])
            self.assertIn("previous assistant responses", kwargs["input"])
            self.assertIn("off-screen", kwargs["input"])
            self.assertIn("--ephemeral", command)
            output = Path(command[command.index("--output-last-message") + 1])
            output.write_text('{"lines":["Basil and mint."]}')
            return subprocess.CompletedProcess(command, 0)
        with patch("codex_bridge.subprocess.run", side_effect=process):
            run_codex("Continue the visible conversation", [PNG, PAGE], mode="ink", model="test", timeout=12, executable="codex")
        self.assertTrue(all(not path.exists() for path in images))

    def test_subprocess_enables_live_search_and_authorized_full_tool_access(self):
        def process(command, **kwargs):
            self.assertIsInstance(command, list)
            self.assertNotIn("shell", kwargs)
            self.assertEqual(command[command.index("--sandbox") + 1], "danger-full-access")
            self.assertLess(command.index("--search"), command.index("exec"))
            self.assertNotIn("--disable", command)
            self.assertIn("--json", command)
            self.assertIn("Before saying you do not know an external fact, try a focused web lookup", kwargs["input"])
            self.assertIn("short source name/domain", kwargs["input"])
            self.assertIn("selected user writing explicitly requests", kwargs["input"])
            self.assertNotIn("Do not run tools", kwargs["input"])
            self.assertEqual(command[-1], "-")
            self.assertIn("Answer", kwargs["input"])
            image = Path(command[command.index("--image") + 1])
            self.assertEqual(image.read_bytes(), PNG)
            output = Path(command[command.index("--output-last-message") + 1])
            output.write_text('{"text":"Four."}')
            return subprocess.CompletedProcess(command, 0)
        with patch("codex_bridge.subprocess.run", side_effect=process):
            result = run_codex("Answer", [PNG], mode="text", model="test", timeout=12, executable="codex")
        self.assertEqual(result, {"text": "Four."})

    def test_web_activity_counts_completed_lookups_without_returning_content(self):
        events = [
            {"type": "item.started", "item": {"type": "web_search", "query": "PRIVATE_QUERY"}},
            {"type": "item.completed", "item": {"type": "web_search", "query": "PRIVATE_QUERY"}},
            {"type": "item.completed", "item": {"type": "agent_message", "text": "PRIVATE_ANSWER"}},
            {"type": "item.completed", "item": None},
            [],
        ]
        self.assertEqual(web_lookup_count("\n".join(json.dumps(e) for e in events)), 1)
        self.assertEqual(web_lookup_count("not json\n{}\nnull"), 0)
        self.assertEqual(web_lookup_count(None), 0)

    def test_web_sources_fit_the_page_without_discarding_the_answer(self):
        raw = {"lines": ["The answer.", "Source: [Product specifications](https://example.com/products/a/long/path)"]}
        answer = format_answer_sources(raw, "ink", "Reply layout: use at most 4 lines, each at most 32 characters.")
        self.assertEqual(answer["lines"][0], "The answer.")
        self.assertIn("Product specifications (example.com)", " ".join(answer["lines"]))
        self.assertTrue(all(len(line) <= 32 for line in answer["lines"]))
        validate_answer(answer, "ink")
        with self.assertRaises(RequestError):
            format_answer_sources(raw, "ink", "Reply layout: use at most 2 lines, each at most 12 characters.")
        self.assertEqual(format_answer_sources({"lines": [42]}, "ink", ""), {"lines": [42]})
        spaced = format_answer_sources({"lines": ["First.", "", "Second."]}, "ink",
                                       "Reply layout: use at most 2 lines, each at most 52 characters.")
        self.assertEqual(validate_answer(spaced, "ink"), {"lines": ["First.", "Second."]})

    def test_timeout_becomes_gateway_timeout(self):
        with patch("codex_bridge.subprocess.run", side_effect=subprocess.TimeoutExpired("codex", 1)):
            with self.assertRaises(RequestError) as raised:
                run_codex("Answer", [PNG], mode="text", model=None, timeout=1, executable="codex")
        self.assertEqual(raised.exception.status, 504)

    def test_ink_layout_bounds(self):
        validate_answer({"lines": ["A short answer"]}, "ink")
        for lines in ([], ["a"] * 129, ["x" * 57], [42], ["", "  "]):
            with self.assertRaises(RequestError):
                validate_answer({"lines": lines}, "ink")

    def test_paginated_answer_reflows_without_losing_any_content(self):
        raw = {"lines": [f"Part {i}: the complete explanation is preserved even when this model line is too wide." for i in range(30)]}
        answer = format_answer_sources(raw, "ink", "Reply pagination: enabled. Reply layout: use at most 128 lines, each at most 40 characters.")
        validate_answer(answer, "ink")
        self.assertGreater(len(answer["lines"]), 16)
        self.assertTrue(all(len(line) <= 40 for line in answer["lines"]))
        self.assertEqual(" ".join(answer["lines"]), " ".join(raw["lines"]))
        with self.assertRaises(RequestError):
            format_answer_sources(raw, "ink", "Reply layout: use at most 4 lines, each at most 40 characters.")

    def test_blank_separators_do_not_discard_a_valid_answer(self):
        answer = {"lines": ["First paragraph.", "", "  ", "Second paragraph."]}
        self.assertEqual(validate_answer(answer, "ink"), {"lines": ["First paragraph.", "Second paragraph."]})

    def test_native_typing_rejects_controls_and_unmapped_characters(self):
        for text in ("\x1b", "hello\x08", "caf\u00e9"):
            with self.assertRaises(RequestError):
                validate_answer({"text": text}, "text")

    def test_ink_completion_selects_draw_answer(self):
        server = Bridge(0, "t" * 48, mode="ink", runner=lambda *a, **kw: {"lines": ["September 26, 2026", "", "  "]})
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            body = request_body()
            body["tools"][0]["function"]["name"] = "draw_answer"
            req = urllib.request.Request(f"http://127.0.0.1:{server.server_port}/v1/chat/completions",
                                         data=json.dumps(body).encode(), headers={"Authorization": "Bearer " + "t" * 48})
            with urllib.request.urlopen(req) as response:
                function = json.load(response)["choices"][0]["message"]["tool_calls"][0]["function"]
            self.assertEqual(function["name"], "draw_answer")
            self.assertEqual(json.loads(function["arguments"]), {"lines": ["September 26, 2026"]})
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


if __name__ == "__main__":
    unittest.main()
