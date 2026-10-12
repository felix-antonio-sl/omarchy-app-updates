import QtQuick
import Quickshell
import Quickshell.Io

Item {
    id: root
    visible: false
    property bool busy: false
    property bool awaiting: false
    property string previousId: ""
    property string expectedId: ""
    property string observedId: ""
    property var receiptState: null
    readonly property string script: decodeURIComponent(Qt.resolvedUrl("../scripts/prepare-engine").toString().replace(/^file:\/\//, ""))
    readonly property string home: Quickshell.env("HOME")
    signal finished
    signal failed(string message)
    function prepare() {
        if (busy)
            return;
        awaiting = true;
        busy = true;
        previousId = receiptState ? receiptState.operation_id : "";
        expectedId = "";
        submit.running = true;
    }
    function consume(value) {
        if (!value || value.unit !== "omarchy-app-updates-prepare.service" || typeof value.operation_id !== "string")
            return;
        receiptState = value;
        if (awaiting && value.operation_id !== previousId && !expectedId)
            expectedId = value.operation_id;
        if (["building", "awaiting_auth"].indexOf(value.phase) >= 0) {
            observedId = value.operation_id;
            busy = true;
            probe.running = true;
        }
        if (value.phase === "completed" && ((awaiting && value.operation_id === expectedId) || (!awaiting && value.operation_id === observedId))) {
            busy = false;
            awaiting = false;
            observedId = "";
            finished();
        }
        if (value.phase === "failed") {
            busy = false;
            if (awaiting && value.operation_id === expectedId) {
                awaiting = false;
                failed(value.message);
            }
        }
    }
    FileView {
        id: receipt
        path: root.home + "/.local/state/omarchy-app-updates/engine-preparation.json"
        watchChanges: true
        printErrors: false
        onLoaded: {
            try {
                root.consume(JSON.parse(text()));
            } catch (_) {}
        }
        onFileChanged: reload()
    }
    Process {
        id: submit
        command: ["/usr/bin/systemd-run", "--user", "--collect", "--unit=omarchy-app-updates-prepare.service", "--property=KillMode=process", "--property=TimeoutStopSec=5", "--setenv=HOME=" + root.home, "--", "/usr/bin/bash", root.script]
        onExited: function (code) {
            if (code !== 0) {
                root.busy = false;
                root.awaiting = false;
                root.failed("No se pudo iniciar la preparación; comprueba que no haya otro trabajo activo");
            }
        }
    }
    Process {
        id: probe
        command: ["/usr/bin/systemctl", "--user", "show", "--property=ActiveState", "--value", "omarchy-app-updates-prepare.service"]
        stdout: StdioCollector {
            onStreamFinished: {
                var active = text.trim();
                if (root.busy && active !== "active" && active !== "activating" && root.receiptState && root.receiptState.phase !== "completed") {
                    root.busy = false;
                    root.awaiting = false;
                    root.failed("La unidad de preparación terminó sin un recibo completo; vuelve a prepararla");
                }
            }
        }
    }
    Timer {
        interval: 1500
        repeat: true
        running: root.busy
        onTriggered: {
            receipt.reload();
            if (!probe.running)
                probe.running = true;
        }
    }
}
