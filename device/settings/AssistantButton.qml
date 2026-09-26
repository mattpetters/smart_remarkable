import QtQuick
import QtQml

Rectangle {
    id: button
    property bool sendSelection: false
    property bool available: false
    property bool busy: false
    property bool panelOpen: false
    property bool waiting: false
    signal activated()
    color: "white"

    function request(method, suffix, callback) {
        var xhr = new XMLHttpRequest()
        xhr.open(method, "http://127.0.0.1:8766/settings" + suffix)
        xhr.setRequestHeader("X-Smart-Remarkable", "1")
        xhr.onreadystatechange = function() {
            if (xhr.readyState === XMLHttpRequest.DONE) callback(xhr.status, xhr.responseText)
        }
        xhr.send()
    }
    function refresh() {
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
    Timer { interval: 750; repeat: true; running: button.visible; onTriggered: button.refresh() }
    Text {
        anchors.centerIn: parent
        text: button.sendSelection ? (button.busy ? "..." : "Ask") : "AI"
        font.pixelSize: Math.min(button.width * 0.30, 32)
        font.bold: true
        color: button.available && !button.busy && !button.panelOpen ? "black" : "#888888"
    }
    TapHandler { gesturePolicy: TapHandler.ReleaseWithinBounds; onTapped: button.activate() }
}
