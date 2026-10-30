// Names and signatures are the contract with api.ts.

use std::path::Path;

use tauri::State;

use crate::error::Result;
use crate::forge;
use crate::state::AppState;
use crate::types::*;

#[tauri::command(async)]
pub fn forge_hosts(state: State<'_, AppState>, repo: String) -> Result<ForgeHosts> {
    forge::forge_hosts(&state, Path::new(&repo))
}

#[tauri::command(async)]
pub fn commit_url(state: State<'_, AppState>, repo: String, sha: String) -> Result<Option<String>> {
    forge::commit_url(&state, Path::new(&repo), &sha)
}

#[tauri::command(async)]
pub fn list_mrs(
    state: State<'_, AppState>,
    repo: String,
    filter: MrFilter,
) -> Result<Vec<MergeRequest>> {
    forge::list_mrs(&state, Path::new(&repo), filter)
}

#[tauri::command(async)]
pub fn current_mr(state: State<'_, AppState>, repo: String) -> Result<Option<MergeRequest>> {
    forge::current_mr(&state, Path::new(&repo))
}

#[tauri::command(async)]
pub fn get_mr(state: State<'_, AppState>, repo: String, number: u64) -> Result<MergeRequest> {
    forge::get_mr(&state, Path::new(&repo), number)
}

#[tauri::command(async)]
pub fn mr_threads(state: State<'_, AppState>, repo: String, number: u64) -> Result<Vec<Thread>> {
    forge::mr_threads(&state, Path::new(&repo), number)
}

#[tauri::command(async)]
pub fn mr_action(
    state: State<'_, AppState>,
    repo: String,
    number: u64,
    action: MrAction,
) -> Result<()> {
    forge::mr_action(&state, Path::new(&repo), number, &action)
}

#[tauri::command(async)]
pub fn new_mr(state: State<'_, AppState>, repo: String) -> Result<NewMr> {
    forge::new_mr(&state, Path::new(&repo))
}

#[tauri::command(async)]
pub fn create_mr(
    state: State<'_, AppState>,
    repo: String,
    input: CreateMr,
) -> Result<MergeRequest> {
    forge::create_mr(&state, Path::new(&repo), &input)
}

#[tauri::command(async)]
pub fn list_pipelines(
    state: State<'_, AppState>,
    repo: String,
    all: bool,
) -> Result<Vec<Pipeline>> {
    forge::list_pipelines(&state, Path::new(&repo), all)
}

#[tauri::command(async)]
pub fn mr_pipelines(
    state: State<'_, AppState>,
    repo: String,
    number: u64,
) -> Result<Vec<Pipeline>> {
    forge::mr_pipelines(&state, Path::new(&repo), number)
}

#[tauri::command(async)]
pub fn pipeline_jobs(
    state: State<'_, AppState>,
    repo: String,
    project: Project,
    id: u64,
) -> Result<Vec<Stage>> {
    forge::pipeline_jobs(&state, Path::new(&repo), &project, id)
}

#[tauri::command(async)]
pub fn pipeline_action(
    state: State<'_, AppState>,
    repo: String,
    project: Project,
    id: u64,
    action: PipelineAction,
) -> Result<()> {
    forge::pipeline_action(&state, Path::new(&repo), &project, id, action)
}

#[tauri::command(async)]
pub fn job_action(
    state: State<'_, AppState>,
    repo: String,
    project: Project,
    id: u64,
    action: JobAction,
) -> Result<()> {
    forge::job_action(&state, Path::new(&repo), &project, id, action)
}

#[tauri::command(async)]
pub fn job_log(
    state: State<'_, AppState>,
    repo: String,
    project: Project,
    id: u64,
    offset: u64,
) -> Result<JobLogChunk> {
    forge::job_log(&state, Path::new(&repo), &project, id, offset)
}

#[tauri::command(async)]
pub fn workflows(
    state: State<'_, AppState>,
    repo: String,
    project: Project,
) -> Result<Vec<Workflow>> {
    forge::workflows(&state, Path::new(&repo), &project)
}

#[tauri::command(async)]
pub fn run_pipeline(
    state: State<'_, AppState>,
    repo: String,
    input: RunPipeline,
) -> Result<Pipeline> {
    forge::run_pipeline(&state, Path::new(&repo), &input)
}
