// Scaffold: fixture data. Agent B replaces the bodies; signatures are the contract with api.ts.

use tauri::State;

use crate::error::Result;
use crate::state::AppState;
use crate::types::*;

fn fixture() -> Account {
    Account {
        id: "acc-1".into(),
        provider: Provider::GitlabSelfHosted,
        base_url: "https://git.example.com".into(),
        username: "oauth2".into(),
        label: "Work GitLab".into(),
    }
}

#[tauri::command(async)]
pub fn list_accounts(_state: State<'_, AppState>) -> Result<Vec<Account>> {
    Ok(vec![fixture()])
}

#[tauri::command(async)]
pub fn save_account(_state: State<'_, AppState>, input: AccountInput) -> Result<Account> {
    Ok(Account {
        id: input.id.unwrap_or_else(|| "acc-2".into()),
        provider: input.provider,
        base_url: input.base_url,
        username: input.username,
        label: input.label,
    })
}

#[tauri::command(async)]
pub fn remove_account(_state: State<'_, AppState>, id: String) -> Result<()> {
    let _ = id;
    Ok(())
}

#[tauri::command(async)]
pub fn verify_account(_state: State<'_, AppState>, id: String) -> Result<VerifyResult> {
    let _ = id;
    Ok(VerifyResult::Failed {
        status: Some(403),
        error: "Forbidden".into(),
    })
}

#[tauri::command(async)]
pub fn repo_accounts(_state: State<'_, AppState>, repo: String) -> Result<Vec<HostAccount>> {
    let _ = repo;
    Ok(vec![HostAccount {
        host: "git.example.com".into(),
        remotes: vec!["origin".into()],
        account_id: Some(fixture().id),
        ambiguous: false,
    }])
}
