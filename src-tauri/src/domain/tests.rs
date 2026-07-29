use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use proptest::prelude::*;

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
        provider_id: Some("ollama".into()),
        template_revision: Some(1),
        provider_name: "Local test".into(),
        dialect: ProviderDialect::Ollama,
        stream_protocol: Some(StreamProtocol::OllamaNdjson),
        auth_placement: Some(AuthPlacement::None),
        auth_header_name: None,
        additional_headers: BTreeMap::new(),
        base_url: "http://127.0.0.1:11434".into(),
        model: model.into(),
        parameters: BTreeMap::new(),
    }
}

fn context_checkpoint(
    id: &str,
    kind: ContextCheckpointKind,
    anchor_run_id: &str,
    first_kept_run_id: Option<&str>,
    summary: &str,
    source_run_ids: &[&str],
    created_at: i64,
) -> ContextCheckpoint {
    ContextCheckpoint {
        id: id.into(),
        workspace_id: "workspace".into(),
        maintenance_run_id: format!("maintenance-{id}"),
        kind,
        branch_pointer_id: Some("branch-main".into()),
        branch_revision: Some(1),
        anchor_run_id: anchor_run_id.into(),
        first_kept_run_id: first_kept_run_id.map(str::to_owned),
        summary: summary.into(),
        summary_content_block_id: format!("block-system-{}", sha256_hex(summary.as_bytes())),
        source_run_ids: source_run_ids.iter().map(|id| (*id).into()).collect(),
        source_hash: format!("source-hash-{id}"),
        provider: None,
        created_at,
    }
}

fn compile_input_with_all_checkpoints(
    request: ContextCompileRequest,
    checkpoints: Vec<ContextCheckpoint>,
) -> ContextCompileInput {
    let eligible_checkpoint_ids = checkpoints
        .iter()
        .map(|checkpoint| checkpoint.id.clone())
        .collect();
    ContextCompileInput::new(request)
        .with_checkpoints(checkpoints)
        .with_eligible_checkpoint_ids(eligible_checkpoint_ids)
}

fn context_pin(
    source_ref: ContextSourceRef,
    content_block_id: impl Into<String>,
    content: &str,
) -> ContextPin {
    ContextPin {
        source_ref,
        content_block_id: content_block_id.into(),
        content_hash: sha256_hex(content.as_bytes()),
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
            pinned_sources: vec![context_pin(
                ContextSourceRef::new(ContextSourceRefKind::ContentBlock, "pin-1"),
                "pin-1",
                "explicit evidence",
            )],
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
fn v4_context_items_have_typed_source_and_content_identity_and_mandatory_boundaries() {
    let root = Turn::root("root", "workspace", "root question", 1);
    let run_root = finished_run("run-root", "root", "root answer");
    let pin = ContentBlock {
        id: "pin-1".into(),
        workspace_id: "workspace".into(),
        role: MessageRole::User,
        content: "explicit evidence".into(),
        content_hash: "47603a7b9b1ccfc6da8e9c1af402bbd83fd52dbbfa1f937f4af6fff232d6440b".into(),
        created_at: 2,
    };
    let graph = ConversationGraph::try_new(vec![root], vec![run_root], vec![pin]).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());

    let preview = compiler
        .inspect(
            &graph,
            ContextCompileRequest {
                workspace_id: "workspace".into(),
                system_prompt: "system rules".into(),
                parent_run_id: Some("run-root".into()),
                current_prompt: "current question".into(),
                overrides: ContextOverrides {
                    pinned_sources: vec![
                        context_pin(
                            ContextSourceRef::new(ContextSourceRefKind::ContentBlock, "pin-1"),
                            "pin-1",
                            "explicit evidence",
                        ),
                        context_pin(
                            ContextSourceRef::new(ContextSourceRefKind::ContentBlock, "pin-1"),
                            "pin-1",
                            "explicit evidence",
                        ),
                    ],
                    excluded_source_ids: vec![
                        "workspace-system:workspace".into(),
                        "turn-prompt:root".into(),
                        "current-prompt:workspace".into(),
                    ],
                },
                provider: None,
            },
        )
        .unwrap();

    assert_eq!(preview.manifest.compiler_version, "4");
    assert_eq!(
        preview.warnings,
        vec![ContextWarning::DuplicatePinnedSource(
            "content-block:pin-1".into()
        )]
    );
    assert_eq!(preview.manifest.warnings, preview.warnings);
    assert_eq!(
        preview
            .raw_items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "system rules",
            "root question",
            "root answer",
            "current question"
        ]
    );
    assert_eq!(
        preview
            .manifest
            .items
            .iter()
            .map(|item| (
                item.source_ref.stable_id(),
                item.content_block_id.as_str(),
                item.content_hash.as_str(),
                item.mandatory,
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "workspace-system:workspace".into(),
                "block-system-97e2b5a8fd07a75081a1205a6f5154b39730b01846a2a399eaf5ae41b00b22a1",
                "97e2b5a8fd07a75081a1205a6f5154b39730b01846a2a399eaf5ae41b00b22a1",
                true,
            ),
            (
                "model-run:run-root".into(),
                "block-assistant-11096e058908f63470e94944f18f999e7b6643de070def2352b24d44b2c07dcc",
                "11096e058908f63470e94944f18f999e7b6643de070def2352b24d44b2c07dcc",
                false,
            ),
            (
                "content-block:pin-1".into(),
                "pin-1",
                "47603a7b9b1ccfc6da8e9c1af402bbd83fd52dbbfa1f937f4af6fff232d6440b",
                false,
            ),
            (
                "current-prompt:workspace".into(),
                "block-user-72ad45b358f34972966fd6791b48f08858dabb6ce75b6176c9ebf0174827cfe9",
                "72ad45b358f34972966fd6791b48f08858dabb6ce75b6176c9ebf0174827cfe9",
                true,
            ),
        ]
    );
}

#[test]
fn typed_pin_selects_the_exact_later_run_when_content_identity_is_shared() {
    let root = Turn::root("root", "workspace", "root question", 1);
    let run_root = finished_run("run-root", "root", "same answer");
    let child = Turn::branch("child", "workspace", "run-root", "child question", 2);
    let run_child = finished_run("run-child", "child", "same answer");
    let graph =
        ConversationGraph::try_new(vec![root, child], vec![run_root, run_child], Vec::new())
            .unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let shared_hash = sha256_hex(b"same answer");
    let shared_block_id = format!("block-assistant-{shared_hash}");

    let preview = compiler
        .inspect(
            &graph,
            ContextCompileRequest {
                workspace_id: "workspace".into(),
                system_prompt: "system".into(),
                parent_run_id: Some("run-child".into()),
                current_prompt: "next".into(),
                overrides: ContextOverrides {
                    pinned_sources: vec![ContextPin {
                        source_ref: ContextSourceRef::new(
                            ContextSourceRefKind::ModelRun,
                            "run-child",
                        ),
                        content_block_id: shared_block_id,
                        content_hash: shared_hash,
                    }],
                    excluded_source_ids: Vec::new(),
                },
                provider: None,
            },
        )
        .unwrap();

    let root_item = preview
        .manifest
        .items
        .iter()
        .find(|item| item.source_ref.stable_id() == "model-run:run-root")
        .unwrap();
    let child_item = preview
        .manifest
        .items
        .iter()
        .find(|item| item.source_ref.stable_id() == "model-run:run-child")
        .unwrap();
    assert_eq!(
        root_item.inclusion_reason,
        InclusionReason::ExactAncestorPath
    );
    assert_eq!(child_item.inclusion_reason, InclusionReason::ExplicitPin);
}

#[test]
fn pinned_content_identity_must_match_the_immutable_content() {
    let invalid_pin = ContentBlock {
        id: "pin-1".into(),
        workspace_id: "workspace".into(),
        role: MessageRole::User,
        content: "evidence".into(),
        content_hash: "not-the-content-hash".into(),
        created_at: 1,
    };

    let error = ConversationGraph::try_new(Vec::new(), Vec::new(), vec![invalid_pin])
        .expect_err("a pinned content identity cannot alias different bytes");

    assert_eq!(
        error,
        DomainError::InvalidContentBlockHash { id: "pin-1".into() }
    );
}

#[test]
fn latest_compaction_replaces_only_the_ancestry_before_its_kept_boundary() {
    let root = Turn::root("root", "workspace", "question root", 1);
    let run_root = finished_run("run-root", "root", "answer root");
    let turn_a = Turn::branch("turn-a", "workspace", "run-root", "question A", 2);
    let run_a = finished_run("run-a", "turn-a", "answer A");
    let turn_b = Turn::branch("turn-b", "workspace", "run-a", "question B", 3);
    let run_b = finished_run("run-b", "turn-b", "answer B");
    let turn_c = Turn::branch("turn-c", "workspace", "run-b", "question C", 4);
    let run_c = finished_run("run-c", "turn-c", "answer C");
    let turn_d = Turn::branch("turn-d", "workspace", "run-c", "question D", 5);
    let run_d = finished_run("run-d", "turn-d", "answer D");
    let graph = ConversationGraph::try_new(
        vec![root, turn_a, turn_b, turn_c, turn_d],
        vec![run_root, run_a, run_b, run_c, run_d],
        Vec::new(),
    )
    .unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let request = ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: "system".into(),
        parent_run_id: Some("run-d".into()),
        current_prompt: "question next".into(),
        overrides: ContextOverrides::default(),
        provider: None,
    };
    let mut old = context_checkpoint(
        "old",
        ContextCheckpointKind::Compaction,
        "run-b",
        Some("run-a"),
        "obsolete summary",
        &["run-root", "run-a"],
        10,
    );
    old.branch_revision = Some(1);
    let mut latest = context_checkpoint(
        "latest",
        ContextCheckpointKind::Compaction,
        "run-c",
        Some("run-b"),
        "latest summary",
        &["run-root", "run-a"],
        20,
    );
    latest.branch_revision = Some(2);

    let preview = compiler
        .inspect(
            &graph,
            compile_input_with_all_checkpoints(request.clone(), vec![latest.clone(), old.clone()]),
        )
        .unwrap();

    assert_eq!(
        preview
            .manifest
            .items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "system",
            "latest summary",
            "question B",
            "answer B",
            "question C",
            "answer C",
            "question D",
            "answer D",
            "question next",
        ]
    );
    assert_eq!(
        preview
            .raw_items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "system",
            "question root",
            "answer root",
            "question A",
            "answer A",
            "question B",
            "answer B",
            "question C",
            "answer C",
            "question D",
            "answer D",
            "question next",
        ]
    );
    assert_eq!(
        preview
            .applied_checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.checkpoint_id.as_str()),
        Some("latest")
    );
    assert_eq!(
        preview.manifest.checkpoint_provenance,
        preview.applied_checkpoint
    );

    let fallback = compiler
        .inspect(
            &graph,
            compile_input_with_all_checkpoints(
                ContextCompileRequest {
                    overrides: ContextOverrides {
                        pinned_sources: Vec::new(),
                        excluded_source_ids: vec!["checkpoint-summary:latest".into()],
                    },
                    ..request
                },
                vec![latest, old],
            ),
        )
        .unwrap();
    assert_eq!(
        fallback
            .applied_checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.checkpoint_id.as_str()),
        Some("old")
    );
    assert_eq!(
        fallback
            .manifest
            .items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "system",
            "obsolete summary",
            "question A",
            "answer A",
            "question B",
            "answer B",
            "question C",
            "answer C",
            "question D",
            "answer D",
            "question next",
        ]
    );
}

#[test]
fn later_compaction_at_the_same_anchor_supersedes_the_older_effective_context() {
    let root = Turn::root("root", "workspace", "question root", 1);
    let run_root = finished_run("run-root", "root", "answer root");
    let leaf = Turn::branch("leaf", "workspace", "run-root", "question leaf", 2);
    let run_leaf = finished_run("run-leaf", "leaf", "answer leaf");
    let graph =
        ConversationGraph::try_new(vec![root, leaf], vec![run_root, run_leaf], Vec::new()).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let request = ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: "system".into(),
        parent_run_id: Some("run-leaf".into()),
        current_prompt: "question next".into(),
        overrides: ContextOverrides::default(),
        provider: None,
    };
    let older = context_checkpoint(
        "older-at-leaf",
        ContextCheckpointKind::Compaction,
        "run-leaf",
        Some("run-leaf"),
        "obsolete summary",
        &["run-root"],
        10,
    );
    let newer = context_checkpoint(
        "newer-at-leaf",
        ContextCheckpointKind::Compaction,
        "run-leaf",
        Some("run-leaf"),
        "replacement summary",
        &["run-root"],
        11,
    );

    let preview = compiler
        .inspect(
            &graph,
            compile_input_with_all_checkpoints(request, vec![newer, older]),
        )
        .unwrap();

    assert_eq!(
        preview
            .manifest
            .items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "system",
            "replacement summary",
            "question leaf",
            "answer leaf",
            "question next",
        ],
        "only the strictly later summary and its kept tail may reach Provider context"
    );
    assert_eq!(
        preview
            .applied_checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.checkpoint_id.as_str()),
        Some("newer-at-leaf")
    );
    assert!(
        preview
            .manifest
            .items
            .iter()
            .all(|item| item.content != "obsolete summary"
                && item.content != "question root"
                && item.content != "answer root"),
        "the prior summary and compacted source path must not leak into effective items"
    );
    assert_eq!(
        preview
            .raw_items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "system",
            "question root",
            "answer root",
            "question leaf",
            "answer leaf",
            "question next",
        ],
        "immutable raw history remains inspectable after repeated compaction"
    );
}

#[test]
fn checkpoint_visibility_distinguishes_same_anchor_forks_created_before_and_after_it() {
    let root = Turn::root("root", "workspace", "question root", 1);
    let run_root = finished_run("run-root", "root", "answer root");
    let before = Turn::branch("turn-before", "workspace", "run-root", "question before", 2);
    let run_before = finished_run("run-before", "turn-before", "answer before");
    let after = Turn::branch("turn-after", "workspace", "run-root", "question after", 3);
    let run_after = finished_run("run-after", "turn-after", "answer after");
    let graph = ConversationGraph::try_new(
        vec![root, before, after],
        vec![run_root, run_before, run_after],
        Vec::new(),
    )
    .unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let checkpoint = context_checkpoint(
        "compact-at-root",
        ContextCheckpointKind::Compaction,
        "run-root",
        None,
        "root summary",
        &["run-root"],
        10,
    );
    let request = |parent_run_id: &str| ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: "system".into(),
        parent_run_id: Some(parent_run_id.into()),
        current_prompt: "next".into(),
        overrides: ContextOverrides::default(),
        provider: None,
    };

    let before_preview = compiler
        .inspect(
            &graph,
            ContextCompileInput::new(request("run-before"))
                .with_checkpoints(vec![checkpoint.clone()])
                .with_eligible_checkpoint_ids(Vec::new()),
        )
        .unwrap();
    assert_eq!(before_preview.applied_checkpoint, None);
    assert_eq!(
        before_preview
            .manifest
            .items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "system",
            "question root",
            "answer root",
            "question before",
            "answer before",
            "next",
        ]
    );

    let after_preview = compiler
        .inspect(
            &graph,
            ContextCompileInput::new(request("run-after"))
                .with_checkpoints(vec![checkpoint])
                .with_eligible_checkpoint_ids(vec!["compact-at-root".into()]),
        )
        .unwrap();
    assert_eq!(
        after_preview
            .applied_checkpoint
            .as_ref()
            .map(|checkpoint| checkpoint.checkpoint_id.as_str()),
        Some("compact-at-root")
    );
    assert_eq!(
        after_preview
            .manifest
            .items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "system",
            "root summary",
            "question after",
            "answer after",
            "next",
        ]
    );
}

#[test]
fn branch_scoped_checkpoint_fails_closed_without_visibility_evidence() {
    let root = Turn::root("root", "workspace", "question root", 1);
    let run_root = finished_run("run-root", "root", "answer root");
    let graph = ConversationGraph::try_new(vec![root], vec![run_root], Vec::new()).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let checkpoint = context_checkpoint(
        "compact",
        ContextCheckpointKind::Compaction,
        "run-root",
        None,
        "root summary",
        &["run-root"],
        10,
    );
    let request = ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: "system".into(),
        parent_run_id: Some("run-root".into()),
        current_prompt: "next".into(),
        overrides: ContextOverrides::default(),
        provider: None,
    };

    let error = compiler
        .inspect(
            &graph,
            ContextCompileInput::new(request).with_checkpoints(vec![checkpoint]),
        )
        .expect_err("branch-scoped checkpoint selection requires explicit visibility evidence");

    assert_eq!(
        error,
        DomainError::MissingCheckpointVisibilityEvidence {
            checkpoint_id: "compact".into(),
        }
    );
}

#[test]
fn checkpoints_follow_their_anchor_path_and_branch_summaries_are_injected_in_order() {
    let root = Turn::root("root", "workspace", "question root", 1);
    let run_root = finished_run("run-root", "root", "answer root");
    let turn_a = Turn::branch("turn-a", "workspace", "run-root", "question A", 2);
    let run_a = finished_run("run-a", "turn-a", "answer A");
    let turn_b = Turn::branch("turn-b", "workspace", "run-a", "question B", 3);
    let run_b = finished_run("run-b", "turn-b", "answer B");
    let before = Turn::branch("turn-before", "workspace", "run-root", "question before", 4);
    let run_before = finished_run("run-before", "turn-before", "answer before");
    let after = Turn::branch("turn-after", "workspace", "run-b", "question after", 5);
    let run_after = finished_run("run-after", "turn-after", "answer after");
    let graph = ConversationGraph::try_new(
        vec![root, turn_a, turn_b, before, after],
        vec![run_root, run_a, run_b, run_before, run_after],
        Vec::new(),
    )
    .unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let compaction = context_checkpoint(
        "compact",
        ContextCheckpointKind::Compaction,
        "run-b",
        Some("run-a"),
        "compacted root",
        &["run-root"],
        10,
    );
    let branch_summary = context_checkpoint(
        "branch",
        ContextCheckpointKind::BranchSummary,
        "run-b",
        None,
        "branch evidence",
        &["run-before"],
        20,
    );
    let sibling_summary = context_checkpoint(
        "sibling",
        ContextCheckpointKind::BranchSummary,
        "run-before",
        None,
        "must not leak",
        &["run-b"],
        30,
    );
    let checkpoints = vec![sibling_summary, branch_summary, compaction];
    let request = |parent_run_id: &str| ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: "system".into(),
        parent_run_id: Some(parent_run_id.into()),
        current_prompt: "next".into(),
        overrides: ContextOverrides::default(),
        provider: None,
    };

    let before_preview = compiler
        .inspect(
            &graph,
            compile_input_with_all_checkpoints(request("run-before"), checkpoints.clone()),
        )
        .unwrap();
    assert_eq!(before_preview.applied_checkpoint, None);
    assert_eq!(
        before_preview
            .manifest
            .items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "system",
            "question root",
            "answer root",
            "question before",
            "answer before",
            "must not leak",
            "next",
        ]
    );

    let after_preview = compiler
        .inspect(
            &graph,
            compile_input_with_all_checkpoints(request("run-after"), checkpoints.clone()),
        )
        .unwrap();
    assert_eq!(
        after_preview
            .manifest
            .items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "system",
            "compacted root",
            "question A",
            "answer A",
            "question B",
            "answer B",
            "branch evidence",
            "question after",
            "answer after",
            "next",
        ]
    );
    assert_eq!(
        after_preview
            .manifest
            .branch_summary_provenance
            .iter()
            .map(|checkpoint| checkpoint.checkpoint_id.as_str())
            .collect::<Vec<_>>(),
        vec!["branch"]
    );

    let without_branch_summary = compiler
        .inspect(
            &graph,
            compile_input_with_all_checkpoints(
                ContextCompileRequest {
                    overrides: ContextOverrides {
                        pinned_sources: Vec::new(),
                        excluded_source_ids: vec!["branch-summary:branch".into()],
                    },
                    ..request("run-after")
                },
                checkpoints,
            ),
        )
        .unwrap();
    assert_eq!(
        without_branch_summary
            .manifest
            .items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "system",
            "compacted root",
            "question A",
            "answer A",
            "question B",
            "answer B",
            "question after",
            "answer after",
            "next",
        ]
    );
    assert!(
        without_branch_summary
            .manifest
            .branch_summary_provenance
            .is_empty()
    );
}

#[test]
fn checkpoint_provenance_is_locked_into_the_context_hash() {
    let root = Turn::root("root", "workspace", "question root", 1);
    let run_root = finished_run("run-root", "root", "answer root");
    let graph = ConversationGraph::try_new(vec![root], vec![run_root], Vec::new()).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let request = ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: "system".into(),
        parent_run_id: Some("run-root".into()),
        current_prompt: "next".into(),
        overrides: ContextOverrides::default(),
        provider: None,
    };
    let checkpoint = context_checkpoint(
        "compact",
        ContextCheckpointKind::Compaction,
        "run-root",
        None,
        "same summary",
        &["run-root"],
        10,
    );
    let mut changed_provenance = checkpoint.clone();
    changed_provenance.source_hash = "different-source-hash".into();

    let first = compiler
        .inspect(
            &graph,
            compile_input_with_all_checkpoints(request.clone(), vec![checkpoint]),
        )
        .unwrap();
    let second = compiler
        .inspect(
            &graph,
            compile_input_with_all_checkpoints(request, vec![changed_provenance]),
        )
        .unwrap();

    assert_eq!(first.messages, second.messages);
    assert_ne!(first.preview_hash, second.preview_hash);
}

#[test]
fn persisted_branch_summary_cannot_carry_a_compaction_boundary() {
    let root = Turn::root("root", "workspace", "question root", 1);
    let run_root = finished_run("run-root", "root", "answer root");
    let graph = ConversationGraph::try_new(vec![root], vec![run_root], Vec::new()).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let checkpoint = context_checkpoint(
        "branch",
        ContextCheckpointKind::BranchSummary,
        "run-root",
        Some("run-root"),
        "branch evidence",
        &["run-root"],
        10,
    );

    let error = compiler
        .inspect(
            &graph,
            compile_input_with_all_checkpoints(
                ContextCompileRequest {
                    workspace_id: "workspace".into(),
                    system_prompt: "system".into(),
                    parent_run_id: Some("run-root".into()),
                    current_prompt: "next".into(),
                    overrides: ContextOverrides::default(),
                    provider: None,
                },
                vec![checkpoint],
            ),
        )
        .expect_err("branch-summary checkpoints never define a first-kept Run");

    assert_eq!(
        error,
        DomainError::InvalidCheckpointBoundary {
            checkpoint_id: "branch".into(),
            first_kept_run_id: "run-root".into(),
        }
    );
}

#[test]
fn checkpoint_summary_content_can_be_pinned_while_its_projection_is_excluded() {
    let root = Turn::root("root", "workspace", "question root", 1);
    let run_root = finished_run("run-root", "root", "answer root");
    let graph = ConversationGraph::try_new(vec![root], vec![run_root], Vec::new()).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let checkpoint = context_checkpoint(
        "compact",
        ContextCheckpointKind::Compaction,
        "run-root",
        None,
        "root summary",
        &["run-root"],
        10,
    );
    let summary_content_block_id = checkpoint.summary_content_block_id.clone();
    let preview = compiler
        .inspect(
            &graph,
            compile_input_with_all_checkpoints(
                ContextCompileRequest {
                    workspace_id: "workspace".into(),
                    system_prompt: "system".into(),
                    parent_run_id: Some("run-root".into()),
                    current_prompt: "next".into(),
                    overrides: ContextOverrides {
                        pinned_sources: vec![context_pin(
                            ContextSourceRef::new(
                                ContextSourceRefKind::ContentBlock,
                                summary_content_block_id.clone(),
                            ),
                            summary_content_block_id.clone(),
                            "root summary",
                        )],
                        excluded_source_ids: vec!["checkpoint-summary:compact".into()],
                    },
                    provider: None,
                },
                vec![checkpoint],
            ),
        )
        .unwrap();

    assert_eq!(preview.applied_checkpoint, None);
    assert_eq!(
        preview
            .manifest
            .items
            .iter()
            .map(|item| item.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "system",
            "question root",
            "answer root",
            "root summary",
            "next",
        ]
    );
    let pinned = &preview.manifest.items[3];
    assert_eq!(
        pinned.source_ref.stable_id(),
        format!("content-block:{summary_content_block_id}")
    );
    assert_eq!(pinned.content_block_id, summary_content_block_id);
    assert_eq!(pinned.inclusion_reason, InclusionReason::ExplicitPin);
    assert!(
        preview
            .raw_items
            .iter()
            .all(|item| item.source_ref.stable_id() != "checkpoint-summary:compact"),
        "rawItems is only the exact uncompressed route; checkpoint evidence is audited separately",
    );
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
fn depth_one_thousand_graph_validation_and_context_rebuild_stay_bounded() {
    let depth = 1_000;
    let mut turns = Vec::with_capacity(depth);
    let mut runs = Vec::with_capacity(depth);
    turns.push(Turn::root("turn-0", "workspace", "question-0", 1));
    runs.push(finished_run("run-0", "turn-0", "answer-0"));
    for index in 1..depth {
        turns.push(Turn::branch(
            format!("turn-{index}"),
            "workspace",
            format!("run-{}", index - 1),
            format!("question-{index}"),
            i64::try_from(index + 1).unwrap(),
        ));
        runs.push(finished_run(
            &format!("run-{index}"),
            &format!("turn-{index}"),
            &format!("answer-{index}"),
        ));
    }

    let validation_started = Instant::now();
    let graph = ConversationGraph::try_new(turns, runs, Vec::new()).unwrap();
    let validation_elapsed = validation_started.elapsed();
    assert!(
        validation_elapsed <= Duration::from_millis(250),
        "depth-{depth} graph validation took {validation_elapsed:?}"
    );

    let compiler = ContextCompiler::new(ContextPolicy::default());
    let request = ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: "system".into(),
        parent_run_id: Some(format!("run-{}", depth - 1)),
        current_prompt: "next".into(),
        overrides: ContextOverrides::default(),
        provider: None,
    };
    // Use the best of several identical runs so the gate measures compiler work
    // rather than test-runner scheduling while unrelated tests execute in parallel.
    let mut rebuild_elapsed = Duration::MAX;
    for _ in 0..8 {
        let rebuild_started = Instant::now();
        let preview = compiler.inspect(&graph, request.clone()).unwrap();
        rebuild_elapsed = rebuild_elapsed.min(rebuild_started.elapsed());
        assert_eq!(preview.raw_items.len(), depth * 2 + 2);
    }
    assert!(
        rebuild_elapsed <= Duration::from_millis(100),
        "depth-{depth} context rebuild took {rebuild_elapsed:?}"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn property_random_trees_match_an_independent_root_to_leaf_walker(
        parent_choices in prop::collection::vec(any::<u16>(), 1..64),
        selected_seed in any::<u16>(),
    ) {
        let node_count = parent_choices.len() + 1;
        let mut parent_indices = vec![None];
        let mut turns = vec![Turn::root("turn-0", "workspace", "question-0", 1)];
        let mut runs = vec![finished_run("run-0", "turn-0", "answer-0")];
        for index in 1..node_count {
            let parent_index = usize::from(parent_choices[index - 1]) % index;
            parent_indices.push(Some(parent_index));
            turns.push(Turn::branch(
                format!("turn-{index}"),
                "workspace",
                format!("run-{parent_index}"),
                format!("question-{index}"),
                i64::try_from(index + 1).unwrap(),
            ));
            runs.push(finished_run(
                &format!("run-{index}"),
                &format!("turn-{index}"),
                &format!("answer-{index}"),
            ));
        }
        let selected = usize::from(selected_seed) % node_count;
        let graph = ConversationGraph::try_new(turns, runs, Vec::new()).unwrap();
        let preview = ContextCompiler::new(ContextPolicy::default())
            .inspect(
                &graph,
                ContextCompileRequest {
                    workspace_id: "workspace".into(),
                    system_prompt: "system".into(),
                    parent_run_id: Some(format!("run-{selected}")),
                    current_prompt: "next".into(),
                    overrides: ContextOverrides::default(),
                    provider: None,
                },
            )
            .unwrap();

        let mut reverse_path = Vec::new();
        let mut cursor = selected;
        loop {
            reverse_path.push(cursor);
            let Some(parent) = parent_indices[cursor] else {
                break;
            };
            cursor = parent;
        }
        reverse_path.reverse();
        let mut expected = vec!["system".to_owned()];
        for index in reverse_path {
            expected.push(format!("question-{index}"));
            expected.push(format!("answer-{index}"));
        }
        expected.push("next".into());

        prop_assert_eq!(
            preview
                .manifest
                .items
                .iter()
                .map(|item| item.content.clone())
                .collect::<Vec<_>>(),
            expected
        );
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
fn resolved_template_revision_changes_invalidate_the_preview_hash() {
    let graph = ConversationGraph::try_new(Vec::new(), Vec::new(), Vec::new()).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let request = ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: String::new(),
        parent_run_id: None,
        current_prompt: "same prompt".into(),
        overrides: ContextOverrides::default(),
        provider: Some(provider("model-a")),
    };
    let preview = compiler.inspect(&graph, request.clone()).unwrap();
    let mut changed = request;
    changed.provider.as_mut().unwrap().template_revision = Some(2);

    let error = compiler
        .compile(&graph, changed, &preview.preview_hash)
        .expect_err("resolved template revision is part of the inspected request");

    assert!(matches!(error, DomainError::PreviewHashMismatch { .. }));
}

#[test]
fn context_preview_rejects_unresolved_provider_metadata() {
    let graph = ConversationGraph::try_new(Vec::new(), Vec::new(), Vec::new()).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let mut unresolved = provider("model-a");
    unresolved.stream_protocol = None;

    let error = compiler
        .inspect(
            &graph,
            ContextCompileRequest {
                workspace_id: "workspace".into(),
                system_prompt: String::new(),
                parent_run_id: None,
                current_prompt: "prompt".into(),
                overrides: ContextOverrides::default(),
                provider: Some(unresolved),
            },
        )
        .expect_err("send previews require a fully resolved Provider Template");

    assert_eq!(
        error,
        DomainError::UnresolvedProviderMetadata {
            field: "stream protocol"
        }
    );
}

#[test]
fn canonical_hash_frames_variable_provider_groups() {
    let graph = ConversationGraph::try_new(Vec::new(), Vec::new(), Vec::new()).unwrap();
    let compiler = ContextCompiler::new(ContextPolicy::default());
    let mut without_header = provider("header-value");
    without_header.base_url = "header-name".into();
    without_header
        .parameters
        .insert("base-url".into(), "model".into());
    let mut with_header = provider("model");
    with_header.base_url = "base-url".into();
    with_header
        .additional_headers
        .insert("header-name".into(), "header-value".into());
    let request = |provider| ContextCompileRequest {
        workspace_id: "workspace".into(),
        system_prompt: String::new(),
        parent_run_id: None,
        current_prompt: "same prompt".into(),
        overrides: ContextOverrides::default(),
        provider: Some(provider),
    };

    let first = compiler.inspect(&graph, request(without_header)).unwrap();
    let second = compiler.inspect(&graph, request(with_header)).unwrap();

    assert_ne!(
        first.preview_hash, second.preview_hash,
        "header and parameter group boundaries must be part of the canonical framing"
    );
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

#[test]
fn provider_template_catalog_exposes_protocol_and_auth_without_secrets() {
    let templates = provider_templates();
    let ids = templates
        .iter()
        .map(|template| template.provider_id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), templates.len(), "provider ids must be unique");
    assert_eq!(
        ids,
        std::collections::BTreeSet::from([
            "anthropic",
            "azure-openai",
            "google",
            "ollama",
            "openai",
            "openai-compatible",
            "openrouter",
        ])
    );
    for template in templates {
        assert!(template.revision > 0);
        assert_eq!(
            template.protocol.requires_additional_headers,
            !template.protocol.additional_headers.is_empty()
        );
        assert!(!template.default_base_url.contains("@"));
    }

    let openai = provider_template("openai").expect("OpenAI template");
    assert_eq!(openai.default_base_url, "https://api.openai.com/v1");
    assert_eq!(openai.protocol.stream_protocol, StreamProtocol::OpenAiSse);
    assert_eq!(openai.protocol.auth_placement, AuthPlacement::BearerHeader);
    assert_eq!(openai.protocol.models_endpoint, Some("/models"));
    assert!(openai.runtime_available);

    let anthropic = provider_template("anthropic").expect("Anthropic template");
    assert_eq!(
        anthropic.protocol.stream_protocol,
        StreamProtocol::AnthropicSse
    );
    assert_eq!(
        anthropic.protocol.auth_placement,
        AuthPlacement::ApiKeyHeader
    );
    assert_eq!(anthropic.protocol.auth_header_name, Some("x-api-key"));
    assert!(anthropic.protocol.requires_additional_headers);
    assert_eq!(
        anthropic.protocol.additional_headers,
        &[StaticHeader {
            name: "anthropic-version",
            value: "2023-06-01",
        }]
    );
    assert_eq!(anthropic.revision, 2);
    assert!(anthropic.runtime_available);

    let google = provider_template("google").expect("Google template");
    assert_eq!(google.protocol.stream_protocol, StreamProtocol::GoogleSse);
    assert_eq!(google.protocol.auth_header_name, Some("x-goog-api-key"));
    assert_eq!(google.revision, 2);
    assert!(google.runtime_available);

    let ollama = provider_template("ollama").expect("Ollama template");
    assert_eq!(ollama.revision, 2);
    assert_eq!(
        ollama.protocol.auth_placement,
        AuthPlacement::BearerHeader,
        "a session credential must remain usable for migrated proxied Ollama profiles"
    );
    assert_eq!(ollama.protocol.auth_header_name, Some("Authorization"));
    assert_eq!(ollama.default_base_url, "http://127.0.0.1:11434");
}
