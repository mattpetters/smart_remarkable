import base64
import json
from pathlib import Path
import subprocess
import threading
import unittest
from unittest.mock import patch
import urllib.error
import urllib.request

from codex_bridge import Bridge, PNG_PREFIX, RequestError, run_codex, selection_input, validate_answer

PNG = b"\x89PNG\r\n\x1a\nfixture"


def request_body():
    return {"model": "codex", "messages": [{"role": "user", "content": [
        {"type": "text", "text": "Answer the handwriting"},
        {"type": "image_url", "image_url": {"url": PNG_PREFIX + base64.b64encode(PNG).decode()}},
    ]}], "tools": [{"type": "function", "function": {"name": "draw_text"}}]}


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

    def post(self, body, token="t" * 48):
        data = body if isinstance(body, bytes) else json.dumps(body).encode()
        req = urllib.request.Request(self.url + "/v1/chat/completions", data=data,
                                     headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"})
        try:
            response = urllib.request.urlopen(req)
        except urllib.error.HTTPError as e:
            response = e
        with response:
            return response.status, json.load(response)

    def test_selection_round_trip(self):
        status, body = self.post(request_body())
        self.assertEqual(status, 200)
        call = body["choices"][0]["message"]["tool_calls"][0]["function"]
        self.assertEqual(call["name"], "draw_text")
        self.assertEqual(json.loads(call["arguments"]), {"text": "Four."})
        self.assertEqual(self.calls[0][1], PNG)

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


class CodexTests(unittest.TestCase):
    def test_subprocess_uses_image_schema_stdin_and_read_only(self):
        def process(command, **kwargs):
            self.assertIsInstance(command, list)
            self.assertNotIn("shell", kwargs)
            self.assertEqual(command[command.index("--sandbox") + 1], "read-only")
            self.assertEqual(command[-1], "-")
            self.assertIn("Answer", kwargs["input"])
            image = Path(command[command.index("--image") + 1])
            self.assertEqual(image.read_bytes(), PNG)
            output = Path(command[command.index("--output-last-message") + 1])
            output.write_text('{"text":"Four."}')
            return subprocess.CompletedProcess(command, 0)
        with patch("codex_bridge.subprocess.run", side_effect=process):
            result = run_codex("Answer", PNG, mode="text", model="test", timeout=12, executable="codex")
        self.assertEqual(result, {"text": "Four."})

    def test_timeout_becomes_gateway_timeout(self):
        with patch("codex_bridge.subprocess.run", side_effect=subprocess.TimeoutExpired("codex", 1)):
            with self.assertRaises(RequestError) as raised:
                run_codex("Answer", PNG, mode="text", model=None, timeout=1, executable="codex")
        self.assertEqual(raised.exception.status, 504)

    def test_ink_layout_bounds(self):
        validate_answer({"lines": ["A short answer"]}, "ink")
        for lines in ([], ["a"] * 9, ["x" * 41], [42]):
            with self.assertRaises(RequestError):
                validate_answer({"lines": lines}, "ink")

    def test_native_typing_rejects_controls_and_unmapped_characters(self):
        for text in ("\x1b", "hello\x08", "caf\u00e9"):
            with self.assertRaises(RequestError):
                validate_answer({"text": text}, "text")

    def test_ink_completion_selects_draw_answer(self):
        server = Bridge(0, "t" * 48, mode="ink", runner=lambda *a, **kw: {"lines": ["September 26, 2026"]})
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
