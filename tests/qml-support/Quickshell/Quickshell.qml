pragma Singleton
import QtQuick

QtObject {
    function env(name) {
        return name === "HOME" ? "/nonexistent-fixture-home" : "";
    }
    function execDetached(argv) {
        throw new Error("External commands forbidden in the QtTest harness");
    }
}
