use std::collections::BTreeMap;

use super::*;

fn queued_run() -> ModelRun {
    ModelRun::queued(RunDraft {
        id: "run-1".into(),
        turn_id: "turn-1".into(),
        provider_profile_id: Some("provider-1".into()),
        model: "test-model".into(),
        created_at: 10,
    })
}

fn finished_run(id: &str, turn_id: &str, output: &str) -> ModelRun {
    let mut run = ModelRun::queued(RunDraft {
        id: id.into(),
        turn_id: turn_id.into(),
        provider_profile_id: Some("provider-1".into()),
        model: "test-model".into(),
        created_at: 1,
    });
    run.connect(2).unwrap();
    run.begin_streaming(3).unwrap();
    run.checkpoint(output, "", 4).unwrap();
    run.complete(None, 5).unwrap();
    run
}

fn provider(model: &str) -> ProviderSnapshot {
    ProviderSnapshot {
        profile_id: "provider-1".into(),
        provider_name: "Local test".into(),
        dialect: ProviderDialect::Ollama,
        base_url: "http://127.0.0.1:11434".into(),
        model: model.into(),
        parameters: BTreeMap::new(),
    }
}

#[test]
fn completed_run_is_immutable() {
    let mut run = queued_run();
    run.connect(11).expect("queued run starts connecting");
    run.begin_streaming(12)
        .expect("connected run begins streaming");
    run.checkpoint("partial", "", 13)
        .expect("running run accepts a checkpoint");
    run.complete(Some(RunUsage::new(4, 7)), 14)
        .expect("running run completes");

    let error = run
        .checkpoint(" should not append", "", 15)
        .expect_err("completed output is immutable");

    assert_eq!(
        error,
        DomainError::TerminalRunMutation(RunStatus::Completed)
    );
    assert_eq!(run.output_markdown(), "partial");
    assert_eq!(run.status(), RunStatus::Completed);
}

#[test]
fn exact_parent_run_must_belong_to_the_same_workspace() {
    let parent_turn = Turn::root("turn-a", "workspace-a", "root prompt", 1);
    let parent_run = ModelRun::queued(RunDraft {
        id: "run-a".into(),
        turn_id: parent_turn.id.clone(),
        provider_profile_id: Some("provider-1".into()),
        model: "test-model".into(),
        created_at: 2,
    });
    let foreign_child = Turn::branch(
        "turn-b",
        "workspace-b",
        parent_run.id.clone(),
        "must not cross workspaces",
        3,
    );

    let error = ConversationGraph::try_new(
        vec![parent_turn, foreign_child],
        vec![parent_run],
        Vec::new(),
    )
    .expect_err("cross-workspace parent links are invalid");

    assert_eq!(
        error,
        DomainError::CrossWorkspaceParent {
            turn_id: "turn-b".into(),
            parent_run_id: "run-a".into(),
        }
    );
}

#[test]
fn context_contains_only_the_exact_ancestor_run_path() {
    let root = Turn::root("root", "workspace", "root question", 1);
    let run_root = finished_run("run-root", "root", "root answer");
    let branch_a = Turn::branch("a", "workspace", "run-root", "question A", 2);
    let run_a = finished_run("run-a", "a", "answer A");
    let branch_b = Turn::branch("b", "workspace", "run-root", "question B", 2);
    let run_b = finished_run("run-b", "b", "answer B");
    let graph = ConversationGraph::try_new(
        vec![root, branch_a, branch_b],
        vec![run_root, run_a, run_b],
        Vec::new(),
    )
    .unwrap();
    let compiler = ContextCompiler::new(ContextPolicy {
        compiler_version: "test-v1".into(),
        max_chars: 10_000,
    });

    let preview = compiler
        .inspect(
            &graph,
            ContextCompileRequest {
                workspace_id: "workspace".into(),
                system_prompt: "system rules".into(),
                parent_run_id: Some("run-a".into()),
                current_prompt: "follow A".into(),
                overrides: ContextOverrides::default(),
                provider: None,
            },
        )
        .unwrap();

    let contents: Vec<_> = preview
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(
        contents,
        vec![
            "system rules",
            "root question",
            "root answer",
            "question A",
            "answer A",
            "follow A",
        ]
    );
    assert!(!contents.contains(&"question B"));
    assert!(!contents.contains(&"answer B"));
}

#[test]
fn pin_and_exclude_are_applied_without_mutating_ancestor_content() {
    let root = Turn::root("root", "workspace", "root question", 1);
    let run_root = finished_run("run-root", "root", "root answer");
    let pin = ContentBlock {
        id: "pin-1".into(),
        workspace_id: "workspace".into(),
        role: MessageRole::User,
        content: "explicit evidence".into(),
        content_hash: sha256_hex(b"explicit evidence"),
        created_at: 2,
    };
    let graph = ConversationGraph::try_new(vec![root], vec![run_root], vec![pin]).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let request = ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: "system".into(),
        parent_run_id: Some("run-root".into()),
        current_prompt: "next".into(),
        overrides: ContextOverrides {
            pinned_source_ids: vec!["pin-1".into()],
            excluded_source_ids: vec!["root".into()],
        },
        provider: None,
    };

    let preview = compiler.inspect(&graph, request).unwrap();
    let contents: Vec<_> = preview
        .manifest
        .items
        .iter()
        .map(|item| item.content.as_str())
        .collect();

    assert_eq!(
        contents,
        vec!["system", "root answer", "explicit evidence", "next"]
    );
    assert_eq!(graph.turn("root").unwrap().prompt_markdown, "root question");
}

#[test]
fn over_limit_context_can_be_inspected_but_not_compiled_for_sending() {
    let graph = ConversationGraph::try_new(Vec::new(), Vec::new(), Vec::new()).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy {
        compiler_version: "test-v1".into(),
        max_chars: 5,
    });
    let request = ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: String::new(),
        parent_run_id: None,
        current_prompt: "123456".into(),
        overrides: ContextOverrides::default(),
        provider: None,
    };

    let preview = compiler.inspect(&graph, request.clone()).unwrap();
    assert_eq!(
        preview.warnings,
        vec![ContextWarning::ExceedsLimit {
            estimated_chars: 6,
            max_chars: 5,
        }]
    );

    let error = compiler
        .compile(&graph, request, &preview.preview_hash)
        .expect_err("over-limit context must not be sent");
    assert_eq!(
        error,
        DomainError::ContextTooLarge {
            estimated_chars: 6,
            max_chars: 5,
        }
    );
}

#[test]
fn changed_context_is_rejected_when_preview_hash_is_stale() {
    let graph = ConversationGraph::try_new(Vec::new(), Vec::new(), Vec::new()).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let inspected = ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: String::new(),
        parent_run_id: None,
        current_prompt: "first draft".into(),
        overrides: ContextOverrides::default(),
        provider: None,
    };
    let preview = compiler.inspect(&graph, inspected).unwrap();
    let changed = ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: String::new(),
        parent_run_id: None,
        current_prompt: "changed draft".into(),
        overrides: ContextOverrides::default(),
        provider: None,
    };

    let error = compiler
        .compile(&graph, changed, &preview.preview_hash)
        .expect_err("stale previews cannot authorize a different request");

    assert!(matches!(error, DomainError::PreviewHashMismatch { .. }));
}

#[test]
fn canonical_hash_uses_standard_sha256() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn interrupted_run_rehydrates_with_partial_output_and_remains_immutable() {
    let run = ModelRun::rehydrate(
        RunDraft {
            id: "run-restored".into(),
            turn_id: "turn-1".into(),
            provider_profile_id: None,
            model: "removed-profile-model".into(),
            created_at: 1,
        },
        RunStateSnapshot {
            status: RunStatus::Interrupted,
            output_markdown: "recoverable partial text".into(),
            reasoning_markdown: String::new(),
            error: None,
            usage: None,
            started_at: Some(2),
            checkpointed_at: Some(3),
            finished_at: Some(4),
        },
    )
    .expect("persisted interrupted run is valid");

    assert_eq!(run.status(), RunStatus::Interrupted);
    assert_eq!(run.output_markdown(), "recoverable partial text");
    assert_eq!(run.state_snapshot().finished_at, Some(4));
}

#[test]
fn branch_cannot_target_an_unfinished_run() {
    let parent_turn = Turn::root("root", "workspace", "root", 1);
    let parent_run = ModelRun::queued(RunDraft {
        id: "run-active".into(),
        turn_id: parent_turn.id.clone(),
        provider_profile_id: Some("provider-1".into()),
        model: "test-model".into(),
        created_at: 2,
    });
    let child = Turn::branch("child", "workspace", "run-active", "too early", 3);

    let error = ConversationGraph::try_new(vec![parent_turn, child], vec![parent_run], Vec::new())
        .expect_err("a branch needs a stable exact answer version");

    assert_eq!(
        error,
        DomainError::ParentRunNotBranchable {
            turn_id: "child".into(),
            parent_run_id: "run-active".into(),
            status: RunStatus::Queued,
        }
    );
}

#[test]
fn provider_failure_can_finish_a_run_before_streaming_begins() {
    let mut run = queued_run();
    run.connect(2).unwrap();

    run.fail("connection refused", 3)
        .expect("connection failures are terminal provider outcomes");

    assert_eq!(run.status(), RunStatus::Failed);
    assert_eq!(run.error(), Some("connection refused"));
    assert_eq!(run.finished_at(), Some(3));
}

#[test]
fn property_wide_branches_never_leak_sibling_context() {
    let root = Turn::root("root", "workspace", "root prompt", 1);
    let root_run = finished_run("root-run", "root", "root answer");
    let mut turns = vec![root];
    let mut runs = vec![root_run];
    for index in 0..24 {
        let turn_id = format!("branch-{index}");
        let run_id = format!("run-{index}");
        turns.push(Turn::branch(
            &turn_id,
            "workspace",
            "root-run",
            format!("question-marker-{index}-end"),
            2 + index,
        ));
        runs.push(finished_run(
            &run_id,
            &turn_id,
            &format!("answer-marker-{index}-end"),
        ));
    }
    let graph = ConversationGraph::try_new(turns, runs, Vec::new()).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());

    for selected in 0..24 {
        let preview = compiler
            .inspect(
                &graph,
                ContextCompileRequest {
                    workspace_id: "workspace".into(),
                    system_prompt: String::new(),
                    parent_run_id: Some(format!("run-{selected}")),
                    current_prompt: "continue".into(),
                    overrides: ContextOverrides::default(),
                    provider: None,
                },
            )
            .unwrap();
        let joined = preview
            .messages
            .iter()
            .map(|message| message.content.as_str())
            .collect::<Vec<_>>()
            .join("|");
        assert!(joined.contains(&format!("question-marker-{selected}-end")));
        assert!(joined.contains(&format!("answer-marker-{selected}-end")));
        for sibling in 0..24 {
            if sibling != selected {
                assert!(!joined.contains(&format!("question-marker-{sibling}-end")));
                assert!(!joined.contains(&format!("answer-marker-{sibling}-end")));
            }
        }
    }
}

#[test]
fn cyclic_exact_parent_links_are_rejected() {
    let turn_a = Turn::branch("a", "workspace", "run-b", "a", 1);
    let turn_b = Turn::branch("b", "workspace", "run-a", "b", 2);
    let run_a = finished_run("run-a", "a", "answer a");
    let run_b = finished_run("run-b", "b", "answer b");

    let error = ConversationGraph::try_new(vec![turn_a, turn_b], vec![run_a, run_b], Vec::new())
        .expect_err("the conversation topology must remain acyclic");

    assert!(matches!(error, DomainError::CyclicAncestry { .. }));
}

#[test]
fn provider_snapshot_changes_invalidate_the_preview_hash() {
    let graph = ConversationGraph::try_new(Vec::new(), Vec::new(), Vec::new()).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let inspected = ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: String::new(),
        parent_run_id: None,
        current_prompt: "same prompt".into(),
        overrides: ContextOverrides::default(),
        provider: Some(provider("model-a")),
    };
    let preview = compiler.inspect(&graph, inspected).unwrap();
    let changed_provider = ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: String::new(),
        parent_run_id: None,
        current_prompt: "same prompt".into(),
        overrides: ContextOverrides::default(),
        provider: Some(provider("model-b")),
    };

    let error = compiler
        .compile(&graph, changed_provider, &preview.preview_hash)
        .expect_err("provider and model are part of the inspected request");

    assert!(matches!(error, DomainError::PreviewHashMismatch { .. }));
}

#[test]
fn structured_run_failure_survives_rehydration() {
    let run = ModelRun::rehydrate(
        RunDraft {
            id: "run-failed".into(),
            turn_id: "turn-root".into(),
            provider_profile_id: Some("provider-1".into()),
            model: "model".into(),
            created_at: 1,
        },
        RunStateSnapshot {
            status: RunStatus::Failed,
            output_markdown: "partial".into(),
            reasoning_markdown: String::new(),
            error: Some(RunFailure {
                code: "rate_limit".into(),
                message: "Too many requests".into(),
                retryable: true,
                status: Some(429),
            }),
            usage: None,
            started_at: Some(2),
            checkpointed_at: Some(2),
            finished_at: Some(3),
        },
    )
    .unwrap();

    assert_eq!(run.error(), Some("Too many requests"));
    assert_eq!(run.failure().unwrap().code, "rate_limit");
    assert!(run.failure().unwrap().retryable);
    assert_eq!(run.failure().unwrap().status, Some(429));
}
