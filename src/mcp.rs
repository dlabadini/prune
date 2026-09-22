//! MCP server so an agent can ask Prune for findings, plans, and recent git edits.

use crate::plan::RefactorPlan;
use crate::Analyzer;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{tool, tool_router, ServiceExt};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct PruneServer {
    root: PathBuf,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ScanArgs {
    /// Project directory. Defaults to the directory the server was started in.
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PlanArgs {
    /// Finding id, or a unique prefix of one.
    finding_id: String,
    /// Project directory. Defaults to the directory the server was started in.
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct VerifyArgs {
    /// Finding id, or a unique prefix of one.
    finding_id: String,
    /// Project directory. Defaults to the directory the server was started in.
    #[serde(default)]
    path: Option<String>,
    /// Analyze indexed data only (skip file walk).
    #[serde(default)]
    quick: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ComposeArgs {
    /// Finding id, or a unique prefix of one.
    finding_id: String,
    /// Project directory. Defaults to the directory the server was started in.
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RecentArgs {
    /// Project directory. Defaults to the directory the server was started in.
    #[serde(default)]
    path: Option<String>,
    /// How many commits to walk. Defaults to 30.
    #[serde(default)]
    commits: Option<usize>,
}

#[tool_router(server_handler)]
impl PruneServer {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    #[tool(description = "Scan a project for structural anti-patterns and return JSON findings")]
    fn scan(
        &self,
        Parameters(args): Parameters<ScanArgs>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        let root = self.resolve(args.path.as_deref());
        let mut analyzer = Analyzer::new(&root).map_err(tool_error)?;
        let result = analyzer.scan().map_err(tool_error)?;
        let payload = json!({
            "root": root,
            "version": result.score.version,
            "score": result.score,
            "files_scanned": result.files_scanned,
            "symbols_found": result.symbols_found,
            "findings": result.findings,
        });
        Ok(rmcp::model::CallToolResult::success(vec![
            rmcp::model::ContentBlock::json(payload).map_err(tool_error)?,
        ]))
    }

    #[tool(description = "Verify a finding is resolved after refactoring")]
    fn verify(
        &self,
        Parameters(args): Parameters<VerifyArgs>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        let root = self.resolve(args.path.as_deref());
        let mut analyzer = Analyzer::new(&root).map_err(tool_error)?;
        let quick = args.quick.unwrap_or(false);
        let result = crate::verify_finding(&mut analyzer, &args.finding_id, quick)
            .map_err(tool_error)?;
        Ok(rmcp::model::CallToolResult::success(vec![
            rmcp::model::ContentBlock::json(json!(result)).map_err(tool_error)?,
        ]))
    }

    #[tool(description = "Return structured compose/rename instructions for one finding")]
    fn compose_plan(
        &self,
        Parameters(args): Parameters<ComposeArgs>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        let root = self.resolve(args.path.as_deref());
        let mut analyzer = Analyzer::new(&root).map_err(tool_error)?;
        let result = analyzer.analyze().map_err(tool_error)?;
        let finding = result
            .findings
            .iter()
            .find(|finding| {
                finding.id == args.finding_id || finding.id.starts_with(&args.finding_id)
            })
            .ok_or_else(|| tool_error(format!("finding {} was not found", args.finding_id)))?;
        let payload = json!({
            "finding_id": finding.id,
            "action_summary": finding.action_summary,
            "prune_action": finding.prune_action,
            "related_concepts": finding.related_concepts,
        });
        Ok(rmcp::model::CallToolResult::success(vec![
            rmcp::model::ContentBlock::json(payload).map_err(tool_error)?,
        ]))
    }

    #[tool(description = "Generate a markdown refactor plan for one finding")]
    fn plan(
        &self,
        Parameters(args): Parameters<PlanArgs>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        let root = self.resolve(args.path.as_deref());
        let mut analyzer = Analyzer::new(&root).map_err(tool_error)?;
        let result = analyzer.analyze().map_err(tool_error)?;
        let finding = result
            .findings
            .iter()
            .find(|finding| {
                finding.id == args.finding_id || finding.id.starts_with(&args.finding_id)
            })
            .ok_or_else(|| tool_error(format!("finding {} was not found", args.finding_id)))?;
        let plan = RefactorPlan::generate(finding);
        Ok(rmcp::model::CallToolResult::success(vec![
            rmcp::model::ContentBlock::text(plan.as_markdown()),
        ]))
    }

    #[tool(description = "List files touched in recent git history")]
    fn recent_files(
        &self,
        Parameters(args): Parameters<RecentArgs>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        let root = self.resolve(args.path.as_deref());
        let commits = args.commits.unwrap_or(30);
        let mut paths: Vec<_> = crate::gitinfo::recent_paths(&root, commits)
            .into_iter()
            .collect();
        paths.sort();
        let payload = json!({ "root": root, "commits": commits, "files": paths });
        Ok(rmcp::model::CallToolResult::success(vec![
            rmcp::model::ContentBlock::json(payload).map_err(tool_error)?,
        ]))
    }

    fn resolve(&self, path: Option<&str>) -> PathBuf {
        path.map(PathBuf::from).unwrap_or_else(|| self.root.clone())
    }
}

fn tool_error(error: impl std::fmt::Display) -> rmcp::ErrorData {
    rmcp::ErrorData::internal_error(error.to_string(), None)
}

/// Serve Prune tools over MCP stdio until the client disconnects.
pub async fn serve(root: &Path) -> anyhow::Result<()> {
    let server = PruneServer::new(root.to_path_buf());
    let running = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|error| anyhow::anyhow!(error))?;
    running
        .waiting()
        .await
        .map_err(|error| anyhow::anyhow!(error))?;
    Ok(())
}
