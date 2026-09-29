#!/bin/sh
# Track only this client's reverse-forward session. Never reap an unknown owner.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
record="$root/tunnel-owner"

identity() {
    case "$1" in ''|*[!0-9]*) return 1 ;; esac
    test "$1" -gt 1 || return 1
    test "$(cat "/proc/$1/comm")" = dropbear || return 1
    # Dropbear is launched per connection; do not accept a listening SSH daemon.
    tr '\000' '\n' < "/proc/$1/cmdline" | grep -qx -- '-i' || return 1
    printf '%s %s %s\n' "$1" "$(cat /proc/sys/kernel/random/boot_id)" "$(awk '{print $22}' "/proc/$1/stat")"
}

owns_forward() {
    inode=$(awk '$2 == "0100007F:223D" && $4 == "0A" {print $10}' /proc/net/tcp)
    test -n "$inode" || return 1
    for fd in /proc/"$1"/fd/*; do
        if test "$(readlink "$fd" 2>/dev/null || true)" = "socket:[$inode]"; then
            return 0
        fi
    done
    return 1
}

case "${1:-}" in
    remember)
        session=${2:-}
        fingerprint=$(identity "$session") || exit 1
        owns_forward "$session" || exit 1
        umask 077
        printf '%s\n' "$fingerprint" > "$record.tmp"
        mv "$record.tmp" "$record"
        ;;
    reap)
        test -f "$record" || exit 1
        saved=$(cat "$record")
        session=${saved%% *}
        test "$(identity "$session")" = "$saved" || exit 1
        owns_forward "$session" || exit 1
        # A healthy owner may belong to a still-running Mac connection.
        if wget -T 3 -qO /dev/null http://127.0.0.1:8765/health; then
            exit 1
        fi
        # Recheck process lifetime and socket ownership immediately before TERM.
        test "$(identity "$session")" = "$saved" || exit 1
        owns_forward "$session" || exit 1
        kill -TERM "$session"
        rm -f "$record"
        ;;
    *) exit 2 ;;
esac
