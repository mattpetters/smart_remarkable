import json
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from agent_process import read_only_events, run_agent
from backend_router import catalog, route, valid_models, valid_order
from codex_bridge import Bridge, RequestError
from claude_backend import run_claude


class RoutingTests(unittest.TestCase):
    def setUp(self):
        self.server = Bridge(0, 't' * 48, mode='ink', model='test-model', timeout=360)
        self.addCleanup(self.server.server_close)
        self.prefs = {'auto_fallback': True, 'backend_order': ['codex', 'hermes', 'claude']}

    def fail(self, provider, safe=True):
        def run(*args, **kw):
            self.calls.append((provider, kw['timeout']))
            raise RequestError(504, 'Timed out', safe_to_fallback=safe)
        return run

    def test_explicit_order_reaches_third_provider_and_receipt_caches_whole_chain(self):
        self.calls = []
        self.server.runner = self.fail('codex')
        with patch('hermes_backend.run_hermes', side_effect=self.fail('hermes')), patch('claude_backend.run_claude', return_value={'lines':['Complete.']}) as claude:
            first = self.server.answer('fixture-key', 'digest', False, '', [b'png'], 'draw_answer', preferences=self.prefs)
            second = self.server.answer('fixture-key', 'digest', True, '', [b'png'], 'draw_answer', preferences=self.prefs)
        self.assertEqual(first, second)
        self.assertEqual(first[0], 200)
        self.assertEqual(first[1]['remarkable_backend']['provider'], 'claude')
        self.assertEqual([p for p,t in self.calls], ['codex', 'hermes'])
        self.assertTrue(all(0 < t <= 150 for p,t in self.calls))
        claude.assert_called_once()

    def test_actions_stop_fallback_and_all_failures_release_busy_lock(self):
        self.calls=[]; self.server.runner=self.fail('codex', safe=False)
        with patch('hermes_backend.run_hermes') as hermes, patch('claude_backend.run_claude') as claude:
            result=self.server.answer('actions-key','digest',False,'',[b'png'],'draw_answer',preferences=self.prefs)
        self.assertEqual(result[0],502)
        self.assertIn('possible tool actions',result[1]['error']['message'])
        hermes.assert_not_called();claude.assert_not_called()
        self.assertFalse(self.server.inference_lock.locked())

    def test_exhaustion_and_disabled_fallback(self):
        self.calls=[];self.server.runner=self.fail('codex')
        with patch('hermes_backend.run_hermes',side_effect=self.fail('hermes')), patch('claude_backend.run_claude',side_effect=self.fail('claude')):
            result=self.server.answer('exhausted-key','digest',False,'',[b'png'],'draw_answer',preferences=self.prefs)
        self.assertIn('All configured backends',result[1]['error']['message'])
        self.assertEqual([p for p,t in self.calls], ['codex','hermes','claude'])
        self.calls=[]
        self.server.answer('disabled-key','digest',False,'',[b'png'],'draw_answer',preferences={**self.prefs,'auto_fallback':False})
        self.assertEqual([p for p,t in self.calls], ['codex'])

    def test_model_and_order_validation(self):
        for order in ([],['codex','codex'],['cloud'],['codex',{}]): self.assertFalse(valid_order(order))
        self.assertTrue(valid_order(['hermes','claude','codex']))
        for models in ({'secret':'x'},{'claude':'--flag'},{'codex':'x\nsecret'},[]): self.assertFalse(valid_models(models))
        self.assertTrue(valid_models({'claude':'sonnet','hermes':'local/model-4bit'}))

    def test_claude_catalog_works_without_private_config_and_preserves_custom_models(self):
        defaults = catalog(self.server)['claude']
        self.assertEqual(defaults['model'], 'sonnet')
        for model in ('fable', 'haiku', 'opus', 'sonnet', 'claude-fable-5-1', 'claude-opus-5-5'):
            self.assertIn(model, defaults['models'])
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / 'backends.json'
            self.server.backend_config = config
            config.write_text(json.dumps({'claude': {'model': 'haiku',
                'models': ['haiku', 'claude-custom', '--flag', None]}}))
            result = catalog(self.server)['claude']
        self.assertEqual(result['model'], 'haiku')
        self.assertEqual(result['models'][0], 'haiku')
        self.assertEqual(result['models'].count('haiku'), 1)
        self.assertIn('claude-custom', result['models'])
        self.assertIn('fable', result['models'])
        self.assertNotIn('--flag', result['models'])
        self.assertNotIn(None, result['models'])

    def test_audit_detects_started_actions_and_malformed_events(self):
        events=[{'type':'thread.started'}, {'type':'item.started','item':{'type':'web_search'}}]
        self.assertTrue(read_only_events('\n'.join(map(json.dumps,events)),'codex'))
        events.append({'type':'item.started','item':{'type':'command_execution'}})
        self.assertFalse(read_only_events('\n'.join(map(json.dumps,events)),'codex'))
        for data in ('','invalid','null',json.dumps({'type':'new.event'})):
            self.assertFalse(read_only_events(data,'codex'))
        for name, safe in [('WebSearch',True),('Bash',False),('Edit',False),('unknown',False)]:
            data=json.dumps({'type':'assistant','message':{'content':[{'type':'tool_use','name':name}]}})
            self.assertEqual(read_only_events(data,'claude'),safe)

    def test_claude_native_images_and_structured_result(self):
        def run(command, **kw):
            data=json.loads(kw['input'])['message']['content']
            self.assertEqual(data[1]['source']['data'],'aW1hZ2U=')
            self.assertIn('--safe-mode',command);self.assertIn('--no-session-persistence',command)
            self.assertIn('--permission-prompts',command)
            return subprocess.CompletedProcess(command,0,json.dumps({'type':'result','subtype':'success','is_error':False,'structured_output':{'lines':['A useful answer.'],'illustrations':[]}}),'')
        with patch('claude_backend.run_agent',side_effect=run):
            result=run_claude('',[b'image'],mode='ink',model='sonnet',timeout=10)
        self.assertEqual(result['lines'],['A useful answer.'])

    def test_timeout_kills_child_before_it_can_commit_later(self):
        with tempfile.TemporaryDirectory() as directory:
            output=Path(directory)/'late-action'
            child='import time,pathlib; time.sleep(0.6); pathlib.Path('+repr(str(output))+').write_text("bad")'
            parent='import subprocess,sys,time; subprocess.Popen([sys.executable,"-c",'+repr(child)+']); time.sleep(30)'
            with self.assertRaises(subprocess.TimeoutExpired):
                run_agent([sys.executable,'-c',parent],timeout=0.2)
            time.sleep(0.7)
            self.assertFalse(output.exists())

if __name__=='__main__': unittest.main()
