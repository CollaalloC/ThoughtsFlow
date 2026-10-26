//! Validate the identity proved by a successful Orca envelope before committing it.
use serde_json::Value;

pub(super) enum ExpectedReceipt<'a> {
    Terminal,
    Run {
        id: Option<&'a str>,
        coordinator: &'a str,
    },
    StartTask {
        run_id: &'a str,
        requested_model: Option<&'a str>,
    },
    Reply {
        question_id: &'a str,
        thread_id: Option<&'a str>,
        run_id: &'a str,
    },
    Release {
        dispatch_id: &'a str,
    },
    Output {
        dispatch_id: &'a str,
        task_id: &'a str,
        run_id: &'a str,
        runtime_id: &'a str,
    },
}

fn required<'a>(receipt: &'a Value, pointer: &str) -> Result<&'a str, String> {
    receipt
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("Orca 回执缺少 {pointer}；已保留原始回执，不会自动重发。"))
}

fn equals(receipt: &Value, pointer: &str, expected: &str) -> Result<(), String> {
    if required(receipt, pointer)? != expected {
        return Err(format!(
            "Orca 回执的 {pointer} 与本次操作不符；请检查 Orca，不会自动重发。"
        ));
    }
    Ok(())
}

pub(super) fn validate(receipt: &Value, expected: ExpectedReceipt<'_>) -> Result<(), String> {
    match expected {
        ExpectedReceipt::Terminal => {
            required(receipt, "/result/terminal/handle")?;
            if required(receipt, "/result/terminal/paneKey").is_err() {
                required(receipt, "/result/terminal/tabId")?;
            }
        }
        ExpectedReceipt::Run { id, coordinator } => {
            required(receipt, "/result/run/id")?;
            if let Some(id) = id {
                equals(receipt, "/result/run/id", id)?;
            }
            equals(receipt, "/result/run/coordinator_handle", coordinator)?;
            if !receipt
                .pointer("/result/run/consumer_generation")
                .and_then(Value::as_i64)
                .is_some_and(|generation| generation > 0)
            {
                return Err(
                    "Orca Run 回执缺少有效 consumer_generation；不会使用未确认的绑定。".into(),
                );
            }
        }
        ExpectedReceipt::StartTask {
            run_id,
            requested_model,
        } => {
            required(receipt, "/result/dispatchId")?;
            required(receipt, "/result/taskId")?;
            equals(receipt, "/result/runId", run_id)?;
            if let Some(model) = requested_model {
                // This proves launch parameters only, not the model actually used by OMP.
                for selection in ["requested", "effective"] {
                    let prefix = format!("/result/launch/{selection}");
                    equals(receipt, &format!("{prefix}/agent"), "omp")?;
                    equals(receipt, &format!("{prefix}/model"), model)?;
                    if receipt.pointer(&format!("{prefix}/effort")) != Some(&Value::Null) {
                        return Err(format!(
                            "Orca 回执的 {prefix}/effort 未确认无覆盖；已保留原始回执，不会自动重发。"
                        ));
                    }
                }
            }
        }
        ExpectedReceipt::Reply {
            question_id,
            thread_id,
            run_id,
        } => {
            let answer = required(receipt, "/result/message/id")?;
            let actual_thread = required(receipt, "/result/message/thread_id")?;
            if actual_thread != question_id && Some(actual_thread) != thread_id {
                return Err("Orca 回复回执属于其他问题线程；不会自动重发。".into());
            }
            equals(receipt, "/result/message/run_id", run_id)?;
            if receipt
                .pointer("/result/question")
                .is_some_and(|question| !question.is_null())
            {
                equals(receipt, "/result/message/thread_id", question_id)?;
                equals(receipt, "/result/question/message_id", question_id)?;
                equals(receipt, "/result/question/answer_message_id", answer)?;
                equals(receipt, "/result/question/run_id", run_id)?;
                equals(receipt, "/result/question/status", "answered")?;
            }
        }
        ExpectedReceipt::Release { dispatch_id } => {
            equals(receipt, "/result/dispatchId", dispatch_id)?;
            if !matches!(
                required(receipt, "/result/state")?,
                "released" | "already_released" | "retained" | "release_pending"
            ) {
                return Err(
                    "Orca 释放回执未确认受支持的处理结果，请检查 Orca；不会自动重发。".into(),
                );
            }
        }
        ExpectedReceipt::Output {
            dispatch_id,
            task_id,
            run_id,
            runtime_id,
        } => {
            equals(receipt, "/_meta/runtimeId", runtime_id)?;
            equals(receipt, "/result/dispatchId", dispatch_id)?;
            if receipt
                .pointer("/result/projection")
                .is_some_and(|projection| !projection.is_null())
            {
                equals(receipt, "/result/projection/dispatchId", dispatch_id)?;
                equals(receipt, "/result/projection/taskId", task_id)?;
                equals(receipt, "/result/projection/runId", run_id)?;
            }
        }
    }
    Ok(())
}
