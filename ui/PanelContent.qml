pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as Controls
import qs.Commons
import qs.Ui as Ui
import "ViewModel.js" as VM

Item {
    id: root
    required property var controller
    property string page: "apps"
    property string filter: "all"
    property string query: ""
    property var focusedApp: null
    property bool recipesReviewed: false
    property string reopenAppId: ""
    readonly property string planText: VM.planText(controller.plan)
    readonly property var rows: VM.visibleRows(controller.report, filter, query)
    readonly property var job: controller.job
    signal closeRequested
    signal switchRequested(int direction)
    focus: true
    Keys.onEscapePressed: closeRequested()
    Keys.onPressed: function (event) {
        if (event.key === Qt.Key_Tab && (event.modifiers & Qt.ControlModifier)) {
            root.switchRequested((event.modifiers & Qt.ShiftModifier) ? -1 : 1);
            event.accepted = true;
        }
    }
    function chooseReopen(targetId) {
        if (!root.job || !VM.terminal(root.job.operation.phase))
            return;
        var app = VM.appForTarget(root.job.report, targetId);
        if (app)
            root.reopenAppId = app.id;
    }
    function confirmReopen() {
        if (root.reopenAppId && root.job) {
            root.controller.reopen(root.job.operation.id, root.reopenAppId);
            root.reopenAppId = "";
        }
    }
    function toggleSelection(id) {
        var ids = (controller.report.selected || []).slice();
        var at = ids.indexOf(id);
        if (at >= 0)
            ids.splice(at, 1);
        else
            ids.push(id);
        controller.saveSelection(ids);
    }
    function showHistory() {
        page = "history";
        controller.loadHistory();
    }
    function confirmPlan(digest) {
        if (controller.plan && controller.plan.digest === digest && !controller.busy && controller.engineReady) {
            controller.applyPlan(digest);
        }
    }
    Connections {
        target: root.controller
        ignoreUnknownSignals: true
        function onPlanChanged() {
            root.recipesReviewed = false;
            if (root.controller.plan)
                root.page = "plan";
        }
        function onJobChanged() {
            if (root.controller.job && !VM.terminal(root.controller.job.operation.phase))
                root.page = "job";
        }
    }
    Column {
        id: header
        anchors {
            left: parent.left
            right: parent.right
            top: parent.top
        }
        spacing: Style.space(8)
        Flow {
            width: parent.width
            spacing: Style.space(10)
            Label {
                text: "Apps"
                font.pixelSize: Style.font.heading
                font.bold: true
            }
            Label {
                text: VM.updateCount(root.controller.report) + " disponibles"
                color: Color.muted
            }
            Ui.Button {
                text: "Comprobar"
                focusable: true
                enabled: !root.controller.busy
                onClicked: root.controller.refresh(true)
            }
            Ui.Button {
                text: "Revisar plan"
                focusable: true
                enabled: !root.controller.busy && root.controller.engineReady && VM.updateCount(root.controller.report) > 0
                onClicked: root.controller.preparePlan(root.controller.report.selected || [])
            }
            Ui.PanelActionButton {
                iconText: "×"
                tooltipText: "Cerrar"
                focusable: true
                onClicked: root.closeRequested()
            }
        }
        Label {
            width: parent.width
            text: VM.checkedLabel(root.controller.report)
            color: Color.muted
            font.pixelSize: Style.font.caption
        }
        Label {
            objectName: "errorLabel"
            width: parent.width
            visible: text !== ""
            text: root.controller.error || ""
            color: Color.urgent
        }
        Label {
            width: parent.width
            visible: root.controller.report.coverage && root.controller.report.coverage.status !== "verified"
            text: "Revisar cobertura de updater.py: " + (root.controller.report.coverage.reason || "")
            color: Color.urgent
        }
        Row {
            spacing: Style.space(4)
            Ui.Button {
                text: "Aplicaciones"
                focusable: true
                selected: root.page === "apps"
                onClicked: root.page = "apps"
            }
            Ui.Button {
                text: "Plan"
                focusable: true
                selected: root.page === "plan"
                enabled: root.controller.plan !== null
                onClicked: root.page = "plan"
            }
            Ui.Button {
                text: "Trabajo"
                focusable: true
                selected: root.page === "job"
                enabled: root.job !== null
                onClicked: root.page = "job"
            }
            Ui.Button {
                text: "Historial"
                focusable: true
                selected: root.page === "history"
                onClicked: root.showHistory()
            }
        }
    }
    Controls.ScrollView {
        id: scroll
        anchors {
            top: header.bottom
            topMargin: Style.space(12)
            bottom: parent.bottom
            left: parent.left
            right: parent.right
        }
        clip: true
        contentWidth: availableWidth
        Column {
            width: scroll.availableWidth
            spacing: Style.space(12)
            Column {
                width: parent.width
                spacing: Style.space(8)
                visible: !root.controller.engineReady
                Label {
                    width: parent.width
                    text: "Prepara el motor para usar Apps. Se compilan las fuentes con Cargo.lock y se instala el paquete propio del ejecutor mediante autorización gráfica."
                }
                Label {
                    width: parent.width
                    visible: root.controller.preparingEngine === true
                    text: root.controller.preparationMessage || "Preparando el motor"
                    color: Color.accent
                }
                Ui.Button {
                    text: "Preparar motor"
                    focusable: true
                    enabled: !root.controller.busy
                    onClicked: root.controller.prepareEngine()
                }
            }
            Column {
                width: parent.width
                spacing: Style.space(8)
                visible: root.page === "apps"
                Ui.TextField {
                    width: parent.width
                    placeholderText: "Buscar aplicación"
                    text: root.query
                    onTextChanged: root.query = text
                    Keys.onEscapePressed: root.closeRequested()
                }
                Row {
                    spacing: Style.space(4)
                    Repeater {
                        model: [
                            {
                                id: "all",
                                name: "Todas"
                            },
                            {
                                id: "selected",
                                name: "Elegidas"
                            },
                            {
                                id: "updates",
                                name: "Actualizaciones"
                            },
                            {
                                id: "problems",
                                name: "Problemas"
                            }
                        ]
                        delegate: Ui.Button {
                            required property var modelData
                            text: modelData.name
                            selected: root.filter === modelData.id
                            focusable: true
                            onClicked: root.filter = modelData.id
                        }
                    }
                }
                Label {
                    width: parent.width
                    visible: root.rows.length === 0
                    text: "No hay aplicaciones para este filtro."
                    color: Color.muted
                }
                Repeater {
                    model: root.rows
                    delegate: Rectangle {
                        id: row
                        required property var modelData
                        readonly property var checks: VM.checksFor(root.controller.report, row.modelData)
                        width: parent.width
                        height: rowBody.implicitHeight + Style.space(16)
                        radius: Style.cornerRadius
                        color: hover.hovered ? Style.hoverFillFor(Color.foreground, Color.accent) : Style.normalFillFor(Color.foreground, Color.accent)
                        HoverHandler {
                            id: hover
                        }
                        Column {
                            id: rowBody
                            width: parent.width - Style.space(16)
                            x: Style.space(8)
                            y: Style.space(8)
                            spacing: Style.space(5)
                            Row {
                                width: parent.width
                                spacing: Style.space(8)
                                Ui.Button {
                                    text: (root.controller.report.selected || []).indexOf(row.modelData.id) >= 0 ? "✓" : "○"
                                    focusable: true
                                    tooltipText: "Incluir o excluir de mi lista"
                                    onClicked: root.toggleSelection(row.modelData.id)
                                }
                                Label {
                                    objectName: "appName_" + row.modelData.id
                                    width: parent.width - Style.space(120)
                                    text: row.modelData.name
                                    font.bold: true
                                    anchors.verticalCenter: parent.verticalCenter
                                }
                                Ui.Button {
                                    text: "Detalles"
                                    focusable: true
                                    onClicked: {
                                        root.focusedApp = row.modelData;
                                        root.page = "details";
                                    }
                                }
                            }
                            Repeater {
                                model: row.checks
                                delegate: Label {
                                    required property var modelData
                                    objectName: "targetState_" + modelData.target_id
                                    width: parent.width
                                    text: (row.checks.length > 1 ? (modelData.target_id.indexOf("steam-client") >= 0 ? "Cliente: " : "Lanzador: ") : "") + VM.stateLabel(modelData) + " · " + VM.version(modelData.installed)
                                    color: modelData.stale ? Color.muted : (modelData.state === "update_available" ? Color.accent : Color.foreground)
                                    font.pixelSize: Style.font.caption
                                }
                            }
                        }
                    }
                }
            }
            Column {
                width: parent.width
                spacing: Style.space(10)
                visible: root.page === "details"
                Label {
                    width: parent.width
                    text: root.focusedApp ? root.focusedApp.name : ""
                    font.bold: true
                    font.pixelSize: Style.font.title
                }
                Repeater {
                    model: root.focusedApp ? VM.checksFor(root.controller.report, root.focusedApp) : []
                    delegate: Column {
                        required property var modelData
                        width: parent.width
                        spacing: Style.space(5)
                        Label {
                            width: parent.width
                            text: VM.stateLabel(parent.modelData) + "\nInstalada: " + VM.version(parent.modelData.installed) + "\nCanal: " + VM.version(parent.modelData.available) + "\nFabricante: " + VM.version(parent.modelData.published) + "\n" + (parent.modelData.source || "") + "\n" + (parent.modelData.details || []).join("\n")
                        }
                        Repeater {
                            model: VM.sourceLinks(parent.modelData)
                            delegate: Ui.Button {
                                required property string modelData
                                text: "Abrir fuente"
                                focusable: true
                                onClicked: Qt.openUrlExternally(modelData)
                            }
                        }
                        Ui.Button {
                            visible: VM.canEnrollTelegram(root.focusedApp, parent.modelData)
                            text: "Verificar binario de Telegram"
                            focusable: true
                            enabled: !root.controller.busy
                            onClicked: root.controller.enrollTelegram(parent.modelData.target_id)
                        }
                    }
                }
                Ui.Button {
                    text: "Volver a aplicaciones"
                    focusable: true
                    onClicked: root.page = "apps"
                }
            }
            Column {
                width: parent.width
                spacing: Style.space(10)
                visible: root.page === "plan"
                Label {
                    width: parent.width
                    text: root.planText
                }
                Recipes {
                    width: parent.width
                    recipes: root.controller.recipes || []
                    visible: recipes.length > 0
                }
                Ui.Button {
                    text: root.recipesReviewed ? "✓ Revisé las recetas y sus cambios" : "○ He revisado las recetas y sus cambios"
                    visible: (root.controller.recipes || []).length > 0
                    focusable: true
                    onClicked: root.recipesReviewed = !root.recipesReviewed
                }
                Label {
                    width: parent.width
                    text: "Revisa todo el alcance. Arch incluye los paquetes del sistema necesarios para mantener una actualización coherente."
                    color: Color.muted
                }
                Row {
                    spacing: Style.space(8)
                    Ui.Button {
                        text: "Aprobar plan"
                        focusable: true
                        enabled: root.controller.plan !== null && !root.controller.busy && root.controller.engineReady && ((root.controller.recipes || []).length === 0 || root.recipesReviewed)
                        onClicked: root.confirmPlan(root.controller.plan.digest)
                    }
                    Ui.Button {
                        text: "Volver"
                        focusable: true
                        onClicked: root.page = "apps"
                    }
                }
            }
            Column {
                width: parent.width
                spacing: Style.space(10)
                visible: root.page === "job" && root.job !== null
                Label {
                    width: parent.width
                    text: root.job ? VM.phaseLabel(root.job.operation.phase) : ""
                    font.bold: true
                }
                Label {
                    width: parent.width
                    text: root.job && root.job.operation.events.length ? root.job.operation.events[root.job.operation.events.length - 1].message : ""
                }
                Label {
                    width: parent.width
                    text: root.job ? VM.planText(root.job.plan) : ""
                    visible: root.job && root.job.pending !== null
                }
                Flow {
                    width: parent.width
                    spacing: Style.space(6)
                    Repeater {
                        model: root.job && root.job.pending ? root.job.pending.choices : []
                        delegate: Ui.Button {
                            required property var modelData
                            text: typeof modelData === "object" ? "Elegir " + modelData.select_provider : (modelData === "approve" ? "Confirmar este alcance" : "Cancelar preparación")
                            focusable: true
                            onClicked: root.controller.answer(root.job.operation.id, root.job.pending.question_id, root.job.pending.digest, modelData)
                        }
                    }
                }
                Label {
                    width: parent.width
                    visible: root.job && root.job.operation.phase === "installing"
                    text: "La instalación está confirmada. Puedes cerrar el panel y volver para ver el resultado."
                    color: Color.muted
                }
                Column {
                    width: parent.width
                    spacing: Style.space(8)
                    visible: root.reopenAppId !== ""
                    Label {
                        width: parent.width
                        text: "Guarda tus cambios. Se solicitará el cierre de la aplicación y luego se abrirá su lanzador. No se forzará el cierre."
                    }
                    Row {
                        spacing: Style.space(8)
                        Ui.Button {
                            text: "Cerrar y reabrir"
                            focusable: true
                            onClicked: root.confirmReopen()
                        }
                        Ui.Button {
                            text: "Dejar para después"
                            focusable: true
                            onClicked: root.reopenAppId = ""
                        }
                    }
                }
                Repeater {
                    model: root.job ? root.job.operation.results : []
                    delegate: Column {
                        id: resultRow
                        required property var modelData
                        readonly property var app: root.job ? VM.appForTarget(root.job.report, modelData.target_id) : null
                        width: parent.width
                        Label {
                            width: parent.width
                            text: resultRow.modelData.target_id + " · " + VM.outcomeLabel(resultRow.modelData.outcome) + (resultRow.modelData.new_version ? " · " + VM.version(resultRow.modelData.new_version) : "") + (resultRow.modelData.error ? "\n" + resultRow.modelData.error.message : "")
                        }
                        Ui.Button {
                            text: "Reabrir"
                            focusable: true
                            visible: resultRow.app !== null && resultRow.modelData.outcome === "needs_reopen" && VM.terminal(root.job.operation.phase)
                            enabled: !VM.isVoxtype(resultRow.app)
                            onClicked: root.chooseReopen(resultRow.modelData.target_id)
                        }
                        Label {
                            width: parent.width
                            visible: resultRow.app !== null && VM.isVoxtype(resultRow.app) && resultRow.modelData.outcome === "needs_reopen"
                            text: "Voxtype seguirá activo. Aplica la nueva versión desde su configuración cuando termines el dictado."
                            color: Color.muted
                        }
                    }
                }
            }
            Column {
                width: parent.width
                spacing: Style.space(10)
                visible: root.page === "history"
                Label {
                    width: parent.width
                    text: (root.controller.history || []).length ? "Trabajos anteriores" : "Todavía no hay trabajos registrados."
                }
                Repeater {
                    model: root.controller.history || []
                    delegate: Column {
                        required property var modelData
                        width: parent.width
                        spacing: Style.space(5)
                        Label {
                            width: parent.width
                            text: VM.phaseLabel(parent.modelData.phase) + " · " + parent.modelData.id
                            font.bold: true
                        }
                        Repeater {
                            model: parent.modelData.results
                            delegate: Label {
                                required property var modelData
                                width: parent.width
                                text: modelData.target_id + " · " + VM.outcomeLabel(modelData.outcome)
                            }
                        }
                    }
                }
            }
        }
    }
}
