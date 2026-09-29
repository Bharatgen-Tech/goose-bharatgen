use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::future::join_all;
use futures::StreamExt;
use rmcp::model::Role;
use tokio_util::sync::CancellationToken;

use crate::agents::final_output_tool::FinalOutputTool;
use crate::agents::state_machine::ops_maxturns::MAX_TURNS_MESSAGE;
use crate::agents::state_machine::{
    applied, awaits_tool_responses, messages_since_kickoff, not_applicable, trailing_error,
    yielded, Emitter, GooseEffect, Operation, OperationResult,
};
use crate::agents::subagent_handler::from_foreground_subagent_session;
use crate::agents::SessionConfig;
use crate::conversation::message::{Message, MessageContent};
use crate::conversation::Conversation;
use crate::session::{Session, SessionManager, SessionType};

const OPERATION_NAME: &str = "foreground_subagent";

pub(super) fn foreground_child_ids_in_message(content: &[MessageContent]) -> Vec<String> {
    content
        .iter()
        .filter_map(|content| {
            let MessageContent::ToolResponse(response) = content else {
                return None;
            };
            let result = response.tool_result.as_ref().ok()?;
            let meta = result.meta.as_ref()?;
            if result.is_error == Some(true)
                || meta.0.get("foreground_subagent") != Some(&serde_json::Value::Bool(true))
            {
                return None;
            }
            meta.0
                .get("subagent_session_id")?
                .as_str()
                .map(str::to_owned)
        })
        .collect()
}

pub struct ForegroundSubagentOperation {
    session_manager: Arc<SessionManager>,
    use_login_shell_path: bool,
    cancel: CancellationToken,
}

impl ForegroundSubagentOperation {
    pub fn new(
        session_manager: Arc<SessionManager>,
        use_login_shell_path: bool,
        cancel: CancellationToken,
    ) -> Self {
        Self {
            session_manager,
            use_login_shell_path,
            cancel,
        }
    }

    async fn advance_child(&self, child: &Session) -> Result<()> {
        let agent = from_foreground_subagent_session(
            self.session_manager.clone(),
            &child.id,
            self.use_login_shell_path,
        )
        .await?;
        let recipe = child
            .recipe
            .as_ref()
            .ok_or_else(|| anyhow!("Subagent {} has no saved recipe", child.id))?;
        let max_turns = recipe
            .settings
            .as_ref()
            .and_then(|settings| settings.max_turns)
            .ok_or_else(|| anyhow!("Subagent {} has no saved turn limit", child.id))?;
        let session_config = SessionConfig {
            id: child.id.clone(),
            schedule_id: None,
            max_turns: Some(max_turns as u32),
            retry_config: recipe.retry.clone(),
        };
        let mut events = agent
            .stream_state_machine_session(session_config, self.cancel.clone())
            .await?;
        while let Some(event) = events.next().await {
            event?;
        }
        Ok(())
    }

    async fn run_child(&self, parent_id: &str, child_id: &str) -> String {
        let child = match self.session_manager.get_session(child_id, true).await {
            Ok(child) => child,
            Err(error) => return format!("Subagent {child_id} failed: {error}"),
        };
        if child.session_type != SessionType::SubAgent
            || child.parent_session_id.as_deref() != Some(parent_id)
        {
            return format!("Subagent {child_id} failed: it does not belong to this session");
        }
        if let Some(output) = child
            .conversation
            .as_ref()
            .and_then(|conversation| FinalOutputTool::successful_output(conversation.messages()))
        {
            return format!("Subagent {child_id} completed: {output}");
        }

        let run_result = self.advance_child(&child).await;
        let child = match self.session_manager.get_session(child_id, true).await {
            Ok(child) => child,
            Err(error) => return format!("Subagent {child_id} failed: {error}"),
        };
        let messages = child.conversation.as_ref().map(Conversation::messages);
        if let Some(output) =
            messages.and_then(|messages| FinalOutputTool::successful_output(messages))
        {
            return format!("Subagent {child_id} completed: {output}");
        }
        if let Err(error) = run_result {
            return format!("Subagent {child_id} failed: {error}");
        }
        if let Some(error) = child.conversation.as_ref().and_then(trailing_error) {
            return format!("Subagent {child_id} failed: {error:?}");
        }

        let reason = messages
            .and_then(|messages| {
                if messages
                    .last()
                    .is_some_and(|message| message.as_concat_text() == MAX_TURNS_MESSAGE)
                {
                    let last_assistant_text = messages
                        .iter()
                        .rev()
                        .filter(|message| message.role == Role::Assistant)
                        .map(Message::as_concat_text)
                        .find(|text| !text.is_empty() && text != MAX_TURNS_MESSAGE);
                    Some(format!(
                        "max turns reached{}",
                        last_assistant_text
                            .map(|text| format!("; last response: {text}"))
                            .unwrap_or_default()
                    ))
                } else {
                    messages
                        .last()
                        .map(Message::as_concat_text)
                        .filter(|text| !text.is_empty())
                }
            })
            .unwrap_or_else(|| "stopped without final output".to_string());
        format!("Subagent {child_id} failed: {reason}")
    }
}

#[async_trait]
impl Operation<Session, GooseEffect> for ForegroundSubagentOperation {
    fn name(&self) -> &'static str {
        OPERATION_NAME
    }

    async fn run(
        &self,
        session: &Session,
        conversation: &Conversation,
        _emit: &Emitter,
    ) -> Result<OperationResult<GooseEffect>> {
        if session.session_type == SessionType::SubAgent {
            return not_applicable();
        }
        if awaits_tool_responses(messages_since_kickoff(conversation)?) {
            return not_applicable();
        }
        let waiting = self
            .session_manager
            .pending_foreground_subagents(&session.id)
            .await?;
        if waiting.is_empty() {
            return not_applicable();
        }
        let results = join_all(
            waiting
                .iter()
                .map(|child_id| self.run_child(&session.id, child_id)),
        )
        .await;
        if self.cancel.is_cancelled() {
            return yielded();
        }
        let delivery = Message::user()
            .with_text(results.join("\n\n"))
            .with_visibility(false, true);
        applied([GooseEffect::DeliverForegroundSubagents {
            message: delivery,
            child_ids: waiting,
        }])
    }
}

#[cfg(test)]
mod tests {
    use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock, MetaObject};
    use tempfile::TempDir;

    use super::*;
    use crate::agents::final_output_tool::{FINAL_OUTPUT_SUCCESS_MESSAGE, FINAL_OUTPUT_TOOL_NAME};
    use crate::config::GooseMode;

    #[test]
    fn identifies_foreground_children_in_tool_responses() {
        let mut meta = MetaObject::new();
        meta.0.insert(
            "foreground_subagent".to_string(),
            serde_json::Value::Bool(true),
        );
        meta.0.insert(
            "subagent_session_id".to_string(),
            serde_json::Value::String("child-1".to_string()),
        );
        let message = Message::user()
            .with_tool_response(
                "delegate-call",
                Ok(
                    CallToolResult::success(vec![ContentBlock::text("scheduled")])
                        .with_meta(Some(meta)),
                ),
            )
            .with_tool_response(
                "other-call",
                Ok(CallToolResult::success(vec![ContentBlock::text("done")])),
            );

        assert_eq!(
            foreground_child_ids_in_message(&message.content),
            vec!["child-1"]
        );
    }

    #[test]
    fn waits_for_all_tool_responses_before_running_children() {
        let requests = Message::assistant()
            .with_tool_request("delegate-call", Ok(CallToolRequestParams::new("delegate")))
            .with_tool_request("other-call", Ok(CallToolRequestParams::new("other")));
        let response = Message::user().with_tool_response(
            "delegate-call",
            Ok(CallToolResult::success(vec![ContentBlock::text(
                "scheduled",
            )])),
        );
        let mut messages = vec![requests, response];
        assert!(awaits_tool_responses(&messages));

        messages.push(Message::user().with_tool_response(
            "other-call",
            Ok(CallToolResult::success(vec![ContentBlock::text("done")])),
        ));
        assert!(!awaits_tool_responses(&messages));
    }

    #[tokio::test]
    async fn completed_child_uses_saved_final_output_without_running_again() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let manager = Arc::new(SessionManager::new(temp_dir.path().to_path_buf()));
        let parent = manager
            .create_session(
                temp_dir.path().to_path_buf(),
                "parent".to_string(),
                SessionType::User,
                GooseMode::Auto,
            )
            .await?;
        let child = manager
            .create_session(
                temp_dir.path().to_path_buf(),
                "child".to_string(),
                SessionType::SubAgent,
                GooseMode::Auto,
            )
            .await?;
        manager
            .update(&child.id)
            .parent_session_id(Some(parent.id.clone()))
            .apply()
            .await?;
        let arguments = serde_json::json!({"result": "done"})
            .as_object()
            .unwrap()
            .clone();
        manager
            .add_message(
                &child.id,
                &Message::assistant().with_tool_request(
                    "final-output-call",
                    Ok(CallToolRequestParams::new(FINAL_OUTPUT_TOOL_NAME)
                        .with_arguments(arguments)),
                ),
            )
            .await?;
        manager
            .add_message(
                &child.id,
                &Message::user().with_tool_response(
                    "final-output-call",
                    Ok(CallToolResult::success(vec![ContentBlock::text(
                        FINAL_OUTPUT_SUCCESS_MESSAGE,
                    )])),
                ),
            )
            .await?;

        let operation = ForegroundSubagentOperation::new(manager, false, CancellationToken::new());
        assert_eq!(
            operation.run_child(&parent.id, &child.id).await,
            format!("Subagent {} completed: {{\"result\":\"done\"}}", child.id)
        );
        Ok(())
    }
}
