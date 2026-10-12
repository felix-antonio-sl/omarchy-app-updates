import QtQuick

Item {
    id: root
    visible: false
    property bool busy: false
    signal finished
    signal failed(string message)
    function prepare() {
        failed("La preparación gráfica del motor se habilitará con el instalador del plugin");
    }
}
