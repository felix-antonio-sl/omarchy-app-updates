import QtQuick
import Quickshell.Io
import "ViewModel.js" as VM

Item {
    id: root
    visible: false
    property bool autoStart: true
    property bool panelVisible: false
    readonly property string executable: decodeURIComponent(Qt.resolvedUrl("../bin/app-updates").toString().replace(/^file:\/\//, ""))
    property var report: ({
            apps: [],
            checks: [],
            selected: [],
            coverage: {
                status: "needs_review",
                reason: "Inventario pendiente"
            },
            operation: null,
            checked_at: null
        })
    property var plan: null
    property var recipes: []
    property var job: null
    property var history: []
    property string error: ""
    property bool engineReady: true
    property var queue: []
    property var inflight: null
    property int serial: 0
    property bool handled: false
    property bool refreshOnStatus: false
    property bool forceRefresh: false
    readonly property bool activeJob: job !== null && !VM.terminal(job.operation.phase)
    readonly property bool busy: client.running || activeJob || bootstrap.busy
    signal enginePreparationRequested
    function request(action, payload) {
        if ((action === "operation" || action === "status") && !(payload && payload.decision) && (queue.some(function (r) {
                return r.action === action;
            }) || (inflight && inflight.action === action)))
            return;
        queue = queue.concat([
            {
                protocol_version: 1,
                request_id: "ui_" + (++serial),
                action: action,
                payload: payload || {}
            }
        ]);
        pump();
    }
    function pump() {
        if (client.running || inflight || queue.length === 0)
            return;
        inflight = queue[0];
        queue = queue.slice(1);
        handled = false;
        client.stdinEnabled = true;
        client.running = true;
    }
    function refresh(force) {
        error = "";
        refreshOnStatus = true;
        forceRefresh = !!force;
        request("status", {});
    }
    function saveSelection(ids) {
        error = "";
        plan = null;
        recipes = [];
        request("selection", {
            selected: ids
        });
    }
    function preparePlan(ids) {
        if (busy)
            return;
        error = "";
        request("plan", {
            selected: ids
        });
    }
    function applyPlan(digest) {
        if (!plan || plan.digest !== digest) {
            error = "Cambió el alcance; revisa el plan vigente";
            return;
        }
        if (busy || !engineReady)
            return;
        error = "";
        request("apply", {
            plan_id: plan.id,
            digest: digest
        });
    }
    function answer(operationId, questionId, digest, value) {
        if (!job || !job.pending || job.operation.id !== operationId || job.pending.question_id !== questionId || job.pending.digest !== digest) {
            error = "La decisión pertenece a otro alcance o pregunta";
            return;
        }
        request("operation", {
            operation_id: operationId,
            decision: {
                question_id: questionId,
                digest: digest,
                value: value
            }
        });
    }
    function loadHistory() {
        request("history", {});
    }
    function reopen(operationId, appId) {
        request("reopen", {
            operation_id: operationId,
            app_id: appId
        });
    }
    function prepareEngine() {
        enginePreparationRequested();
        bootstrap.prepare();
    }
    function acceptResponse(response) {
        if (!inflight || response.protocol_version !== 1 || response.request_id !== inflight.request_id) {
            error = "Respuesta de otra solicitud o protocolo incompatible";
            return;
        }
        handled = true;
        if (response.error) {
            error = response.error.message || "No se pudo completar la solicitud";
            if (["needs_engine_preparation", "abi_incompatible"].indexOf(response.error.code) >= 0)
                engineReady = false;
            if (inflight.action === "apply")
                plan = null;
            return;
        }
        var action = inflight.action;
        var value = response.result;
        if (["status", "inventory", "check", "selection"].indexOf(action) >= 0) {
            if (!value || !Array.isArray(value.apps) || !Array.isArray(value.checks)) {
                error = "El reporte no tiene el formato esperado";
                return;
            }
            report = value;
            if (action === "status") {
                if (value.operation && (!job || job.operation.id !== value.operation.id))
                    request("operation", {
                        operation_id: value.operation.id
                    });
                if (refreshOnStatus) {
                    refreshOnStatus = false;
                    if (!value.operation || VM.terminal(value.operation.phase)) {
                        request("inventory", {});
                        request("check", {
                            force: forceRefresh
                        });
                    }
                }
            }
        } else if (action === "plan") {
            plan = value.plan;
            recipes = value.recipes || [];
        } else if (action === "apply") {
            if (value && value.accepted && value.operation_id) {
                plan = null;
                request("operation", {
                    operation_id: value.operation_id
                });
            } else
                error = "No se recibió un identificador de trabajo";
        } else if (action === "operation") {
            if (!value || !value.operation) {
                error = "Recibo del trabajo no válido";
                return;
            }
            var wasActive = activeJob;
            job = value;
            if (wasActive && VM.terminal(value.operation.phase))
                request("status", {});
            if (value.operation.results.some(function (r) {
                return r.error && ["needs_engine_preparation", "abi_incompatible"].indexOf(r.error.code) >= 0;
            }))
                engineReady = false;
        } else if (action === "history") {
            history = Object.keys(value || {}).map(function (k) {
                return value[k];
            }).reverse();
        } else if (action === "reopen") {
            if (job)
                request("operation", {
                    operation_id: job.operation.id
                });
        }
    }
    Process {
        id: client
        objectName: "clientProcess"
        command: [root.executable]
        stdinEnabled: true
        onStarted: {
            write(JSON.stringify(root.inflight) + "\n");
            stdinEnabled = false;
        }
        stdout: StdioCollector {
            onStreamFinished: {
                try {
                    root.acceptResponse(JSON.parse(text));
                } catch (_) {
                    root.handled = true;
                    root.error = "El motor no respondió con datos válidos; vuelve a prepararlo";
                    root.engineReady = false;
                }
            }
        }
        onExited: function (code) {
            if (!root.handled) {
                root.error = "No se pudo iniciar el motor; prepara una versión compatible";
                root.engineReady = false;
            }
            root.inflight = null;
            Qt.callLater(root.pump);
        }
    }
    Bootstrap {
        id: bootstrap
        onFinished: {
            root.engineReady = true;
            root.refresh(false);
        }
        onFailed: function (message) {
            root.error = message;
        }
    }
    Timer {
        interval: 1500
        repeat: true
        running: root.activeJob
        onTriggered: if (root.job)
            root.request("operation", {
                operation_id: root.job.operation.id
            })
    }
    Component.onCompleted: if (autoStart)
        request("status", {})
}
