//! hamoru CLI — LLM Orchestration Infrastructure as Code.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use clap::{Parser, Subcommand};
use futures::StreamExt;
use hamoru_core::config::{self, HamoruConfig, ProviderType};
use hamoru_core::error::HamoruError;
use hamoru_core::provider::ProviderRegistry;
use hamoru_core::provider::factory::build_registry;
use hamoru_core::provider::types::*;
use hamoru_core::telemetry::sqlite::{SqliteTelemetryStore, migrate_from_json};
use hamoru_core::telemetry::{HistoryEntry, TelemetryStore};
use tracing_subscriber::EnvFilter;

/// hamoru: Terraform for LLMs.
///
/// Declaratively manage multiple LLM providers, automatically select optimal
/// models based on cost/quality/latency policies, and execute multi-step workflows.
#[derive(Parser)]
#[command(name = "hamoru", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new hamoru project (create .hamoru/).
    Init,

    /// Telemetry-based cost impact prediction.
    Plan,

    /// Show current configuration overview.
    Status,

    /// Execute a prompt, workflow, or collaboration.
    Run(RunArgs),

    /// Start the OpenAI-compatible API server.
    Serve(ServeArgs),

    /// Manage LLM providers.
    #[command(subcommand)]
    Providers(ProvidersCommands),

    /// Manage agent definitions and collaborations.
    #[command(subcommand)]
    Agents(AgentsCommands),

    /// View cost and performance metrics.
    Metrics(MetricsArgs),

    /// Manage telemetry data.
    #[command(subcommand)]
    Telemetry(TelemetryCommands),
}

#[derive(clap::Args)]
struct RunArgs {
    /// Workflow file to execute.
    #[arg(short, long)]
    workflow: Option<String>,

    /// Policy to apply.
    #[arg(short, long)]
    policy: Option<String>,

    /// Direct model specification (provider:model).
    #[arg(short, long)]
    model: Option<String>,

    /// Agent collaboration to execute.
    #[arg(short = 'a', long)]
    collaboration: Option<String>,

    /// Tags for policy-based routing.
    #[arg(short, long, value_delimiter = ',')]
    tags: Vec<String>,

    /// Disable streaming (print full response at once).
    #[arg(long, default_value_t = false)]
    no_stream: bool,

    /// The prompt or task description.
    prompt: String,
}

#[derive(clap::Args)]
struct ServeArgs {
    /// Port to listen on.
    #[arg(long, default_value = "3000")]
    port: u16,

    /// Address to bind to (default: localhost only).
    #[arg(long, default_value = "127.0.0.1")]
    bind: String,
}

#[derive(Subcommand)]
enum ProvidersCommands {
    /// List all configured providers and their models.
    List,
    /// Test connectivity to all configured providers.
    Test,
}

#[derive(Subcommand)]
enum AgentsCommands {
    /// List all defined agents.
    List,
    /// Dry-run a collaboration pattern.
    Test {
        /// Collaboration name to test.
        collaboration: String,
    },
}

#[derive(clap::Args)]
struct MetricsArgs {
    /// Time period for the report (e.g., "7d", "30d").
    #[arg(long, default_value = "7d")]
    period: String,
}

#[derive(Subcommand)]
enum TelemetryCommands {
    /// Show telemetry details.
    Show,
    /// Sync telemetry from remote.
    Pull,
    /// Sync telemetry to remote.
    Push,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init => init_project().await,
        Commands::Plan => plan_command().await,
        Commands::Status => {
            eprintln!("Not yet implemented: status");
            Ok(())
        }
        Commands::Run(args) => run_prompt(args).await,
        Commands::Serve(_args) => {
            eprintln!("Not yet implemented: serve");
            Ok(())
        }
        Commands::Providers(cmd) => match cmd {
            ProvidersCommands::List => providers_list().await,
            ProvidersCommands::Test => providers_test().await,
        },
        Commands::Agents(cmd) => match cmd {
            AgentsCommands::List => {
                eprintln!("Not yet implemented: agents list");
                Ok(())
            }
            AgentsCommands::Test { .. } => {
                eprintln!("Not yet implemented: agents test");
                Ok(())
            }
        },
        Commands::Metrics(args) => metrics_report(args).await,
        Commands::Telemetry(cmd) => match cmd {
            TelemetryCommands::Show => telemetry_show().await,
            TelemetryCommands::Pull => {
                eprintln!(
                    "Not yet implemented: telemetry pull (requires remote storage configuration)"
                );
                Ok(())
            }
            TelemetryCommands::Push => {
                eprintln!(
                    "Not yet implemented: telemetry push (requires remote storage configuration)"
                );
                Ok(())
            }
        },
    };

    if let Err(e) = result {
        eprintln!("Error: {}", format_cli_error(&e));
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Config helpers
// ---------------------------------------------------------------------------

/// Finds the hamoru config file, checking common locations.
fn find_config_path() -> Result<PathBuf, HamoruError> {
    for path in ["hamoru.yaml", ".hamoru/hamoru.yaml"] {
        let p = PathBuf::from(path);
        if p.exists() {
            return Ok(p);
        }
    }
    Err(HamoruError::ConfigError {
        reason: "No hamoru.yaml found. Run 'hamoru init' to create one.".to_string(),
    })
}

/// Searches for `hamoru.policy.yaml` in standard locations.
fn find_policy_config_path() -> Result<PathBuf, HamoruError> {
    for path in ["hamoru.policy.yaml", ".hamoru/hamoru.policy.yaml"] {
        let p = PathBuf::from(path);
        if p.exists() {
            return Ok(p);
        }
    }
    Err(HamoruError::ConfigError {
        reason: "No hamoru.policy.yaml found. Run 'hamoru init' to create one, \
                 or use -m provider:model for direct model selection."
            .to_string(),
    })
}

/// Loads config and builds a provider registry.
fn load_and_build() -> Result<(HamoruConfig, ProviderRegistry), HamoruError> {
    let path = find_config_path()?;
    let config = config::load_config(&path)?;
    let registry = build_registry(&config)?;
    Ok((config, registry))
}

/// Appends a CLI-layer remediation hint to an error message.
fn format_cli_error(e: &HamoruError) -> String {
    match e {
        HamoruError::CredentialNotFound { provider } => {
            let hint = match provider.as_str() {
                "anthropic" => {
                    "\n\nHint: Set HAMORU_ANTHROPIC_API_KEY environment variable.\n      Get your key at https://console.anthropic.com/"
                }
                _ => "",
            };
            format!("{e}{hint}")
        }
        HamoruError::ConfigError { .. } => {
            format!("{e}\n\nHint: Run 'hamoru init' to create a configuration file.")
        }
        _ => e.to_string(),
    }
}

// ---------------------------------------------------------------------------
// providers list / test
// ---------------------------------------------------------------------------

async fn providers_list() -> Result<(), HamoruError> {
    let (_config, registry) = load_and_build()?;

    for provider in registry.iter() {
        let models = provider.list_models().await?;
        println!("Provider: {}", provider.id());
        if models.is_empty() {
            println!("  (no models)");
        }
        for model in &models {
            let caps: Vec<&str> = model
                .capabilities
                .iter()
                .map(|c| match c {
                    Capability::Chat => "chat",
                    Capability::Vision => "vision",
                    Capability::FunctionCalling => "function_calling",
                    Capability::Reasoning => "reasoning",
                    Capability::PromptCaching => "prompt_caching",
                })
                .collect();
            println!(
                "  {} (context: {}k, in: ${:.6}/tok, out: ${:.6}/tok) [{}]",
                model.id,
                model.context_window / 1000,
                model.cost_per_input_token,
                model.cost_per_output_token,
                caps.join(", ")
            );
        }
        println!();
    }
    Ok(())
}

async fn providers_test() -> Result<(), HamoruError> {
    let (config, registry) = load_and_build()?;

    for pc in &config.providers {
        let provider = match registry.get(&pc.name) {
            Some(p) => p,
            None => {
                println!("  \u{2717} {}: not found in registry", pc.name);
                continue;
            }
        };

        let start = Instant::now();
        let result = match pc.provider_type {
            ProviderType::Ollama => provider.list_models().await.map(|_| ()),
            ProviderType::Anthropic => {
                // Lightweight check: send minimal request
                let models = provider.list_models().await?;
                let model = models.first().map(|m| m.id.clone()).unwrap_or_default();
                provider
                    .chat(ChatRequest {
                        model,
                        messages: vec![Message {
                            role: Role::User,
                            content: MessageContent::Text("Hi".to_string()),
                        }],
                        temperature: None,
                        max_tokens: Some(1),
                        tools: None,
                        stream: false,
                    })
                    .await
                    .map(|_| ())
            }
        };
        let elapsed = start.elapsed().as_millis();

        match result {
            Ok(()) => println!("  \u{2713} {}: healthy ({elapsed}ms)", pc.name),
            Err(e) => println!("  \u{2717} {}: {}", pc.name, format_cli_error(&e)),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// run
// ---------------------------------------------------------------------------

async fn run_prompt(args: RunArgs) -> Result<(), HamoruError> {
    let (_config, registry) = load_and_build()?;

    // Resolve provider/model: direct (-m) or policy-based (-p / --tags)
    let (provider_id, model, tags) = if let Some(ref model_spec) = args.model {
        // Direct model mode (existing behavior)
        let (pid, m) = parse_model_spec(Some(model_spec))?;
        (pid, m, args.tags.clone())
    } else if args.policy.is_some() || !args.tags.is_empty() {
        // Policy mode
        let policy_path = find_policy_config_path()?;
        let policy_config = hamoru_core::policy::config::load_policy_config(&policy_path)?;

        // Pre-fetch models from all providers
        let mut all_models = Vec::new();
        for provider in registry.iter() {
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

        // Build routing request
        let request = hamoru_core::policy::RoutingRequest {
            tags: args.tags.clone(),
            policy_name: args.policy.clone(),
            ..Default::default()
        };

        // Load metrics cache for scoring
        ensure_hamoru_dir().await?;
        let store = create_telemetry_store().await?;
        let period = Duration::from_secs(7 * 24 * 3600);
        let metrics = store.query_detailed_metrics(period).await?;

        let engine = hamoru_core::policy::DefaultPolicyEngine::new(policy_config);
        let selection = hamoru_core::policy::PolicyEngine::select_model(
            &engine,
            &request,
            &all_models,
            &metrics,
        )?;

        eprintln!(
            "Selected: {}:{} (reason: {}, est. ${:.4})",
            selection.provider,
            selection.model,
            selection.reason,
            selection.estimated_cost.unwrap_or(0.0)
        );

        (selection.provider, selection.model, args.tags.clone())
    } else {
        return Err(HamoruError::ConfigError {
            reason: "Specify -m provider:model, -p policy, or --tags tag1,tag2. \
                     Run 'hamoru init' to create default configs."
                .to_string(),
        });
    };

    let provider = registry
        .get(&provider_id)
        .ok_or_else(|| HamoruError::ConfigError {
            reason: format!("Provider '{provider_id}' not found in config. Check hamoru.yaml."),
        })?;

    let request = ChatRequest {
        model: model.clone(),
        messages: vec![Message {
            role: Role::User,
            content: MessageContent::Text(args.prompt),
        }],
        temperature: None,
        max_tokens: None,
        tools: None,
        stream: !args.no_stream,
    };

    let (usage, latency_ms) = if args.no_stream {
        let resp = provider.chat(request).await?;
        println!("{}", resp.content);
        (resp.usage, resp.latency_ms)
    } else {
        let mut stream = provider.chat_stream(request).await?;
        let mut usage = None;
        let start = Instant::now();
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result?;
            if !chunk.delta.is_empty() {
                print!("{}", chunk.delta);
                let _ = std::io::stdout().flush();
            }
            if chunk.usage.is_some() {
                usage = chunk.usage;
            }
        }
        println!();
        let latency = start.elapsed().as_millis() as u64;
        (usage.unwrap_or_default(), latency)
    };

    // Ensure .hamoru/ directory exists
    ensure_hamoru_dir().await?;

    // Telemetry recording
    let telemetry = create_telemetry_store().await?;
    let model_info = provider.model_info(&model).await.ok();
    let cost = model_info
        .map(|mi| usage.calculate_cost(&mi))
        .unwrap_or(0.0);
    let entry = HistoryEntry {
        timestamp: Utc::now(),
        provider: provider_id,
        model,
        tokens: usage.clone(),
        cost,
        latency_ms,
        success: true,
        tags,
    };
    telemetry.record(&entry).await?;

    // Stats to stderr
    eprintln!(
        "\n---\nTokens: {} in / {} out | Cost: ${:.6} | Latency: {}ms",
        usage.input_tokens, usage.output_tokens, cost, latency_ms
    );
    Ok(())
}

/// Creates a SQLite telemetry store, migrating from JSON if needed.
///
/// If `.hamoru/state.json` exists and `.hamoru/state.db` does not,
/// automatically migrates data and renames the JSON file.
/// Uses the config's `telemetry.local.path` if a config file is available,
/// otherwise defaults to `.hamoru/state.db`.
async fn create_telemetry_store() -> Result<SqliteTelemetryStore, HamoruError> {
    let db_path = find_config_path()
        .ok()
        .and_then(|p| config::load_config(&p).ok())
        .map(|c| PathBuf::from(c.telemetry_local_path()))
        .unwrap_or_else(|| PathBuf::from(".hamoru/state.db"));
    let json_path = PathBuf::from(".hamoru/state.json");

    let store = SqliteTelemetryStore::new(&db_path).await?;

    // Auto-migrate from Phase 1 JSON format if needed
    if json_path.exists() {
        let result = migrate_from_json(&json_path, &store).await?;
        if result.entries_migrated > 0 {
            tracing::info!(
                migrated = result.entries_migrated,
                skipped = result.entries_skipped,
                "Migrated telemetry data from state.json to state.db"
            );
        }
        // Rename to prevent re-migration
        let migrated_path = json_path.with_extension("json.migrated");
        if let Err(e) = tokio::fs::rename(&json_path, &migrated_path).await {
            tracing::warn!("Failed to rename state.json after migration: {e}");
        }
    }

    Ok(store)
}

/// Parses a human-readable duration string like "1d", "7d", "30d".
fn parse_period(s: &str) -> Result<Duration, HamoruError> {
    let s = s.trim();
    if let Some(days) = s.strip_suffix('d') {
        let n: u64 = days.parse().map_err(|_| HamoruError::ConfigError {
            reason: format!("Invalid period '{s}'. Expected format: Nd (e.g., 1d, 7d, 30d)"),
        })?;
        if n == 0 {
            return Err(HamoruError::ConfigError {
                reason: "Period must be at least 1 day.".to_string(),
            });
        }
        Ok(Duration::from_secs(n * 24 * 3600))
    } else if let Some(hours) = s.strip_suffix('h') {
        let n: u64 = hours.parse().map_err(|_| HamoruError::ConfigError {
            reason: format!("Invalid period '{s}'. Expected format: Nh (e.g., 1h, 24h)"),
        })?;
        if n == 0 {
            return Err(HamoruError::ConfigError {
                reason: "Period must be at least 1 hour.".to_string(),
            });
        }
        Ok(Duration::from_secs(n * 3600))
    } else {
        Err(HamoruError::ConfigError {
            reason: format!("Invalid period '{s}'. Expected format: Nd or Nh (e.g., 7d, 24h)"),
        })
    }
}

/// Formats a MetricsCache into a human-readable report string.
fn format_metrics_report(
    cache: &hamoru_core::telemetry::MetricsCache,
    period_label: &str,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("Metrics report ({period_label}):\n"));
    out.push_str(&format!(
        "  Total requests: {}\n",
        cache.total.total_requests
    ));
    out.push_str(&format!("  Total cost: ${:.2}\n", cache.total.total_cost));
    if cache.total.total_requests > 0 {
        out.push_str(&format!(
            "  Avg latency: {:.1}ms\n",
            cache.total.avg_latency_ms
        ));
    }

    if !cache.by_model.is_empty() {
        out.push_str("  Model breakdown:\n");
        // Sort by cost descending
        let mut models: Vec<_> = cache.by_model.iter().collect();
        models.sort_by(|a, b| {
            b.1.cost
                .partial_cmp(&a.1.cost)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for (model, metrics) in &models {
            out.push_str(&format!(
                "    {}: {} requests (${:.2})\n",
                model, metrics.requests, metrics.cost
            ));
        }
    }

    out
}

/// Runs the `hamoru telemetry show` command.
async fn telemetry_show() -> Result<(), HamoruError> {
    ensure_hamoru_dir().await?;
    let store = create_telemetry_store().await?;

    let count = store.entry_count().await?;
    let db_path = store.path().to_path_buf();

    println!("Telemetry store: {}", db_path.display());

    // File size
    if let Ok(metadata) = tokio::fs::metadata(&db_path).await {
        let size_kb = metadata.len() as f64 / 1024.0;
        if size_kb < 1024.0 {
            println!("  Size: {size_kb:.1} KB");
        } else {
            println!("  Size: {:.1} MB", size_kb / 1024.0);
        }
    }

    println!("  Total entries: {count}");

    if let Some((oldest, newest)) = store.date_range().await? {
        // Truncate to date portion for readability
        let oldest_date = oldest.split('T').next().unwrap_or(&oldest);
        let newest_date = newest.split('T').next().unwrap_or(&newest);
        println!("  Date range: {oldest_date} to {newest_date}");
    }

    // Show top models
    if count > 0 {
        let cache = store
            .query_detailed_metrics(Duration::from_secs(365 * 24 * 3600))
            .await?;
        if !cache.by_model.is_empty() {
            println!("  Top models:");
            let mut models: Vec<_> = cache.by_model.iter().collect();
            models.sort_by(|a, b| b.1.requests.cmp(&a.1.requests));
            for (model, metrics) in models.iter().take(5) {
                println!(
                    "    {}: {} requests (${:.2})",
                    model, metrics.requests, metrics.cost
                );
            }
        }
    }

    Ok(())
}

/// Runs the `hamoru plan` command.
///
/// Shows telemetry-based cost projection. Policy-aware cost impact
/// prediction is available after configuring policies (Phase 3).
async fn plan_command() -> Result<(), HamoruError> {
    ensure_hamoru_dir().await?;
    let store = create_telemetry_store().await?;
    let period = Duration::from_secs(7 * 24 * 3600);
    let cache = store.query_detailed_metrics(period).await?;
    let projection = hamoru_core::telemetry::projection::project_costs(&cache);

    if projection.daily_requests == 0.0 {
        println!("No telemetry data available. Run some prompts first with 'hamoru run'.");
        return Ok(());
    }

    println!("Telemetry summary (last {}d):", projection.data_period_days);
    println!(
        "  Daily avg: ${:.2}/day ({:.0} requests/day)",
        projection.daily_cost, projection.daily_requests
    );

    if !projection.top_models.is_empty() {
        println!("  Model breakdown:");
        for m in &projection.top_models {
            println!(
                "    {}: {:.0} req/day (${:.2}/day, {:.1}%)",
                m.model, m.daily_requests, m.daily_cost, m.pct_of_total
            );
        }
    }

    println!(
        "  Confidence: {:.0}% ({}d of data)",
        projection.confidence * 100.0,
        projection.data_period_days
    );
    println!();
    println!("Note: Policy-based cost prediction available after configuring policies.");

    Ok(())
}

/// Runs the `hamoru metrics` command.
async fn metrics_report(args: MetricsArgs) -> Result<(), HamoruError> {
    ensure_hamoru_dir().await?;
    let period = parse_period(&args.period)?;
    let store = create_telemetry_store().await?;
    let cache = store.query_detailed_metrics(period).await?;
    let report = format_metrics_report(&cache, &args.period);
    print!("{report}");
    Ok(())
}

fn parse_model_spec(spec: Option<&str>) -> Result<(String, String), HamoruError> {
    let spec = spec.ok_or_else(|| HamoruError::ConfigError {
        reason: "Missing -m flag. Usage: hamoru run -m provider:model 'prompt'".to_string(),
    })?;
    let (provider, model) = spec.split_once(':').ok_or_else(|| HamoruError::ConfigError {
        reason: format!(
            "Invalid model spec '{spec}'. Format: provider:model (e.g., claude:claude-sonnet-4-6)"
        ),
    })?;
    if provider.is_empty() || model.is_empty() {
        return Err(HamoruError::ConfigError {
            reason: format!(
                "Invalid model spec '{spec}'. Both provider and model must be non-empty."
            ),
        });
    }
    Ok((provider.to_string(), model.to_string()))
}

// ---------------------------------------------------------------------------
// init
// ---------------------------------------------------------------------------

/// Default hamoru.yaml template.
const INIT_TEMPLATE: &str = r#"version: "1"

providers:
  - name: claude
    type: anthropic
    models:
      - claude-sonnet-4-6
      - claude-haiku-4-5

  # Uncomment to add Ollama (local models):
  # - name: local
  #   type: ollama
  #   endpoint: http://localhost:11434
"#;

const POLICY_INIT_TEMPLATE: &str = r#"# hamoru.policy.yaml — Policy Engine configuration
#
# Policies define model selection strategies based on constraints and preferences.
# Routing rules map task tags to policies automatically.

policies:
  - name: cost-optimized
    description: Prefer the cheapest model that meets basic requirements
    constraints:
      max_cost_per_request: 0.01
    preferences:
      priority: cost

  - name: quality-first
    description: Prefer the highest-quality model available
    constraints:
      min_quality_tier: high
    preferences:
      priority: quality

routing_rules:
  - match:
      tags: [review, architecture, security]
    policy: quality-first
  - default:
      policy: cost-optimized

cost_limits:
  max_cost_per_day: 10.00
  alert_threshold: 0.8
"#;

async fn init_project() -> Result<(), HamoruError> {
    let config_path = Path::new(".hamoru/hamoru.yaml");
    if config_path.exists() {
        eprintln!("Configuration already exists at .hamoru/hamoru.yaml. Skipping.");
        return Ok(());
    }

    ensure_hamoru_dir().await?;
    tokio::fs::write(config_path, INIT_TEMPLATE)
        .await
        .map_err(|e| HamoruError::ConfigError {
            reason: format!("Failed to write config file: {e}"),
        })?;

    // Set file permissions on unix (mode 600 for config)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ =
            tokio::fs::set_permissions(config_path, std::fs::Permissions::from_mode(0o600)).await;
    }

    println!("Created .hamoru/hamoru.yaml");

    // Create policy config
    let policy_path = Path::new(".hamoru/hamoru.policy.yaml");
    if !policy_path.exists() {
        tokio::fs::write(policy_path, POLICY_INIT_TEMPLATE)
            .await
            .map_err(|e| HamoruError::ConfigError {
                reason: format!("Failed to write policy config file: {e}"),
            })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = tokio::fs::set_permissions(policy_path, std::fs::Permissions::from_mode(0o600))
                .await;
        }

        println!("Created .hamoru/hamoru.policy.yaml");
    }

    println!();
    println!("Next steps:");
    println!("  1. Set your API key: export HAMORU_ANTHROPIC_API_KEY=sk-...");
    println!("  2. Test connectivity: hamoru providers test");
    println!("  3. Run a prompt: hamoru run -m claude:claude-sonnet-4-6 'Hello'");
    println!("  4. Use policies: hamoru run -p cost-optimized 'Hello'");
    Ok(())
}

/// Ensures the .hamoru/ directory exists with appropriate permissions.
async fn ensure_hamoru_dir() -> Result<(), HamoruError> {
    tokio::fs::create_dir_all(".hamoru")
        .await
        .map_err(|e| HamoruError::ConfigError {
            reason: format!("Failed to create .hamoru/ directory: {e}"),
        })?;

    // Set directory permissions on unix (mode 700)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(".hamoru", std::fs::Permissions::from_mode(0o700)).await;
    }

    Ok(())
}
