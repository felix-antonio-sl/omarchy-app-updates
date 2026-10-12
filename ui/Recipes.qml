pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls as Controls
import qs.Commons
import qs.Ui as Ui

Column {
    id: root
    property var recipes: []
    property int currentRecipe: 0
    property int currentFile: 0
    readonly property var recipe: recipes[currentRecipe] || null
    readonly property var file: recipe && recipe.files ? recipe.files[currentFile] : null
    spacing: Style.space(8)
    Label {
        text: "Recetas AUR · revisa el código y sus cambios"
        font.bold: true
        width: parent.width
    }
    Flow {
        width: parent.width
        spacing: Style.space(4)
        Repeater {
            model: root.recipes
            delegate: Ui.Button {
                required property var modelData
                required property int index
                text: modelData.package_base
                selected: root.currentRecipe === index
                focusable: true
                onClicked: {
                    root.currentRecipe = index;
                    root.currentFile = 0;
                }
            }
        }
    }
    Label {
        width: parent.width
        text: root.recipe ? "Commit: " + root.recipe.commit + "\nSHA-256: " + root.recipe.digest : ""
        font.pixelSize: Style.font.caption
    }
    Label {
        width: parent.width
        text: root.recipe ? (root.recipe.dependencies || []).map(function (d) {
            return d.kind + ": " + d.expression;
        }).join("\n") : ""
    }
    Flow {
        width: parent.width
        spacing: Style.space(4)
        Repeater {
            model: root.recipe ? root.recipe.files : []
            delegate: Ui.Button {
                required property var modelData
                required property int index
                text: modelData.path
                selected: root.currentFile === index
                focusable: true
                onClicked: root.currentFile = index
            }
        }
    }
    Controls.ScrollView {
        width: parent.width
        height: Style.space(220)
        Controls.TextArea {
            text: root.file ? root.file.content : ""
            textFormat: TextEdit.PlainText
            readOnly: true
            selectByMouse: true
            wrapMode: TextEdit.NoWrap
            color: Color.foreground
            font.family: "monospace"
            font.pixelSize: Style.font.caption
            background: Rectangle {
                color: Qt.darker(Color.background, 1.1)
            }
        }
    }
    Label {
        text: "Cambios respecto de la revisión anterior"
        font.bold: true
        width: parent.width
    }
    Controls.ScrollView {
        width: parent.width
        height: Style.space(120)
        Controls.TextArea {
            text: root.recipe ? root.recipe.diff : ""
            textFormat: TextEdit.PlainText
            readOnly: true
            selectByMouse: true
            wrapMode: TextEdit.NoWrap
            color: Color.foreground
            font.family: "monospace"
            font.pixelSize: Style.font.caption
            background: Rectangle {
                color: Qt.darker(Color.background, 1.1)
            }
        }
    }
}
