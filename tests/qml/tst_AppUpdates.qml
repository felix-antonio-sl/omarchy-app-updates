import QtQuick
import QtTest
import "../../ui" as Apps
import "../../ui/ViewModel.js" as VM

TestCase {
    id: test
    name: "AppUpdates"
    when: windowShown
    width: 720
    height: 640
    property var fixture
    QtObject {
        id: fake
        property var report: ({
                apps: [],
                checks: [],
                selected: [],
                coverage: {
                    status: "verified"
                }
            })
        property bool busy: false
        property string error: ""
        property var plan: null
        property var recipes: []
        property var job: null
        property var history: []
        property bool engineReady: true
        property var requests: []
        function saveSelection(ids) {
            requests.push({
                action: "selection",
                ids: ids
            });
            report = Object.assign({}, report, {
                selected: ids
            });
        }
        function refresh(force) {
            requests.push({
                action: "check",
                force: force
            });
        }
        function preparePlan(ids) {
            requests.push({
                action: "plan",
                ids: ids
            });
        }
        function applyPlan(digest) {
            requests.push({
                action: "apply",
                digest: digest
            });
        }
        function answer(op, q, d, v) {
            requests.push({
                action: "decision",
                op: op,
                q: q,
                d: d,
                v: v
            });
        }
        function loadHistory() {
            requests.push({
                action: "history"
            });
        }
        function reopen(op, app) {
            requests.push({
                action: "reopen",
                op: op,
                app: app
            });
        }
        function prepareEngine() {
            requests.push({
                action: "engine"
            });
        }
    }
    Component {
        id: clientFactory
        Apps.Controller {
            autoStart: false
        }
    }
    Component {
        id: bodyFactory
        Apps.PanelContent {
            width: 700
            height: 600
            controller: fake
        }
    }
    property var body
    function initTestCase() {
        var xhr = new XMLHttpRequest();
        xhr.open("GET", Qt.resolvedUrl("../fixtures/ui-report.json"), false);
        xhr.send();
        fixture = JSON.parse(xhr.responseText);
    }
    function init() {
        fake.report = JSON.parse(JSON.stringify(fixture));
        fake.requests = [];
        fake.error = "";
        fake.plan = null;
        fake.job = null;
        fake.history = [];
        fake.busy = false;
        fake.engineReady = true;
        body = createTemporaryObject(bodyFactory, test);
        verify(body !== null);
    }
    function test_count_and_filters() {
        compare(VM.updateCount(fake.report), 2);
        compare(VM.visibleRows(fake.report, "updates", "").length, 2);
        compare(VM.visibleRows(fake.report, "all", "whatsapp")[0].id, "web:whatsapp");
        compare(VM.visibleRows(fake.report, "selected", "").length, 3);
    }
    function test_literal_text_stale_and_errors() {
        var text = findChild(body, "appName_repo:chrome");
        verify(text !== null);
        compare(text.textFormat, Text.PlainText);
        compare(text.text, "Chrome <b>literal</b>");
        verify(VM.stateLabel(fake.report.checks[2]).indexOf("anterior") >= 0);
        fake.error = "Autorización cancelada <b>";
        wait(0);
        compare(findChild(body, "errorLabel").textFormat, Text.PlainText);
        compare(findChild(body, "errorLabel").text, fake.error);
        verify(VM.checkedLabel(fake.report).length > 0);
    }
    function test_selection_and_history_without_executing_binary() {
        body.toggleSelection("repo:chrome");
        compare(fake.requests[0].action, "selection");
        verify(fake.requests[0].ids.indexOf("repo:chrome") < 0);
        body.showHistory();
        compare(fake.requests[1].action, "history");
    }
    function test_full_plan_and_revised_digest() {
        fake.plan = {
            id: "p",
            digest: "new",
            steps: [
                {
                    kind: "arch_full",
                    targets: [],
                    package_changes: [
                        {
                            name: "extra-library",
                            old_version: "1",
                            new_version: "2",
                            reason: "system",
                            action: "upgrade"
                        }
                    ],
                    flatpak_changes: [],
                    downloads: []
                }
            ],
            notices: ["Alcance completo"]
        };
        wait(0);
        verify(body.planText.indexOf("extra-library") >= 0);
        body.confirmPlan("old");
        compare(fake.requests.length, 0);
        body.confirmPlan("new");
        compare(fake.requests[0].digest, "new");
        fake.error = "Autorización cancelada";
        fake.plan = null;
        body.confirmPlan("new");
        compare(fake.requests.length, 1);
    }
    function test_escape_and_cached_operation() {
        fake.job = {
            operation: {
                id: "op",
                phase: "installing",
                events: [],
                results: []
            },
            pending: null
        };
        var closed = false;
        body.closeRequested.connect(function () {
            closed = true;
        });
        body.forceActiveFocus();
        keyClick(Qt.Key_Escape);
        verify(closed);
        compare(fake.job.operation.id, "op");
    }
    function test_source_urls_are_not_arbitrary_commands() {
        verify(VM.safeLink("https://telegram.org/dl/desktop/linux"));
        verify(!VM.safeLink("file:///etc/passwd"));
        verify(!VM.safeLink("javascript:alert(1)"));
        verify(!VM.safeLink("https://telegram.org.evil.example/"));
        verify(!VM.safeLink("https://github.com/attacker/arbitrary"));
    }
    function test_controller_uses_json_stdin_and_rejects_foreign_response() {
        var client = createTemporaryObject(clientFactory, test);
        verify(client !== null);
        client.report = fake.report;
        client.saveSelection(["repo:steam"]);
        var process = findChild(client, "clientProcess");
        verify(process !== null);
        process.started();
        var request = JSON.parse(process.writes[0]);
        compare(request.action, "selection");
        compare(request.payload.selected[0], "repo:steam");
        compare(process.command.length, 1);
        client.acceptResponse({
            protocol_version: 1,
            request_id: "foreign",
            result: {
                apps: [],
                checks: []
            }
        });
        compare(client.report.apps.length, 5);
        client.acceptResponse({
            protocol_version: 1,
            request_id: request.request_id,
            result: fake.report
        });
        verify(client.report.apps.length === 5);
        client.plan = {
            id: "plan",
            digest: "new"
        };
        client.applyPlan("old");
        verify(client.error.indexOf("alcance") >= 0);
    }
    function test_decision_queued_during_observation_is_not_discarded() {
        var client = createTemporaryObject(clientFactory, test);
        verify(client !== null);
        client.job = {
            operation: {
                id: "op",
                phase: "awaiting_decision",
                events: [],
                results: []
            },
            pending: {
                question_id: "q",
                digest: "d",
                choices: ["approve", "reject"]
            }
        };
        client.request("operation", {
            operation_id: "op"
        });
        client.answer("op", "q", "d", "approve");
        compare(client.queue.length, 1);
        compare(client.queue[0].payload.decision.digest, "d");
    }
    function test_reopen_requires_an_explicit_second_action() {
        fake.job = {
            operation: {
                id: "op",
                phase: "completed",
                events: [],
                results: [
                    {
                        target_id: "repo:chrome",
                        outcome: "needs_reopen"
                    }
                ]
            },
            report: fake.report,
            pending: null
        };
        wait(0);
        compare(fake.requests.length, 0);
        body.chooseReopen("repo:chrome");
        compare(fake.requests.length, 0);
        body.confirmReopen();
        compare(fake.requests[0].action, "reopen");
        compare(fake.requests[0].app, "repo:chrome");
    }
}
