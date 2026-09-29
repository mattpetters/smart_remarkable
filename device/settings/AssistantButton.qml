import QtQuick
import QtQml

Rectangle {
    id: button
    property bool compact: false
    property bool pollWhenHidden: false
    property bool sendSelection: false
    property bool available: false
    property bool busy: false
    property bool panelOpen: false
    property bool waiting: false
    property string providerLabel: ""
    property string phase: ""
    property var inflight: null
    property double requestStarted: 0
    signal activated()
    color: "white"

    function request(method, suffix, callback) {
        var xhr = new XMLHttpRequest()
        inflight = xhr
        requestStarted = Date.now()
        xhr.open(method, "http://127.0.0.1:8766/settings" + suffix)
        xhr.setRequestHeader("X-Smart-Remarkable", "1")
        xhr.onreadystatechange = function() {
            if (xhr.readyState === XMLHttpRequest.DONE) { if (inflight === xhr) inflight = null; callback(xhr.status, xhr.responseText) }
        }
        xhr.send()
    }
    function refresh() {
        if (inflight && Date.now() - requestStarted > 5000) { inflight.abort(); inflight = null; waiting = false }
        if (waiting) return
        waiting = true
        request("GET", "", function(status, text) {
            waiting = false
            available = status === 200
            if (!available) return
            try {
                var state = JSON.parse(text)
                busy = state.busy
                panelOpen = state.open
                providerLabel = state.provider_label || ""
                phase = state.phase || "Connecting"
            } catch (_) { available = false }
        })
    }
    function activate() {
        if (!available || busy || waiting || panelOpen) return
        waiting = true
        activated()
        request("POST", sendSelection ? "/send" : "/open", function(status, text) {
            waiting = false
            if (status === 202) busy = true
            else if (status === 200) panelOpen = true
            refresh()
        })
    }
    Component.onCompleted: refresh()
    Timer { interval: 750; repeat: true; running: button.visible || button.pollWhenHidden; onTriggered: button.refresh() }
    Text {
        anchors.centerIn: parent
        text: button.sendSelection ? (button.busy ? "..." : "Ask") : "AI"
        font.pixelSize: button.compact ? 24 : Math.min(button.width * 0.30, 32)
        font.bold: true
        color: button.available && !button.busy && !button.panelOpen ? "black" : "#888888"
    }
    Rectangle {
        visible: button.sendSelection && button.busy && button.phase === "Thinking"
        x: button.width + 18; y: 0; width: 640; height: 88
        color: "white"; border.color: "black"; border.width: 2; radius: 6
        Text { anchors.fill: parent; anchors.margins: 10; text: button.phase + "\n" + button.providerLabel; font.pixelSize: 22; elide: Text.ElideRight }
    }
    TapHandler { gesturePolicy: TapHandler.ReleaseWithinBounds; onTapped: button.activate() }
}
