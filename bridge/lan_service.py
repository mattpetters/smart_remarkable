#!/usr/bin/env python3
"""Keep the Mac bridge and tablet gesture listener available without reading pages."""
import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import fcntl
import json
import os
from pathlib import Path
import plistlib
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
import urllib.request

LABEL = "com.smart-remarkable.lan-client"
REMOTE = "/home/root/smart-remarkable"
UNIT = "smart-remarkable-lan.service"
PORT = 8765


class Unavailable(Exception):
    pass


class Service:
    def __init__(self, repo=None, home=None):
        self.repo = Path(repo or Path(__file__).resolve().parents[1])
        self.home = Path(home or Path.home())
        self.runtime = self.repo / "tmp/lan-client"
        self.storage = self.home / "Library/Application Support/Smart Remarkable"
        self.token = self.storage / "bridge.token"
        self.plist = self.home / "Library/LaunchAgents" / (LABEL + ".plist")
        self.socket = self.runtime / "ssh.sock"
        self.host = os.environ.get("REMARKABLE_HOST", "rmpp-wifi")
        self.hosts = list(dict.fromkeys([self.host] + [h for h in os.environ.get("REMARKABLE_FALLBACK_HOSTS", "").split(",") if h]))
        self.mode = os.environ.get("REMARKABLE_RESPONSE_MODE", "ink")
        self.model = os.environ.get("REMARKABLE_CODEX_MODEL", "")
        self.codex = os.environ.get("REMARKABLE_CODEX_BIN") or shutil.which("codex")
        self.domain = "gui/" + str(os.getuid())
        self.job = self.domain + "/" + LABEL
        self.bridge_failures = 0
        if self.mode not in ("ink", "text"):
            raise Unavailable("REMARKABLE_RESPONSE_MODE must be ink or text")
        # These paths enter SSH's remote shell and Unix-domain socket APIs.
        if any(not host or host.startswith("-") or any(c.isspace() for c in host) for host in self.hosts):
            raise Unavailable("REMARKABLE_HOST must be an SSH hostname or alias")
        if len(os.fsencode(self.socket)) >= 100:
            raise Unavailable("Repository path is too long for the SSH control socket")
        self.runtime.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.runtime.chmod(0o700)

    def run(self, args, *, input=None, timeout=15):
        # Output can contain credentials when writing device.env. Never log it.
        try:
            return subprocess.run(args, input=input, text=True, capture_output=True,
                                  timeout=timeout)
        except (OSError, subprocess.TimeoutExpired):
            raise Unavailable("A service command was unavailable or timed out") from None

    def require(self, args, message, **kwargs):
        result = self.run(args, **kwargs)
        if result.returncode:
            raise Unavailable(message)
        return result.stdout.strip()

    def ssh(self, command, **kwargs):
        return self.run(["ssh", "-S", str(self.socket), "-o", "BatchMode=yes",
                         "-o", "ConnectTimeout=5", "-o", "ConnectionAttempts=1",
                         "-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2",
                         self.host, command], **kwargs)

    @contextmanager
    def lock(self, name, blocking=True):
        with (self.runtime / name).open("a") as handle:
            try:
                fcntl.flock(handle, fcntl.LOCK_EX | (0 if blocking else fcntl.LOCK_NB))
            except BlockingIOError:
                raise Unavailable("The availability supervisor is already running") from None
            yield

    def prepare_token(self):
        import secrets
        self.storage.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.storage.chmod(0o700)
        legacy = self.runtime / "bridge.token"
        if not self.token.exists():
            value = legacy.read_text() if legacy.exists() else secrets.token_urlsafe(36) + "\n"
            # Exclusive creation never overwrites an existing credential.
            with self.token.open("x") as out:
                out.write(value)
            self.token.chmod(0o600)
        value = self.token.read_text().strip()
        if len(value) < 32 or not value.isascii() or any(c.isspace() for c in value):
            raise Unavailable("Stored bridge token is invalid; restore a valid credential")
        self.token.chmod(0o600)
        if legacy.exists():
            if legacy.read_text().strip() != value:
                raise Unavailable("Legacy and durable tokens differ; preserved both for recovery")
            # The running bridge loaded its token into memory at startup.
            legacy.unlink()
        legacy_env = self.runtime / "device.env"
        if legacy_env.exists() and legacy_env.read_text().strip() == "OPENAI_API_KEY=" + value:
            legacy_env.unlink()
        return value

    def health(self):
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{PORT}/health", timeout=2) as response:
                return json.load(response)
        except (OSError, ValueError):
            return None

    def owned_bridge_pid(self):
        try:
            pid = int((self.runtime / "bridge.pid").read_text())
        except (OSError, ValueError):
            return None
        if pid <= 1:
            return None
        command = self.run(["ps", "-p", str(pid), "-o", "command="]).stdout
        return pid if str(self.repo / "bridge/codex_bridge.py") in command else None

    def ensure_bridge(self):
        health, pid = self.health(), self.owned_bridge_pid()
        if health is not None:
            if not pid or health != {"status": "ready", "backend": "codex", "response_mode": self.mode}:
                raise Unavailable("Port 8765 belongs to another bridge or mode; stop it before changing configuration")
            self.bridge_failures = 0
            return
        if pid:
            self.bridge_failures += 1
            if self.bridge_failures < 3:
                raise Unavailable("Bridge health check failed; waiting before recovery")
            os.kill(pid, signal.SIGTERM)
            for _ in range(20):
                if not self.owned_bridge_pid():
                    break
                time.sleep(0.1)
            if self.owned_bridge_pid():
                raise Unavailable("Unresponsive bridge has not stopped yet")
        if not self.codex:
            raise Unavailable("Codex CLI was not found; install it or set REMARKABLE_CODEX_BIN")
        try:
            with socket.socket() as probe:
                # Match HTTPServer's reuse policy so recent closed connections
                # do not block a bridge reload while in TCP TIME_WAIT.
                probe.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                probe.bind(("127.0.0.1", PORT))
        except OSError:
            raise Unavailable("Port 8765 is occupied; leaving the other process alone") from None
        args = [sys.executable, str(self.repo / "bridge/codex_bridge.py"),
                "--token-file", str(self.token), "--port", str(PORT),
                "--mode", self.mode, "--codex", self.codex,
                "--backend-config", str(self.home / ".config/smart-remarkable/backends.json")]
        if self.model:
            args += ["--model", self.model]
        with (self.runtime / "bridge.log").open("a") as log:
            child = subprocess.Popen(args, stdin=subprocess.DEVNULL, stdout=log,
                                     stderr=subprocess.STDOUT, cwd=self.repo, start_new_session=True)
        (self.runtime / "bridge.pid").write_text(str(child.pid) + "\n")
        for _ in range(30):
            if self.health() == {"status": "ready", "backend": "codex", "response_mode": self.mode}:
                self.bridge_failures = 0
                return
            if child.poll() is not None:
                break
            time.sleep(0.1)
        raise Unavailable("Bridge failed to start; see tmp/lan-client/bridge.log")

    def tunnel_alive(self):
        return self.run(["ssh", "-S", str(self.socket), "-O", "check", self.host]).returncode == 0

    def close_tunnel(self):
        self.run(["ssh", "-S", str(self.socket), "-O", "exit", self.host])

    def reap_stale_tunnel(self):
        # Use a fresh connection: the failed forward's old control socket cannot
        # be trusted. The tablet helper verifies our recorded process lifetime,
        # listening socket, and failed health check before terminating anything.
        result = self.run(["ssh", "-S", "none", "-o", "BatchMode=yes",
                           "-o", "ConnectTimeout=5", "-o", "ConnectionAttempts=1",
                           self.host, f"sh {REMOTE}/tunnel-owner.sh reap"])
        return result.returncode == 0

    def ensure_tunnel(self):
        # Keep a healthy connection; choose endpoints again only on reconnect.
        # Every alias must identify the same tablet using its existing SSH key.
        endpoints = [self.host] if self.tunnel_alive() else self.hosts
        failure = None
        for host in endpoints:
            self.host = host
            try:
                self.ensure_tunnel_at_host()
                return
            except Unavailable as error:
                failure = error
        raise failure or Unavailable("No tablet SSH endpoint is configured")

    def ensure_tunnel_at_host(self):
        if not self.tunnel_alive():
            command = ["ssh", "-M", "-S", str(self.socket), "-fNT",
                       "-o", "BatchMode=yes", "-o", "ConnectTimeout=5",
                       "-o", "ConnectionAttempts=1", "-o", "ExitOnForwardFailure=yes",
                       "-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=2",
                       "-R", f"127.0.0.1:{PORT}:127.0.0.1:{PORT}", self.host]
            message = "Tablet SSH forwarding is unavailable; retrying when it reconnects"
            try:
                self.require(command, message)
            except Unavailable:
                if not self.reap_stale_tunnel():
                    raise
                self.require(command, message)
        result = self.ssh(f"wget -T 3 -qO- http://127.0.0.1:{PORT}/health")
        try:
            healthy = result.returncode == 0 and json.loads(result.stdout) == self.health()
        except ValueError:
            healthy = False
        if not healthy:
            self.close_tunnel()
            raise Unavailable("Tablet cannot reach the bridge; rebuilding the tunnel on the next check")
        # A shell channel on the multiplexed connection has the forward's
        # Dropbear session as its parent. Older deployments lack the helper.
        owner = self.ssh(f'if test -f {REMOTE}/tunnel-owner.sh; then sh {REMOTE}/tunnel-owner.sh remember "$PPID"; fi')
        if owner.returncode:
            raise Unavailable("Tablet tunnel is healthy, but recovery ownership could not be recorded")

    def ensure_tablet(self, token):
        state = self.ssh(f"systemctl is-active {UNIT}")
        if state.stdout.strip() in ("active", "activating"):
            return  # Adopt the existing listener without interrupting a request.
        if self.ssh("systemctl is-active --quiet xochitl").returncode:
            raise Unavailable("Waiting for the tablet notebook app to start")
        # Restore the optional, previously enabled panel after a tablet reboot.
        # The helper only loads it when the notebook binary's fingerprint still
        # matches the tested firmware; a firmware change keeps the stock UI.
        restore = self.ssh(f"if test -f {REMOTE}/settings-enabled && test -x {REMOTE}/settings-ui.sh; then {REMOTE}/settings-ui.sh restore; fi", timeout=30)
        if restore.returncode:
            raise Unavailable("Tablet settings overlay could not be restored")
        result = self.ssh(f"umask 077; mkdir -p {REMOTE}; cat > {REMOTE}/device.env",
                          input="OPENAI_API_KEY=" + token + "\n")
        if result.returncode:
            raise Unavailable("Could not provision the tablet bridge credential")
        fragment = self.ssh(f"systemctl show {UNIT} -p FragmentPath --value")
        if fragment.returncode == 0 and fragment.stdout.strip():
            result = self.ssh(f"systemctl reset-failed {UNIT}; systemctl start {UNIT}")
        else:
            args = ["systemd-run", "--unit=" + UNIT, "--uid=root", "--collect",
                    "--property=WorkingDirectory=" + REMOTE,
                    "--property=EnvironmentFile=" + REMOTE + "/device.env",
                    "--property=Restart=on-failure", "--property=RestartSec=3",
                    REMOTE + "/smart_remarkable", "--engine", "openai", "--engine-base-url",
                    f"http://127.0.0.1:{PORT}", "--model", "codex", "--select-mode",
                    "--trigger-corner", "four-finger", "--prompt",
                    "selection_concise_ink.json" if self.mode == "ink" else "selection_concise.json",
                    "--no-draw-progress"]
            if self.mode == "ink":
                args.append("--no-keyboard")
            result = self.ssh(shlex.join(args))
        if result.returncode or self.ssh(f"systemctl is-active --quiet {UNIT}").returncode:
            raise Unavailable("Tablet listener failed to start; check its journal and deployed binaries")

    def reconcile(self):
        with self.lock("operation.lock"):
            token = self.prepare_token()
            self.ensure_bridge()
            self.ensure_tunnel()
            self.ensure_tablet(token)

    def supervise(self):
        stop = threading.Event()
        for sig in (signal.SIGTERM, signal.SIGINT):
            signal.signal(sig, lambda *_: stop.set())
        previous = None
        with self.lock("supervisor.lock", blocking=False):
            while not stop.is_set():
                try:
                    self.reconcile()
                    state = "ready"
                except Unavailable as error:
                    state = str(error)
                record = {"checked_at": datetime.now(timezone.utc).isoformat(), "status": state}
                (self.runtime / "availability.json").write_text(json.dumps(record) + "\n")
                if state != previous:
                    print(json.dumps(record), flush=True)
                    previous = state
                # Never capture handwriting or replay a failed prompt during recovery.
                stop.wait(15 if state == "ready" else 30)

    def agent_loaded(self):
        return self.run(["launchctl", "print", self.job]).returncode == 0

    def load_agent(self):
        self.require(["launchctl", "enable", self.job], "Could not enable the LaunchAgent")
        if not self.agent_loaded():
            self.require(["launchctl", "bootstrap", self.domain, str(self.plist)],
                         "Could not load the LaunchAgent")

    def install(self):
        self.prepare_token()
        if not self.codex:
            raise Unavailable("Codex CLI must be installed before enabling autostart")
        self.plist.parent.mkdir(parents=True, exist_ok=True)
        environment = {"PATH": os.environ.get("PATH", "/usr/bin:/bin:/usr/sbin:/sbin"),
                       "REMARKABLE_HOST": self.hosts[0], "REMARKABLE_RESPONSE_MODE": self.mode,
                       "REMARKABLE_CODEX_BIN": self.codex}
        if len(self.hosts) > 1:
            environment["REMARKABLE_FALLBACK_HOSTS"] = ",".join(self.hosts[1:])
        if self.model:
            environment["REMARKABLE_CODEX_MODEL"] = self.model
        if os.environ.get("CODEX_HOME"):
            environment["CODEX_HOME"] = os.environ["CODEX_HOME"]
        config = {"Label": LABEL,
                  "ProgramArguments": [sys.executable, str(Path(__file__).resolve()), "supervise"],
                  "WorkingDirectory": str(self.repo), "EnvironmentVariables": environment,
                  "RunAtLoad": True, "KeepAlive": True, "ThrottleInterval": 15,
                  "Umask": 0o077, "StandardOutPath": str(self.runtime / "supervisor.log"),
                  "StandardErrorPath": str(self.runtime / "supervisor.log")}
        if self.agent_loaded() and self.plist.exists() and plistlib.loads(self.plist.read_bytes()) != config:
            raise Unavailable("Stop the service before changing its installed configuration")
        self.plist.write_bytes(plistlib.dumps(config))
        self.plist.chmod(0o600)
        self.load_agent()
        print("Autostart enabled: runs at Mac login and restores the bridge, tunnel, and tablet listener.")

    def stop(self, remove=False):
        if self.plist.exists():
            self.require(["launchctl", "disable", self.job], "Could not disable autostart")
            if self.agent_loaded():
                self.require(["launchctl", "bootout", self.job], "Could not unload autostart")
            if remove:
                self.plist.unlink()
        with self.lock("operation.lock"):
            result = self.ssh(f"systemctl stop {UNIT}")
            if result.returncode:
                print("Tablet is unreachable; its listener may remain active until stopped on the device.")
            self.close_tunnel()
            pid = self.owned_bridge_pid()
            if pid:
                os.kill(pid, signal.SIGTERM)
            (self.runtime / "bridge.pid").unlink(missing_ok=True)
        print("Stopped. Credentials retained; start re-enables an installed LaunchAgent.")

    def status(self):
        result = {"bridge": self.health(), "autostart_installed": self.plist.exists(),
                  "supervisor_running": self.agent_loaded(), "ssh_tunnel": self.tunnel_alive()}
        remote = self.ssh(f"systemctl is-active xochitl; systemctl is-active {UNIT}")
        states = remote.stdout.strip().splitlines()
        result["tablet"] = states if states else "unreachable"
        print(json.dumps(result, indent=2))



def main():
    os.umask(0o077)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["start", "stop", "status", "supervise",
                                          "install-autostart", "uninstall-autostart"])
    args = parser.parse_args()
    action = args.action
    try:
        service = Service()
        if action == "supervise":
            service.supervise()
        elif action == "install-autostart":
            service.install()
        elif action in ("stop", "uninstall-autostart"):
            service.stop(remove=action == "uninstall-autostart")
        elif action == "status":
            service.status()
        else:
            if service.plist.exists():
                service.load_agent()
                # The installed environment owns model/mode selection. A shell's
                # default configuration must not race it to start the bridge.
                print("Automatic recovery enabled with the installed configuration. Use status to check readiness.")
            else:
                service.reconcile()
                print("Ready in any open notebook: lasso a question, then tap with four fingers.")
    except Unavailable as error:
        print(str(error), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
