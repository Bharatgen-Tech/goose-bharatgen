use std::sync::Arc;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::agents::state_machine::ops_recipe::RecipeOperation;
use crate::agents::state_machine::{
    not_applicable, pending_tool_confirmations, trailing_error, yielded, Emitter, GooseEffect,
    Operation, OperationResult,
};
use crate::agents::subagent_handler::from_foreground_subagent_session;
use crate::agents::SessionConfig;
use crate::conversation::message::{Message, MessageContent};
use crate::conversation::Conversation;
use crate::session::{Session, SessionManager, SessionType};

const OPERATION_NAME: &str = "foreground_subagent";
const OUTCOME_NOTE: &str = "outcome";
const DELIVERY_NOTE: &str = "delivered_child_session_id";

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

    fn find_child_id_from_delegate_response(conversation: &Conversation) -> Option<&str> {
        for message in conversation.messages().iter().rev() {
            if message
                .metadata
                .operation_note(OPERATION_NAME, DELIVERY_NOTE)
                .is_some()
            {
                return None;
            }
            for content in &message.content {
                let MessageContent::ToolResponse(response) = content else {
                    continue;
                };
                let Ok(result) = &response.tool_result else {
                    continue;
                };
                if result.is_error == Some(true)
                    || result
                        .meta
                        .as_ref()
                        .and_then(|meta| meta.0.get("foreground_subagent"))
                        != Some(&serde_json::Value::Bool(true))
                {
                    continue;
                }
                return result
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.0.get("subagent_session_id"))
                    .and_then(serde_json::Value::as_str);
            }
        }
        None
    }

    fn child_has_outcome(session: &Session) -> bool {
        session.conversation.as_ref().is_some_and(|conversation| {
            RecipeOperation::successful_final_output(conversation.messages()).is_some()
                || conversation.messages().iter().any(|message| {
                    message
                        .metadata
                        .operation_note(OPERATION_NAME, OUTCOME_NOTE)
                        .is_some()
                })
        })
    }

    async fn record_outcome(
        &self,
        child_id: &str,
        status: &str,
        error: Option<String>,
    ) -> Result<()> {
        let mut message = Message::user()
            .with_text("Foreground subagent outcome")
            .with_visibility(false, false);
        message.metadata.set_operation_note(
            OPERATION_NAME,
            OUTCOME_NOTE,
            serde_json::json!({ "status": status, "error": error }),
        );
        self.session_manager.add_message(child_id, &message).await
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
        let Some(child_id) = Self::find_child_id_from_delegate_response(conversation) else {
            return not_applicable();
        };
        if !pending_tool_confirmations(conversation).is_empty() {
            return not_applicable();
        }
        let child = self.session_manager.get_session(child_id, true).await?;
        if child.session_type != SessionType::SubAgent
            || child.parent_session_id.as_deref() != Some(session.id.as_str())
        {
            return Err(anyhow!(
                "Session {child_id} is not a child of {}",
                session.id
            ));
        }
        if Self::child_has_outcome(&child) {
            return yielded();
        }

        let advance_result = self.advance_child(&child).await;
        let child = self.session_manager.get_session(child_id, true).await?;
        if Self::child_has_outcome(&child) {
            return yielded();
        }
        if self.cancel.is_cancelled() {
            self.record_outcome(child_id, "cancelled", None).await?;
            return yielded();
        }
        if let Err(error) = advance_result {
            self.record_outcome(child_id, "failure", Some(error.to_string()))
                .await?;
            return yielded();
        }

        let conversation = child
            .conversation
            .as_ref()
            .ok_or_else(|| anyhow!("Subagent {child_id} has no conversation"))?;
        if let Some(error) = trailing_error(conversation) {
            self.record_outcome(child_id, "failure", Some(format!("{error:?}")))
                .await?;
        } else {
            self.record_outcome(child_id, "yielded", None).await?;
        }
        yielded()
    }
}

#[cfg(test)]
mod tests {
    use rmcp::model::{CallToolResult, ContentBlock, MetaObject};

    use super::*;

    #[test]
    fn finds_new_child_after_previous_child_was_delivered() {
        let response = |call_id: &str, child_id: &str| {
            let mut meta = MetaObject::new();
            meta.0.insert(
                "foreground_subagent".to_string(),
                serde_json::Value::Bool(true),
            );
            meta.0.insert(
                "subagent_session_id".to_string(),
                serde_json::Value::String(child_id.to_string()),
            );
            Message::user().with_tool_response(
                call_id,
                Ok(
                    CallToolResult::success(vec![ContentBlock::text("scheduled")])
                        .with_meta(Some(meta)),
                ),
            )
        };
        let mut messages = vec![response("delegate-call-1", "child-1")];
        assert_eq!(
            ForegroundSubagentOperation::find_child_id_from_delegate_response(
                &Conversation::new_unvalidated(messages.clone())
            ),
            Some("child-1")
        );

        let mut delivery = Message::user().with_text("child completed");
        delivery.metadata.set_operation_note(
            OPERATION_NAME,
            DELIVERY_NOTE,
            serde_json::Value::String("child-1".to_string()),
        );
        messages.push(delivery);
        assert!(
            ForegroundSubagentOperation::find_child_id_from_delegate_response(
                &Conversation::new_unvalidated(messages.clone())
            )
            .is_none()
        );

        messages.push(response("delegate-call-2", "child-2"));
        assert_eq!(
            ForegroundSubagentOperation::find_child_id_from_delegate_response(
                &Conversation::new_unvalidated(messages)
            ),
            Some("child-2")
        );
    }
}
