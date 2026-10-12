The installed Quickshell types are registered statically by its executable and
cannot be loaded by the standalone QtTest runner. `tools/test-qml` copies the
installed Omarchy Ui and Commons sources into a temporary import bridge, then
uses these small no-I/O doubles for Quickshell's environment, Process, collectors
and FileView. They never start commands or read user configuration. Tests exercise
the actual panel/controller code, styles, selections and JSON protocol behavior.
Compositor focus, IPC and plugin lifecycle are separately verified in Omarchy.

`tools/lint-qml` uses the real Quickshell type metadata and installed Omarchy
modules, without these doubles. The upstream Style.font and Style.bar objects
are exposed as QObject in that metadata; their member warnings also occur on the
stock clock. QProcess::ExitStatus metadata warns on the installed MINSAL
controller as well. These known metadata diagnostics are recorded separately
from plugin errors; new missing properties must still be fixed.
