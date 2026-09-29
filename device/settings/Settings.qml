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
    property string inkColor: "blue"
    property bool pageContext: true
    property bool autoFallback: true
    property var backendOrder: ["codex", "hermes", "claude"]
    property var selectedModels: ({})
    property var catalog: ({})
    property var inflight: null
    property double requestStarted: 0
    function reorder(index, offset) {
        var order = backendOrder.slice()
        var other = index + offset
        if (other < 0 || other >= order.length) return
        var old = order[index]; order[index] = order[other]; order[other] = old
        backendOrder = order
        selectedBackend = order[0]
    }
    function modelName(provider) {
        return selectedModels[provider] || (catalog[provider] || {}).model || "default"
    }
    function nextModel(provider) {
        var choices = (catalog[provider] || {}).models || []
        if (choices.length < 2) return
        var models = Object.assign({}, selectedModels)
        models[provider] = choices[(choices.indexOf(modelName(provider)) + 1) % choices.length]
        selectedModels = models
    }
    property string message: ""

    function request(method, path, body, callback) {
        var xhr = new XMLHttpRequest()
        inflight = xhr
        requestStarted = Date.now()
        xhr.open(method, "http://127.0.0.1:8766/settings" + path)
        xhr.setRequestHeader("X-Smart-Remarkable", "1")
        xhr.setRequestHeader("Content-Type", "application/json")
        xhr.onreadystatechange = function() {
            if (xhr.readyState === XMLHttpRequest.DONE) { if (inflight === xhr) inflight = null; callback(xhr.status, xhr.responseText) }
        }
        xhr.send(body ? JSON.stringify(body) : "")
    }
    function refresh() {
        if (inflight && Date.now() - requestStarted > 5000) { inflight.abort(); inflight = null; waiting = false; saving = false }
        if (waiting || saving) return
        waiting = true
        request("GET", "", null, function(status, text) {
            waiting = false
            if (saving) return
            if (status !== 200) { if (panelOpen) message = "Assistant unavailable. Tap Close to return."; return }
            var state
            try { state = JSON.parse(text) } catch (_) { return }
            catalog = state.catalog || {}
            if (state.open && !panelOpen) {
                selectedBackend = state.preferences.backend
                selectedLength = state.preferences.reply_length
                inkColor = state.preferences.ink_color || "blue"
                pageContext = state.preferences.page_context
                autoFallback = state.preferences.auto_fallback
                var order = state.preferences.backend_order || ["codex", "hermes", "claude"]
                backendOrder = [selectedBackend].concat(order.filter(function(p) { return p !== selectedBackend }))
                selectedModels = state.preferences.models || {}
                message = ""
            }
            panelOpen = state.open
        })
    }
    function save() {
        if (saving) return
        saving = true
        request("POST", "", {backend: selectedBackend, reply_length: selectedLength, ink_color: inkColor, page_context: pageContext, auto_fallback: autoFallback, backend_order: backendOrder, models: selectedModels}, function(status, text) {
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
        width: Math.min(280, (settingsContent.width - 18) / 2); height: 86
        color: chosen ? "black" : "white"
        border.color: "black"; border.width: 2
        radius: 8
        Text { anchors.centerIn: parent; text: choice.label; color: choice.chosen ? "white" : "black"; font.pixelSize: Math.min(28, choice.width / 4.4) }
        TapHandler { gesturePolicy: TapHandler.ReleaseWithinBounds; onTapped: choice.picked() }
    }
    Rectangle {
        id: settingsCard
        width: Math.min(parent.width - 80, 1040)
        height: Math.min(parent.height - 80, 1840)
        anchors.centerIn: parent
        color: "white"; border.color: "black"; border.width: 3; radius: 12
        Flickable {
            anchors.fill: parent; anchors.margins: 42
            clip: true
            contentWidth: width
            contentHeight: settingsContent.height
            boundsBehavior: Flickable.StopAtBounds
        Column {
            id: settingsContent
            width: parent.width; spacing: 24
            Text { text: "Notebook assistant"; font.pixelSize: Math.min(44, parent.width / 11); font.bold: true }
            Text { text: "Provider and model priority"; font.pixelSize: 30 }
            Repeater {
                model: root.backendOrder
                delegate: Row {
                    required property string modelData
                    required property int index
                    spacing: 12
                    Rectangle {
                        width: settingsContent.width - 214; height: 100; color: "white"; border.color: "black"; radius: 8
                        Column {
                            x: 18; y: 12; spacing: 8; width: parent.width - 36
                            Text { text: (index + 1) + ". " + modelData; font.pixelSize: 28; font.bold: true }
                            Text { text: root.modelName(modelData); font.pixelSize: 22; width: parent.width; elide: Text.ElideRight }
                        }
                        TapHandler { onTapped: root.nextModel(modelData) }
                    }
                    Choice { width: 95; height: 100; label: "Up"; onPicked: root.reorder(index, -1) }
                    Choice { width: 95; height: 100; label: "Down"; onPicked: root.reorder(index, 1) }
                }
            }
            Text { text: "Tap a model to cycle configured choices. All use your Mac."; font.pixelSize: 24; wrapMode: Text.WordWrap; width: parent.width }
            Row {
                spacing: 18
                Choice { label: "Fallbacks on"; chosen: root.autoFallback; onPicked: root.autoFallback = true }
                Choice { label: "First only"; chosen: !root.autoFallback; onPicked: root.autoFallback = false }
            }
            Text { text: "On failure, try the next provider. Possible tool actions stop automatic replay. Hermes is local; Codex and Claude use cloud models."; font.pixelSize: 24; wrapMode: Text.WordWrap; width: parent.width }
            Text { text: "Reply length"; font.pixelSize: 30 }
            Row {
                spacing: 14
                Choice { width: (settingsContent.width - 28) / 3; label: "Brief"; chosen: root.selectedLength === "brief"; onPicked: root.selectedLength = "brief" }
                Choice { width: (settingsContent.width - 28) / 3; label: "Balanced"; chosen: root.selectedLength === "balanced"; onPicked: root.selectedLength = "balanced" }
                Choice { width: (settingsContent.width - 28) / 3; label: "Detailed"; chosen: root.selectedLength === "detailed"; onPicked: root.selectedLength = "detailed" }
            }
            Text { text: "Include visible-page context"; font.pixelSize: 30 }
            Row {
                spacing: 18
                Choice { label: "On"; chosen: root.pageContext; onPicked: root.pageContext = true }
                Choice { label: "Off"; chosen: !root.pageContext; onPicked: root.pageContext = false }
            }
            Text { text: "Answer ink"; font.pixelSize: 30 }
            Row {
                spacing: 14
                Choice { width: (settingsContent.width - 42) / 4; label: "Blue"; chosen: root.inkColor === "blue"; onPicked: root.inkColor = "blue" }
                Choice { width: (settingsContent.width - 42) / 4; label: "Red"; chosen: root.inkColor === "red"; onPicked: root.inkColor = "red" }
                Choice { width: (settingsContent.width - 42) / 4; label: "Cyan"; chosen: root.inkColor === "cyan"; onPicked: root.inkColor = "cyan" }
                Choice { width: (settingsContent.width - 42) / 4; label: "Magenta"; chosen: root.inkColor === "magenta"; onPicked: root.inkColor = "magenta" }
            }
            Text { text: "Lasso your question, then tap Ask (Move selection menu) or use four fingers.\nAI or five fingers opens settings."; font.pixelSize: 24; wrapMode: Text.WordWrap; width: parent.width }
            Text { text: root.message; font.pixelSize: 24; width: parent.width; wrapMode: Text.WordWrap; height: 70 }
            Row {
                spacing: 18
                Choice { label: "Close"; onPicked: root.closePanel() }
                Choice { label: "Save"; chosen: true; onPicked: root.save() }
            }
        }
        }
    }
}
