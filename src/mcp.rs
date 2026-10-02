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

#[derive(Debug, Deserialize, JsonSchema)]
struct SymbolArgs {
    /// File that declares the export, relative to the project root.
    file: String,
    /// Exported name (`default` for a default export).
    symbol: String,
    /// Project directory. Defaults to the directory the server was started in.
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RenameArgs {
    /// File that declares the export, relative to the project root.
    file: String,
    /// Current exported name.
    symbol: String,
    /// New name.
    to: String,
    /// Project directory. Defaults to the directory the server was started in.
    #[serde(default)]
    path: Option<String>,
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

    #[tool(
        description = "Before editing: who uses export X of a file? Returns its usage class (used, test_only, same_file_only, unused, unreachable), every importing/referencing file (through barrels, namespaces, Convex api/internal chains), and barrels that re-export it"
    )]
    fn who_uses(
        &self,
        Parameters(args): Parameters<SymbolArgs>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        let report = self.usage_report(args.path.as_deref())?;
        let usage = report
            .symbol_usage(&args.file, &args.symbol)
            .ok_or_else(|| tool_error(format!("{} does not export {}", args.file, args.symbol)))?;
        Ok(rmcp::model::CallToolResult::success(vec![
            rmcp::model::ContentBlock::json(json!(usage)).map_err(tool_error)?,
        ]))
    }

    #[tool(
        description = "Before deleting: is it safe to delete (or un-export) export X of a file? Returns safe_to_delete, safe_to_unexport, and the reasons (reachable users, tests, framework roots, re-exports)"
    )]
    fn safe_to_delete(
        &self,
        Parameters(args): Parameters<SymbolArgs>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        let report = self.usage_report(args.path.as_deref())?;
        let verdict = report
            .delete_verdict(&args.file, &args.symbol)
            .ok_or_else(|| tool_error(format!("{} does not export {}", args.file, args.symbol)))?;
        Ok(rmcp::model::CallToolResult::success(vec![
            rmcp::model::ContentBlock::json(json!(verdict)).map_err(tool_error)?,
        ]))
    }

    #[tool(
        description = "Before renaming: which files and how many edits does renaming export X touch (declaration, barrels, importers, namespace members)? Does not write anything; `prune rename --apply` performs it"
    )]
    fn rename_impact(
        &self,
        Parameters(args): Parameters<RenameArgs>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        let root = self.resolve(args.path.as_deref());
        let mut analyzer = Analyzer::new(&root).map_err(tool_error)?;
        analyzer.scan().map_err(tool_error)?;
        let plan = crate::fix::plan_rename(analyzer.database(), &root, &args.file, &args.symbol, &args.to)
            .map_err(tool_error)?;
        let files: Vec<_> = plan
            .edits
            .iter()
            .map(|(file, edits)| json!({ "file": file, "edits": edits.len() }))
            .collect();
        let payload = json!({
            "from": plan.from,
            "to": plan.to,
            "refused": plan.refused,
            "files": files,
        });
        Ok(rmcp::model::CallToolResult::success(vec![
            rmcp::model::ContentBlock::json(payload).map_err(tool_error)?,
        ]))
    }

    fn usage_report(
        &self,
        path: Option<&str>,
    ) -> Result<crate::graph::usage::UsageReport, rmcp::ErrorData> {
        let root = self.resolve(path);
        let mut analyzer = Analyzer::new(&root).map_err(tool_error)?;
        analyzer.scan().map_err(tool_error)?;
        crate::graph::usage::UsageReport::load(analyzer.database()).map_err(tool_error)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn who_uses_tool_reports_importers() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("package.json"), r#"{"name":"t","main":"main.ts"}"#).unwrap();
        std::fs::write(
            dir.path().join(".prunerc"),
            "[thresholds]\nembeddings_enabled = false\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("lib.ts"), "export function helper() { return 1; }\n").unwrap();
        std::fs::write(
            dir.path().join("main.ts"),
            "import { helper } from \"./lib\";\nconsole.log(helper());\n",
        )
        .unwrap();
        let server = PruneServer::new(dir.path().to_path_buf());
        let result = server
            .who_uses(Parameters(SymbolArgs {
                file: "lib.ts".into(),
                symbol: "helper".into(),
                path: None,
            }))
            .unwrap();
        let text = serde_json::to_string(&result).unwrap();
        assert!(text.contains("main.ts"), "{text}");
        assert!(text.contains("\\\"used\\\"") || text.contains("used"), "{text}");
    }
}
