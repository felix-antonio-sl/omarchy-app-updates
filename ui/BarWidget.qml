import QtQuick
import Quickshell.Io
import qs.Commons
import qs.Ui as Ui
import "ViewModel.js" as VM

Ui.BarWidget {
    id: root
    moduleName: "korvo.app-updates"
    readonly property bool opened: panel.opened
    readonly property bool popoutSwitchClosing: panel.popoutSwitchClosing
    readonly property real openPanelIndicatorWidth: button.labelWidth
    readonly property real openPanelIndicatorHeight: Math.max(Style.space(10), Math.round(Style.bar.iconSlot * 0.55))
    function open() {
        panel.open();
    }
    function close() {
        panel.close();
    }
    function toggle() {
        panel.toggle();
    }
    function closeForPopoutSwitch() {
        panel.closeForPopoutSwitch();
    }
    implicitWidth: button.implicitWidth
    implicitHeight: button.implicitHeight
    Controller {
        id: client
        panelVisible: root.opened
    }
    AppsPanel {
        id: panel
        bar: root.bar
        settings: root.settings
        anchorItem: button
        hostWidget: root
        backend: client
    }
    IpcHandler {
        target: "korvo.app-updates"
        function open(): void {
            root.open();
        }
        function close(): void {
            root.close();
        }
        function toggle(): void {
            root.toggle();
        }
        function geometry(): string {
            return panel.geometry();
        }
        function refresh(): void {
            client.refresh(true);
        }
    }
    Ui.WidgetButton {
        id: button
        anchors.fill: parent
        bar: root.bar
        text: (client.activeJob ? "󰑓" : "󰏖") + (root.vertical ? "" : (VM.updateCount(client.report) > 0 ? " Apps " + VM.updateCount(client.report) : " Apps"))
        labelVisible: true
        tooltipText: client.activeJob ? VM.phaseLabel(client.job.operation.phase) : "Versiones y actualizaciones de tus aplicaciones"
        onPressed: function (code) {
            if (code === Qt.LeftButton)
                root.toggle();
        }
    }
}
