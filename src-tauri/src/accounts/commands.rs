use std::path::Path;

use tauri::State;

use crate::accounts;
use crate::error::Result;
use crate::state::AppState;
use crate::types::*;

#[tauri::command(async)]
pub fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>> {
    accounts::load(&state.config_dir)
}

#[tauri::command(async)]
pub fn save_account(state: State<'_, AppState>, input: AccountInput) -> Result<Account> {
    accounts::save(&state, input)
}

#[tauri::command(async)]
pub fn remove_account(state: State<'_, AppState>, id: String) -> Result<()> {
    accounts::remove(&state, &id)
}

#[tauri::command(async)]
pub fn verify_account(state: State<'_, AppState>, id: String) -> Result<VerifyResult> {
    accounts::verify(&state, &id)
}

#[tauri::command(async)]
pub fn repo_accounts(state: State<'_, AppState>, repo: String) -> Result<Vec<HostAccount>> {
    accounts::repo_accounts(&state, Path::new(&repo))
}
