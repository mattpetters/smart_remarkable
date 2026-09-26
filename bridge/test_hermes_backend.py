import base64
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from codex_bridge import RequestError
from hermes_backend import parse_answer, run_hermes
from hermes_worker import final_text


class HermesTests(unittest.TestCase):
    def test_only_structured_ink_replies_reach_the_tablet(self):
        with self.assertRaises(ValueError):
            parse_answer("The graph is displayed above.", "ink")
        self.assertEqual(parse_answer('```json\n{"lines":["Hello"],"illustrations":[]}\n```', "ink"),
                         {"lines":["Hello"],"illustrations":[]})
    def test_partial_or_interrupted_agent_turn_is_not_a_complete_answer(self):
        for flag, value in [("error", "failed"), ("failed", True), ("partial", True), ("interrupted", True), ("completed", False)]:
            with self.assertRaises(RuntimeError):
                final_text({"final_response": "Some unfinished text", flag: value})
        self.assertEqual(final_text({"completed": True, "final_response": "Complete."}), "Complete.")

    def test_local_images_are_native_and_worker_is_isolated(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/'venv/bin').mkdir(parents=True)
            (root/'venv/bin/python').touch()
            (root/'run_agent.py').touch()
            config = {'agent_path': str(root), 'model': 'local-vision'}
            def invoke(command, **kwargs):
                request = json.loads(Path(command[2]).read_text())
                self.assertEqual([base64.b64decode(item['image_url']['url'].split(',')[1])
                                  for item in request['content'][1:]], [b'selection', b'page'])
                self.assertEqual(request['base_url'], 'http://127.0.0.1:8000/v1')
                settings = json.loads((Path(kwargs['env']['HERMES_HOME'])/'config.yaml').read_text())
                self.assertTrue(settings['model']['supports_vision'])
                self.assertNotIn('OPENAI_API_KEY', kwargs['env'])
                Path(command[3]).write_text(json.dumps({'text': json.dumps({'lines':['One complete answer with a long source attribution.'], 'illustrations':[]})}))
                return subprocess.CompletedProcess(command, 0, b'', b'')
            with patch('hermes_backend.run_agent', side_effect=invoke):
                result = run_hermes('Reply pagination: enabled. Reply layout: use at most 128 lines, each at most 20 characters.',
                                    [b'selection', b'page'], mode='ink', timeout=180, config=config)
            self.assertTrue(all(len(line) <= 20 for line in result['lines']))
            self.assertEqual(' '.join(result['lines']), 'One complete answer with a long source attribution.')

    def test_missing_or_remote_backend_never_falls_back_to_cloud(self):
        for config in ({}, {'agent_path': '/missing', 'model': 'test', 'base_url': 'https://example.com/v1'}):
            with patch('hermes_backend.run_agent') as run:
                with self.assertRaises(RequestError) as error:
                    run_hermes('', [b'image'], mode='ink', timeout=180, config=config)
                self.assertEqual(error.exception.status, 503)
                run.assert_not_called()
