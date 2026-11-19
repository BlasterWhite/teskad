//! Decoding tests of the generated protocol types (generated.rs).
//!
//! `testdata/schema-samples.jsonl` is written by scripts/codex-protocol/generate.mjs from the
//! pinned release's schema alone: a JSON instance of every type, every variant and every value.
//! The tests below are written here, from that schema and from openai/codex's own statement of the
//! wire format at the pinned tag (rust-v0.170.0, codex-rs/app-server-protocol/src/protocol/v2/
//! tests.rs, named where a case follows one of its tests). They check what the samples cannot:
//! whole JSON-RPC messages, the params and answers Hangar sends, and what a newer Codex may send.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::generated::*;

const SAMPLES: &str = include_str!("testdata/schema-samples.jsonl");

fn read<T: serde::de::DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).expect("the value decodes")
}

fn write<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("the value encodes")
}

#[test]
fn every_shape_the_schema_allows_reads_and_writes_back_unchanged() {
    let mut failures = Vec::new();
    let mut cases = 0;
    for line in SAMPLES.lines() {
        let case: Value = serde_json::from_str(line).expect("a sample is a line of JSON");
        let ty = case["type"].as_str().expect("a sample names its type");
        let label = case["case"].as_str().expect("a sample names its case");
        let json = &case["json"];
        cases += 1;
        match checks::decode_named(ty, json.clone()) {
            None => failures.push(format!("{ty}: no Rust type of that name")),
            Some(Err(e)) => failures.push(format!("{ty} ({label}): {e}\n    {json}")),
            Some(Ok((back, debug))) => {
                if debug.contains("Unrecognized(") {
                    failures.push(format!("{ty} ({label}): read as Unrecognized: {debug}"));
                } else if &back != json {
                    failures.push(format!(
                        "{ty} ({label}): wrote back\n    {back}\n  instead of\n    {json}"
                    ));
                }
            }
        }
    }
    assert!(
        cases > 600,
        "only {cases} samples: was testdata/schema-samples.jsonl generated?"
    );
    assert!(
        failures.is_empty(),
        "{} of {cases} samples:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn the_pin_is_the_release_the_samples_were_written_from() {
    assert_eq!(CODEX_VERSION, "0.170.0");
    assert_eq!(<requests::TurnSteer as ClientRequest>::METHOD, "turn/steer");
    assert_eq!(
        <requests::ThreadInjectItems as ClientRequest>::METHOD,
        "thread/inject_items"
    );
    assert_eq!(
        <notifications::Initialized as ClientNotification>::METHOD,
        "initialized"
    );
    assert_eq!(
        <server_requests::ItemFileChangeRequestApproval as ServerRequestMethod>::METHOD,
        "item/fileChange/requestApproval"
    );
    assert!(ServerNotification::METHODS.contains(&"item/completed"));
    assert!(ServerRequest::METHODS.contains(&"item/tool/requestUserInput"));
}

// -------------------------------------------------------------------------------- whole messages

#[test]
fn a_notification_reads_from_the_whole_message() {
    let notification: ServerNotification = read(json!({
        "jsonrpc": "2.0",
        "method": "turn/started",
        "params": {
            "threadId": "thr_1",
            "turn": { "id": "turn_1", "items": [], "status": "inProgress" },
        },
    }));
    assert_eq!(notification.method(), "turn/started");
    let ServerNotification::TurnStarted(started) = notification else {
        panic!("not turn/started: {notification:?}");
    };
    assert_eq!(started.thread_id, "thr_1");
    assert_eq!(started.turn.id, "turn_1");
    assert_eq!(started.turn.status, TurnStatus::InProgress);
}

#[test]
fn a_server_request_reads_with_its_id_beside_it() {
    let message = json!({
        "id": 42,
        "method": "item/commandExecution/requestApproval",
        "params": {
            "itemId": "call_1",
            "threadId": "thr_1",
            "turnId": "turn_1",
            "startedAtMs": 1_763_510_400_000_i64,
            "command": "cargo test -p hangar-core",
            "cwd": "/work/hangar",
            "reason": "The tests need the network.",
        },
    });
    let request: ServerRequest = read(message.clone());
    assert_eq!(request.method(), "item/commandExecution/requestApproval");
    let ServerRequest::ItemCommandExecutionRequestApproval(params) = request else {
        panic!("not a command approval: {request:?}");
    };
    assert_eq!(params.command.as_deref(), Some("cargo test -p hangar-core"));
    assert_eq!(params.cwd.as_deref(), Some("/work/hangar"));
    assert_eq!(params.item_id, "call_1");
    // The id stays the client's business: it reads it from the message itself.
    assert_eq!(message["id"], json!(42));
}

#[test]
fn a_question_reads_with_its_options() {
    let request: ServerRequest = read(json!({
        "id": "req-7",
        "method": "item/tool/requestUserInput",
        "params": {
            "itemId": "call_2",
            "threadId": "thr_1",
            "turnId": "turn_1",
            "isBlocking": true,
            "questions": [{
                "id": "framework",
                "header": "Framework",
                "question": "Which test framework?",
                "options": [
                    { "label": "cargo test", "description": "The workspace's own" },
                    { "label": "nextest", "description": "Faster, needs an install" },
                ],
            }],
        },
    }));
    let ServerRequest::ItemToolRequestUserInput(params) = request else {
        panic!("not a question: {request:?}");
    };
    assert_eq!(params.questions.len(), 1);
    assert_eq!(params.questions[0].id, "framework");
    let options = params.questions[0].options.as_ref().expect("options");
    assert_eq!(options[1].label, "nextest");
}

// -------------------------------------------------------------------------------- what Hangar sends

#[test]
fn the_handshake_hangar_sends_is_the_design_s() {
    // docs/native/agents.md §4.3: initialize {clientInfo: {name: "Hangar"}, capabilities: {experimentalApi: false}}.
    let params = InitializeParams {
        client_info: ClientInfo {
            name: "Hangar".into(),
            title: None,
            version: "2.4.3".into(),
        },
        capabilities: Some(InitializeCapabilities {
            experimental_api: Some(false),
            ..Default::default()
        }),
    };
    assert_eq!(
        write(&params),
        json!({
            "clientInfo": { "name": "Hangar", "version": "2.4.3" },
            "capabilities": { "experimentalApi": false },
        })
    );
    let answer: <requests::Initialize as ClientRequest>::Response = read(json!({
        "codexHome": "/home/user/.codex",
        "platformFamily": "unix",
        "platformOs": "linux",
        "userAgent": "codex_cli_rs/0.170.0",
    }));
    assert_eq!(answer.codex_home, "/home/user/.codex");
}

#[test]
fn params_hangar_sends_carry_only_what_it_sets() {
    assert_eq!(write(&ThreadStartParams::default()), json!({}));
    let start = ThreadStartParams {
        cwd: Some("/work/hangar".into()),
        model: Some("gpt-5.5".into()),
        developer_instructions: Some("You are a worker of the workspace.".into()),
        sandbox: Some(SandboxMode::WorkspaceWrite),
        ..Default::default()
    };
    assert_eq!(
        write(&start),
        json!({
            "cwd": "/work/hangar",
            "model": "gpt-5.5",
            "developerInstructions": "You are a worker of the workspace.",
            "sandbox": "workspace-write",
        })
    );

    let text = |t: &str| {
        UserInput::Text(TextUserInput {
            text: t.into(),
            text_elements: None,
        })
    };
    let turn = TurnStartParams {
        thread_id: "thr_1".into(),
        input: vec![text("Run the tests.")],
        ..Default::default()
    };
    assert_eq!(
        write(&turn),
        json!({ "threadId": "thr_1", "input": [{ "type": "text", "text": "Run the tests." }] })
    );
    let steer = TurnSteerParams {
        thread_id: "thr_1".into(),
        expected_turn_id: "turn_1".into(),
        input: vec![text("[Hangar] From agent \"Leo\": the board changed.")],
        client_user_message_id: None,
    };
    assert_eq!(
        write(&steer),
        json!({
            "threadId": "thr_1",
            "expectedTurnId": "turn_1",
            "input": [{ "type": "text", "text": "[Hangar] From agent \"Leo\": the board changed." }],
        })
    );
    let interrupt = TurnInterruptParams {
        thread_id: "thr_1".into(),
        turn_id: "turn_1".into(),
    };
    assert_eq!(
        write(&interrupt),
        json!({ "threadId": "thr_1", "turnId": "turn_1" })
    );
}

#[test]
fn the_answers_hangar_gives_are_codex_s_words() {
    let command = |decision| write(&CommandExecutionRequestApprovalResponse { decision });
    assert_eq!(
        command(CommandExecutionApprovalDecision::Accept),
        json!({ "decision": "accept" })
    );
    assert_eq!(
        command(CommandExecutionApprovalDecision::AcceptForSession),
        json!({ "decision": "acceptForSession" })
    );
    assert_eq!(
        command(CommandExecutionApprovalDecision::Decline),
        json!({ "decision": "decline" })
    );
    assert_eq!(
        command(CommandExecutionApprovalDecision::Cancel),
        json!({ "decision": "cancel" })
    );

    let patch = |decision| write(&FileChangeRequestApprovalResponse { decision });
    assert_eq!(
        patch(FileChangeApprovalDecision::Accept),
        json!({ "decision": "accept" })
    );
    assert_eq!(
        patch(FileChangeApprovalDecision::Decline),
        json!({ "decision": "decline" })
    );

    let answers = ToolRequestUserInputResponse {
        answers: BTreeMap::from([(
            "framework".to_string(),
            ToolRequestUserInputAnswer {
                answers: vec!["nextest".into()],
            },
        )]),
    };
    assert_eq!(
        write(&answers),
        json!({ "answers": { "framework": { "answers": ["nextest"] } } })
    );

    let elicitation = McpServerElicitationRequestResponse {
        action: McpServerElicitationAction::Decline,
        content: None,
        meta: None,
    };
    assert_eq!(write(&elicitation), json!({ "action": "decline" }));

    // The answer's type follows from the request's marker: what the client's generic code relies on.
    let read_back: <server_requests::ItemCommandExecutionRequestApproval as ServerRequestMethod>::Response =
        read(json!({ "decision": "acceptForSession" }));
    assert_eq!(
        read_back.decision,
        CommandExecutionApprovalDecision::AcceptForSession
    );
}

// -------------------------------------------------------------------------------- what Hangar reads

#[test]
fn turn_state_reads_from_the_protocol() {
    // docs/native/agents.md §4.2.3: waiting on approval, idle, and a turn that failed on the plan's limit.
    let waiting: ServerNotification = read(json!({
        "method": "thread/status/changed",
        "params": { "threadId": "thr_1", "status": { "type": "active", "activeFlags": ["waitingOnApproval"] } },
    }));
    let ServerNotification::ThreadStatusChanged(changed) = waiting else {
        panic!("{waiting:?}")
    };
    let ThreadStatus::Active(active) = changed.status else {
        panic!("not active: {:?}", changed.status)
    };
    assert_eq!(
        active.active_flags,
        vec![ThreadActiveFlag::WaitingOnApproval]
    );

    let idle: ThreadStatusChangedNotification =
        read(json!({ "threadId": "thr_1", "status": { "type": "idle" } }));
    assert_eq!(idle.status, ThreadStatus::Idle);

    let failed: ServerNotification = read(json!({
        "method": "turn/completed",
        "params": {
            "threadId": "thr_1",
            "turn": {
                "id": "turn_1",
                "items": [],
                "status": "failed",
                "error": { "message": "You've hit your usage limit.", "codexErrorInfo": "usageLimitExceeded" },
            },
        },
    }));
    let ServerNotification::TurnCompleted(completed) = failed else {
        panic!("{failed:?}")
    };
    assert_eq!(completed.turn.status, TurnStatus::Failed);
    let error = completed.turn.error.expect("the turn's error");
    assert_eq!(
        error.codex_error_info,
        Some(CodexErrorInfo::UsageLimitExceeded)
    );
}

#[test]
fn messages_and_commands_read_as_items() {
    let delta: ServerNotification = read(json!({
        "method": "item/agentMessage/delta",
        "params": { "threadId": "thr_1", "turnId": "turn_1", "itemId": "msg_1", "delta": "All " },
    }));
    let ServerNotification::ItemAgentMessageDelta(delta) = delta else {
        panic!("{delta:?}")
    };
    assert_eq!(delta.delta, "All ");

    let message: ItemCompletedNotification = read(json!({
        "threadId": "thr_1",
        "turnId": "turn_1",
        "completedAtMs": 1_763_510_460_000_i64,
        "item": { "type": "agentMessage", "id": "msg_1", "text": "All tests pass.", "phase": "final_answer" },
    }));
    let ThreadItem::AgentMessage(agent) = message.item else {
        panic!("{:?}", message.item)
    };
    assert_eq!(agent.text, "All tests pass.");
    assert_eq!(agent.phase, Some(MessagePhase::FinalAnswer));

    let command: ThreadItem = read(json!({
        "type": "commandExecution",
        "id": "call_1",
        "command": "cargo test -p hangar-core",
        "cwd": "/work/hangar",
        "processId": null,
        "status": "completed",
        "commandActions": [{ "type": "unknown", "command": "cargo test -p hangar-core" }],
        "aggregatedOutput": "test result: ok",
        "exitCode": 0,
        "durationMs": 5200,
    }));
    let ThreadItem::CommandExecution(command) = command else {
        panic!("{command:?}")
    };
    assert_eq!(command.status, CommandExecutionStatus::Completed);
    assert_eq!(command.exit_code, Some(0));
    assert!(matches!(
        command.command_actions[0],
        CommandAction::Unknown(_)
    ));
}

#[test]
fn a_generated_image_says_where_it_was_saved() {
    // docs/native/agents.md §4.0 and §5.3: the file lands under $CODEX_HOME/generated_images/.
    let saved = "/home/user/.codex/generated_images/thr_1/ig_1.png";
    let completed: ServerNotification = read(json!({
        "method": "item/completed",
        "params": {
            "threadId": "thr_1",
            "turnId": "turn_1",
            "completedAtMs": 1_763_510_460_000_i64,
            "item": {
                "type": "imageGeneration",
                "id": "ig_1",
                "status": "completed",
                "revisedPrompt": "A lighthouse at dusk, flat colours",
                "result": "",
                "savedPath": saved,
            },
        },
    }));
    let ServerNotification::ItemCompleted(completed) = completed else {
        panic!("{completed:?}")
    };
    let ThreadItem::ImageGeneration(image) = completed.item else {
        panic!("{:?}", completed.item)
    };
    assert_eq!(image.saved_path.as_deref(), Some(saved));
    assert_eq!(
        image.revised_prompt.as_deref(),
        Some("A lighthouse at dusk, flat colours")
    );

    // The other shape of the same item: the image by its id in the attachment store.
    let by_file: ThreadItem = read(
        json!({ "type": "imageGeneration", "id": "ig_2", "status": "completed", "fileId": "file_9" }),
    );
    let ThreadItem::ImageGeneration(by_file) = by_file else {
        panic!("{by_file:?}")
    };
    assert_eq!(by_file.file_id.as_deref(), Some("file_9"));
    assert_eq!(by_file.saved_path, None);
}

#[test]
fn token_usage_and_plan_limits_read() {
    // docs/native/agents.md §4.2.7: the live gauge and the plan limits, with their reset times.
    let usage: ServerNotification = read(json!({
        "method": "thread/tokenUsage/updated",
        "params": {
            "threadId": "thr_1",
            "turnId": "turn_1",
            "tokenUsage": {
                "total": { "inputTokens": 120_000, "cachedInputTokens": 90_000, "outputTokens": 4_000,
                           "reasoningOutputTokens": 1_500, "totalTokens": 124_000 },
                "last": { "inputTokens": 30_000, "cachedInputTokens": 28_000, "outputTokens": 800,
                          "reasoningOutputTokens": 200, "totalTokens": 30_800 },
                "modelContextWindow": 272_000,
            },
        },
    }));
    let ServerNotification::ThreadTokenUsageUpdated(usage) = usage else {
        panic!("{usage:?}")
    };
    assert_eq!(usage.token_usage.total.total_tokens, 124_000);
    assert_eq!(usage.token_usage.last.cached_input_tokens, 28_000);
    assert_eq!(usage.token_usage.model_context_window, Some(272_000));

    let limits: ServerNotification = read(json!({
        "method": "account/rateLimits/updated",
        "params": {
            "rateLimits": {
                "limitId": "codex",
                "planType": "plus",
                "primary": { "usedPercent": 42, "windowDurationMins": 300, "resetsAt": 1_763_528_400 },
                "secondary": { "usedPercent": 7, "windowDurationMins": 10_080, "resetsAt": 1_764_000_000 },
            },
        },
    }));
    let ServerNotification::AccountRateLimitsUpdated(limits) = limits else {
        panic!("{limits:?}")
    };
    let primary = limits.rate_limits.primary.expect("the five-hour window");
    assert_eq!(primary.used_percent, 42);
    assert_eq!(primary.resets_at, Some(1_763_528_400));
    assert_eq!(limits.rate_limits.plan_type, Some(PlanType::Plus));
}

// -------------------------------------------------------------------------------- a newer Codex

#[test]
fn fields_a_newer_codex_adds_are_ignored() {
    let completed: ServerNotification = read(json!({
        "method": "item/completed",
        "params": {
            "threadId": "thr_1",
            "turnId": "turn_1",
            "completedAtMs": 1,
            "aFieldOfLaterReleases": { "nested": true },
            "item": { "type": "agentMessage", "id": "msg_1", "text": "Done.", "anotherNewField": [1, 2] },
        },
    }));
    let ServerNotification::ItemCompleted(completed) = completed else {
        panic!("{completed:?}")
    };
    let ThreadItem::AgentMessage(agent) = completed.item else {
        panic!("{:?}", completed.item)
    };
    assert_eq!(agent.text, "Done.");
}

#[test]
fn an_item_type_a_newer_codex_adds_is_kept_as_it_came() {
    let item = json!({ "type": "hologram", "id": "h_1", "depth": 3 });
    let started: ItemStartedNotification = read(
        json!({ "threadId": "thr_1", "turnId": "turn_1", "startedAtMs": 1, "item": item.clone() }),
    );
    assert_eq!(started.item, ThreadItem::Unrecognized(item.clone()));
    assert_eq!(write(&started.item), item);
}

#[test]
fn values_a_newer_codex_adds_are_kept_as_they_came() {
    assert_eq!(
        read::<TurnStatus>(json!("teleported")),
        TurnStatus::Unrecognized("teleported".into())
    );
    assert_eq!(
        write(&TurnStatus::Unrecognized("teleported".into())),
        json!("teleported")
    );
    assert_eq!(
        read::<ThreadActiveFlag>(json!("waitingOnSomethingNew")),
        ThreadActiveFlag::Unrecognized("waitingOnSomethingNew".into())
    );
    // An enum of one value today is open all the same: the next release adds the second.
    assert_eq!(
        read::<ModelRerouteReason>(json!("aNewReason")),
        ModelRerouteReason::Unrecognized("aNewReason".into())
    );
    assert_eq!(
        read::<PlanType>(json!("some_future_plan")),
        PlanType::Unrecognized("some_future_plan".into())
    );
    assert_eq!(
        read::<ThreadStatus>(json!({ "type": "hibernating" })),
        ThreadStatus::Unrecognized(json!({ "type": "hibernating" }))
    );
}

#[test]
fn errors_read_in_both_shapes_and_unknown_ones_are_kept() {
    // The cases of openai/codex's codex_error_info_deserializes_unknown_string_as_other,
    // …_unknown_object_as_other and …_known_object_without_falling_back: Codex itself folds the
    // unknown ones into `other`; Hangar keeps what came.
    assert_eq!(
        read::<CodexErrorInfo>(json!("futureError")),
        CodexErrorInfo::Unrecognized(json!("futureError"))
    );
    let unknown = json!({ "futureError": { "detail": "unknown", "retryAfterSeconds": 30 } });
    assert_eq!(
        read::<CodexErrorInfo>(unknown.clone()),
        CodexErrorInfo::Unrecognized(unknown)
    );
    let known: CodexErrorInfo = read(json!({ "httpConnectionFailed": { "httpStatusCode": 503 } }));
    let CodexErrorInfo::HttpConnectionFailed(failed) = known else {
        panic!("{known:?}")
    };
    assert_eq!(failed.http_status_code, Some(503));
    assert_eq!(
        read::<CodexErrorInfo>(json!("other")),
        CodexErrorInfo::Other
    );
}

#[test]
fn a_notification_outside_the_subset_is_kept_whole() {
    let message = json!({ "method": "thread/realtime/started", "params": { "threadId": "thr_1" } });
    let notification: ServerNotification = read(message.clone());
    assert_eq!(notification, ServerNotification::Unrecognized(message));
    assert_eq!(notification.method(), "thread/realtime/started");
    assert!(!ServerNotification::METHODS.contains(&notification.method()));
}

#[test]
fn a_notification_this_version_cannot_read_says_which_it_was() {
    // turn/completed without its turn: kept, and its method tells the client to report it.
    let message = json!({ "method": "turn/completed", "params": { "threadId": "thr_1" } });
    let notification: ServerNotification = read(message.clone());
    assert_eq!(notification, ServerNotification::Unrecognized(message));
    assert!(ServerNotification::METHODS.contains(&notification.method()));
}

#[test]
fn a_server_request_outside_the_subset_is_kept_with_its_id() {
    // Hangar answers it with methodNotFound (docs/native/agents.md §4.3), which needs the id.
    let message = json!({ "id": 7, "method": "item/tool/call", "params": { "tool": "lookup", "arguments": {} } });
    let request: ServerRequest = read(message.clone());
    assert_eq!(request.method(), "item/tool/call");
    let ServerRequest::Unrecognized(kept) = request else {
        panic!("{request:?}")
    };
    assert_eq!(kept["id"], json!(7));
}

// -------------------------------------------------------------------------------- older servers

#[test]
fn a_thread_answer_without_the_newer_optional_fields_reads() {
    // The answer of openai/codex's thread_lifecycle_responses_default_missing_optional_fields.
    let answer = json!({
        "thread": {
            "id": "thread-id",
            "sessionId": "thread-id",
            "forkedFromId": null,
            "preview": "",
            "ephemeral": false,
            "modelProvider": "openai",
            "createdAt": 1,
            "updatedAt": 1,
            "status": { "type": "idle" },
            "path": null,
            "cwd": "/tmp",
            "cliVersion": "0.0.0",
            "source": "exec",
            "agentNickname": null,
            "agentRole": null,
            "gitInfo": null,
            "name": null,
            "turns": [],
        },
        "model": "gpt-5",
        "modelProvider": "openai",
        "serviceTier": null,
        "cwd": "/tmp",
        "approvalPolicy": "on-request",
        "approvalsReviewer": "user",
        "sandbox": { "type": "dangerFullAccess" },
        "reasoningEffort": null,
    });
    let start: ThreadStartResponse = read(answer.clone());
    assert_eq!(start.thread.id, "thread-id");
    assert_eq!(start.thread.project_id, None);
    assert_eq!(start.approval_policy, AskForApproval::OnRequest);
    assert_eq!(start.sandbox, SandboxPolicy::DangerFullAccess);
    let resume: ThreadResumeResponse = read(answer);
    assert_eq!(resume.thread.status, ThreadStatus::Idle);
}

#[test]
fn an_mcp_status_without_a_thread_reads() {
    // openai/codex's mcp_server_status_updated_accepts_missing_thread_id.
    let status: McpServerStatusUpdatedNotification =
        read(json!({ "name": "optional_broken", "status": "failed", "error": "handshake failed" }));
    assert_eq!(status.thread_id, None);
    assert_eq!(status.status, McpServerStartupState::Failed);
}

#[test]
fn a_turn_without_its_items_view_reads() {
    // openai/codex's turn_defaults_legacy_missing_items_view_to_full.
    let turn: Turn = read(json!({
        "id": "turn_123",
        "items": [],
        "status": "completed",
        "error": null,
        "startedAt": null,
        "completedAt": null,
        "durationMs": null,
    }));
    assert_eq!(turn.items_view, None);
    assert_eq!(turn.status, TurnStatus::Completed);
}

#[test]
fn a_granular_policy_without_its_newer_flags_reads() {
    // openai/codex's ask_for_approval_granular_defaults_missing_optional_flags_to_false.
    let policy: AskForApproval = read(
        json!({ "granular": { "sandbox_approval": true, "rules": false, "mcp_elicitations": true } }),
    );
    let AskForApproval::Granular(granular) = policy else {
        panic!("{policy:?}")
    };
    assert!(granular.sandbox_approval);
    assert!(granular.mcp_elicitations);
    assert_eq!(granular.skill_approval, None);
}

#[test]
fn an_mcp_form_reads_each_field_as_its_kind() {
    // The form of openai/codex's mcp_elicitation_schema_matches_mcp_2025_11_25_primitives, in an
    // `mcpServer/elicitation/request` (docs/native/agents.md §4.2.6: Hangar shows it to the person).
    let request: ServerRequest = read(json!({
        "id": 3,
        "method": "mcpServer/elicitation/request",
        "params": {
            "serverName": "tickets",
            "threadId": "thr_1",
            "turnId": "turn_1",
            "mode": "form",
            "message": "Create the ticket?",
            "requestedSchema": {
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {
                    "email": { "type": "string", "title": "Email", "format": "email", "default": "dev@example.com" },
                    "count": { "type": "integer", "title": "Count", "minimum": 1, "maximum": 5, "default": 3 },
                    "confirmed": { "type": "boolean", "title": "Confirm", "default": true },
                    "legacyChoice": {
                        "type": "string",
                        "title": "Action",
                        "enum": ["allow", "deny"],
                        "enumNames": ["Allow", "Deny"],
                        "default": "allow",
                    },
                    "choice": { "type": "string", "enum": ["low", "high"] },
                    "when": { "type": "date-time" },
                },
                "required": ["email", "confirmed"],
            },
        },
    }));
    let ServerRequest::McpServerElicitationRequest(McpServerElicitationRequestParams::Form(form)) =
        request
    else {
        panic!("not a form: {request:?}");
    };
    assert_eq!(form.server_name, "tickets");
    let field = |name: &str| {
        form.requested_schema
            .properties
            .get(name)
            .expect(name)
            .clone()
    };
    assert!(matches!(
        field("email"),
        McpElicitationPrimitiveSchema::String(_)
    ));
    let McpElicitationPrimitiveSchema::Number(count) = field("count") else {
        panic!("count: {:?}", field("count"));
    };
    assert_eq!(count.r#type, McpElicitationNumberType::Integer);
    assert_eq!(count.maximum, Some(5.0));
    assert!(matches!(
        field("confirmed"),
        McpElicitationPrimitiveSchema::Boolean(_)
    ));
    assert!(matches!(
        field("legacyChoice"),
        McpElicitationPrimitiveSchema::Enum(McpElicitationEnumSchema::LegacyTitled(_))
    ));
    assert!(matches!(
        field("choice"),
        McpElicitationPrimitiveSchema::Enum(McpElicitationEnumSchema::SingleSelect(
            McpElicitationSingleSelectEnumSchema::Untitled(_)
        ))
    ));
    // A kind of field this version does not know: kept, the rest of the form still reads.
    assert_eq!(
        field("when"),
        McpElicitationPrimitiveSchema::Unrecognized(json!({ "type": "date-time" }))
    );
}
