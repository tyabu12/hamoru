//! hamoru CLI — LLM Orchestration Infrastructure as Code.

use clap::{Parser, Subcommand};

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
    /// List all configured providers.
    List,
    /// Test connectivity to all providers.
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
    let cli = Cli::parse();

    match cli.command {
        Commands::Init => eprintln!("Not yet implemented: init"),
        Commands::Plan => eprintln!("Not yet implemented: plan"),
        Commands::Status => eprintln!("Not yet implemented: status"),
        Commands::Run(_args) => eprintln!("Not yet implemented: run"),
        Commands::Serve(_args) => eprintln!("Not yet implemented: serve"),
        Commands::Providers(cmd) => match cmd {
            ProvidersCommands::List => eprintln!("Not yet implemented: providers list"),
            ProvidersCommands::Test => eprintln!("Not yet implemented: providers test"),
        },
        Commands::Agents(cmd) => match cmd {
            AgentsCommands::List => eprintln!("Not yet implemented: agents list"),
            AgentsCommands::Test { .. } => eprintln!("Not yet implemented: agents test"),
        },
        Commands::Metrics(_args) => eprintln!("Not yet implemented: metrics"),
        Commands::Telemetry(cmd) => match cmd {
            TelemetryCommands::Show => eprintln!("Not yet implemented: telemetry show"),
            TelemetryCommands::Pull => eprintln!("Not yet implemented: telemetry pull"),
            TelemetryCommands::Push => eprintln!("Not yet implemented: telemetry push"),
        },
    }
}
