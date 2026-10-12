import QtQuick
import qs.Commons
import qs.Ui

Panel {
    id: root
    moduleName: "korvo.app-updates"
    manageIpc: false
    property var anchorItem: null
    property var hostWidget: null
    property var backend: null
    property var navigationBar: root.bar
    function geometry() {
        return JSON.stringify({
            opened: root.opened,
            x: surface.cardOrigin.x + (surface.screen ? surface.screen.x : 0),
            y: surface.cardOrigin.y + (surface.screen ? surface.screen.y : 0),
            width: surface.contentWidth,
            height: surface.contentHeight
        });
    }
    function open() {
        root.controller.show();
        if (root.backend)
            root.backend.refresh(false);
    }
    function close() {
        root.controller.hide();
    }
    function switchPanel(direction) {
        if (root.navigationBar && typeof root.navigationBar.switchPanelFrom === "function")
            return root.navigationBar.switchPanelFrom(root.hostWidget || root, direction);
        return false;
    }
    KeyboardPanel {
        id: surface
        owner: root.hostWidget || root
        anchorItem: root.anchorItem
        bar: root.bar
        open: root.opened
        focusTarget: body
        contentWidth: surface.fittedContentWidth(Style.space(650))
        contentHeight: surface.fittedContentHeight(Style.space(610))
        PanelContent {
            id: body
            anchors.fill: parent
            controller: root.backend
            onCloseRequested: root.close()
            onSwitchRequested: function (direction) {
                root.switchPanel(direction);
            }
        }
    }
}
