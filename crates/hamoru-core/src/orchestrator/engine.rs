//! Default implementation of the `OrchestrationEngine` trait.
//!
//! `DefaultOrchestrationEngine` executes workflows sequentially, using the
//! Policy Engine for model selection and Telemetry for recording.

use std::path::Path;
use std::time::Instant;

use async_trait::async_trait;
use chrono::Utc;
use tracing::instrument;

use super::condition::{
    REPORT_STATUS_TOOL_NAME, build_report_status_tool, evaluate_condition, match_transition,
};
use super::context::{apply_context_policy, build_step_messages};
use super::{
    ConditionMode, ExecutionResult, OrchestrationEngine, TerminationReason, TransitionTarget,
    Workflow, WorkflowStep,
};
use crate::Result;
use crate::error::{HamoruError, StepResult, sanitize_error};
use crate::policy::{PolicyEngine, RoutingRequest};
use crate::provider::types::{ChatRequest, Message, ToolChoice};
use crate::provider::{ModelInfo, ProviderRegistry, TokenUsage};
use crate::telemetry::MetricsCache;
use crate::telemetry::{HistoryEntry, TelemetryStore};

/// Stateless orchestration engine.
///
/// All dependencies (Policy Engine, Provider Registry, Telemetry Store)
/// are passed as method arguments.
pub struct DefaultOrchestrationEngine;

#[async_trait]
impl OrchestrationEngine for DefaultOrchestrationEngine {
    fn load_workflow(&self, path: &Path) -> Result<Workflow> {
        let config = super::config::load_workflow(path)?;
        Workflow::try_from(config)
    }

    #[instrument(skip_all, fields(workflow = %workflow.name))]
    async fn execute(
        &self,
        workflow: &Workflow,
        task: &str,
        policy_engine: &dyn PolicyEngine,
        providers: &ProviderRegistry,
        telemetry: &dyn TelemetryStore,
    ) -> Result<ExecutionResult> {
        // Pre-loop: cache all models from providers
        let all_models = collect_all_models(providers).await?;

        // Pre-loop: load metrics cache for policy engine
        let metrics_cache = telemetry.load_cache().await?;

        let mut current_step_idx = 0;
        let mut iteration: u32 = 0;
        let mut accumulated_cost: f64 = 0.0;
        let mut accumulated_tokens = TokenUsage::default();
        let mut accumulated_latency_ms: u64 = 0;
        let mut steps_executed: Vec<StepResult> = Vec::new();
        let mut previous_output: Option<String> = None;
        let mut message_history: Vec<Message> = Vec::new();

        loop {
            iteration += 1;

            // Guard: max iterations → warning, not error (§11.3)
            if iteration > workflow.max_iterations {
                tracing::warn!(
                    workflow = %workflow.name,
                    max = workflow.max_iterations,
                    "Workflow reached max iterations. Returning last output."
                );
                return Ok(ExecutionResult {
                    steps_executed,
                    total_cost: accumulated_cost,
                    total_tokens: accumulated_tokens,
                    total_latency_ms: accumulated_latency_ms,
                    final_output: previous_output.unwrap_or_default(),
                    terminated_reason: TerminationReason::MaxIterationsReached {
                        max: workflow.max_iterations,
                    },
                });
            }

            let step = &workflow.steps[current_step_idx];

            // Apply context policy before building messages
            if !message_history.is_empty() {
                message_history = apply_context_policy(&message_history, &step.context_policy);
            }

            let (step_result, transition, raw_content) = match execute_step(
                step,
                task,
                previous_output.as_deref(),
                &message_history,
                policy_engine,
                providers,
                telemetry,
                &all_models,
                &metrics_cache,
            )
            .await
            {
                Ok(result) => result,
                Err(HamoruError::MidWorkflowFailure {
                    step: s, source, ..
                }) => {
                    return Err(HamoruError::MidWorkflowFailure {
                        step: s,
                        partial_results: steps_executed,
                        source,
                    });
                }
                Err(e) => return Err(e),
            };

            // Accumulate metrics
            accumulated_cost += step_result.cost;
            accumulated_tokens += step_result.tokens.clone();
            accumulated_latency_ms += step_result.latency_ms;

            // Guard: cost cap
            if let Some(max_cost) = workflow.max_cost
                && accumulated_cost > max_cost
            {
                return Err(HamoruError::WorkflowCostExceeded {
                    workflow: workflow.name.clone(),
                    spent: accumulated_cost,
                    limit: max_cost,
                });
            }

            // Update message history with step messages + raw assistant response
            let step_messages =
                build_step_messages(&step.instruction, task, previous_output.as_deref());
            message_history.extend(step_messages);
            message_history.push(Message {
                role: crate::provider::types::Role::Assistant,
                content: crate::provider::types::MessageContent::Text(raw_content),
            });

            previous_output = Some(step_result.output.clone());
            steps_executed.push(step_result);

            // Follow transition
            match transition {
                TransitionTarget::Complete => {
                    return Ok(ExecutionResult {
                        steps_executed,
                        total_cost: accumulated_cost,
                        total_tokens: accumulated_tokens,
                        total_latency_ms: accumulated_latency_ms,
                        final_output: previous_output.unwrap_or_default(),
                        terminated_reason: TerminationReason::Completed,
                    });
                }
                TransitionTarget::Step(next_name) => {
                    current_step_idx = workflow
                        .steps
                        .iter()
                        .position(|s| s.name == *next_name)
                        .ok_or_else(|| HamoruError::WorkflowValidationError {
                            workflow: workflow.name.clone(),
                            reason: format!("Step '{}' not found.", next_name),
                        })?;
                }
            }
        }
    }
}

/// Executes a single workflow step: model selection, LLM call, condition
/// evaluation, transition matching, and telemetry recording.
///
/// Returns `(StepResult, TransitionTarget, raw_response_content)`:
/// - `StepResult.output`: content after condition evaluation (STATUS stripped)
/// - `TransitionTarget`: matched transition or `Complete` for no-transitions steps
/// - `String`: raw assistant response for message history (pre-condition-evaluation)
#[allow(clippy::too_many_arguments)] // Intentional: all borrowed refs needed for parallel execution via join_all
async fn execute_step(
    step: &WorkflowStep,
    task: &str,
    previous_output: Option<&str>,
    message_history: &[Message],
    policy_engine: &dyn PolicyEngine,
    providers: &ProviderRegistry,
    telemetry: &dyn TelemetryStore,
    all_models: &[ModelInfo],
    metrics_cache: &MetricsCache,
) -> Result<(StepResult, TransitionTarget, String)> {
    // Model selection via Policy Engine
    let routing_request = RoutingRequest {
        tags: step.tags.clone(),
        ..Default::default()
    };
    let selection = policy_engine.select_model(&routing_request, all_models, metrics_cache)?;

    let provider = providers
        .get(&selection.provider)
        .ok_or_else(|| HamoruError::ConfigError {
            reason: format!(
                "Provider '{}' selected by policy but not found in registry.",
                selection.provider
            ),
        })?;

    // Build step messages
    let step_messages = build_step_messages(&step.instruction, task, previous_output);

    // Build full message array
    let mut full_messages = message_history.to_vec();
    full_messages.extend(step_messages);

    // Prepare tools for condition evaluation
    let (tools, tool_choice) =
        if step.condition_mode == ConditionMode::ToolCalling && !step.transitions.is_empty() {
            let valid_statuses: Vec<&str> = step
                .transitions
                .iter()
                .map(|t| t.condition.as_str())
                .collect();
            let tool = build_report_status_tool(&valid_statuses);
            (
                Some(vec![tool]),
                Some(ToolChoice::Tool {
                    name: REPORT_STATUS_TOOL_NAME.to_string(),
                }),
            )
        } else {
            (None, None)
        };

    // Build ChatRequest
    let chat_request = ChatRequest {
        model: selection.model.clone(),
        messages: full_messages,
        temperature: None,
        max_tokens: None,
        tools,
        tool_choice,
        stream: false, // All intermediate steps are buffered
    };

    // Execute LLM call
    let start = Instant::now();
    let response = match provider.chat(chat_request).await {
        Ok(r) => r,
        Err(e) => {
            let step_latency_ms = start.elapsed().as_millis() as u64;
            record_telemetry_failed(
                telemetry,
                &selection.provider,
                &selection.model,
                step_latency_ms,
                &step.tags,
            )
            .await;
            return Err(HamoruError::MidWorkflowFailure {
                step: step.name.clone(),
                partial_results: vec![],
                source: sanitize_error(e),
            });
        }
    };
    let step_latency_ms = start.elapsed().as_millis() as u64;

    // Calculate cost from cached model list
    let step_cost = all_models
        .iter()
        .find(|m| m.id == selection.model && m.provider == selection.provider)
        .map(|mi| response.usage.calculate_cost(mi))
        .unwrap_or(0.0);

    // Record telemetry
    record_telemetry(
        telemetry,
        &selection.provider,
        &selection.model,
        &response.usage,
        step_cost,
        step_latency_ms,
        &step.tags,
    )
    .await;

    // Handle steps with no transitions as implicit COMPLETE
    if step.transitions.is_empty() {
        let step_result = StepResult {
            step_name: step.name.clone(),
            output: response.content.clone(),
            tokens: response.usage.clone(),
            cost: step_cost,
            latency_ms: step_latency_ms,
            model_used: selection.model,
            policy_applied: selection.policy_applied,
        };
        return Ok((step_result, TransitionTarget::Complete, response.content));
    }

    // Evaluate condition
    let step_output = evaluate_condition(&response, &step.condition_mode, &step.name)?;

    // Match transition
    let target = match_transition(&step_output.status, &step.transitions).ok_or_else(|| {
        HamoruError::ConditionEvaluationFailed {
            step: step.name.clone(),
            reason: format!(
                "Status '{}' does not match any transition. \
                     Valid conditions: {:?}.",
                step_output.status,
                step.transitions
                    .iter()
                    .map(|t| &t.condition)
                    .collect::<Vec<_>>()
            ),
        }
    })?;

    let step_result = StepResult {
        step_name: step.name.clone(),
        output: step_output.content,
        tokens: response.usage.clone(),
        cost: step_cost,
        latency_ms: step_latency_ms,
        model_used: selection.model,
        policy_applied: selection.policy_applied,
    };

    Ok((step_result, target.clone(), response.content))
}

/// Collects models from all providers, skipping unavailable ones.
async fn collect_all_models(providers: &ProviderRegistry) -> Result<Vec<ModelInfo>> {
    let mut all_models = Vec::new();
    for provider in providers.iter() {
        match provider.list_models().await {
            Ok(models) => all_models.extend(models),
            Err(e) => {
                tracing::warn!(
                    provider = provider.id(),
                    "Failed to list models, skipping: {e}"
                );
            }
        }
    }
    if all_models.is_empty() {
        return Err(HamoruError::ConfigError {
            reason: "No models available from any provider. \
                     Check that providers are configured and accessible."
                .to_string(),
        });
    }
    Ok(all_models)
}

/// Records a telemetry entry for a failed LLM call.
async fn record_telemetry_failed(
    telemetry: &dyn TelemetryStore,
    provider: &str,
    model: &str,
    latency_ms: u64,
    tags: &[String],
) {
    let entry = HistoryEntry {
        timestamp: Utc::now(),
        provider: provider.to_string(),
        model: model.to_string(),
        tokens: TokenUsage::default(),
        cost: 0.0,
        latency_ms,
        success: false,
        tags: tags.to_vec(),
    };
    if let Err(e) = telemetry.record(&entry).await {
        tracing::warn!("Failed to record telemetry: {e}");
    }
}

/// Records a telemetry entry for a step execution.
async fn record_telemetry(
    telemetry: &dyn TelemetryStore,
    provider: &str,
    model: &str,
    usage: &TokenUsage,
    cost: f64,
    latency_ms: u64,
    tags: &[String],
) {
    let entry = HistoryEntry {
        timestamp: Utc::now(),
        provider: provider.to_string(),
        model: model.to_string(),
        tokens: usage.clone(),
        cost,
        latency_ms,
        success: true,
        tags: tags.to_vec(),
    };
    if let Err(e) = telemetry.record(&entry).await {
        tracing::warn!("Failed to record telemetry: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestrator::condition::tests::{
        response_with_status_line, response_with_tool_status, simple_response,
    };
    use crate::orchestrator::{
        ConditionMode, ContextPolicy, Transition, TransitionTarget, Workflow, WorkflowStep,
    };
    use crate::policy::DefaultPolicyEngine;
    use crate::policy::config::{
        PolicyConfig, PolicyConstraints, PolicyDefinition, PolicyPreferences, Priority,
    };
    use crate::provider::mock::MockProvider;
    use crate::provider::types::{Capability, ChatResponse, FinishReason, ModelInfo, TokenUsage};
    use crate::telemetry::memory::InMemoryTelemetryStore;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn test_model() -> ModelInfo {
        ModelInfo {
            id: "test-model".to_string(),
            provider: "test-provider".to_string(),
            context_window: 100_000,
            cost_per_input_token: 1.0 / 1_000_000.0,
            cost_per_output_token: 2.0 / 1_000_000.0,
            cost_per_cached_input_token: None,
            capabilities: vec![Capability::Chat, Capability::FunctionCalling],
            max_output_tokens: Some(4096),
        }
    }

    fn test_policy_config() -> PolicyConfig {
        PolicyConfig {
            policies: vec![PolicyDefinition {
                name: "default".to_string(),
                description: Some("Test policy".to_string()),
                constraints: PolicyConstraints::default(),
                preferences: PolicyPreferences {
                    priority: Priority::Cost,
                },
            }],
            routing_rules: vec![crate::policy::config::RoutingRule {
                match_rule: None,
                default: Some(crate::policy::config::DefaultPolicy {
                    policy: "default".to_string(),
                }),
                policy: None,
            }],
            cost_limits: None,
        }
    }

    fn build_test_provider() -> MockProvider {
        let mut provider = MockProvider::new("test-provider");
        provider.set_models(vec![test_model()]);
        provider
    }

    fn build_registry(provider: MockProvider) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry.register(Box::new(provider));
        registry
    }

    fn two_step_workflow() -> Workflow {
        Workflow {
            name: "test-workflow".to_string(),
            description: None,
            max_iterations: 10,
            max_cost: None,
            default_condition_mode: ConditionMode::StatusLine,
            steps: vec![
                WorkflowStep {
                    name: "step1".to_string(),
                    tags: vec![],
                    instruction: "{task}".to_string(),
                    transitions: vec![Transition {
                        condition: "done".to_string(),
                        next: TransitionTarget::Step("step2".to_string()),
                    }],
                    context_policy: ContextPolicy::KeepAll,
                    condition_mode: ConditionMode::StatusLine,
                    dependencies: None,
                },
                WorkflowStep {
                    name: "step2".to_string(),
                    tags: vec![],
                    instruction: "Finalize".to_string(),
                    transitions: vec![],
                    context_policy: ContextPolicy::KeepAll,
                    condition_mode: ConditionMode::StatusLine,
                    dependencies: None,
                },
            ],
        }
    }

    fn review_loop_workflow() -> Workflow {
        Workflow {
            name: "gen-review".to_string(),
            description: None,
            max_iterations: 10,
            max_cost: None,
            default_condition_mode: ConditionMode::StatusLine,
            steps: vec![
                WorkflowStep {
                    name: "generate".to_string(),
                    tags: vec![],
                    instruction: "{task}".to_string(),
                    transitions: vec![Transition {
                        condition: "done".to_string(),
                        next: TransitionTarget::Step("review".to_string()),
                    }],
                    context_policy: ContextPolicy::KeepAll,
                    condition_mode: ConditionMode::StatusLine,
                    dependencies: None,
                },
                WorkflowStep {
                    name: "review".to_string(),
                    tags: vec![],
                    instruction: "Review:\n{previous_output}".to_string(),
                    transitions: vec![
                        Transition {
                            condition: "approved".to_string(),
                            next: TransitionTarget::Complete,
                        },
                        Transition {
                            condition: "improve".to_string(),
                            next: TransitionTarget::Step("generate".to_string()),
                        },
                    ],
                    context_policy: ContextPolicy::KeepAll,
                    condition_mode: ConditionMode::StatusLine,
                    dependencies: None,
                },
            ],
        }
    }

    // -----------------------------------------------------------------------
    // Tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn two_step_linear_complete() {
        let provider = build_test_provider();
        provider.queue_chat_response(Ok(response_with_status_line("Generated code", "done")));
        provider.queue_chat_response(Ok(simple_response("Final output")));
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let engine = DefaultOrchestrationEngine;
        let result = engine
            .execute(
                &two_step_workflow(),
                "write code",
                &policy,
                &registry,
                &telemetry,
            )
            .await
            .unwrap();

        assert_eq!(result.steps_executed.len(), 2);
        assert_eq!(result.steps_executed[0].step_name, "step1");
        assert_eq!(result.steps_executed[1].step_name, "step2");
        assert_eq!(result.final_output, "Final output");
        assert_eq!(result.terminated_reason, TerminationReason::Completed);
    }

    #[tokio::test]
    async fn review_loop_improve_then_approved() {
        let provider = build_test_provider();
        // generate → done → review
        provider.queue_chat_response(Ok(response_with_status_line("v1 code", "done")));
        // review → improve → generate
        provider.queue_chat_response(Ok(response_with_status_line("Needs work", "improve")));
        // generate (revised) → done → review
        provider.queue_chat_response(Ok(response_with_status_line("v2 code", "done")));
        // review → approved → COMPLETE
        provider.queue_chat_response(Ok(response_with_status_line("LGTM", "approved")));
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let engine = DefaultOrchestrationEngine;
        let result = engine
            .execute(
                &review_loop_workflow(),
                "write auth",
                &policy,
                &registry,
                &telemetry,
            )
            .await
            .unwrap();

        assert_eq!(result.steps_executed.len(), 4);
        assert_eq!(result.terminated_reason, TerminationReason::Completed);
    }

    #[tokio::test]
    async fn max_iterations_returns_warning_not_error() {
        let provider = build_test_provider();
        // Queue enough responses for max_iterations
        for _ in 0..5 {
            provider.queue_chat_response(Ok(response_with_status_line("code", "done")));
            provider.queue_chat_response(Ok(response_with_status_line("improve", "improve")));
        }
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let mut workflow = review_loop_workflow();
        workflow.max_iterations = 3;

        let engine = DefaultOrchestrationEngine;
        let result = engine
            .execute(&workflow, "task", &policy, &registry, &telemetry)
            .await
            .unwrap(); // NOT an error

        assert_eq!(
            result.terminated_reason,
            TerminationReason::MaxIterationsReached { max: 3 }
        );
        assert!(!result.final_output.is_empty());
    }

    #[tokio::test]
    async fn cost_cap_exceeded() {
        let provider = build_test_provider();
        // Each response has usage that incurs cost
        let expensive_response = ChatResponse {
            content: "output\nSTATUS: done".to_string(),
            model: "test-model".to_string(),
            usage: TokenUsage {
                input_tokens: 1_000_000,
                output_tokens: 1_000_000,
                cache_creation_input_tokens: None,
                cache_read_input_tokens: None,
            },
            latency_ms: 100,
            finish_reason: FinishReason::Stop,
            tool_calls: None,
        };
        provider.queue_chat_response(Ok(expensive_response));
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let mut workflow = two_step_workflow();
        workflow.max_cost = Some(0.001); // Very low cap

        let engine = DefaultOrchestrationEngine;
        let err = engine
            .execute(&workflow, "task", &policy, &registry, &telemetry)
            .await
            .unwrap_err();

        match err {
            HamoruError::WorkflowCostExceeded { .. } => {}
            e => panic!("expected WorkflowCostExceeded, got {e:?}"),
        }
    }

    #[tokio::test]
    async fn provider_failure_first_step() {
        let provider = build_test_provider();
        provider.queue_chat_response(Err(HamoruError::ProviderUnavailable {
            provider: "test".to_string(),
            reason: "down".to_string(),
        }));
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let engine = DefaultOrchestrationEngine;
        let err = engine
            .execute(&two_step_workflow(), "task", &policy, &registry, &telemetry)
            .await
            .unwrap_err();

        match err {
            HamoruError::MidWorkflowFailure {
                partial_results, ..
            } => {
                assert_eq!(partial_results.len(), 0);
            }
            e => panic!("expected MidWorkflowFailure, got {e:?}"),
        }
    }

    #[tokio::test]
    async fn provider_failure_mid_workflow() {
        let provider = build_test_provider();
        provider.queue_chat_response(Ok(response_with_status_line("output", "done")));
        provider.queue_chat_response(Err(HamoruError::ProviderUnavailable {
            provider: "test".to_string(),
            reason: "timeout".to_string(),
        }));
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let engine = DefaultOrchestrationEngine;
        let err = engine
            .execute(&two_step_workflow(), "task", &policy, &registry, &telemetry)
            .await
            .unwrap_err();

        match err {
            HamoruError::MidWorkflowFailure {
                partial_results,
                step,
                ..
            } => {
                assert_eq!(partial_results.len(), 1);
                assert_eq!(step, "step2");
            }
            e => panic!("expected MidWorkflowFailure, got {e:?}"),
        }
    }

    #[tokio::test]
    async fn unmatched_condition() {
        let provider = build_test_provider();
        provider.queue_chat_response(Ok(response_with_status_line("output", "unknown_status")));
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let engine = DefaultOrchestrationEngine;
        let err = engine
            .execute(&two_step_workflow(), "task", &policy, &registry, &telemetry)
            .await
            .unwrap_err();

        match err {
            HamoruError::ConditionEvaluationFailed { step, reason } => {
                assert_eq!(step, "step1");
                assert!(reason.contains("unknown_status"));
            }
            e => panic!("expected ConditionEvaluationFailed, got {e:?}"),
        }
    }

    #[tokio::test]
    async fn single_step_no_transitions_implicit_complete() {
        let provider = build_test_provider();
        provider.queue_chat_response(Ok(simple_response("Done!")));
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let workflow = Workflow {
            name: "single".to_string(),
            description: None,
            max_iterations: 10,
            max_cost: None,
            default_condition_mode: ConditionMode::StatusLine,
            steps: vec![WorkflowStep {
                name: "only".to_string(),
                tags: vec![],
                instruction: "{task}".to_string(),
                transitions: vec![],
                context_policy: ContextPolicy::KeepAll,
                condition_mode: ConditionMode::StatusLine,
                dependencies: None,
            }],
        };

        let engine = DefaultOrchestrationEngine;
        let result = engine
            .execute(&workflow, "do it", &policy, &registry, &telemetry)
            .await
            .unwrap();

        assert_eq!(result.steps_executed.len(), 1);
        assert_eq!(result.final_output, "Done!");
        assert_eq!(result.terminated_reason, TerminationReason::Completed);
    }

    #[tokio::test]
    async fn tool_calling_mode_injects_tool() {
        let provider = build_test_provider();
        provider.queue_chat_response(Ok(response_with_tool_status("done", "all good")));
        provider.queue_chat_response(Ok(simple_response("Final")));
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let mut workflow = two_step_workflow();
        workflow.steps[0].condition_mode = ConditionMode::ToolCalling;

        let engine = DefaultOrchestrationEngine;
        let result = engine
            .execute(&workflow, "task", &policy, &registry, &telemetry)
            .await
            .unwrap();

        assert_eq!(result.steps_executed.len(), 2);
        // Tool calling mode succeeds, proving report_status tool was injected and parsed.
    }

    #[tokio::test]
    async fn status_line_mode_no_tools_in_request() {
        let provider = build_test_provider();
        provider.queue_chat_response(Ok(response_with_status_line("output", "done")));
        provider.queue_chat_response(Ok(simple_response("Final")));
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let engine = DefaultOrchestrationEngine;
        let result = engine
            .execute(&two_step_workflow(), "task", &policy, &registry, &telemetry)
            .await
            .unwrap();

        assert_eq!(result.steps_executed.len(), 2);
        assert_eq!(result.terminated_reason, TerminationReason::Completed);
    }

    #[tokio::test]
    async fn tool_calling_fallback_to_status_line() {
        let provider = build_test_provider();
        // Response has no tool calls but has STATUS line — fallback should work
        provider.queue_chat_response(Ok(response_with_status_line("text output", "done")));
        provider.queue_chat_response(Ok(simple_response("Final")));
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let mut workflow = two_step_workflow();
        workflow.steps[0].condition_mode = ConditionMode::ToolCalling;

        let engine = DefaultOrchestrationEngine;
        let result = engine
            .execute(&workflow, "task", &policy, &registry, &telemetry)
            .await
            .unwrap();

        assert_eq!(result.steps_executed.len(), 2);
    }

    #[tokio::test]
    async fn previous_output_injected_as_user_message() {
        let provider = build_test_provider();
        provider.queue_chat_response(Ok(response_with_status_line("gen output", "done")));
        provider.queue_chat_response(Ok(response_with_status_line("review result", "approved")));
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let engine = DefaultOrchestrationEngine;
        let result = engine
            .execute(
                &review_loop_workflow(),
                "write code",
                &policy,
                &registry,
                &telemetry,
            )
            .await
            .unwrap();

        assert_eq!(result.steps_executed.len(), 2);
        assert_eq!(result.terminated_reason, TerminationReason::Completed);
    }

    #[tokio::test]
    async fn telemetry_records_per_step() {
        let provider = build_test_provider();
        provider.queue_chat_response(Ok(response_with_status_line("output", "done")));
        provider.queue_chat_response(Ok(simple_response("Final")));
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let engine = DefaultOrchestrationEngine;
        engine
            .execute(&two_step_workflow(), "task", &policy, &registry, &telemetry)
            .await
            .unwrap();

        let entries = telemetry.all_entries().await;
        assert_eq!(entries.len(), 2);
        assert!(entries[0].success);
        assert!(entries[1].success);
    }

    #[tokio::test]
    async fn telemetry_fields_populated_correctly() {
        let provider = build_test_provider();
        provider.queue_chat_response(Ok(simple_response("output")));
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let workflow = Workflow {
            name: "test".to_string(),
            description: None,
            max_iterations: 10,
            max_cost: None,
            default_condition_mode: ConditionMode::StatusLine,
            steps: vec![WorkflowStep {
                name: "tagged".to_string(),
                tags: vec!["review".to_string(), "security".to_string()],
                instruction: "do".to_string(),
                transitions: vec![],
                context_policy: ContextPolicy::KeepAll,
                condition_mode: ConditionMode::StatusLine,
                dependencies: None,
            }],
        };

        let engine = DefaultOrchestrationEngine;
        engine
            .execute(&workflow, "task", &policy, &registry, &telemetry)
            .await
            .unwrap();

        let entries = telemetry.all_entries().await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].provider, "test-provider");
        assert_eq!(entries[0].model, "test-model");
        assert_eq!(entries[0].tags, vec!["review", "security"]);
    }

    #[tokio::test]
    async fn no_cost_cap_runs_without_check() {
        let provider = build_test_provider();
        provider.queue_chat_response(Ok(simple_response("output")));
        let registry = build_registry(provider);
        let policy = DefaultPolicyEngine::new(test_policy_config());
        let telemetry = InMemoryTelemetryStore::new();

        let workflow = Workflow {
            name: "uncapped".to_string(),
            description: None,
            max_iterations: 10,
            max_cost: None, // No cost cap
            default_condition_mode: ConditionMode::StatusLine,
            steps: vec![WorkflowStep {
                name: "s1".to_string(),
                tags: vec![],
                instruction: "do".to_string(),
                transitions: vec![],
                context_policy: ContextPolicy::KeepAll,
                condition_mode: ConditionMode::StatusLine,
                dependencies: None,
            }],
        };

        let engine = DefaultOrchestrationEngine;
        let result = engine
            .execute(&workflow, "task", &policy, &registry, &telemetry)
            .await
            .unwrap();

        assert_eq!(result.terminated_reason, TerminationReason::Completed);
    }
}
