import json
import os
from pathlib import Path
import plistlib
import socket
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from lan_service import LABEL, Service, UNIT, Unavailable


def result(output="", code=0):
    return subprocess.CompletedProcess([], code, stdout=output, stderr="")


class AvailabilityTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = patch.dict(os.environ, {"REMARKABLE_HOST": "test-tablet",
                                          "REMARKABLE_CODEX_BIN": "/test/codex",
                                          "REMARKABLE_RESPONSE_MODE": "ink",
                                          "REMARKABLE_CODEX_MODEL": "test-model"})
        self.env.start()
        self.addCleanup(self.env.stop)
        self.service = Service(repo=self.root / "r", home=self.root / "h")

    def test_overlay_recovery_failure_does_not_start_listener(self):
        commands = []
        def ssh(command, **kwargs):
            commands.append(command)
            if command == f"systemctl is-active {UNIT}": return result("inactive", 3)
            if "settings-ui.sh restore" in command: return result(code=1)
            return result()
        with patch.object(self.service, "ssh", side_effect=ssh):
            with self.assertRaisesRegex(Unavailable, "settings overlay"):
                self.service.ensure_tablet("test-token")
        self.assertFalse(any("systemd-run" in command or "cat >" in command for command in commands))

    def test_adopts_running_bridge_without_interrupting_request(self):
        with patch.object(self.service, "health", return_value={"status": "ready", "backend": "codex", "response_mode": "ink"}), \
             patch.object(self.service, "owned_bridge_pid", return_value=123), \
             patch("lan_service.subprocess.Popen") as spawn:
            self.service.ensure_bridge()
        spawn.assert_not_called()

    def test_does_not_adopt_another_process_on_bridge_port(self):
        with patch.object(self.service, "health", return_value={"status": "ready", "backend": "codex", "response_mode": "ink"}), \
             patch.object(self.service, "owned_bridge_pid", return_value=None), \
             patch("lan_service.subprocess.Popen") as spawn:
            with self.assertRaises(Unavailable):
                self.service.ensure_bridge()
        spawn.assert_not_called()

    def test_reload_reuses_recently_closed_connections_but_not_a_live_listener(self):
        ready = {"status": "ready", "backend": "codex", "response_mode": "ink"}
        with socket.socket() as listener:
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
            listener.listen()
            with patch("lan_service.PORT", port), \
                 patch.object(self.service, "health", return_value=None), \
                 patch.object(self.service, "owned_bridge_pid", return_value=None), \
                 patch("lan_service.subprocess.Popen") as spawn:
                with self.assertRaises(Unavailable):
                    self.service.ensure_bridge()
                spawn.assert_not_called()
            # The server closes first, leaving a connection in TIME_WAIT.
            with socket.create_connection(("127.0.0.1", port)) as client:
                connection, _ = listener.accept()
                connection.close()
                self.assertEqual(client.recv(1), b"")
        with patch("lan_service.PORT", port), \
             patch.object(self.service, "health", side_effect=[None, ready]), \
             patch.object(self.service, "owned_bridge_pid", return_value=None), \
             patch("lan_service.subprocess.Popen") as spawn:
            spawn.return_value.pid = 123
            self.service.ensure_bridge()
        spawn.assert_called_once()

    def test_reconnects_tunnel_without_restarting_active_listener(self):
        health = {"status": "ready", "backend": "codex", "response_mode": "ink"}
        def ssh(command, **kwargs):
            return result(json.dumps(health) if command.startswith("wget") else "active")
        with patch.object(self.service, "tunnel_alive", return_value=False), \
             patch.object(self.service, "require") as require, \
             patch.object(self.service, "ssh", side_effect=ssh) as remote, \
             patch.object(self.service, "health", return_value=health):
            self.service.ensure_tunnel()
            self.service.ensure_tablet("test-token")
        command = require.call_args[0][0]
        self.assertIn("ServerAliveCountMax=2", command)
        self.assertIn("127.0.0.1:8765:127.0.0.1:8765", command)
        self.assertEqual(len(remote.call_args_list), 3)
        self.assertFalse(any("start" in call.args[0] for call in remote.call_args_list))

    def test_orphaned_owned_forward_is_reaped_then_reconnected(self):
        health = {"status": "ready", "backend": "codex", "response_mode": "ink"}
        with patch.object(self.service, "tunnel_alive", return_value=False), \
             patch.object(self.service, "require", side_effect=[Unavailable("occupied"), ""]) as connect, \
             patch.object(self.service, "reap_stale_tunnel", return_value=True) as reap, \
             patch.object(self.service, "ssh", return_value=result(json.dumps(health))) as remote, \
             patch.object(self.service, "health", return_value=health):
            self.service.ensure_tunnel()
        reap.assert_called_once()
        self.assertEqual(connect.call_count, 2)
        self.assertIn('remember "$PPID"', remote.call_args.args[0])

    def test_unknown_or_healthy_port_owner_is_not_replaced(self):
        with patch.object(self.service, "tunnel_alive", return_value=False), \
             patch.object(self.service, "require", side_effect=Unavailable("occupied")) as connect, \
             patch.object(self.service, "reap_stale_tunnel", return_value=False), \
             patch.object(self.service, "ssh") as remote:
            with self.assertRaises(Unavailable):
                self.service.ensure_tunnel()
        connect.assert_called_once()
        remote.assert_not_called()

    def test_broken_reverse_forward_is_closed_before_retry(self):
        with patch.object(self.service, "tunnel_alive", return_value=True), \
             patch.object(self.service, "ssh", return_value=result(code=1)), \
             patch.object(self.service, "close_tunnel") as close:
            with self.assertRaises(Unavailable):
                self.service.ensure_tunnel()
        close.assert_called_once()

    def test_tablet_reboot_reprovisions_transient_listener(self):
        commands = []
        def ssh(command, **kwargs):
            commands.append((command, kwargs))
            if command == f"systemctl is-active {UNIT}":
                return result("inactive", 3)
            return result()
        with patch.object(self.service, "ssh", side_effect=ssh):
            self.service.ensure_tablet("t" * 48)
        provision = next(kwargs for command, kwargs in commands if "device.env" in command and "cat >" in command)
        self.assertEqual(provision["input"], "OPENAI_API_KEY=" + "t" * 48 + "\n")
        start = next(command for command, _ in commands if command.startswith("systemd-run"))
        self.assertIn("Restart=on-failure", start)
        self.assertIn("--trigger-corner four-finger", start)
        self.assertNotIn("t" * 48, start)
        self.assertFalse(any("capture" in command for command, _ in commands))

    def test_failed_existing_unit_is_reset_and_restarted(self):
        def ssh(command, **kwargs):
            if command == f"systemctl is-active {UNIT}":
                return result("failed", 3)
            if command.startswith("systemctl show"):
                return result("/run/systemd/transient/" + UNIT)
            return result()
        with patch.object(self.service, "ssh", side_effect=ssh) as remote:
            self.service.ensure_tablet("t" * 48)
        self.assertTrue(any("reset-failed" in call.args[0] for call in remote.call_args_list))
        self.assertFalse(any("systemd-run" in call.args[0] for call in remote.call_args_list))

    def test_waits_for_notebook_app_before_starting_listener(self):
        with patch.object(self.service, "ssh", return_value=result(code=3)) as remote:
            with self.assertRaises(Unavailable):
                self.service.ensure_tablet("t" * 48)
        self.assertEqual(remote.call_count, 2)

    def test_migrates_token_without_rotating_running_bridge_credential(self):
        legacy = self.service.runtime / "bridge.token"
        legacy.write_text("t" * 48 + "\n")
        legacy_env = self.service.runtime / "device.env"
        legacy_env.write_text("OPENAI_API_KEY=" + "t" * 48 + "\n")
        self.assertEqual(self.service.prepare_token(), "t" * 48)
        self.assertFalse(legacy.exists())
        self.assertFalse(legacy_env.exists())
        self.assertEqual(self.service.token.read_text().strip(), "t" * 48)
        self.assertEqual(self.service.token.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.service.prepare_token(), "t" * 48)

    def test_conflicting_tokens_are_both_preserved(self):
        self.service.prepare_token()
        original = self.service.token.read_bytes()
        legacy = self.service.runtime / "bridge.token"
        legacy.write_text("other" * 12)
        with self.assertRaises(Unavailable):
            self.service.prepare_token()
        self.assertEqual(self.service.token.read_bytes(), original)
        self.assertTrue(legacy.exists())

    def test_launchagent_restarts_supervisor_and_contains_no_token(self):
        with patch.object(self.service, "agent_loaded", return_value=False), \
             patch.object(self.service, "load_agent"):
            self.service.install()
        config = plistlib.loads(self.service.plist.read_bytes())
        self.assertTrue(config["RunAtLoad"])
        self.assertTrue(config["KeepAlive"])
        self.assertEqual(config["ProgramArguments"][-1], "supervise")
        self.assertEqual(config["EnvironmentVariables"]["REMARKABLE_CODEX_MODEL"], "test-model")
        self.assertNotIn(self.service.token.read_text().strip(), self.service.plist.read_text())

    def test_stop_disables_recovery_and_retains_credentials(self):
        self.service.prepare_token()
        self.service.plist.parent.mkdir(parents=True)
        self.service.plist.write_text("fixture")
        with patch.object(self.service, "agent_loaded", return_value=True), \
             patch.object(self.service, "require") as require, \
             patch.object(self.service, "ssh", return_value=result()), \
             patch.object(self.service, "close_tunnel"), \
             patch.object(self.service, "owned_bridge_pid", return_value=None):
            self.service.stop()
        self.assertEqual([call.args[0][1] for call in require.call_args_list], ["disable", "bootout"])
        self.assertTrue(self.service.token.exists())
        self.assertTrue(self.service.plist.exists())



if __name__ == "__main__":
    unittest.main()
