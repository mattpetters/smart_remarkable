#!/usr/bin/env python3
"""Local OpenAI-compatible selection adapter for the authenticated Codex CLI.

Listens only on loopback. Forward it to the tablet over SSH, not an open LAN port.
The tablet receives text or answer lines, never executable tool instructions.
"""
import argparse
import base64
import binascii
from datetime import datetime
import hmac
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

MAX_BODY = 16 * 1024 * 1024
MAX_IMAGE = 10 * 1024 * 1024
PNG_PREFIX = "data:image/png;base64,"


class RequestError(Exception):
    def __init__(self, status, message):
        self.status, self.message = status, message


def selection_input(body):
    """Only accept the upstream client's text and inline PNG format."""
    if not isinstance(body, dict) or body.get("stream"):
        raise RequestError(400, "Expected a non-streaming JSON request")
    messages = body.get("messages")
    if not isinstance(messages, list) or not 1 <= len(messages) <= 8:
        raise RequestError(400, "Expected 1-8 messages")
    texts, images = [], []
    for message in messages:
        if not isinstance(message, dict):
            raise RequestError(400, "Invalid message")
        content = message.get("content")
        if isinstance(content, str):
            texts.append(content)
            continue
        if not isinstance(content, list):
            raise RequestError(400, "Invalid message content")
        for item in content:
            if not isinstance(item, dict):
                raise RequestError(400, "Invalid content item")
            if item.get("type") == "text" and isinstance(item.get("text"), str):
                texts.append(item["text"])
            elif item.get("type") == "image_url":
                image = item.get("image_url")
                url = image.get("url", "") if isinstance(image, dict) else ""
                if not isinstance(url, str) or not url.startswith(PNG_PREFIX):
                    raise RequestError(400, "Only inline PNG images are accepted")
                try:
                    raw = base64.b64decode(url[len(PNG_PREFIX):], validate=True)
                except (binascii.Error, ValueError):
                    raise RequestError(400, "Invalid PNG base64") from None
                if not raw.startswith(b"\x89PNG\r\n\x1a\n") or len(raw) > MAX_IMAGE:
                    raise RequestError(400, "Invalid or oversized PNG")
                images.append(raw)
            else:
                raise RequestError(400, "Unsupported content type")
    if len(images) != 1 or sum(map(len, texts)) > 32000:
        raise RequestError(400, "Expected one selection image and at most 32000 text characters")
    tools = body.get("tools", [])
    if not isinstance(tools, list):
        raise RequestError(400, "Invalid tools")
    names = {
        t.get("function", {}).get("name") for t in tools
        if isinstance(t, dict) and isinstance(t.get("function"), dict)
        and isinstance(t["function"].get("name"), str)
    }
    return "\n\n".join(texts), images[0], names


def validate_answer(answer, mode):
    if not isinstance(answer, dict):
        raise RequestError(502, "Codex did not return an answer object")
    if mode == "text":
        text = answer.get("text")
        if set(answer) != {"text"} or not isinstance(text, str) or not text.strip() or len(text) > 1600:
            raise RequestError(502, "Codex returned invalid answer text")
        if any(c != "\n" and not 32 <= ord(c) <= 126 for c in text):
            raise RequestError(502, "Native typing currently supports printable ASCII and newlines only")
    else:
        lines = answer.get("lines")
        if set(answer) != {"lines"} or not isinstance(lines, list) or not 1 <= len(lines) <= 8:
            raise RequestError(502, "Codex returned invalid answer lines")
        if any(not isinstance(s, str) or not s.strip() or len(s) > 40 for s in lines):
            raise RequestError(502, "Codex answer lines exceed the page layout")
    return answer


def run_codex(prompt, png, *, mode, model, timeout, executable):
    prop = {"type": "string"} if mode == "text" else {"type": "array", "items": {"type": "string"}}
    field = "text" if mode == "text" else "lines"
    schema = {"type": "object", "properties": {field: prop}, "required": [field], "additionalProperties": False}
    instructions = (
        "You answer a user's handwritten selection from a reMarkable tablet. "
        f"Current local date and time on the Mac: {datetime.now().astimezone().isoformat()}. "
        "Read the attached image and answer its question. Do not run tools, inspect files, or change anything. "
        "If handwriting is ambiguous, ask a short clarification rather than guessing. "
        "This is a small e-ink page: plain text only, no Markdown styling, no preamble. "
        + ("Answer in at most 80 words and 1600 characters, using ASCII characters only. " if mode == "text" else
           "Return 1-8 short lines, each at most 26 characters. ")
        + "The client context below describes the selection. Return the answer in the requested JSON schema, "
        "not a tool call.\n\nClient context:\n" + prompt
    )
    with tempfile.TemporaryDirectory(prefix="remarkable-codex-") as temp:
        root = Path(temp)
        image, output, schema_path = root / "selection.png", root / "answer.json", root / "schema.json"
        image.write_bytes(png)
        schema_path.write_text(json.dumps(schema))
        command = [executable, "exec", "--ignore-user-config", "--ephemeral", "--skip-git-repo-check",
                   "--sandbox", "read-only", "-c", 'approval_policy="never"', "--cd", temp,
                   "--color", "never", "--image", str(image), "--output-schema", str(schema_path),
                   "--output-last-message", str(output)]
        if model:
            command.extend(["--model", model])
        command.append("-")
        try:
            result = subprocess.run(command, input=instructions, text=True, capture_output=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            raise RequestError(504, "Codex timed out; please retry the selection") from None
        except OSError:
            raise RequestError(503, "Codex executable is unavailable") from None
        if result.returncode or not output.exists():
            # CLI logs can contain prompt/image content or account details; do not serve them.
            raise RequestError(502, "Codex failed; check codex login status and model availability on the Mac")
        try:
            answer = json.loads(output.read_text())
        except (ValueError, OSError):
            raise RequestError(502, "Codex returned malformed JSON") from None
        return validate_answer(answer, mode)


class Bridge(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, port, token, *, mode="text", model=None, timeout=180, executable="codex", runner=run_codex):
        super().__init__(("127.0.0.1", port), Handler)
        self.token, self.mode, self.model = token, mode, model
        self.timeout_seconds, self.executable, self.runner = timeout, executable, runner
        self.inference_lock = threading.Lock()


class Handler(BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        self.connection.settimeout(15)

    def log_message(self, fmt, *args):
        pass  # No handwritten content, bearer tokens, or request bodies in logs.

    def reply(self, status, body):
        raw = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        try:
            self.wfile.write(raw)
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_GET(self):
        if self.path == "/health":
            self.reply(200, {"status": "ready", "backend": "codex", "response_mode": self.server.mode})
        else:
            self.reply(404, {"error": {"message": "Not found"}})

    def do_POST(self):
        server = self.server
        started = time.monotonic()
        try:
            if self.path != "/v1/chat/completions":
                raise RequestError(404, "Not found")
            auth = self.headers.get("Authorization", "")
            if not hmac.compare_digest(auth.encode(), ("Bearer " + server.token).encode()):
                raise RequestError(401, "Unauthorized")
            if self.headers.get("Transfer-Encoding"):
                raise RequestError(400, "Chunked requests are unsupported")
            try:
                size = int(self.headers.get("Content-Length", "0"))
            except ValueError:
                raise RequestError(400, "Invalid Content-Length") from None
            if not 0 < size <= MAX_BODY:
                raise RequestError(413, "Invalid request size")
            raw = self.rfile.read(size)
            if len(raw) != size:
                raise RequestError(400, "Incomplete request body")
            try:
                body = json.loads(raw)
            except (ValueError, UnicodeDecodeError):
                raise RequestError(400, "Invalid JSON") from None
            prompt, png, names = selection_input(body)
            tool = "draw_text" if server.mode == "text" else "draw_answer"
            if tool not in names:
                raise RequestError(400, "Client did not register " + tool)
            if not server.inference_lock.acquire(blocking=False):
                raise RequestError(429, "An answer is already being generated")
            try:
                answer = server.runner(prompt, png, mode=server.mode, model=server.model,
                                       timeout=server.timeout_seconds, executable=server.executable)
                validate_answer(answer, server.mode)
            finally:
                server.inference_lock.release()
            self.reply(200, {"id": "remarkable-" + str(time.time_ns()), "object": "chat.completion",
                             "created": int(time.time()), "model": "codex",
                             "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {
                                 "role": "assistant", "content": None, "tool_calls": [{
                                     "id": "answer", "type": "function", "function": {
                                         "name": tool, "arguments": json.dumps(answer)}}]}}]})
            print(f"Answered selection in {time.monotonic() - started:.1f}s ({server.mode})", flush=True)
        except RequestError as e:
            self.reply(e.status, {"error": {"message": e.message}})
        except (TimeoutError, ConnectionError):
            self.close_connection = True
        except Exception:
            self.reply(500, {"error": {"message": "Internal bridge error"}})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--token-file", type=Path, required=True)
    parser.add_argument("--mode", choices=["text", "ink"], default="text")
    parser.add_argument("--model", default=os.environ.get("REMARKABLE_CODEX_MODEL"))
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--codex", default="codex")
    args = parser.parse_args()
    if args.token_file.stat().st_mode & 0o077:
        parser.error("Token file must be private (chmod 600)")
    token = args.token_file.read_text().strip()
    if len(token) < 32 or not token.isascii() or any(c.isspace() for c in token):
        parser.error("Token must be at least 32 ASCII characters with no whitespace")
    server = Bridge(args.port, token, mode=args.mode, model=args.model, timeout=args.timeout, executable=args.codex)
    print(f"Codex bridge listening on 127.0.0.1:{server.server_port} ({args.mode})", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
