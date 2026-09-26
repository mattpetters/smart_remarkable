import QtQuick
import QtQml

Rectangle {
    id: root
    anchors.fill: parent
    z: 1000000
    color: "#eeeeee"
    visible: panelOpen
    property bool panelOpen: false
    property bool waiting: false
    property bool saving: false
    property string selectedBackend: "codex"
    property string selectedLength: "balanced"
    property bool pageContext: true
    property string message: ""

    function request(method, path, body, callback) {
        var xhr = new XMLHttpRequest()
        xhr.open(method, "http://127.0.0.1:8766/settings" + path)
        xhr.setRequestHeader("X-Smart-Remarkable", "1")
        xhr.setRequestHeader("Content-Type", "application/json")
        xhr.onreadystatechange = function() {
            if (xhr.readyState === XMLHttpRequest.DONE) callback(xhr.status, xhr.responseText)
        }
        xhr.send(body ? JSON.stringify(body) : "")
    }
    function refresh() {
        if (waiting || saving) return
        waiting = true
        request("GET", "", null, function(status, text) {
            waiting = false
            if (saving) return
            if (status !== 200) { if (panelOpen) message = "Assistant unavailable. Tap Close to return."; return }
            var state
            try { state = JSON.parse(text) } catch (_) { return }
            if (state.open && !panelOpen) {
                selectedBackend = state.preferences.backend
                selectedLength = state.preferences.reply_length
                pageContext = state.preferences.page_context
                message = ""
            }
            panelOpen = state.open
        })
    }
    function save() {
        if (saving) return
        saving = true
        request("POST", "", {backend: selectedBackend, reply_length: selectedLength, page_context: pageContext}, function(status, text) {
            saving = false
            if (status === 200) panelOpen = false
            else message = "Could not save. Your previous settings are unchanged."
        })
    }
    function closePanel() {
        if (saving) return
        saving = true
        panelOpen = false
        request("POST", "/close", null, function(status, text) { saving = false })
    }
    Timer { interval: 750; running: true; repeat: true; onTriggered: root.refresh() }
    // Consume touches on the overlay so they never reach notebook tools.
    MultiPointTouchArea { anchors.fill: parent; minimumTouchPoints: 1; maximumTouchPoints: 10 }

    component Choice: Rectangle {
        id: choice
        property string label
        property bool chosen: false
        signal picked()
        width: 280; height: 86
        color: chosen ? "black" : "white"
        border.color: "black"; border.width: 2
        radius: 8
        Text { anchors.centerIn: parent; text: choice.label; color: choice.chosen ? "white" : "black"; font.pixelSize: 28 }
        TapHandler { gesturePolicy: TapHandler.ReleaseWithinBounds; onTapped: choice.picked() }
    }
    Rectangle {
        width: Math.min(parent.width - 80, 1040)
        height: Math.min(parent.height - 80, 1120)
        anchors.centerIn: parent
        color: "white"; border.color: "black"; border.width: 3; radius: 12
        Column {
            x: 48; y: 42; width: parent.width - 96; spacing: 30
            Text { text: "Notebook assistant"; font.pixelSize: 44; font.bold: true }
            Text { text: "Backend"; font.pixelSize: 30 }
            Row {
                spacing: 18
                Choice { label: "Codex"; chosen: root.selectedBackend === "codex"; onPicked: root.selectedBackend = "codex" }
                Choice { label: "Hermes / oMLX"; chosen: root.selectedBackend === "hermes"; onPicked: root.selectedBackend = "hermes" }
            }
            Text { text: "Both use your Mac. Hermes uses local inference."; font.pixelSize: 24; wrapMode: Text.WordWrap; width: parent.width }
            Text { text: "Reply length"; font.pixelSize: 30 }
            Row {
                spacing: 14
                Choice { width: 245; label: "Brief"; chosen: root.selectedLength === "brief"; onPicked: root.selectedLength = "brief" }
                Choice { width: 245; label: "Balanced"; chosen: root.selectedLength === "balanced"; onPicked: root.selectedLength = "balanced" }
                Choice { width: 245; label: "Detailed"; chosen: root.selectedLength === "detailed"; onPicked: root.selectedLength = "detailed" }
            }
            Text { text: "Include visible-page context"; font.pixelSize: 30 }
            Row {
                spacing: 18
                Choice { label: "On"; chosen: root.pageContext; onPicked: root.pageContext = true }
                Choice { label: "Off"; chosen: !root.pageContext; onPicked: root.pageContext = false }
            }
            Text { text: "Lasso your question, then tap Ask.\nAI opens settings. Four/five fingers also work."; font.pixelSize: 24; wrapMode: Text.WordWrap; width: parent.width }
            Text { text: root.message; font.pixelSize: 24; width: parent.width; wrapMode: Text.WordWrap; height: 70 }
            Row {
                spacing: 18
                Choice { label: "Close"; onPicked: root.closePanel() }
                Choice { label: "Save"; chosen: true; onPicked: root.save() }
            }
        }
    }
}
